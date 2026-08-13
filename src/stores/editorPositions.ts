import { saveWorkspaceEditorPositions } from "../services/native";
import type { Note, NoteEditorPosition } from "../types";

const BROWSER_STORAGE_KEY = "obsidian-at-home.editor-positions.v1";
const STORAGE_VERSION = 1;
const PERSIST_DELAY = 600;

interface EditorPositionVault {
  backend: "native" | "browser";
  path: string | null;
  positions: Map<string, NoteEditorPosition>;
  dirtyVersion: number;
  savedVersion: number;
  persistTimer?: ReturnType<typeof setTimeout>;
  saveInFlight: Promise<PersistPositionResult> | null;
  saveInFlightVersion: number;
  storageRevision: string | null;
  writable: boolean;
}

interface NormalizedPositions {
  positions: Map<string, NoteEditorPosition>;
  changed: boolean;
}

interface StoredPositionsSource {
  revision: string | null;
  value: unknown;
  writable: boolean;
  needsRewrite: boolean;
}

interface PersistPositionResult {
  revision: string | null;
  saved: boolean;
}

interface EditorPositionCapture {
  vaultId: string;
  noteId: string;
  read: () => NoteEditorPosition | undefined;
}

const positionVaults = new Map<string, EditorPositionVault>();
const positionCaptures = new Set<EditorPositionCapture>();

export function editorPositionVaultId(
  backend: "native" | "browser",
  path: string | null,
): string {
  return backend === "native" ? `native:${path ?? ""}` : "browser";
}

export function initializeNoteEditorPositions(
  backend: "native" | "browser",
  path: string | null,
  notes: Note[],
  storedPositions?: unknown,
  storedPositionsWritable = true,
  storedPositionsRevision: string | null = null,
): string {
  const vaultId = editorPositionVaultId(backend, path);
  const source = backend === "browser"
    ? readBrowserPositions()
    : {
        value: storedPositions,
        writable: storedPositionsWritable,
        needsRewrite: false,
        revision: storedPositionsRevision,
      };
  const normalized = normalizePositions(source.value, notes);
  const existing = positionVaults.get(vaultId);

  if (!existing) {
    const vault: EditorPositionVault = {
      backend,
      path,
      positions: normalized.positions,
      dirtyVersion: 0,
      savedVersion: 0,
      saveInFlight: null,
      saveInFlightVersion: 0,
      storageRevision: source.revision,
      writable: source.writable,
    };
    positionVaults.set(vaultId, vault);
    if (source.needsRewrite || normalized.changed) {
      markPositionsDirty(vault);
    }

    return vaultId;
  }

  const preserveUnsaved = source.writable
    && existing.writable
    && existing.storageRevision === source.revision
    && hasUnsavedPositions(existing);

  existing.backend = backend;
  existing.path = path;
  existing.writable = source.writable;
  if (!source.writable) {
    clearTimeout(existing.persistTimer);
    existing.persistTimer = undefined;
  }

  if (preserveUnsaved) {
    for (const [noteId, position] of normalized.positions) {
      if (!existing.positions.has(noteId)) {
        existing.positions.set(noteId, position);
      }
    }
  } else {
    existing.positions = normalized.positions;
    existing.dirtyVersion = 0;
    existing.savedVersion = 0;
  }
  existing.storageRevision = source.revision;

  const pruned = prunePositionMap(existing.positions, new Set(notes.map((note) => note.id)));
  if (source.needsRewrite || normalized.changed || pruned) {
    markPositionsDirty(existing);
  }

  return vaultId;
}

export function getNoteEditorPosition(
  vaultId: string,
  noteId: string,
  content: string,
): NoteEditorPosition | undefined {
  const vault = positionVaults.get(vaultId);
  const position = vault?.positions.get(noteId);
  const normalized = normalizeNoteEditorPosition(position, normalizedDocumentLength(content));

  if (!vault || !position) {
    return normalized;
  }
  if (!normalized) {
    vault.positions.delete(noteId);
    markPositionsDirty(vault);
  } else if (!editorPositionsMatch(position, normalized)) {
    vault.positions.set(noteId, normalized);
    markPositionsDirty(vault);
  }

  return normalized;
}

export function setNoteEditorPosition(
  vaultId: string,
  noteId: string,
  position: NoteEditorPosition,
): void {
  const normalized = normalizeNoteEditorPosition(position, Number.MAX_SAFE_INTEGER);
  if (!normalized) {
    return;
  }

  const vault = positionVaults.get(vaultId) ?? createTransientVault(vaultId);
  const previous = vault.positions.get(noteId);
  if (previous && editorPositionsMatch(previous, normalized)) {
    return;
  }

  vault.positions.set(noteId, normalized);
  markPositionsDirty(vault);
}

export function registerNoteEditorPositionCapture(
  vaultId: string,
  noteId: string,
  read: () => NoteEditorPosition | undefined,
): () => boolean {
  const capture = { vaultId, noteId, read };
  positionCaptures.add(capture);

  return () => positionCaptures.delete(capture);
}

