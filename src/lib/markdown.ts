import type { Note } from "../types";
import { leadingFrontmatterEnd } from "./frontmatter";
import { markdownHeadingSlug } from "./headingLinks";
import { highlightCode } from "./highlight";
import { findClosingInlineMarkupDelimiter } from "./inlineMarkup";
import {
  markdownAttachmentPresentation,
  parseMarkdownAttachmentAt,
  type MarkdownAttachmentMetadata,
  type ParsedMarkdownAttachment,
} from "./markdownAttachments";
import {
  markdownImageStyle,
  parseMarkdownImageAt,
  type ParsedMarkdownImage,
} from "./markdownImages";
import { parseWikiLinkAt } from "./wikiLinks";

export interface MarkdownRenderOptions {
  /** Recognize an extensionless attachment only when its vault inventory contains the file. */
  acceptExtensionlessAttachment?: (destination: string) => boolean;
  /** Used only to decorate resolvable/unresolvable wiki links. */
  resolveWikiLink?: (
    target: string,
    heading?: string,
  ) => Note | boolean | null | undefined;
  headingIdPrefix?: string;
  externalLinksInNewTab?: boolean;
  resolveAttachment?: (
    attachment: ParsedMarkdownAttachment,
  ) => MarkdownAttachmentMetadata | null | undefined;
  resolveImage?: (image: ParsedMarkdownImage) => string | null | undefined;
}

interface RenderContext {
  options: MarkdownRenderOptions;
  depth: number;
  listDepth: number;
  taskCounter: { value: number };
}

interface ListTextPart {
  type: "text";
  value: string;
}

interface ListHtmlPart {
  type: "html";
  value: string;
}

interface ListItem {
  checked?: boolean;
  taskIndex?: number;
  parts: Array<ListTextPart | ListHtmlPart>;
}

interface MatchedListItem {
  ordered: boolean;
  start: number;
  text: string;
  indent: number;
}

interface RenderedList {
  html: string;
  nextIndex: number;
}

interface ParsedMarkdownLink {
  html: string;
  end: number;
}

/**
 * Render the Markdown subset used by the app. Raw HTML is escaped except for a
 * strict `<br>` line-break allowlist. Scriptable URLs and event-handler
 * injection are not permitted, so the returned string is suitable for Vue's
 * `v-html`.
 */
export function renderMarkdown(
  markdown: string,
  options: MarkdownRenderOptions = {},
): string {
  const safeMarkdown = markdown.replace(/\0/g, "");

  return renderBlocks(stripLeadingFrontmatterForPreview(safeMarkdown), {
    options,
    depth: 0,
    listDepth: 0,
    taskCounter: { value: 0 },
  });
}

function stripLeadingFrontmatterForPreview(markdown: string): string {
  const frontmatterEnd = leadingFrontmatterEnd(markdown);

  return frontmatterEnd === undefined
    ? markdown
    : markdown.slice(frontmatterEnd);
}

