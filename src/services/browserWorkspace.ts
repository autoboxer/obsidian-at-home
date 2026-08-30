import { normalizeNoteEditorPosition } from "../stores/editorPositions";
import type { Note, RecentlyDeletedNote, VaultData } from "../types";

const STORAGE_KEY = "obsidian-at-home.vault.v1";
const WORKSPACE_VERSION = 2;

export const RECENTLY_DELETED_LIMIT = 100_000;
export const RECENTLY_DELETED_RETENTION = 7 * 24 * 60 * 60 * 1_000;

export interface StoredBrowserWorkspace {
  vault: VaultData;
  recentlyDeletedNotes: RecentlyDeletedNote[];
  migrationFingerprint: string;
  needsRewrite: boolean;
}

export function readBrowserWorkspace(
  normalizeVault: (vault: Partial<VaultData>) => VaultData,
): StoredBrowserWorkspace | null {
  const raw = readCriticalStorage();
  if (!raw) {
    return null;
  }

  try {
    const parsed = JSON.parse(raw) as unknown;
    if (isRecord(parsed) && "version" in parsed) {
      if (parsed.version !== WORKSPACE_VERSION) {
        throw new Error(
          `Saved browser notes use unsupported version ${String(parsed.version)}.`,
        );
      }
      if (!isRecord(parsed.vault)) {
        throw new Error("Saved browser notes contain an invalid vault.");
      }

      const parsedVault = parsed.vault as Partial<VaultData>;
      if (!Array.isArray(parsedVault.notes) || !Array.isArray(parsedVault.folders)) {
        throw new Error("Saved browser notes contain an invalid vault.");
      }
      const vault = normalizeVault(parsedVault);

      return {
        vault,
        recentlyDeletedNotes: parseRecentlyDeletedNotes(parsed.recentlyDeletedNotes),
        migrationFingerprint: storageFingerprint(JSON.stringify(vault)),
        needsRewrite: parsedVault.selectedFolderId !== vault.selectedFolderId
          || !stringArraysMatch(parsedVault.recentNoteIds, vault.recentNoteIds)
          || !embedSettingsMatch(
            parsedVault.imageEmbedSettings,
            vault.imageEmbedSettings,
          )
          || !embedSettingsMatch(
            parsedVault.attachmentEmbedSettings,
            vault.attachmentEmbedSettings,
          ),
      };
    }

    if (!isRecord(parsed) || !Array.isArray(parsed.notes) || !Array.isArray(parsed.folders)) {
      throw new Error("Saved browser notes have an invalid format.");
    }
    const vault = normalizeVault(parsed as Partial<VaultData>);

    return {
      vault,
      recentlyDeletedNotes: [],
      migrationFingerprint: storageFingerprint(JSON.stringify(vault)),
      needsRewrite: true,
    };
  } catch (error) {
    throw new Error(
      error instanceof Error && error.message.startsWith("Saved browser notes")
        ? error.message
        : "Saved browser notes could not be parsed safely.",
    );
  }
}

export function writeBrowserWorkspace(
  vault: VaultData,
  recentlyDeletedNotes: RecentlyDeletedNote[],
): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({
    version: WORKSPACE_VERSION,
    vault,
    recentlyDeletedNotes,
  }));
}

export function compareRecentlyDeletedNotes(
  first: RecentlyDeletedNote,
  second: RecentlyDeletedNote,
): number {
  return second.deletedAt - first.deletedAt || second.id.localeCompare(first.id);
}

function parseRecentlyDeletedNotes(value: unknown): RecentlyDeletedNote[] {
  if (!Array.isArray(value) || value.length > RECENTLY_DELETED_LIMIT) {
    throw new Error("Saved browser notes contain an invalid Recently Deleted collection.");
  }

  const ids = new Set<string>();
  const notes = value.map((entry) => parseRecentlyDeletedNote(entry));
  for (const entry of notes) {
    if (ids.has(entry.id)) {
      throw new Error("Saved browser notes contain duplicate Recently Deleted entries.");
    }
    ids.add(entry.id);
  }

  return notes.sort(compareRecentlyDeletedNotes);
}

