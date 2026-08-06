import type { Note } from "../types";
import { parseWikiLinkAt } from "./wikiLinks";

export interface MarkdownRenderOptions {
  /** Used only to decorate resolvable/unresolvable wiki links. */
  resolveWikiLink?: (
    target: string,
    heading?: string,
  ) => Note | boolean | null | undefined;
  headingIdPrefix?: string;
  externalLinksInNewTab?: boolean;
}

interface RenderContext {
  options: MarkdownRenderOptions;
  depth: number;
}

interface ListItem {
  text: string;
  checked?: boolean;
}

interface ParsedMarkdownLink {
  html: string;
  end: number;
}

/**
 * Render the Markdown subset used by the app without permitting raw HTML,
 * scriptable URLs, or event-handler injection. The returned string is suitable
 * for Vue's `v-html`.
 */
export function renderMarkdown(
  markdown: string,
  options: MarkdownRenderOptions = {},
): string {
  return renderBlocks(markdown.replace(/\0/g, ""), { options, depth: 0 });
}

/** Escape arbitrary user text for insertion into HTML text or attributes. */
export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** Return a browser-safe Markdown link destination, or `undefined`. */
export function sanitizeLinkUrl(value: string): string | undefined {
  const url = value
    .trim()
    .replace(/^<|>$/g, "")
    .replace(/[\u0000-\u001f\u007f]/g, "");
  if (!url) return undefined;

  const compact = url.replace(/[\s\u00a0]+/g, "").toLocaleLowerCase();
  const scheme = compact.match(/^([a-z][a-z0-9+.-]*):/i)?.[1];
  if (scheme && !["http", "https", "mailto"].includes(scheme)) {
    return undefined;
  }

  // Backslashes can be interpreted as slashes by URL parsers, so reject them
  // instead of allowing a disguised scheme-relative destination.
  if (url.includes("\\")) return undefined;
  return url;
}

