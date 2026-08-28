const RESERVED_WORKSPACE_FOLDERS = new Set([
  ".git",
  ".obsidian",
  ".obsidian-at-home",
  ".trash",
]);
const WINDOWS_RESERVED_NAMES = /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i;

export const VAULT_IMAGE_DRAG_MIME = "application/x-obsidian-at-home-image";
export const NOTE_IMAGE_DRAG_MIME = "application/x-obsidian-at-home-note-image";

export interface ValidatedImageFolderPath {
  error?: string;
  value: string;
}

export function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).length;
}

export function validateImageFolderPath(input: string): ValidatedImageFolderPath {
  const value = input.trim().replace(/^\/+|\/+$/g, "");
  if (!value) {
    return {
      error: "Enter a folder inside the vault.",
      value,
    };
  }
  if (value.includes("\\")) {
    return {
      error: "Use forward slashes between folders.",
      value,
    };
  }

  const components = value.split("/");
  if (components.length > 120) {
    return { error: "The folder path has too many levels.", value };
  }
  for (const component of components) {
    const normalized = component.trim();
    if (!normalized || normalized === "." || normalized === "..") {
      return { error: "The folder path cannot contain empty, . or .. segments.", value };
    }
    if (
      utf8ByteLength(normalized) > 180
      || normalized.endsWith(".")
      || normalized.endsWith(" ")
      || /[\u0000-\u001f\u007f/:*?"<>|]/u.test(normalized)
      || WINDOWS_RESERVED_NAMES.test(normalized)
      || RESERVED_WORKSPACE_FOLDERS.has(normalized.toLocaleLowerCase())
    ) {
      return { error: `“${component}” is not a safe vault folder name.`, value };
    }
  }

  return { value: components.map((component) => component.trim()).join("/") };
}

export function decodeMarkdownImageDestination(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

export function resolveMarkdownImagePath(
  noteRelativePath: string,
  destination: string,
): string | undefined {
  if (!destination || destination.includes("#") || destination.includes("?")) {
    return undefined;
  }
  const decoded = decodeMarkdownImageDestination(destination);
  if (
    !decoded
    || decoded.startsWith("//")
    || decoded.includes("\\")
    || decoded.includes("?")
    || /[\u0000-\u001f\u007f]/u.test(decoded)
    || /^[A-Za-z][A-Za-z\d+.-]*:/u.test(decoded)
  ) {
    return undefined;
  }
  const parts = decoded.startsWith("/")
    ? []
    : noteRelativePath.split("/").slice(0, -1);
  for (const component of decoded.replace(/^\/+/, "").split("/")) {
    if (!component || component === ".") {
      continue;
    }
    if (component === "..") {
      if (!parts.pop()) {
        return undefined;
      }
    } else {
      parts.push(component);
    }
  }

  return parts.join("/");
}

export function imageAltFromPath(relativePath: string): string {
  const fileName = relativePath.split("/").at(-1) ?? "Image";
  const stem = fileName.replace(/\.[^.]+$/, "").trim();

  return stem || "Image";
}

export function imageMediaTypeForPath(path: string): string {
  const extension = path.split(/[?#]/, 1)[0]?.split(".").at(-1)?.toLocaleLowerCase();
  switch (extension) {
    case "avif": return "image/avif";
    case "bmp": return "image/bmp";
    case "gif": return "image/gif";
    case "jpg":
    case "jpeg": return "image/jpeg";
    case "webp": return "image/webp";
    default: return "image/png";
  }
}

export function pastedImageFileName(now = new Date()): string {
  const stamp = [
    now.getFullYear(),
    String(now.getMonth() + 1).padStart(2, "0"),
    String(now.getDate()).padStart(2, "0"),
    "-",
    String(now.getHours()).padStart(2, "0"),
    String(now.getMinutes()).padStart(2, "0"),
    String(now.getSeconds()).padStart(2, "0"),
  ].join("");

  return `Pasted image ${stamp}.png`;
}