export function deleteNoteEditorPosition(vaultId: string, noteId: string): void {
  removePositionCaptures(vaultId, new Set([noteId]));
  const vault = positionVaults.get(vaultId);
  if (vault?.positions.delete(noteId)) {
    markPositionsDirty(vault);
  }
}

export function pruneNoteEditorPositions(vaultId: string, notes: Note[]): void {
  const noteIds = new Set(notes.map((note) => note.id));
  removePositionCaptures(vaultId, noteIds, true);
  const vault = positionVaults.get(vaultId);
  if (
    vault
    && prunePositionMap(vault.positions, noteIds)
  ) {
    markPositionsDirty(vault);
  }
}

export function hasPendingNoteEditorPositions(): boolean {
  captureOpenEditorPositions();

  return [...positionVaults.values()].some(hasUnsavedPositions);
}

export async function flushNoteEditorPositions(vaultId?: string): Promise<boolean> {
  captureOpenEditorPositions(vaultId);
  const vaults = vaultId
    ? [positionVaults.get(vaultId)].filter((vault): vault is EditorPositionVault => Boolean(vault))
    : [...positionVaults.values()];
  const results = await Promise.all(vaults.map(flushPositionVault));

  return results.every(Boolean);
}

function captureOpenEditorPositions(vaultId?: string): void {
  for (const capture of positionCaptures) {
    if (vaultId && capture.vaultId !== vaultId) {
      continue;
    }

    const position = capture.read();
    if (position) {
      setNoteEditorPosition(capture.vaultId, capture.noteId, position);
    }
  }
}

function removePositionCaptures(
  vaultId: string,
  noteIds: Set<string>,
  keepMatching = false,
): void {
  for (const capture of positionCaptures) {
    if (
      capture.vaultId === vaultId
      && noteIds.has(capture.noteId) !== keepMatching
    ) {
      positionCaptures.delete(capture);
    }
  }
}

export function normalizeNoteEditorPosition(
  value: unknown,
  documentLength: number,
): NoteEditorPosition | undefined {
  if (!isRecord(value) || !isRecord(value.selection) || !isRecord(value.viewport)) {
    return undefined;
  }

  const { anchor, head } = value.selection;
  const {
    anchor: viewportAnchor,
    offset,
    left,
  } = value.viewport;
  if (
    !isFiniteNumber(anchor)
    || !isFiniteNumber(head)
    || !isFiniteNumber(viewportAnchor)
    || !isFiniteNumber(offset)
    || !isFiniteNumber(left)
  ) {
    return undefined;
  }

  const maximum = Math.max(0, Math.trunc(documentLength));

  return {
    selection: {
      anchor: clampDocumentOffset(anchor, maximum),
      head: clampDocumentOffset(head, maximum),
    },
    viewport: {
      anchor: clampDocumentOffset(viewportAnchor, maximum),
      offset,
      left: Math.max(0, left),
    },
  };
}

function normalizePositions(value: unknown, notes: Note[]): NormalizedPositions {
  const positions = new Map<string, NoteEditorPosition>();
  if (value === undefined || value === null) {
    return { positions, changed: false };
  }
  if (!isRecord(value)) {
    return { positions, changed: true };
  }

  const notesById = new Map(notes.map((note) => [note.id, note]));
  let changed = false;
  for (const [noteId, storedPosition] of Object.entries(value)) {
    const note = notesById.get(noteId);
    if (!note) {
      changed = true;
      continue;
    }

    const normalized = normalizeNoteEditorPosition(
      storedPosition,
      normalizedDocumentLength(note.content),
    );
    if (!normalized) {
      changed = true;
      continue;
    }

    positions.set(noteId, normalized);
    changed ||= !editorPositionsMatch(storedPosition, normalized);
  }

  return { positions, changed };
}

function normalizedDocumentLength(value: string): number {
  return value.replace(/\r\n|\r/g, "\n").length;
}

function clampDocumentOffset(value: number, maximum: number): number {
  return Math.min(maximum, Math.max(0, Math.trunc(value)));
}

function createTransientVault(vaultId: string): EditorPositionVault {
  const native = vaultId.startsWith("native:");
  const vault: EditorPositionVault = {
    backend: native ? "native" : "browser",
    path: native ? vaultId.slice("native:".length) || null : null,
    positions: new Map(),
    dirtyVersion: 0,
    savedVersion: 0,
    saveInFlight: null,
    saveInFlightVersion: 0,
    storageRevision: null,
    writable: true,
  };
  positionVaults.set(vaultId, vault);

  return vault;
}

function prunePositionMap(
  positions: Map<string, NoteEditorPosition>,
  noteIds: Set<string>,
): boolean {
  let changed = false;
  for (const noteId of positions.keys()) {
    if (!noteIds.has(noteId)) {
      positions.delete(noteId);
      changed = true;
    }
  }

  return changed;
}