function renderBlocks(markdown: string, context: RenderContext): string {
  // Prevent pathological recursive blockquotes from consuming the call stack.
  if (context.depth > 16) return `<p>${renderInline(markdown, context)}</p>`;

  const lines = markdown.replace(/\r\n?/g, "\n").split("\n");
  const blocks: string[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index]!;
    if (!line.trim()) {
      index += 1;
      continue;
    }

    const fence = line.match(/^ {0,3}(`{3,}|~{3,})([^\n]*)$/);
    if (fence) {
      const marker = fence[1]![0]!;
      const markerLength = fence[1]!.length;
      const info = fence[2]!.trim().split(/\s+/, 1)[0] ?? "";
      const language = info.replace(/[^a-zA-Z0-9_+-]/g, "").slice(0, 40);
      const code: string[] = [];
      index += 1;

      while (index < lines.length) {
        const candidate = lines[index]!;
        const closing = candidate.match(/^ {0,3}(`+|~+)\s*$/);
        if (
          closing &&
          closing[1]![0] === marker &&
          closing[1]!.length >= markerLength
        ) {
          index += 1;
          break;
        }
        code.push(candidate);
        index += 1;
      }

      const className = language ? ` class="language-${escapeHtml(language)}"` : "";
      blocks.push(`<pre><code${className}>${escapeHtml(code.join("\n"))}</code></pre>`);
      continue;
    }

    const heading = line.match(/^ {0,3}(#{1,6})[\t ]+(.+?)\s*$/);
    if (heading) {
      const level = heading[1]!.length;
      const text = heading[2]!.replace(/[\t ]+#+[\t ]*$/, "");
      const prefix = context.options.headingIdPrefix ?? "";
      const id = `${prefix}${slugifyHeading(text)}`;
      blocks.push(
        `<h${level} id="${escapeHtml(id)}">${renderInline(text, context)}</h${level}>`,
      );
      index += 1;
      continue;
    }

    if (/^ {0,3}(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      blocks.push("<hr>");
      index += 1;
      continue;
    }

    if (/^ {0,3}>/.test(line)) {
      const quoted: string[] = [];
      while (index < lines.length) {
        const quoteLine = lines[index]!;
        const match = quoteLine.match(/^ {0,3}> ?(.*)$/);
        if (match) {
          quoted.push(match[1]!);
          index += 1;
          continue;
        }
        if (!quoteLine.trim() && /^ {0,3}>/.test(lines[index + 1] ?? "")) {
          quoted.push("");
          index += 1;
          continue;
        }
        break;
      }
      blocks.push(
        `<blockquote>${renderBlocks(quoted.join("\n"), {
          ...context,
          depth: context.depth + 1,
        })}</blockquote>`,
      );
      continue;
    }

    const listMatch = matchListItem(line);
    if (listMatch) {
      const ordered = listMatch.ordered;
      const start = listMatch.start;
      const items: ListItem[] = [];

      while (index < lines.length) {
        const item = matchListItem(lines[index]!);
        if (!item || item.ordered !== ordered) break;
        index += 1;

        const task = item.text.match(/^\[([ xX])\][\t ]+(.*)$/);
        let text = task ? task[2]! : item.text;
        const continuations: string[] = [];
        while (
          index < lines.length &&
          /^ {2,}\S/.test(lines[index]!) &&
          !matchListItem(lines[index]!)
        ) {
          continuations.push(lines[index]!.trim());
          index += 1;
        }
        if (continuations.length) text += `\n${continuations.join("\n")}`;
        items.push({
          text,
          ...(task ? { checked: task[1]!.toLocaleLowerCase() === "x" } : {}),
        });
      }

      const hasTasks = items.some((item) => item.checked !== undefined);
      const tag = ordered ? "ol" : "ul";
      const startAttribute = ordered && start !== 1 ? ` start="${start}"` : "";
      const className = hasTasks ? ' class="task-list"' : "";
      const itemHtml = items.map((item) => {
        const taskClass = item.checked === undefined ? "" : ' class="task-list-item"';
        const checkbox = item.checked === undefined
          ? ""
          : `<input type="checkbox" disabled${item.checked ? " checked" : ""} aria-label="${
            item.checked ? "Completed task" : "Incomplete task"
          }"> `;
        return `<li${taskClass}>${checkbox}${renderInline(item.text, context)}</li>`;
      }).join("");
      blocks.push(`<${tag}${className}${startAttribute}>${itemHtml}</${tag}>`);
      continue;
    }

    if (isTable(lines, index)) {
      const header = splitTableRow(lines[index]!);
      const separators = splitTableRow(lines[index + 1]!);
      const alignments = separators.map(tableAlignment);
      index += 2;
      const rows: string[][] = [];
      while (index < lines.length && looksLikeTableRow(lines[index]!)) {
        rows.push(splitTableRow(lines[index]!));
        index += 1;
      }

      const headerHtml = header.map((cell, cellIndex) =>
        `<th${alignmentClass(alignments[cellIndex])}>${renderInline(cell.trim(), context)}</th>`,
      ).join("");
      const bodyHtml = rows.map((row) => {
        const cells = header.map((_, cellIndex) =>
          `<td${alignmentClass(alignments[cellIndex])}>${renderInline(
            (row[cellIndex] ?? "").trim(),
            context,
          )}</td>`,
        ).join("");
        return `<tr>${cells}</tr>`;
      }).join("");

      blocks.push(
        `<div class="table-wrap"><table><thead><tr>${headerHtml}</tr></thead>${
          bodyHtml ? `<tbody>${bodyHtml}</tbody>` : ""
        }</table></div>`,
      );
      continue;
    }

    const paragraph: string[] = [line];
    index += 1;
    while (
      index < lines.length &&
      lines[index]!.trim() &&
      !isBlockStart(lines, index)
    ) {
      paragraph.push(lines[index]!);
      index += 1;
    }
    blocks.push(`<p>${renderInline(paragraph.join("\n"), context)}</p>`);
  }

  return blocks.join("\n");
}

function renderInline(source: string, context: RenderContext, depth = 0): string {
  if (depth > 12) return escapeHtml(source);

  let html = "";
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index]!;

    if (character === "\\" && /[!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~]/.test(source[index + 1] ?? "")) {
      html += escapeHtml(source[index + 1]!);
      index += 1;
      continue;
    }

    if (character === "`") {
      let delimiterLength = 1;
      while (source[index + delimiterLength] === "`") delimiterLength += 1;
      const delimiter = "`".repeat(delimiterLength);
      const close = source.indexOf(delimiter, index + delimiterLength);
      if (close >= 0) {
        let code = source.slice(index + delimiterLength, close).replace(/\n/g, " ");
        if (/^\s.*\s$/.test(code) && code.trim()) code = code.slice(1, -1);
        html += `<code>${escapeHtml(code)}</code>`;
        index = close + delimiterLength - 1;
        continue;
      }
    }

    if (character === "!" || character === "[") {
      const wikiLink = parseWikiLinkAt(source, index);
      if (wikiLink) {
        const resolution = context.options.resolveWikiLink?.(
          wikiLink.target,
          wikiLink.heading,
        );
        const resolutionClass = context.options.resolveWikiLink
          ? resolution ? " is-resolved" : " is-unresolved"
          : "";
        const headingAttribute = wikiLink.heading
          ? ` data-wiki-heading="${escapeHtml(wikiLink.heading)}"`
          : "";
        const embedAttribute = wikiLink.embedded ? ' data-wiki-embed="true"' : "";
        const label = wikiLink.display || wikiLink.heading || wikiLink.target;
        html += `<a href="#" class="wiki-link${resolutionClass}" data-wiki-target="${escapeHtml(
          wikiLink.target,
        )}"${headingAttribute}${embedAttribute}>${escapeHtml(label)}</a>`;
        index += wikiLink.raw.length - 1;
        continue;
      }

      if (character === "[") {
        const markdownLink = parseMarkdownLink(source, index, context, depth);
        if (markdownLink) {
          html += markdownLink.html;
          index = markdownLink.end;
          continue;
        }
      }
    }

    const strongDelimiter = source.startsWith("**", index)
      ? "**"
      : source.startsWith("__", index) ? "__" : undefined;
    if (strongDelimiter) {
      const close = findClosingDelimiter(source, strongDelimiter, index + 2);
      if (close > index + 2) {
        const inner = source.slice(index + 2, close);
        html += `<strong>${renderInline(inner, context, depth + 1)}</strong>`;
        index = close + 1;
        continue;
      }
    }

    if (source.startsWith("~~", index)) {
      const close = findClosingDelimiter(source, "~~", index + 2);
      if (close > index + 2) {
        html += `<del>${renderInline(source.slice(index + 2, close), context, depth + 1)}</del>`;
        index = close + 1;
        continue;
      }
    }

    if (character === "*" || character === "_") {
      const isIntrawordUnderscore = character === "_" &&
        /[\p{L}\p{N}]/u.test(source[index - 1] ?? "") &&
        /[\p{L}\p{N}]/u.test(source[index + 1] ?? "");
      const close = isIntrawordUnderscore
        ? -1
        : findClosingDelimiter(source, character, index + 1);
      if (close > index + 1) {
        html += `<em>${renderInline(source.slice(index + 1, close), context, depth + 1)}</em>`;
        index = close;
        continue;
      }
    }

    if (character === "\n") {
      const hardBreak = source[index - 1] === "\\" || source.slice(Math.max(0, index - 2), index) === "  ";
      html += hardBreak ? "<br>" : " ";
      continue;
    }

    html += escapeHtml(character);
  }

  return html;
}

