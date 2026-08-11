import { sanitizeLinkUrl } from "./markdown";
import { parseWikiLinkAt } from "./wikiLinks";

export type LiveMarkdownInlineMark =
  | "code"
  | "emphasis"
  | "strikethrough"
  | "strong";

export type LiveMarkdownInlineKind =
  | "link"
  | "text"
  | "unsafe-link"
  | "wiki-link";

export interface LiveMarkdownInlineSegment {
  kind: LiveMarkdownInlineKind;
  text: string;
  marks: LiveMarkdownInlineMark[];
  href?: string;
  title?: string;
  wikiTarget?: string;
  wikiHeading?: string;
  embedded?: boolean;
  resolved?: boolean;
}

export interface LiveMarkdownInlineOptions {
  resolveWikiLink?: (
    target: string,
    heading?: string,
  ) => boolean;
}

interface SegmentDecoration {
  kind: LiveMarkdownInlineKind;
  href?: string;
  title?: string;
  wikiTarget?: string;
  wikiHeading?: string;
  embedded?: boolean;
  resolved?: boolean;
}

interface ParsedMarkdownLink {
  label: string;
  end: number;
  href?: string;
  title?: string;
}

const ESCAPABLE_PUNCTUATION = /[!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~]/;
const MAX_NESTING_DEPTH = 12;

export function parseLiveMarkdownInline(
  source: string,
  options: LiveMarkdownInlineOptions = {},
): LiveMarkdownInlineSegment[] {
  const segments: LiveMarkdownInlineSegment[] = [];
  parseRange(source, segments, [], { kind: "text" }, options, 0);

  return segments;
}

