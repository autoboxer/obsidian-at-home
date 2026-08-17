import { parseLiveMarkdownBlocks } from "./liveMarkdown";

export interface MarkdownHeading {
  contentFrom: number;
  contentTo: number;
  from: number;
  level: number;
  slug: string;
  text: string;
  to: number;
}

export interface MarkdownHeadingTarget {
  heading: string;
  noteTarget: string;
}

export function parseMarkdownHeadings(markdown: string): MarkdownHeading[] {
  const normalized = markdown.replace(/\r\n|\r/g, "\n");

  return parseLiveMarkdownBlocks(normalized).flatMap((block) => {
    if (block.type !== "heading" || !block.headingLevel) {
      return [];
    }

    const source = normalized.slice(block.content.from, block.content.to);
    const text = markdownHeadingText(source);

    return [{
      contentFrom: block.content.from,
      contentTo: block.content.to,
      from: block.from,
      level: block.headingLevel,
      slug: markdownHeadingSlug(text),
      text,
      to: block.to,
    }];
  });
}

export function findMarkdownHeading(
  markdown: string,
  target: string,
): MarkdownHeading | undefined {
  const trimmedTarget = target.trim();
  if (!trimmedTarget) {
    return undefined;
  }

  const headings = parseMarkdownHeadings(markdown);
  const comparisonKey = markdownHeadingComparisonKey(trimmedTarget);

  return headings.find((heading) =>
    markdownHeadingComparisonKey(heading.text) === comparisonKey
  ) ?? headings.find((heading) =>
    heading.slug === markdownHeadingSlug(trimmedTarget)
  );
}

export function parseMarkdownHeadingTarget(
  href: string,
): MarkdownHeadingTarget | undefined {
  const destination = href.trim().replace(/^<|>$/g, "");
  if (
    !destination
    || destination.startsWith("//")
    || /^[a-z][a-z0-9+.-]*:/i.test(destination)
  ) {
    return undefined;
  }

  const fragmentStart = destination.indexOf("#");
  if (fragmentStart < 0) {
    return undefined;
  }

  const rawNoteTarget = destination.slice(0, fragmentStart);
  const heading = decodeUriComponent(destination.slice(fragmentStart + 1)).trim();
  if (!heading || rawNoteTarget.includes("?")) {
    return undefined;
  }

  return {
    heading,
    noteTarget: decodeUriComponent(rawNoteTarget).trim(),
  };
}

export function markdownHeadingSlug(value: string): string {
  const slug = markdownHeadingComparisonKey(value)
    .replace(/[^\p{L}\p{N}]+/gu, "-")
    .replace(/^-+|-+$/g, "");

  return slug || "section";
}

function markdownHeadingText(value: string): string {
  return value
    .replace(
      /!?\[\[([^\]|#]+)(?:#[^\]|]+)?(?:\|([^\]]+))?\]\]/g,
      (_match, target: string, display: string | undefined) => display ?? target,
    )
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/`+([^`]*)`+/g, "$1")
    .replace(/\\([!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~])/g, "$1")
    .replace(/[*_~]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function markdownHeadingComparisonKey(value: string): string {
  return markdownHeadingText(value)
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLocaleLowerCase()
    .replace(/\s+/g, " ")
    .trim();
}

function decodeUriComponent(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}
