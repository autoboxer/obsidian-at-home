import type { NoteEditorPosition } from "../types";

const positionsByVault = new Map<string, Map<string, NoteEditorPosition>>();

export function editorPositionVaultId(
  backend: "native" | "browser",
  path: string | null,
): string {
  return backend === "native" ? `native:${path ?? ""}` : "browser";
}

export function getNoteEditorPosition(
  vaultId: string,
  noteId: string,
  content: string,
): NoteEditorPosition | undefined {
  const positions = positionsByVault.get(vaultId);
  const position = positions?.get(noteId);
  const normalized = normalizeNoteEditorPosition(position, normalizedDocumentLength(content));

  if (!normalized && position) {
    positions?.delete(noteId);
  } else if (normalized) {
    positions?.set(noteId, normalized);
  }

  return normalized;
}

export function setNoteEditorPosition(
  vaultId: string,
  noteId: string,
  position: NoteEditorPosition,
): void {
  let positions = positionsByVault.get(vaultId);
  if (!positions) {
    positions = new Map();
    positionsByVault.set(vaultId, positions);
  }

  positions.set(noteId, copyNoteEditorPosition(position));
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

function normalizedDocumentLength(value: string): number {
  return value.replace(/\r\n|\r/g, "\n").length;
}

function clampDocumentOffset(value: number, maximum: number): number {
  return Math.min(maximum, Math.max(0, Math.trunc(value)));
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
  return typeof value === "object" && value !== null;
}
