import { markdownLanguage } from "@codemirror/lang-markdown";
import { leadingFrontmatterEnd } from "./frontmatter";

const ASSET_FRAGMENT_PREFIX = "oah-image=";
const MAX_IMAGE_DIMENSION = 10_000;

export interface MarkdownImageSize {
  height?: number;
  width?: number;
}

export interface ParsedMarkdownImage extends MarkdownImageSize {
  alt: string;
  assetId?: string;
  destination: string;
  end: number;
  raw: string;
  start: number;
  title?: string;
}

export interface FormatMarkdownImageOptions extends MarkdownImageSize {
  alt: string;
  assetId?: string;
  destination: string;
  inTable?: boolean;
  title?: string;
}

/** Parse an inline CommonMark image and the app's optional size suffix. */
export function parseMarkdownImageAt(
  source: string,
  start: number,
): ParsedMarkdownImage | undefined {
  if (!source.startsWith("![", start)) {
    return undefined;
  }

  const labelEnd = findClosingLabel(source, start + 2);
  if (labelEnd < 0 || source[labelEnd + 1] !== "(") {
    return undefined;
  }

  const destinationEnd = findClosingDestination(source, labelEnd + 2);
  if (destinationEnd < 0) {
    return undefined;
  }

  const destinationParts = parseDestination(
    source.slice(labelEnd + 2, destinationEnd).trim(),
  );
  if (!destinationParts) {
    return undefined;
  }

  const label = unescapeMarkdownPunctuation(source.slice(start + 2, labelEnd));
  const { alt, ...size } = parseImageLabel(label);
  const asset = splitAssetFragment(destinationParts.destination);

  return {
    alt,
    destination: asset.destination,
    end: destinationEnd,
    raw: source.slice(start, destinationEnd + 1),
    start,
    ...(asset.assetId ? { assetId: asset.assetId } : {}),
    ...(destinationParts.title ? { title: destinationParts.title } : {}),
    ...size,
  };
}

/** Return only images that the editor's Markdown parser recognizes as syntax. */
export function parseMarkdownImages(source: string): ParsedMarkdownImage[] {
  const bodyStart = leadingFrontmatterEnd(source) ?? 0;
  const images: ParsedMarkdownImage[] = [];
  markdownLanguage.parser.parse(source).iterate({
    enter(node) {
      if (node.name !== "Image" || node.from < bodyStart) {
        return;
      }
      const image = parseMarkdownImageAt(source, node.from);
      if (image && image.end + 1 === node.to) {
        images.push(image);
      }
    },
  });

  return images;
}

/** Format a portable Markdown image, escaping its size separator in tables. */
export function formatMarkdownImage(
  options: FormatMarkdownImageOptions,
): string {
  const size = formatImageSize(options);
  const separator = options.inTable ? "\\|" : "|";
  const label = `${escapeImageAlt(options.alt, Boolean(options.inTable))}${
    size ? `${separator}${size}` : ""
  }`;
  const path = encodeMarkdownDestination(options.destination);
  const destination = options.assetId
    ? `${path}#${ASSET_FRAGMENT_PREFIX}${encodeURIComponent(options.assetId)}`
    : path;
  const title = options.title
    ? ` "${escapeMarkdownTitle(options.title)}"`
    : "";

  return `![${label}](${destination}${title})`;
}

/** Resolve an image path relative to the Markdown file that contains it. */
export function relativeImageDestination(
  noteRelativePath: string,
  imageRelativePath: string,
): string {
  const noteParts = splitPortablePath(noteRelativePath);
  const imageParts = splitPortablePath(imageRelativePath);
  noteParts.pop();

  let shared = 0;
  while (
    shared < noteParts.length &&
    shared < imageParts.length &&
    noteParts[shared] === imageParts[shared]
  ) {
    shared += 1;
  }

  return [
    ...noteParts.slice(shared).map(() => ".."),
    ...imageParts.slice(shared),
  ].join("/") || imageParts.at(-1) || "image";
}

export function markdownImageStyle(
  image: MarkdownImageSize,
): string | undefined {
  const declarations = [
    image.width ? `width: ${image.width}px` : undefined,
    image.height ? `height: ${image.height}px` : undefined,
  ].filter(Boolean);

  return declarations.length ? declarations.join("; ") : undefined;
}

function findClosingLabel(source: string, start: number): number {
  let depth = 1;
  for (let index = start; index < source.length; index += 1) {
    const character = source[index]!;
    if (character === "\n" || character === "\r") {
      return -1;
    }
    if (character === "\\") {
      index += 1;
    } else if (character === "[") {
      depth += 1;
    } else if (character === "]") {
      depth -= 1;
      if (!depth) {
        return index;
      }
    }
  }

  return -1;
}

