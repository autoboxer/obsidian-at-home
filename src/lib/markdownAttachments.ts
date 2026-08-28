import { markdownLanguage } from "@codemirror/lang-markdown";
import { leadingFrontmatterEnd } from "./frontmatter";
import { parseInlineMarkdownLinkAt } from "./markdownLinks";

const ASSET_FRAGMENT_PREFIX = "oah-asset=";
const IMAGE_EXTENSIONS = new Set([
  "avif",
  "bmp",
  "gif",
  "jpeg",
  "jpg",
  "png",
  "webp",
]);
const MARKDOWN_EXTENSIONS = new Set(["markdown", "md"]);
const EXECUTABLE_EXTENSIONS = new Set([
  "app",
  "appimage",
  "appx",
  "appxbundle",
  "bat",
  "bin",
  "cmd",
  "command",
  "com",
  "cpl",
  "deb",
  "desktop",
  "dmg",
  "exe",
  "fish",
  "hta",
  "jar",
  "js",
  "jse",
  "lnk",
  "msc",
  "msi",
  "msix",
  "msixbundle",
  "msp",
  "mst",
  "pif",
  "pkg",
  "ps1",
  "psm1",
  "py",
  "pyw",
  "reg",
  "rpm",
  "run",
  "scr",
  "sh",
  "tool",
  "vbe",
  "vbs",
  "wsf",
  "wsh",
  "zsh",
]);

export const VAULT_ATTACHMENT_DRAG_MIME = "application/x-obsidian-at-home-vault-attachment";

export interface ParsedMarkdownAttachment {
  assetId?: string;
  destination: string;
  end: number;
  label: string;
  raw: string;
  start: number;
  title?: string;
}

export interface MarkdownAttachmentMetadata {
  byteLength?: number;
  mediaType?: string;
  openingDisabled?: boolean;
  relativePath?: string;
}

export interface MarkdownAttachmentPresentation {
  iconLabel: string;
  name: string;
  sizeLabel: string;
  typeLabel: string;
}

export interface FormatMarkdownAttachmentOptions {
  assetId?: string;
  destination: string;
  inTable?: boolean;
  label: string;
  title?: string;
}

/** Parse a portable local, non-image attachment link. */
export function parseMarkdownAttachmentAt(
  source: string,
  start: number,
): ParsedMarkdownAttachment | undefined {
  const link = parseInlineMarkdownLinkAt(source, start);
  if (!link) {
    return undefined;
  }

  const asset = splitAssetFragment(link.destination);
  if (!isPortableAttachmentDestination(asset.destination, Boolean(asset.assetId))) {
    return undefined;
  }

  return {
    ...link,
    destination: asset.destination,
    ...(asset.assetId ? { assetId: asset.assetId } : {}),
  };
}

/** Return only attachment links recognized by the editor's Markdown parser. */
export function parseMarkdownAttachments(source: string): ParsedMarkdownAttachment[] {
  const bodyStart = leadingFrontmatterEnd(source) ?? 0;
  const attachments: ParsedMarkdownAttachment[] = [];
  markdownLanguage.parser.parse(source).iterate({
    enter(node) {
      if (node.name !== "Link" || node.from < bodyStart) {
        return;
      }
      const attachment = parseMarkdownAttachmentAt(source, node.from);
      if (attachment && attachment.end + 1 === node.to) {
        attachments.push(attachment);
      }
    },
  });

  return attachments;
}

/** Format an ordinary Markdown link with optional stable attachment identity. */
export function formatMarkdownAttachment(
  options: FormatMarkdownAttachmentOptions,
): string {
  const label = escapeAttachmentLabel(options.label, Boolean(options.inTable));
  const path = encodeMarkdownDestination(options.destination);
  const destination = options.assetId
    ? `${path}#${ASSET_FRAGMENT_PREFIX}${encodeURIComponent(options.assetId)}`
    : path;
  const title = options.title
    ? ` "${escapeMarkdownTitle(options.title)}"`
    : "";

  return `[${label}](${destination}${title})`;
}

/** Resolve an attachment path relative to the Markdown file that contains it. */
export function relativeAttachmentDestination(
  noteRelativePath: string,
  attachmentRelativePath: string,
): string {
  const noteParts = splitPortablePath(noteRelativePath);
  const attachmentParts = splitPortablePath(attachmentRelativePath);
  noteParts.pop();

  let shared = 0;
  while (
    shared < noteParts.length
    && shared < attachmentParts.length
    && noteParts[shared] === attachmentParts[shared]
  ) {
    shared += 1;
  }

  return [
    ...noteParts.slice(shared).map(() => ".."),
    ...attachmentParts.slice(shared),
  ].join("/") || attachmentParts.at(-1) || "attachment";
}

export function attachmentLabelFromPath(relativePath: string): string {
  return attachmentFileName(relativePath) || "Attachment";
}

export function markdownAttachmentPresentation(
  attachment: ParsedMarkdownAttachment,
  metadata?: MarkdownAttachmentMetadata,
): MarkdownAttachmentPresentation {
  const path = metadata?.relativePath || attachment.destination;
  const fileName = attachmentFileName(path);
  const extension = attachmentExtension(path);
  const name = attachment.label.trim() || fileName || "Attachment";

  return {
    iconLabel: attachmentIconLabel(extension, metadata?.mediaType),
    name,
    sizeLabel: formatAttachmentSize(metadata?.byteLength),
    typeLabel: attachmentTypeLabel(
      extension,
      metadata?.mediaType,
      metadata?.openingDisabled,
    ),
  };
}