/** Toggle a rendered task checkbox while preserving the rest of the Markdown. */
export function toggleMarkdownTask(
  markdown: string,
  taskIndex: number,
  checked: boolean,
): string {
  if (!Number.isInteger(taskIndex) || taskIndex < 0) {
    return markdown;
  }

  const bodyStart = leadingFrontmatterEnd(markdown) ?? 0;
  let currentTask = 0;
  let cursor = bodyStart;
  let fence: { marker: string; size: number; quoteDepth: number } | undefined;

  while (cursor <= markdown.length) {
    const newline = markdown.indexOf("\n", cursor);
    const lineEnd = newline < 0 ? markdown.length : newline;
    const line = markdown.slice(cursor, lineEnd).replace(/\r$/, "");
    const quotePrefix = line.match(/^((?: {0,3}> ?)+)/)?.[1] ?? "";
    const quoteDepth = quotePrefix.match(/>/g)?.length ?? 0;
    const unquotedLine = line.slice(quotePrefix.length);

    if (fence && quoteDepth < fence.quoteDepth) {
      fence = undefined;
    }

    const fenceMatch = unquotedLine.match(/^ {0,3}(`{3,}|~{3,})/);
    if (fenceMatch) {
      const run = fenceMatch[1]!;
      if (!fence) {
        fence = { marker: run[0]!, size: run.length, quoteDepth };
      } else if (run[0] === fence.marker && run.length >= fence.size) {
        fence = undefined;
      }
    } else if (!fence) {
      const task = line.match(
        /^(?:(?: {0,3}> ?)+)?[ \t]*(?:[-+*]|\d{1,9}[.)])[\t ]+\[([ xX])\]/,
      );
      if (task) {
        if (currentTask === taskIndex) {
          const markerOffset = task[0].lastIndexOf("[") + 1;

          return `${markdown.slice(0, cursor + markerOffset)}${checked ? "x" : " "}${markdown.slice(
            cursor + markerOffset + 1,
          )}`;
        }
        currentTask += 1;
      }
    }

    if (newline < 0) {
      break;
    }
    cursor = newline + 1;
  }

  return markdown;
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
  if (!url) {
    return undefined;
  }

  const compact = url.replace(/[\s\u00a0]+/g, "").toLocaleLowerCase();
  const scheme = compact.match(/^([a-z][a-z0-9+.-]*):/i)?.[1];
  if (scheme && !["http", "https", "mailto"].includes(scheme)) {
    return undefined;
  }

  // Backslashes can be interpreted as slashes by URL parsers, so reject them
  // instead of allowing a disguised scheme-relative destination.
  if (url.includes("\\")) {
    return undefined;
  }

  return url;
}

function renderBlocks(markdown: string, context: RenderContext): string {
  // Prevent pathological recursive blockquotes from consuming the call stack.
  if (context.depth > 16) {
    return `<p>${renderInline(markdown, context)}</p>`;
  }

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

      const rawCode = code.join("\n");
      const highlighted = language ? highlightCode(rawCode, language) : undefined;
      const classes = [language ? `language-${escapeHtml(language)}` : "", highlighted ? "hljs" : ""]
        .filter(Boolean)
        .join(" ");
      const className = classes ? ` class="${classes}"` : "";
      const languageLabel = language ? ` data-language="${escapeHtml(language)}"` : "";
      blocks.push(`<pre${languageLabel}><code${className}>${highlighted ?? escapeHtml(rawCode)}</code></pre>`);
      continue;
    }

    const heading = line.match(/^ {0,3}(#{1,6})[\t ]+(.+?)\s*$/);
    if (heading) {
      const level = heading[1]!.length;
      const text = heading[2]!.replace(/[\t ]+#+[\t ]*$/, "");
      const prefix = context.options.headingIdPrefix ?? "";
      const id = `${prefix}${markdownHeadingSlug(text)}`;
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
      const rendered = renderList(lines, index, context, listMatch.indent);
      blocks.push(rendered.html);
      index = rendered.nextIndex;
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
  if (depth > 12) {
    return escapeHtml(source);
  }

  let html = "";
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index]!;

    if (character === "\\" && /[!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~]/.test(source[index + 1] ?? "")) {
      html += escapeHtml(source[index + 1]!);
      index += 1;
      continue;
    }

    const lineBreakTag = source.slice(index).match(/^<br[ \t]*\/?>/i)?.[0];
    if (lineBreakTag) {
      html += "<br>";
      index += lineBreakTag.length - 1;
      continue;
    }

    if (character === "`") {
      let delimiterLength = 1;
      while (source[index + delimiterLength] === "`") {
        delimiterLength += 1;
      }
      const delimiter = "`".repeat(delimiterLength);
      const close = source.indexOf(delimiter, index + delimiterLength);
      if (close >= 0) {
        let code = source.slice(index + delimiterLength, close).replace(/\n/g, " ");
        if (/^\s.*\s$/.test(code) && code.trim()) {
          code = code.slice(1, -1);
        }
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

      if (character === "!") {
        const image = parseMarkdownImageAt(source, index);
        if (image) {
          html += renderMarkdownImage(image, context);
          index = image.end;
          continue;
        }
      }

      if (character === "[") {
        const attachment = parseMarkdownAttachmentAt(source, index, {
          acceptExtensionless: context.options.acceptExtensionlessAttachment,
        });
        if (attachment) {
          html += renderMarkdownAttachment(attachment, context);
          index = attachment.end;
          continue;
        }

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
      const close = findClosingInlineMarkupDelimiter(
        source,
        strongDelimiter,
        index,
        index + strongDelimiter.length,
      );
      if (close > index + 2) {
        const inner = source.slice(index + 2, close);
        html += `<strong>${renderInline(inner, context, depth + 1)}</strong>`;
        index = close + 1;
        continue;
      }
    }

    if (source.startsWith("~~", index)) {
      const close = findClosingInlineMarkupDelimiter(
        source,
        "~~",
        index,
        index + 2,
      );
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
        : findClosingInlineMarkupDelimiter(
          source,
          character,
          index,
          index + 1,
        );
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

function renderMarkdownAttachment(
  attachment: ParsedMarkdownAttachment,
  context: RenderContext,
): string {
  const metadata = context.options.resolveAttachment?.(attachment) ?? undefined;
  const presentation = markdownAttachmentPresentation(attachment, metadata);
  const assetAttribute = attachment.assetId
    ? ` data-attachment-asset-id="${escapeHtml(attachment.assetId)}"`
    : "";
  const titleAttribute = attachment.title
    ? ` title="${escapeHtml(attachment.title)}"`
    : "";
  const ariaLabel = `${presentation.name}, ${presentation.typeLabel}, ${presentation.sizeLabel}`;

  return `<span class="markdown-attachment-card" role="group" aria-label="${escapeHtml(
    ariaLabel,
  )}" data-attachment-destination="${escapeHtml(attachment.destination)}"${assetAttribute}${titleAttribute}><span class="attachment-card__icon" aria-hidden="true">${escapeHtml(
    presentation.iconLabel,
  )}</span><span class="attachment-card__copy"><span class="attachment-card__name">${escapeHtml(
    presentation.name,
  )}</span><span class="attachment-card__details">${escapeHtml(
    `${presentation.typeLabel} · ${presentation.sizeLabel}`,
  )}</span></span></span>`;
}

function renderMarkdownImage(
  image: ParsedMarkdownImage,
  context: RenderContext,
): string {
  const resolved = context.options.resolveImage
    ? context.options.resolveImage(image)
    : image.destination;
  const source = resolved ? sanitizeImageUrl(resolved) : undefined;
  if (!source) {
    return `<span class="unresolved-image">${escapeHtml(image.alt || "Image")}</span>`;
  }

  const titleAttribute = image.title
    ? ` title="${escapeHtml(image.title)}"`
    : "";
  const style = markdownImageStyle(image);
  const styleAttribute = style ? ` style="${style}"` : "";

  return `<img class="markdown-image" src="${escapeHtml(source)}" alt="${escapeHtml(
    image.alt,
  )}"${titleAttribute}${styleAttribute} loading="lazy" decoding="async">`;
}

/** Return a browser-safe image URL, or `undefined`. */
export function sanitizeImageUrl(value: string): string | undefined {
  const url = value
    .trim()
    .replace(/^<|>$/g, "")
    .replace(/[\u0000-\u001f\u007f]/g, "");
  if (!url || url.includes("\\")) {
    return undefined;
  }

  const compact = url.replace(/[\s\u00a0]+/g, "").toLocaleLowerCase();
  if (/^data:/i.test(compact)) {
    return /^data:image\/(?:avif|bmp|gif|jpeg|png|webp);base64,/i.test(compact)
      ? url
      : undefined;
  }

  const scheme = compact.match(/^([a-z][a-z0-9+.-]*):/i)?.[1];
  if (scheme && !["blob", "http", "https"].includes(scheme)) {
    return undefined;
  }

  return url;
}

function parseMarkdownLink(
  source: string,
  start: number,
  context: RenderContext,
  depth: number,
): ParsedMarkdownLink | undefined {
  const labelEnd = findUnescaped(source, "]", start + 1);
  if (labelEnd < 0 || source[labelEnd + 1] !== "(") {
    return undefined;
  }

  let parentheses = 1;
  let destinationEnd = -1;
  for (let index = labelEnd + 2; index < source.length; index += 1) {
    if (source[index] === "\\") {
      index += 1;
      continue;
    }
    if (source[index] === "(") {
      parentheses += 1;
    }
    if (source[index] === ")") {
      parentheses -= 1;
      if (parentheses === 0) {
        destinationEnd = index;
        break;
      }
    }
  }
  if (destinationEnd < 0) {
    return undefined;
  }

  const label = source.slice(start + 1, labelEnd);
  const rawDestination = source.slice(labelEnd + 2, destinationEnd).trim();
  const destinationMatch = rawDestination.match(
    /^(<[^>]+>|\S+?)(?:\s+(?:"([^"]*)"|'([^']*)'|\(([^)]*)\)))?$/,
  );
  if (!destinationMatch) {
    return undefined;
  }

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

function findUnescaped(source: string, needle: string, start: number): number {
  for (let index = start; index < source.length; index += 1) {
    if (source[index] === "\\") {
      index += 1;
      continue;
    }
    if (source[index] === needle) {
      return index;
    }
  }

  return -1;
}

function matchListItem(line: string): MatchedListItem | undefined {
  const match = line.match(/^([ \t]*)(?:(\d{1,9})[.)]|([-+*]))[\t ]+(.*)$/);
  if (!match) {
    return undefined;
  }
  const ordered = Boolean(match[2]);

  return {
    ordered,
    start: ordered ? Math.max(1, Number.parseInt(match[2]!, 10)) : 1,
    text: match[4]!,
    indent: indentationWidth(match[1]!),
  };
}

function renderList(
  lines: readonly string[],
  startIndex: number,
  context: RenderContext,
  baseIndent: number,
): RenderedList {
  const first = matchListItem(lines[startIndex] ?? "");
  if (!first) {
    return { html: "", nextIndex: startIndex + 1 };
  }

  const ordered = first.ordered;
  const start = first.start;
  const items: ListItem[] = [];
  let index = startIndex;

  while (index < lines.length) {
    const item = matchListItem(lines[index]!);
    if (!item || item.indent !== baseIndent || item.ordered !== ordered) {
      break;
    }
    index += 1;

    const task = item.text.match(/^\[([ xX])\][\t ]+(.*)$/);
    const taskIndex = task ? context.taskCounter.value : undefined;
    if (task) {
      context.taskCounter.value += 1;
    }
    const parts: Array<ListTextPart | ListHtmlPart> = [{
      type: "text",
      value: task ? task[2]! : item.text,
    }];

    while (index < lines.length) {
      const nextLine = lines[index]!;
      if (!nextLine.trim()) {
        let lookahead = index + 1;
        while (lookahead < lines.length && !lines[lookahead]!.trim()) {
          lookahead += 1;
        }
        if (lookahead >= lines.length) {
          break;
        }

        const following = lines[lookahead]!;
        const followingItem = matchListItem(following);
        const followingIndent = indentationWidth(following.match(/^[ \t]*/)?.[0] ?? "");
        if (!followingItem && followingIndent <= baseIndent) {
          break;
        }

        index = lookahead;
        if (followingItem?.indent === baseIndent) {
          break;
        }
        continue;
      }

      const nested = matchListItem(nextLine);
      if (nested) {
        if (nested.indent <= baseIndent) {
          break;
        }
        if (context.listDepth >= 16) {
          parts.push({ type: "text", value: nested.text });
          index += 1;
          continue;
        }
        const rendered = renderList(
          lines,
          index,
          { ...context, listDepth: context.listDepth + 1 },
          nested.indent,
        );
        parts.push({ type: "html", value: rendered.html });
        index = rendered.nextIndex;
        continue;
      }

      if (indentationWidth(nextLine.match(/^[ \t]*/)?.[0] ?? "") <= baseIndent) {
        break;
      }
      const previous = parts.at(-1);
      if (previous?.type === "text") {
        previous.value += `\n${nextLine.trim()}`;
      } else {
        parts.push({ type: "text", value: nextLine.trim() });
      }
      index += 1;
    }

    items.push({
      parts,
      ...(task ? { checked: task[1]!.toLocaleLowerCase() === "x" } : {}),
      ...(taskIndex === undefined ? {} : { taskIndex }),
    });
  }

  const hasTasks = items.some((item) => item.checked !== undefined);
  const tag = ordered ? "ol" : "ul";
  const startAttribute = ordered && start !== 1 ? ` start="${start}"` : "";
  const classes = [`list-depth-${context.listDepth % 3}`, hasTasks ? "task-list" : ""]
    .filter(Boolean)
    .join(" ");
  const itemHtml = items.map((item) => {
    const renderParts = (parts: Array<ListTextPart | ListHtmlPart>) => parts
      .map((part) => part.type === "html" ? part.value : renderInline(part.value, context))
      .join("");

    if (item.checked === undefined) {
      return `<li>${renderParts(item.parts)}</li>`;
    }

    const checkbox = `<input type="checkbox" data-task-index="${item.taskIndex}"${item.checked ? " checked" : ""} aria-label="${
      item.checked ? "Completed task" : "Incomplete task"
    }">`;
    const [firstPart, ...remainingParts] = item.parts;
    const firstHtml = firstPart?.type === "text" ? renderInline(firstPart.value, context) : "";

    return `<li class="task-list-item"><span class="task-list-row">${checkbox}<span>${
      firstHtml
    }</span></span>${renderParts(firstPart?.type === "html" ? item.parts : remainingParts)}</li>`;
  }).join("");

  return {
    html: `<${tag} class="${classes}"${startAttribute}>${itemHtml}</${tag}>`,
    nextIndex: index,
  };
}

function indentationWidth(value: string): number {
  let width = 0;
  for (const character of value) {
    width += character === "\t" ? 4 : 1;
  }

  return width;
}

function isBlockStart(lines: readonly string[], index: number): boolean {
  const line = lines[index] ?? "";

  return /^ {0,3}(?:`{3,}|~{3,}|#{1,6}[\t ]|>)/.test(line) ||
    /^[ \t]*(?:[-+*]|\d{1,9}[.)])[\t ]/.test(line) ||
    /^ {0,3}(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line) ||
    isTable(lines, index);
}

function looksLikeTableRow(line: string): boolean {
  if (!line.includes("|")) {
    return false;
  }

  return splitTableRow(line).length > 1;
}

function isTable(lines: readonly string[], index: number): boolean {
  const header = lines[index] ?? "";
  const delimiter = lines[index + 1] ?? "";
  if (!looksLikeTableRow(header) || !delimiter.includes("|")) {
    return false;
  }
  const cells = splitTableRow(delimiter);

  return cells.length > 0 && cells.every((cell) => /^\s*:?-{3,}:?\s*$/.test(cell));
}

function splitTableRow(line: string): string[] {
  let value = line.trim();
  if (value.startsWith("|")) {
    value = value.slice(1);
  }
  if (value.endsWith("|") && value[value.length - 2] !== "\\") {
    value = value.slice(0, -1);
  }

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
      while (value[index + run] === "`") {
        run += 1;
      }
      if (codeDelimiter === 0) {
        codeDelimiter = run;
      } else if (codeDelimiter === run) {
        codeDelimiter = 0;
      }
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
  if (trimmed.startsWith(":") && trimmed.endsWith(":")) {
    return "center";
  }
  if (trimmed.endsWith(":")) {
    return "right";
  }
  if (trimmed.startsWith(":")) {
    return "left";
  }

  return undefined;
}

function alignmentClass(value: ReturnType<typeof tableAlignment>): string {
  return value ? ` class="align-${value}"` : "";
}