function findClosingDestination(source: string, start: number): number {
  let depth = 1;
  let angleDestination = source[start] === "<";
  let quote: "\"" | "'" | undefined;

  for (let index = start; index < source.length; index += 1) {
    const character = source[index]!;
    if (character === "\n" || character === "\r") {
      return -1;
    }
    if (character === "\\") {
      index += 1;
      continue;
    }
    if (angleDestination) {
      if (character === ">") {
        angleDestination = false;
      }
      continue;
    }
    if (quote) {
      if (character === quote) {
        quote = undefined;
      }
      continue;
    }
    const previousCharacter = source[index - 1];
    const followsTitleSeparator = index > start
      && (previousCharacter === " " || previousCharacter === "\t");
    if (
      (character === "\"" || character === "'")
      && depth === 1
      && followsTitleSeparator
    ) {
      quote = character;
    } else if (character === "(") {
      depth += 1;
    } else if (character === ")") {
      depth -= 1;
      if (!depth) {
        return index;
      }
    }
  }

  return -1;
}

function parseDestination(
  raw: string,
): { destination: string; title?: string } | undefined {
  const match = raw.match(
    /^(<(?:\\.|[^>\\])+>|\S+?)(?:\s+(?:"((?:\\.|[^"\\])*)"|'((?:\\.|[^'\\])*)'|\(((?:\\.|[^)\\])*)\)))?$/,
  );
  if (!match) {
    return undefined;
  }

  const destination = match[1]!.replace(/^<|>$/g, "");
  const title = match[2] ?? match[3] ?? match[4];

  return {
    destination,
    ...(title === undefined ? {} : { title: unescapeMarkdownPunctuation(title) }),
  };
}

function parseImageLabel(label: string): { alt: string } & MarkdownImageSize {
  const match = label.match(/^(.*)\|(\d+)(?:x(\d+))?$/);
  if (match) {
    const width = imageDimension(match[2]);
    const height = imageDimension(match[3]);
    if (width && (match[3] === undefined || height)) {
      return {
        alt: match[1]!,
        width,
        ...(height ? { height } : {}),
      };
    }
  }

  const heightOnly = label.match(/^(.*)\|x(\d+)$/);
  const height = imageDimension(heightOnly?.[2]);
  if (heightOnly && height) {
    return { alt: heightOnly[1]!, height };
  }

  return { alt: label };
}

function imageDimension(value: string | undefined): number | undefined {
  if (!value) {
    return undefined;
  }
  const dimension = Number.parseInt(value, 10);

  return Number.isSafeInteger(dimension) &&
      dimension > 0 &&
      dimension <= MAX_IMAGE_DIMENSION
    ? dimension
    : undefined;
}

function formatImageSize(size: MarkdownImageSize): string {
  const width = imageDimension(size.width?.toString());
  const height = imageDimension(size.height?.toString());
  if (width && height) {
    return `${width}x${height}`;
  }
  if (width) {
    return String(width);
  }
  if (height) {
    return `x${height}`;
  }

  return "";
}

function splitAssetFragment(destination: string): {
  assetId?: string;
  destination: string;
} {
  const marker = `#${ASSET_FRAGMENT_PREFIX}`;
  const markerIndex = destination.lastIndexOf(marker);
  if (markerIndex < 0) {
    return { destination };
  }

  const encodedId = destination.slice(markerIndex + marker.length);
  let assetId: string;
  try {
    assetId = decodeURIComponent(encodedId);
  } catch {
    return { destination };
  }
  if (!/^[A-Za-z0-9_-]{1,180}$/.test(assetId)) {
    return { destination };
  }

  return {
    assetId,
    destination: destination.slice(0, markerIndex),
  };
}

function unescapeMarkdownPunctuation(value: string): string {
  return value.replace(
    /\\([!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~])/g,
    "$1",
  );
}

function escapeImageAlt(value: string, inTable: boolean): string {
  return value
    .replace(/\\/g, "\\\\")
    .replace(/([\[\]])/g, "\\$1")
    .replace(inTable ? /\|/g : /$^/g, "\\|")
    .replace(/[\r\n]+/g, " ");
}

function encodeMarkdownDestination(value: string): string {
  return value
    .split("/")
    .map((part) => part === "." || part === ".." ? part : encodeURIComponent(part))
    .join("/");
}

function escapeMarkdownTitle(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/"/g, "\\\"").replace(/[\r\n]+/g, " ");
}

function splitPortablePath(value: string): string[] {
  return value.split("/").filter((part) => part && part !== ".");
}