function parseRange(
  source: string,
  segments: LiveMarkdownInlineSegment[],
  marks: LiveMarkdownInlineMark[],
  decoration: SegmentDecoration,
  options: LiveMarkdownInlineOptions,
  depth: number,
): void {
  if (depth > MAX_NESTING_DEPTH) {
    appendSegment(segments, source, marks, decoration);

    return;
  }

  for (let index = 0; index < source.length; index += 1) {
    const character = source[index]!;

    if (
      character === "\\" &&
      ESCAPABLE_PUNCTUATION.test(source[index + 1] ?? "")
    ) {
      appendSegment(segments, source[index + 1]!, marks, decoration);
      index += 1;

      continue;
    }

    if (character === "`") {
      const code = parseInlineCode(source, index);
      if (code) {
        appendSegment(segments, code.text, [...marks, "code"], decoration);
        index = code.end;

        continue;
      }
    }

    if (character === "!" || character === "[") {
      const wikiLink = parseWikiLinkAt(source, index);
      if (wikiLink) {
        const resolved = options.resolveWikiLink?.(
          wikiLink.target,
          wikiLink.heading,
        );
        appendSegment(
          segments,
          wikiLink.display || wikiLink.heading || wikiLink.target,
          marks,
          {
            kind: "wiki-link",
            wikiTarget: wikiLink.target,
            ...(wikiLink.heading ? { wikiHeading: wikiLink.heading } : {}),
            ...(wikiLink.embedded ? { embedded: true } : {}),
            ...(resolved === undefined ? {} : { resolved }),
          },
        );
        index += wikiLink.raw.length - 1;

        continue;
      }

      if (character === "[") {
        const markdownLink = parseMarkdownLink(source, index);
        if (markdownLink) {
          parseRange(
            markdownLink.label,
            segments,
            marks,
            markdownLink.href
              ? {
                  kind: "link",
                  href: markdownLink.href,
                  ...(markdownLink.title ? { title: markdownLink.title } : {}),
                }
              : { kind: "unsafe-link" },
            options,
            depth + 1,
          );
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
        parseRange(
          source.slice(index + 2, close),
          segments,
          [...marks, "strong"],
          decoration,
          options,
          depth + 1,
        );
        index = close + 1;

        continue;
      }
    }

    if (source.startsWith("~~", index)) {
      const close = findClosingDelimiter(source, "~~", index + 2);
      if (close > index + 2) {
        parseRange(
          source.slice(index + 2, close),
          segments,
          [...marks, "strikethrough"],
          decoration,
          options,
          depth + 1,
        );
        index = close + 1;

        continue;
      }
    }

    if (character === "*" || character === "_") {
      const intrawordUnderscore = character === "_" &&
        /[\p{L}\p{N}]/u.test(source[index - 1] ?? "") &&
        /[\p{L}\p{N}]/u.test(source[index + 1] ?? "");
      const close = intrawordUnderscore
        ? -1
        : findClosingDelimiter(source, character, index + 1);
      if (close > index + 1) {
        parseRange(
          source.slice(index + 1, close),
          segments,
          [...marks, "emphasis"],
          decoration,
          options,
          depth + 1,
        );
        index = close;

        continue;
      }
    }

    appendSegment(segments, character, marks, decoration);
  }
}

function parseInlineCode(
  source: string,
  start: number,
): { text: string; end: number } | undefined {
  let delimiterLength = 1;
  while (source[start + delimiterLength] === "`") {
    delimiterLength += 1;
  }

  const delimiter = "`".repeat(delimiterLength);
  const close = source.indexOf(delimiter, start + delimiterLength);
  if (close < 0) {
    return undefined;
  }

  let text = source
    .slice(start + delimiterLength, close)
    .replace(/\n/g, " ");
  if (/^\s.*\s$/.test(text) && text.trim()) {
    text = text.slice(1, -1);
  }

  return {
    text,
    end: close + delimiterLength - 1,
  };
}

function parseMarkdownLink(
  source: string,
  start: number,
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

  const rawDestination = source.slice(labelEnd + 2, destinationEnd).trim();
  const destination = rawDestination.match(
    /^(<[^>]+>|\S+?)(?:\s+(?:"([^"]*)"|'([^']*)'|\(([^)]*)\)))?$/,
  );
  if (!destination) {
    return undefined;
  }

  const href = sanitizeLinkUrl(destination[1]!.replace(/^<|>$/g, ""));
  const title = destination[2] ?? destination[3] ?? destination[4];

  return {
    label: source.slice(start + 1, labelEnd),
    end: destinationEnd,
    ...(href ? { href } : {}),
    ...(title ? { title } : {}),
  };
}

function findClosingDelimiter(
  source: string,
  delimiter: string,
  start: number,
): number {
  let cursor = start;
  while (cursor < source.length) {
    const index = source.indexOf(delimiter, cursor);
    if (index < 0) {
      return -1;
    }
    if (source[index - 1] !== "\\") {
      return index;
    }
    cursor = index + delimiter.length;
  }

  return -1;
}

function findUnescaped(
  source: string,
  needle: string,
  start: number,
): number {
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

function appendSegment(
  segments: LiveMarkdownInlineSegment[],
  text: string,
  marks: LiveMarkdownInlineMark[],
  decoration: SegmentDecoration,
): void {
  if (!text) {
    return;
  }

  const previous = segments.at(-1);
  if (
    previous &&
    sameMarks(previous.marks, marks) &&
    previous.kind === decoration.kind &&
    previous.href === decoration.href &&
    previous.title === decoration.title &&
    previous.wikiTarget === decoration.wikiTarget &&
    previous.wikiHeading === decoration.wikiHeading &&
    previous.embedded === decoration.embedded &&
    previous.resolved === decoration.resolved
  ) {
    previous.text += text;

    return;
  }

  segments.push({
    kind: decoration.kind,
    text,
    marks: [...marks],
    ...(decoration.href ? { href: decoration.href } : {}),
    ...(decoration.title ? { title: decoration.title } : {}),
    ...(decoration.wikiTarget !== undefined
      ? { wikiTarget: decoration.wikiTarget }
      : {}),
    ...(decoration.wikiHeading ? { wikiHeading: decoration.wikiHeading } : {}),
    ...(decoration.embedded ? { embedded: true } : {}),
    ...(decoration.resolved === undefined ? {} : { resolved: decoration.resolved }),
  });
}

function sameMarks(
  left: readonly LiveMarkdownInlineMark[],
  right: readonly LiveMarkdownInlineMark[],
): boolean {
  return left.length === right.length &&
    left.every((mark, index) => mark === right[index]);
}