function parseMarkdownLink(
  source: string,
  start: number,
  context: RenderContext,
  depth: number,
): ParsedMarkdownLink | undefined {
  const labelEnd = findUnescaped(source, "]", start + 1);
  if (labelEnd < 0 || source[labelEnd + 1] !== "(") return undefined;

  let parentheses = 1;
  let destinationEnd = -1;
  for (let index = labelEnd + 2; index < source.length; index += 1) {
    if (source[index] === "\\") {
      index += 1;
      continue;
    }
    if (source[index] === "(") parentheses += 1;
    if (source[index] === ")") {
      parentheses -= 1;
      if (parentheses === 0) {
        destinationEnd = index;
        break;
      }
    }
  }
  if (destinationEnd < 0) return undefined;

  const label = source.slice(start + 1, labelEnd);
  const rawDestination = source.slice(labelEnd + 2, destinationEnd).trim();
  const destinationMatch = rawDestination.match(
    /^(<[^>]+>|\S+?)(?:\s+(?:"([^"]*)"|'([^']*)'|\(([^)]*)\)))?$/,
  );
  if (!destinationMatch) return undefined;

  const destination = destinationMatch[1]!.replace(/^<|>$/g, "");
  const title = destinationMatch[2] ?? destinationMatch[3] ?? destinationMatch[4];
  const safeUrl = sanitizeLinkUrl(destination);
  const labelHtml = renderInline(label, context, depth + 1);
  if (!safeUrl) {
    return {
      html: `<span class="unsafe-link">${labelHtml}</span>`,
      end: destinationEnd,
    };
  }

  const isExternal = /^https?:/i.test(safeUrl);
  const targetAttributes = isExternal && context.options.externalLinksInNewTab !== false
    ? ' target="_blank" rel="noopener noreferrer"'
    : "";
  const titleAttribute = title ? ` title="${escapeHtml(title)}"` : "";
  return {
    html: `<a href="${escapeHtml(safeUrl)}"${titleAttribute}${targetAttributes}>${labelHtml}</a>`,
    end: destinationEnd,
  };
}