function markPositionsDirty(vault: EditorPositionVault): void {
  if (!vault.writable) {
    return;
  }

  vault.dirtyVersion += 1;
  clearTimeout(vault.persistTimer);
  vault.persistTimer = setTimeout(() => {
    vault.persistTimer = undefined;
    void flushPositionVault(vault);
  }, PERSIST_DELAY);
}

async function flushPositionVault(vault: EditorPositionVault): Promise<boolean> {
  clearTimeout(vault.persistTimer);
  vault.persistTimer = undefined;

  if (!hasUnsavedPositions(vault)) {
    return true;
  }
  if (vault.saveInFlight) {
    const inFlightVersion = vault.saveInFlightVersion;
    const { saved } = await vault.saveInFlight;

    return hasUnsavedPositions(vault) && (saved || vault.dirtyVersion > inFlightVersion)
      ? flushPositionVault(vault)
      : saved;
  }

  const targetVersion = vault.dirtyVersion;
  const snapshot = positionSnapshot(vault.positions);
  const operation = persistPositionSnapshot(
    vault,
    snapshot,
    vault.storageRevision,
  );
  vault.saveInFlight = operation;
  vault.saveInFlightVersion = targetVersion;
  const result = await operation;
  if (vault.saveInFlight !== operation) {
    return flushPositionVault(vault);
  }
  vault.saveInFlight = null;
  if (!result.saved) {
    return vault.dirtyVersion > targetVersion
      ? flushPositionVault(vault)
      : false;
  }

  vault.storageRevision = result.revision;
  vault.savedVersion = Math.max(vault.savedVersion, targetVersion);

  return hasUnsavedPositions(vault) ? flushPositionVault(vault) : true;
}

async function persistPositionSnapshot(
  vault: EditorPositionVault,
  positions: Record<string, NoteEditorPosition>,
  expectedRevision: string | null,
): Promise<PersistPositionResult> {
  try {
    if (vault.backend === "browser") {
      persistBrowserPositions(positions);
    } else if (vault.path) {
      const revision = await saveWorkspaceEditorPositions(
        vault.path,
        positions,
        expectedRevision,
      );

      return { saved: true, revision };
    } else {
      return { saved: false, revision: expectedRevision };
    }

    return { saved: true, revision: expectedRevision };
  } catch (error) {
    console.warn("Could not save editor positions", error);

    return { saved: false, revision: expectedRevision };
  }
}

function positionSnapshot(
  positions: Map<string, NoteEditorPosition>,
): Record<string, NoteEditorPosition> {
  return Object.fromEntries(
    [...positions.entries()]
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([noteId, position]) => [noteId, copyNoteEditorPosition(position)]),
  );
}

function readBrowserPositions(): StoredPositionsSource {
  let raw: string | null;
  try {
    raw = window.localStorage.getItem(BROWSER_STORAGE_KEY);
  } catch {
    return { value: undefined, writable: false, needsRewrite: false, revision: null };
  }
  if (!raw) {
    return { value: undefined, writable: true, needsRewrite: false, revision: null };
  }

  try {
    const stored = JSON.parse(raw) as unknown;
    if (!isRecord(stored) || !Number.isInteger(stored.version)) {
      return { value: {}, writable: true, needsRewrite: true, revision: null };
    }
    const version = stored.version as number;
    if (version > STORAGE_VERSION) {
      return { value: undefined, writable: false, needsRewrite: false, revision: null };
    }
    if (version < 1) {
      return { value: {}, writable: true, needsRewrite: true, revision: null };
    }

    return {
      value: stored.positions,
      writable: true,
      needsRewrite: version < STORAGE_VERSION || !isRecord(stored.positions),
      revision: null,
    };
  } catch {
    return { value: {}, writable: true, needsRewrite: true, revision: null };
  }
}

function persistBrowserPositions(positions: Record<string, NoteEditorPosition>): void {
  if (typeof localStorage === "undefined") {
    throw new Error("Browser storage is unavailable.");
  }
  if (!Object.keys(positions).length) {
    localStorage.removeItem(BROWSER_STORAGE_KEY);

    return;
  }

  localStorage.setItem(BROWSER_STORAGE_KEY, JSON.stringify({
    version: STORAGE_VERSION,
    positions,
  }));
}

function hasUnsavedPositions(vault: EditorPositionVault): boolean {
  return vault.writable
    && (vault.dirtyVersion > vault.savedVersion || vault.saveInFlight !== null);
}

function editorPositionsMatch(left: unknown, right: NoteEditorPosition): boolean {
  if (!isRecord(left) || !isRecord(left.selection) || !isRecord(left.viewport)) {
    return false;
  }

  return left.selection.anchor === right.selection.anchor
    && left.selection.head === right.selection.head
    && left.viewport.anchor === right.viewport.anchor
    && left.viewport.offset === right.viewport.offset
    && left.viewport.left === right.viewport.left;
}

function copyNoteEditorPosition(position: NoteEditorPosition): NoteEditorPosition {
  return {
    selection: { ...position.selection },
    viewport: { ...position.viewport },
  };
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