export function markdownAttachmentIsArchive(
  destination: string,
  mediaType?: string,
): boolean {
  return isArchive(attachmentExtension(destination), mediaType);
}

export function markdownAttachmentIsExecutable(
  destination: string,
  openingDisabled = false,
): boolean {
  return openingDisabled || EXECUTABLE_EXTENSIONS.has(attachmentExtension(destination));
}

function isPortableAttachmentDestination(
  destination: string,
  hasAssetId: boolean,
): boolean {
  if (!destination || destination.includes("#") || destination.includes("?")) {
    return false;
  }

  let decoded: string;
  try {
    decoded = decodeURIComponent(destination);
  } catch {
    return false;
  }
  if (
    !decoded
    || decoded.startsWith("//")
    || decoded.includes("\\")
    || decoded.includes("?")
    || /[\u0000-\u001f\u007f]/u.test(decoded)
    || /^[A-Za-z][A-Za-z\d+.-]*:/u.test(decoded)
  ) {
    return false;
  }

  const extension = attachmentExtension(destination);
  if (IMAGE_EXTENSIONS.has(extension) || MARKDOWN_EXTENSIONS.has(extension)) {
    return false;
  }

  return hasAssetId || Boolean(extension);
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

function attachmentFileName(destination: string): string {
  const path = destination.split(/[?#]/u, 1)[0] ?? "";
  const encodedName = path.split("/").filter(Boolean).at(-1) ?? "";
  try {
    return decodeURIComponent(encodedName);
  } catch {
    return encodedName;
  }
}

function attachmentExtension(destination: string): string {
  const fileName = attachmentFileName(destination);
  const extension = fileName.match(/\.([^.]+)$/u)?.[1] ?? "";
  return extension.toLocaleLowerCase();
}

function attachmentIconLabel(extension: string, mediaType?: string): string {
  if (mediaType === "application/pdf" || extension === "pdf") {
    return "PDF";
  }
  if (isArchive(extension, mediaType)) {
    return extension === "zip" || mediaType === "application/zip" ? "ZIP" : "ARC";
  }
  if (isWordDocument(extension, mediaType)) {
    return "DOC";
  }
  if (isSpreadsheet(extension, mediaType)) {
    return "XLS";
  }
  if (isPresentation(extension, mediaType)) {
    return "PPT";
  }
  if (mediaType?.startsWith("audio/") || mediaType?.startsWith("video/")) {
    return mediaType.startsWith("audio/") ? "AUD" : "VID";
  }

  return extension ? extension.slice(0, 4).toLocaleUpperCase() : "FILE";
}

function attachmentTypeLabel(
  extension: string,
  mediaType?: string,
  openingDisabled = false,
): string {
  if (openingDisabled || EXECUTABLE_EXTENSIONS.has(extension)) {
    return extension
      ? `${extension.toLocaleUpperCase()} executable · Opening disabled`
      : "Executable file · Opening disabled";
  }
  if (mediaType === "application/pdf" || extension === "pdf") {
    return "PDF document";
  }
  if (isArchive(extension, mediaType)) {
    return "Archive";
  }
  if (isWordDocument(extension, mediaType)) {
    return "Word document";
  }
  if (isSpreadsheet(extension, mediaType)) {
    return "Spreadsheet";
  }
  if (isPresentation(extension, mediaType)) {
    return "Presentation";
  }
  if (mediaType?.startsWith("audio/")) {
    return "Audio";
  }
  if (mediaType?.startsWith("video/")) {
    return "Video";
  }
  if (mediaType?.startsWith("text/")) {
    return "Text file";
  }
  if (extension === "json" || mediaType === "application/json") {
    return "JSON file";
  }

  return extension
    ? `${extension.toLocaleUpperCase()} file · Opens with system default`
    : "Unrecognized file · Opens with system default";
}

function formatAttachmentSize(value: number | undefined): string {
  if (!Number.isSafeInteger(value) || value === undefined || value < 0) {
    return "Size unavailable";
  }
  if (value < 1_024) {
    return `${value} B`;
  }

  const units = ["KB", "MB", "GB", "TB"];
  let size = value / 1_024;
  let unit = units[0]!;
  for (let index = 1; index < units.length && size >= 1_024; index += 1) {
    size /= 1_024;
    unit = units[index]!;
  }
  const precision = size < 10 ? 1 : 0;

  return `${size.toFixed(precision)} ${unit}`;
}

function isArchive(extension: string, mediaType?: string): boolean {
  return ["7z", "bz2", "gz", "rar", "tar", "tgz", "xz", "zip"].includes(extension)
    || [
      "application/gzip",
      "application/vnd.rar",
      "application/x-7z-compressed",
      "application/x-tar",
      "application/zip",
    ].includes(mediaType ?? "");
}

function isWordDocument(extension: string, mediaType?: string): boolean {
  return ["doc", "docx", "odt", "rtf"].includes(extension)
    || mediaType === "application/msword"
    || mediaType === "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
}

function isSpreadsheet(extension: string, mediaType?: string): boolean {
  return ["csv", "ods", "xls", "xlsx"].includes(extension)
    || mediaType === "application/vnd.ms-excel"
    || mediaType === "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
}

function isPresentation(extension: string, mediaType?: string): boolean {
  return ["odp", "ppt", "pptx"].includes(extension)
    || mediaType === "application/vnd.ms-powerpoint"
    || mediaType === "application/vnd.openxmlformats-officedocument.presentationml.presentation";
}

function escapeAttachmentLabel(value: string, inTable: boolean): string {
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