function parseRecentlyDeletedNote(value: unknown): RecentlyDeletedNote {
  if (
    !isRecord(value)
    || typeof value.id !== "string"
    || !/^[A-Za-z0-9_-]{1,180}$/u.test(value.id)
    || typeof value.originalFolderPath !== "string"
    || !isSafeRelativePath(value.originalFolderPath, true)
    || !isSafeTimestamp(value.deletedAt)
    || !isSafeTimestamp(value.expiresAt)
    || value.expiresAt !== value.deletedAt + RECENTLY_DELETED_RETENTION
  ) {
    throw new Error("Saved browser notes contain invalid Recently Deleted metadata.");
  }

  const note = parseDeletedNote(value.note);
  const editorPosition = value.editorPosition === undefined
    ? undefined
    : normalizeNoteEditorPosition(
        value.editorPosition,
        note.content.replace(/\r\n|\r/g, "\n").length,
      );
  if (value.editorPosition !== undefined && !editorPosition) {
    throw new Error("Saved browser notes contain an invalid editor position.");
  }

  return {
    id: value.id,
    note,
    originalFolderPath: value.originalFolderPath,
    deletedAt: value.deletedAt,
    expiresAt: value.expiresAt,
    ...(editorPosition ? { editorPosition } : {}),
  };
}

function parseDeletedNote(value: unknown): Note {
  if (
    !isRecord(value)
    || typeof value.id !== "string"
    || !value.id.trim()
    || typeof value.title !== "string"
    || typeof value.content !== "string"
    || typeof value.relativePath !== "string"
    || !isSafeRelativePath(value.relativePath, true)
    || (value.folderId !== null && typeof value.folderId !== "string")
    || !Array.isArray(value.tags)
    || value.tags.some((tag) => typeof tag !== "string")
    || typeof value.pinned !== "boolean"
    || !isSafeTimestamp(value.createdAt)
    || !isSafeTimestamp(value.updatedAt)
  ) {
    throw new Error("Saved browser notes contain an invalid deleted note.");
  }

  return {
    id: value.id,
    title: value.title,
    content: value.content,
    relativePath: value.relativePath,
    folderId: value.folderId,
    tags: [...value.tags] as string[],
    pinned: value.pinned,
    createdAt: value.createdAt,
    updatedAt: value.updatedAt,
  };
}

function readCriticalStorage(): string | null {
  if (typeof localStorage === "undefined") {
    throw new Error("Saved browser notes are unavailable in this environment.");
  }
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    throw new Error("Saved browser notes could not be read from browser storage.");
  }
}

function isSafeRelativePath(value: string, allowEmpty: boolean): boolean {
  if (!value) {
    return allowEmpty;
  }
  if (
    value.startsWith("/")
    || value.includes("\\")
    || /[\u0000-\u001f\u007f]/u.test(value)
  ) {
    return false;
  }

  return value.split("/").every((part) => part && part !== "." && part !== "..");
}

function isSafeTimestamp(value: unknown): value is number {
  return typeof value === "number"
    && Number.isSafeInteger(value)
    && value >= 0;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringArraysMatch(value: unknown, expected: string[]): boolean {
  return Array.isArray(value)
    && value.length === expected.length
    && value.every((item, index) => item === expected[index]);
}

function embedSettingsMatch(
  value: unknown,
  expected: VaultData["imageEmbedSettings"],
): boolean {
  return isRecord(value)
    && value.location === expected.location
    && value.folderPath === expected.folderPath;
}

function storageFingerprint(value: string): string {
  let hash = 2_166_136_261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16_777_619);
  }

  return `v1-${value.length}-${(hash >>> 0).toString(16)}`;
}