function findClosingDelimiter(source: string, delimiter: string, start: number): number {
  let cursor = start;
  while (cursor < source.length) {
    const index = source.indexOf(delimiter, cursor);
    if (index < 0) return -1;
    if (source[index - 1] !== "\\") return index;
    cursor = index + delimiter.length;
  }
  return -1;
}

function findUnescaped(source: string, needle: string, start: number): number {
  for (let index = start; index < source.length; index += 1) {
    if (source[index] === "\\") {
      index += 1;
      continue;
    }
    if (source[index] === needle) return index;
  }
  return -1;
}

function matchListItem(line: string): {
  ordered: boolean;
  start: number;
  text: string;
} | undefined {
  const unordered = line.match(/^ {0,3}[-+*][\t ]+(.+)$/);
  if (unordered) return { ordered: false, start: 1, text: unordered[1]! };
  const ordered = line.match(/^ {0,3}(\d{1,9})[.)][\t ]+(.+)$/);
  if (!ordered) return undefined;
  return {
    ordered: true,
    start: Math.max(1, Number.parseInt(ordered[1]!, 10)),
    text: ordered[2]!,
  };
}

function isBlockStart(lines: readonly string[], index: number): boolean {
  const line = lines[index] ?? "";
  return /^ {0,3}(?:`{3,}|~{3,}|#{1,6}[\t ]|>|(?:[-+*]|\d+[.)])[\t ])/.test(line) ||
    /^ {0,3}(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line) ||
    isTable(lines, index);
}

function looksLikeTableRow(line: string): boolean {
  if (!line.includes("|")) return false;
  return splitTableRow(line).length > 1;
}

function isTable(lines: readonly string[], index: number): boolean {
  const header = lines[index] ?? "";
  const delimiter = lines[index + 1] ?? "";
  if (!looksLikeTableRow(header) || !delimiter.includes("|")) return false;
  const cells = splitTableRow(delimiter);
  return cells.length > 0 && cells.every((cell) => /^\s*:?-{3,}:?\s*$/.test(cell));
}

function splitTableRow(line: string): string[] {
  let value = line.trim();
  if (value.startsWith("|")) value = value.slice(1);
  if (value.endsWith("|") && value[value.length - 2] !== "\\") value = value.slice(0, -1);

  const cells: string[] = [];
  let cell = "";
  let codeDelimiter = 0;
  for (let index = 0; index < value.length; index += 1) {
    const character = value[index]!;
    if (character === "\\" && value[index + 1] === "|") {
      cell += "|";
      index += 1;
      continue;
    }
    if (character === "`") {
      let run = 1;
      while (value[index + run] === "`") run += 1;
      if (codeDelimiter === 0) codeDelimiter = run;
      else if (codeDelimiter === run) codeDelimiter = 0;
      cell += "`".repeat(run);
      index += run - 1;
      continue;
    }
    if (character === "|" && codeDelimiter === 0) {
      cells.push(cell);
      cell = "";
      continue;
    }
    cell += character;
  }
  cells.push(cell);
  return cells;
}

function tableAlignment(value: string): "left" | "center" | "right" | undefined {
  const trimmed = value.trim();
  if (trimmed.startsWith(":") && trimmed.endsWith(":")) return "center";
  if (trimmed.endsWith(":")) return "right";
  if (trimmed.startsWith(":")) return "left";
  return undefined;
}

function alignmentClass(value: ReturnType<typeof tableAlignment>): string {
  return value ? ` class="align-${value}"` : "";
}

function slugifyHeading(value: string): string {
  const slug = value
    .replace(/!?(?:\[\[)([^\]|#]+)(?:#[^\]|]+)?(?:\|([^\]]+))?\]\]/g, "$2$1")
    .replace(/[`*_~]/g, "")
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLocaleLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, "-")
    .replace(/^-+|-+$/g, "");
  return slug || "section";
}
