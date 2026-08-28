import { markdownLanguage } from "@codemirror/lang-markdown";
import { leadingFrontmatterEnd } from "./frontmatter";
import { parseInlineMarkdownLinkAt } from "./markdownLinks";

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
  const link = parseInlineMarkdownLinkAt(source, start, true);
  if (!link) {
    return undefined;
  }

  const { alt, ...size } = parseImageLabel(link.label);
  const asset = splitAssetFragment(link.destination);

  return {
    alt,
    destination: asset.destination,
    end: link.end,
    raw: link.raw,
    start,
    ...(asset.assetId ? { assetId: asset.assetId } : {}),
    ...(link.title ? { title: link.title } : {}),
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
