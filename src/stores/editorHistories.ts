import type { Note } from "../types";

export interface NoteEditorHistorySnapshot {
  doc: string;
  frontmatterHistoryChanged: boolean;
  history: unknown;
  prefix: string;
  selection: unknown;
}

export interface NoteEditorHistorySession {
  snapshot?: NoteEditorHistorySnapshot;
  close: (snapshot: NoteEditorHistorySnapshot) => void;
  discard: () => void;
}

interface ActiveHistorySession {
  valid: boolean;
}

const MAX_TRACKED_DOCUMENTS = 250;
const historyVaults = new Map<string, Map<string, NoteEditorHistorySnapshot>>();
const activeHistorySessions = new Map<string, Map<string, Set<ActiveHistorySession>>>();
const historyRecency = new Map<string, { noteId: string; vaultId: string }>();

export function openNoteEditorHistory(
  vaultId: string,
  noteId: string,
): NoteEditorHistorySession {
  const snapshot = historyVaults.get(vaultId)?.get(noteId);
  if (snapshot) {
    touchSnapshot(vaultId, noteId);
  }

  const session: ActiveHistorySession = { valid: true };
  const vaultSessions = activeHistorySessions.get(vaultId) ?? new Map();
  const noteSessions = vaultSessions.get(noteId) ?? new Set();
  noteSessions.add(session);
  vaultSessions.set(noteId, noteSessions);
  activeHistorySessions.set(vaultId, vaultSessions);

  return {
    snapshot,
    discard() {
      removeSnapshot(vaultId, noteId);
    },
    close(nextSnapshot) {
      removeActiveSession(vaultId, noteId, session);
      if (!session.valid) {
        return;
      }
      session.valid = false;
      if (!snapshotHasHistory(nextSnapshot)) {
        removeSnapshot(vaultId, noteId);

        return;
      }
      const nextHistories = historyVaults.get(vaultId) ?? new Map();
      nextHistories.set(noteId, nextSnapshot);
      historyVaults.set(vaultId, nextHistories);
      touchSnapshot(vaultId, noteId);
      trimHistorySnapshots();
    },
  };
}

export function deleteNoteEditorHistory(vaultId: string, noteId: string): void {
  invalidateActiveSessions(vaultId, noteId);
  removeSnapshot(vaultId, noteId);
}

export function pruneNoteEditorHistories(vaultId: string, notes: Note[]): void {
  const histories = historyVaults.get(vaultId);
  const vaultSessions = activeHistorySessions.get(vaultId);
  if (!histories && !vaultSessions) {
    return;
  }

  const noteIds = new Set(notes.map((note) => note.id));
  for (const noteId of histories?.keys() ?? []) {
    if (!noteIds.has(noteId)) {
      invalidateActiveSessions(vaultId, noteId);
      removeSnapshot(vaultId, noteId);
    }
  }
  for (const noteId of vaultSessions?.keys() ?? []) {
    if (!noteIds.has(noteId)) {
      invalidateActiveSessions(vaultId, noteId);
    }
  }
  removeEmptyVault(vaultId, histories);
}

function snapshotHasHistory(snapshot: NoteEditorHistorySnapshot): boolean {
  if (!snapshot.history || typeof snapshot.history !== "object") {
    return false;
  }
  const history = snapshot.history as { done?: unknown; undone?: unknown };

  return (Array.isArray(history.done) && history.done.length > 0)
    || (Array.isArray(history.undone) && history.undone.length > 0);
}

function touchSnapshot(vaultId: string, noteId: string): void {
  const key = historyKey(vaultId, noteId);
  historyRecency.delete(key);
  historyRecency.set(key, { vaultId, noteId });
}

function trimHistorySnapshots(): void {
  while (historyRecency.size > MAX_TRACKED_DOCUMENTS) {
    const oldest = historyRecency.values().next().value;
    if (!oldest) {
      return;
    }
    removeSnapshot(oldest.vaultId, oldest.noteId);
  }
}

function removeSnapshot(vaultId: string, noteId: string): void {
  const histories = historyVaults.get(vaultId);
  histories?.delete(noteId);
  historyRecency.delete(historyKey(vaultId, noteId));
  removeEmptyVault(vaultId, histories);
}

function historyKey(vaultId: string, noteId: string): string {
  return JSON.stringify([vaultId, noteId]);
}

function invalidateActiveSessions(vaultId: string, noteId: string): void {
  const vaultSessions = activeHistorySessions.get(vaultId);
  const noteSessions = vaultSessions?.get(noteId);
  for (const session of noteSessions ?? []) {
    session.valid = false;
  }
  vaultSessions?.delete(noteId);
  if (vaultSessions?.size === 0) {
    activeHistorySessions.delete(vaultId);
  }
}

function removeActiveSession(
  vaultId: string,
  noteId: string,
  session: ActiveHistorySession,
): void {
  const vaultSessions = activeHistorySessions.get(vaultId);
  const noteSessions = vaultSessions?.get(noteId);
  noteSessions?.delete(session);
  if (noteSessions?.size === 0) {
    vaultSessions?.delete(noteId);
  }
  if (vaultSessions?.size === 0) {
    activeHistorySessions.delete(vaultId);
  }
}

function removeEmptyVault(
  vaultId: string,
  histories: Map<string, NoteEditorHistorySnapshot> | undefined,
): void {
  if (histories && histories.size === 0) {
    historyVaults.delete(vaultId);
  }
}
