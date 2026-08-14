import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { computed, reactive, watch } from "vue";
import { createEmptyVault, createSeedVault } from "../data/seed";
import { findBacklinks, parseWikiLinks, resolveWikiLink, searchNotes } from "../lib";
import {
  deleteNoteEditorPosition,
  editorPositionVaultId,
  flushNoteEditorPositions,
  hasPendingNoteEditorPositions,
  initializeNoteEditorPositions,
  pruneNoteEditorPositions,
} from "./editorPositions";
import {
  bootstrapWorkspace,
  createWorkspace,
  forgetWorkspace,
  getWorkspaceRevision,
  isTauri,
  openWorkspace,
  pickFolder,
  saveWorkspace,
} from "../services/native";
import type {
  CssSnippet,
  ExportNote,
  ExportSnippet,
  ExportTemplate,
  Folder,
  ImportResult,
  Note,
  NoteTemplate,
  SearchScope,
  ToolView,
  VaultData,
  VaultDescriptor,
  VaultSessionState,
  WorkspaceLoad,
} from "../types";

const STORAGE_KEY = "obsidian-at-home.vault.v1";
const LEGACY_MIGRATED_KEY = "obsidian-at-home.vault.filesystem-migrated.v1";
const APP_ZOOM_KEY = "obsidian-at-home.zoom.v1";
const PERSIST_DELAY = 220;
const EXTERNAL_CHECK_DELAY = 3_000;
const NOTE_NAVIGATION_LIMIT = 100;
const RECENT_NOTE_LIMIT = 10;

export const MIN_ZOOM = 0.7;
export const MAX_ZOOM = 1.5;
const ZOOM_STEP = 0.1;

type FolderSelection = VaultData["selectedFolderId"];
type SmartFolderSelection = "all" | "favorites" | "recent";
type SaveStatus = "saved" | "saving" | "error";
type ToastTone = "neutral" | "success" | "warning";

export const NOTE_DRAG_MIME = "application/x-obsidian-at-home-note-id";
export const FOLDER_DRAG_MIME = "application/x-obsidian-at-home-folder-id";
export const treeDragState = reactive<{ noteId: string | null; folderId: string | null }>({
  noteId: null,
  folderId: null,
});

interface UiState {
  tool: ToolView;
  noteFilter: string;
  commandOpen: boolean;
  contextOpen: boolean;
  explorerOpen: boolean;
  frontmatterVisible: boolean;
  vaultChooserOpen: boolean;
  inspectorTab: "links" | "info";
  saveStatus: SaveStatus;
  lastSavedAt: number;
  zoom: number;
  toast: { id: number; message: string; tone: ToastTone } | null;
}

interface SearchState {
  query: string;
  scope: SearchScope;
  exactTag: string | null;
  quickQuery: string;
  focusRequest: number;
}

interface NoteNavigationState {
  back: string[];
  forward: string[];
}

export const vaultState = reactive<VaultData>(createEmptyVault());

export const vaultSession = reactive<VaultSessionState>({
  phase: "loading",
  backend: isTauri() ? "native" : "browser",
  path: null,
  recentVaults: [],
  error: null,
  busy: false,
  legacyAvailable: false,
  revision: 0,
  conflict: false,
  warnings: [],
});

export const uiState = reactive<UiState>({
  tool: "notes",
  noteFilter: "",
  commandOpen: false,
  contextOpen: true,
  explorerOpen: true,
  frontmatterVisible: false,
  vaultChooserOpen: false,
  inspectorTab: "links",
  saveStatus: "saved",
  lastSavedAt: Date.now(),
  zoom: readStoredZoom(),
  toast: null,
});

export const searchState = reactive<SearchState>({
  query: "",
  scope: "all",
  exactTag: null,
  quickQuery: "",
  focusRequest: 0,
});

const noteNavigationState = reactive<NoteNavigationState>({
  back: [],
  forward: [],
});

let persistTimer: ReturnType<typeof setTimeout> | undefined;
let toastTimer: ReturnType<typeof setTimeout> | undefined;
let externalCheckTimer: ReturnType<typeof setInterval> | undefined;
let initialized = false;
let suppressPersistence = 0;
let dirtyVersion = 0;
let savedVersion = 0;
let sessionGeneration = 0;
let saveInFlight: Promise<boolean> | null = null;
let checkingExternalChanges = false;
let initializePromise: Promise<void> | null = null;
let closeHandlerInstalled = false;
let closingAfterSave = false;

watch(
  vaultState,
  () => {
    if (!initialized || suppressPersistence) {
      return;
    }
    dirtyVersion += 1;
    uiState.saveStatus = "saving";
    clearTimeout(persistTimer);
    persistTimer = setTimeout(() => void flushApplicationState(), PERSIST_DELAY);
  },
  { deep: true, flush: "sync" },
);

watch(
  () => vaultState.snippets.map((snippet) => [snippet.id, snippet.enabled, snippet.css]),
  applyEnabledSnippets,
  { deep: true, immediate: true },
);

watch(
  () => uiState.zoom,
  (zoom) => safeStorageSet(APP_ZOOM_KEY, String(zoom)),
  { flush: "sync" },
);

export function initializeVault(): Promise<void> {
  if (initializePromise) {
    return initializePromise;
  }
  initializePromise = initializeVaultStorage();

  return initializePromise;
}

async function initializeVaultStorage(): Promise<void> {
  vaultSession.error = null;
  vaultSession.phase = "loading";

  if (!isTauri()) {
    const storedVault = readStoredVault();
    const browserVault = storedVault?.vault ?? createSeedVault();
    hydrateVault(browserVault);
    initializeNoteEditorPositions("browser", null, vaultState.notes);
    resetNoteNavigation();
    if (storedVault?.needsRewrite) {
      safeStorageSet(STORAGE_KEY, JSON.stringify(browserVault));
    }
    vaultSession.backend = "browser";
    vaultSession.phase = "ready";
    vaultSession.path = null;
    vaultSession.recentVaults = [];
    vaultSession.legacyAvailable = false;
    vaultSession.revision = 0;
    vaultSession.conflict = false;
    vaultSession.warnings = [];
    initialized = true;
    savedVersion = dirtyVersion;
    installVaultLifecycleHandlers();

    return;
  }

  vaultSession.backend = "native";
  const legacy = readStoredVault();
  vaultSession.legacyAvailable = Boolean(
    legacy && safeStorageGet(LEGACY_MIGRATED_KEY) !== storageFingerprint(legacy.raw),
  );

  try {
    const result = await bootstrapWorkspace(createEmptyVault());
    vaultSession.recentVaults = result.recentVaults;
    if (result.workspace) {
      applyWorkspace(result.workspace, result.recentVaults);
    } else {
      hydrateVault(createEmptyVault());
      resetNoteNavigation();
      vaultSession.phase = "needs-vault";
      vaultSession.path = null;
      vaultSession.revision = 0;
      vaultSession.conflict = false;
      vaultSession.warnings = [];
      uiState.vaultChooserOpen = true;
    }
  } catch (error) {
    hydrateVault(createEmptyVault());
    resetNoteNavigation();
    vaultSession.phase = "error";
    vaultSession.error = errorMessage(error, "The vault list could not be opened.");
    uiState.vaultChooserOpen = true;
  } finally {
    initialized = true;
    savedVersion = dirtyVersion;
    installVaultLifecycleHandlers();
  }
}

export async function createFilesystemVault(name: string, useLegacy = false): Promise<boolean> {
  if (vaultSession.backend !== "native" || vaultSession.busy) {
    return false;
  }
  const cleanName = name.trim();
  if (!cleanName) {
    return false;
  }
  if (!(await flushBeforeVaultChange())) {
    return false;
  }

  vaultSession.busy = true;
  vaultSession.error = null;
  try {
    const parentPath = await pickFolder();
    if (!parentPath) {
      return false;
    }
    const legacy = useLegacy ? readStoredVault() : null;
    if (useLegacy && !legacy) {
      throw new Error("The previous notes could not be read from app storage.");
    }
    const initial = legacy?.vault ?? createSeedVault();
    const workspace = await createWorkspace(parentPath, cleanName, initial);
    applyWorkspace(workspace);

    if (useLegacy && legacy) {
      safeStorageSet(LEGACY_MIGRATED_KEY, storageFingerprint(legacy.raw));
      vaultSession.legacyAvailable = false;
      notify(`Saved ${legacy.vault.notes.length} ${legacy.vault.notes.length === 1 ? "note" : "notes"} as Markdown files`, "success");
    } else {
      notify(`Created ${workspace.descriptor.name}`, "success");
    }

    return true;
  } catch (error) {
    setVaultError(error, "The vault could not be created.");

    return false;
  } finally {
    vaultSession.busy = false;
  }
}

export async function openFilesystemVault(): Promise<boolean> {
  if (vaultSession.backend !== "native" || vaultSession.busy) {
    return false;
  }
  if (!(await flushBeforeVaultChange())) {
    return false;
  }

  vaultSession.busy = true;
  vaultSession.error = null;
  try {
    const path = await pickFolder();
    if (!path) {
      return false;
    }
    const workspace = await openWorkspace(path, createEmptyVault());
    applyWorkspace(workspace);
    notify(`Opened ${workspace.descriptor.name}`, "success");

    return true;
  } catch (error) {
    setVaultError(error, "That folder could not be opened as a vault.");

    return false;
  } finally {
    vaultSession.busy = false;
  }
}

export async function switchFilesystemVault(path: string): Promise<boolean> {
  if (vaultSession.backend !== "native" || vaultSession.busy || path === vaultSession.path) {
    return path === vaultSession.path;
  }
  if (!(await flushBeforeVaultChange())) {
    return false;
  }

  vaultSession.busy = true;
  vaultSession.error = null;
  try {
    const workspace = await openWorkspace(path, createEmptyVault());
    applyWorkspace(workspace);
    notify(`Switched to ${workspace.descriptor.name}`, "success");

    return true;
  } catch (error) {
    setVaultError(error, "That recent vault is no longer available.");

    return false;
  } finally {
    vaultSession.busy = false;
  }
}

export async function forgetCurrentVault(): Promise<boolean> {
  const path = vaultSession.path;
  if (vaultSession.backend !== "native" || !path || vaultSession.busy) {
    return false;
  }
  if (!(await flushBeforeVaultChange())) {
    return false;
  }

  vaultSession.busy = true;
  vaultSession.error = null;
  try {
    const recentVaults = await forgetWorkspace(path);
    sessionGeneration += 1;
    vaultSession.recentVaults = recentVaults;
    vaultSession.path = null;
    vaultSession.revision = 0;
    vaultSession.conflict = false;
    vaultSession.warnings = [];
    vaultSession.phase = "needs-vault";
    hydrateVault(createEmptyVault());
    resetNoteNavigation();
    dirtyVersion = 0;
    savedVersion = 0;
    uiState.vaultChooserOpen = true;
    notify("Vault forgotten; its files are still on disk", "neutral");

    return true;
  } catch (error) {
    setVaultError(error, "The vault could not be removed from the recent list.");

    return false;
  } finally {
    vaultSession.busy = false;
  }
}

export async function showCurrentVaultInFolder(): Promise<void> {
  if (!vaultSession.path || vaultSession.backend !== "native") {
    return;
  }
  try {
    await revealItemInDir(vaultSession.path);
  } catch (error) {
    setVaultError(error, "The vault folder could not be shown.");
    throw error;
  }
}

export async function reloadFilesystemVault(): Promise<boolean> {
  const path = vaultSession.path;
  if (vaultSession.backend !== "native" || !path || vaultSession.busy) {
    return false;
  }
  await flushNoteEditorPositions(currentEditorPositionVaultId());
  vaultSession.busy = true;
  try {
    const workspace = await openWorkspace(path, createEmptyVault());
    applyWorkspace(workspace);
    notify("Reloaded the vault from disk", "success");

    return true;
  } catch (error) {
    setVaultError(error, "The vault could not be reloaded from disk.");

    return false;
  } finally {
    vaultSession.busy = false;
  }
}

export async function overwriteFilesystemVault(): Promise<boolean> {
  const path = vaultSession.path;
  if (vaultSession.backend !== "native" || !path || vaultSession.busy) {
    return false;
  }
  vaultSession.busy = true;
  clearTimeout(persistTimer);
  try {
    const targetVersion = dirtyVersion;
    const currentRevision = await getWorkspaceRevision(path);
    const result = await saveWorkspace(path, snapshotVault(), currentRevision);
    vaultSession.revision = result.revision;
    vaultSession.error = null;
    vaultSession.conflict = false;
    vaultSession.warnings = result.warnings;
    savedVersion = targetVersion;
    uiState.saveStatus = "saved";
    uiState.lastSavedAt = result.savedAt || Date.now();
    notify("Saved the app version over the changed files", "success");

    return true;
  } catch (error) {
    const message = errorMessage(error, "The app version could not be saved.");
    vaultSession.error = message;
    vaultSession.conflict = isRevisionConflict(message);
    uiState.saveStatus = "error";

    return false;
  } finally {
    vaultSession.busy = false;
  }
}

export async function flushVault(): Promise<boolean> {
  clearTimeout(persistTimer);
  if (!initialized || savedVersion >= dirtyVersion) {
    return true;
  }

  if (saveInFlight) {
    const saved = await saveInFlight;
    if (!saved) {
      return false;
    }

    return savedVersion < dirtyVersion ? flushVault() : true;
  }

  const targetVersion = dirtyVersion;
  const generation = sessionGeneration;
  const path = vaultSession.path;
  const snapshot = snapshotVault();
  uiState.saveStatus = "saving";

  const operation = (async (): Promise<boolean> => {
    if (vaultSession.backend === "browser") {
      const saved = persistBrowserVault(snapshot);
      if (saved && generation === sessionGeneration) {
        savedVersion = targetVersion;
      }

      return saved;
    }

    if (vaultSession.phase !== "ready" || !path) {
      uiState.saveStatus = "error";

      return false;
    }

    try {
      const result = await saveWorkspace(path, snapshot, vaultSession.revision);
      if (generation !== sessionGeneration || path !== vaultSession.path) {
        return true;
      }
      vaultSession.revision = result.revision;
      vaultSession.error = null;
      vaultSession.conflict = false;
      vaultSession.warnings = result.warnings;
      savedVersion = targetVersion;
      uiState.saveStatus = "saved";
      uiState.lastSavedAt = result.savedAt || Date.now();
      if (result.warnings.length) {
        notify(result.warnings[0], "warning");
      }

      return true;
    } catch (error) {
      if (generation !== sessionGeneration) {
        return false;
      }
      uiState.saveStatus = "error";
      const message = errorMessage(error, "Changes could not be written to the vault folder.");
      vaultSession.error = message;
      vaultSession.conflict = isRevisionConflict(message);
      uiState.commandOpen = false;
      uiState.vaultChooserOpen = true;
      notify(message, "warning");

      return false;
    }
  })();

  saveInFlight = operation;
  const saved = await operation;
  if (saveInFlight === operation) {
    saveInFlight = null;
  }
  if (saved && generation === sessionGeneration && savedVersion < dirtyVersion) {
    return flushVault();
  }

  return saved;
}

export const activeNote = computed<Note | undefined>(() =>
  vaultState.notes.find((note) => note.id === vaultState.activeNoteId),
);

export const recentNotes = computed<Note[]>(() => {
  const notesById = new Map(vaultState.notes.map((note) => [note.id, note]));

  return vaultState.recentNoteIds.flatMap((id) => {
    const note = notesById.get(id);

    return note ? [note] : [];
  });
});

export const backNavigationNote = computed<Note | undefined>(() =>
  findNoteNavigationTarget(noteNavigationState.back),
);

export const forwardNavigationNote = computed<Note | undefined>(() =>
  findNoteNavigationTarget(noteNavigationState.forward),
);

export const canNavigateBack = computed(() => Boolean(backNavigationNote.value));

export const canNavigateForward = computed(() => Boolean(forwardNavigationNote.value));

export const folderById = computed(() =>
  new Map(vaultState.folders.map((folder) => [folder.id, folder])),
);

export const folderNameMap = computed(() => {
  const names: Record<string, string> = {};
  for (const folder of vaultState.folders) {
    names[folder.id] = folderPath(folder.id);
  }

  return names;
});

export const visibleNotes = computed(() => {
  let notes = vaultState.selectedFolderId === "recent"
    ? [...recentNotes.value]
    : [...vaultState.notes];

  if (vaultState.selectedFolderId === "favorites") {
    notes = notes.filter((note) => note.pinned);
  }

  const filter = uiState.noteFilter.trim();
  if (filter) {
    const matchingIds = new Set(
      searchNotes(notes, filter, { folderNames: folderNameMap.value, limit: notes.length })
        .map((result) => result.note.id),
    );
    notes = notes.filter((note) => matchingIds.has(note.id));
  }

  return vaultState.selectedFolderId === "recent"
    ? notes
    : notes.sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updatedAt - a.updatedAt);
});

export const outgoingLinks = computed(() => {
  if (!activeNote.value) {
    return [];
  }

  return parseWikiLinks(activeNote.value.content).map((link) => ({
    link,
    note: resolveWikiLink(link, vaultState.notes, activeNote.value),
  }));
});

export const backlinks = computed(() =>
  activeNote.value ? findBacklinks(activeNote.value, vaultState.notes) : [],
);

export function selectNote(id: string): void {
  if (!vaultState.notes.some((note) => note.id === id)) {
    return;
  }

  if (id !== vaultState.activeNoteId) {
    recordDirectNoteNavigation(vaultState.activeNoteId, id);
  }
  activateNote(id);
}

export function navigateBack(): boolean {
  return traverseNoteNavigation(noteNavigationState.back, noteNavigationState.forward);
}

export function navigateForward(): boolean {
  return traverseNoteNavigation(noteNavigationState.forward, noteNavigationState.back);
}

export function selectFolder(selection: FolderSelection): void {
  vaultState.selectedFolderId = isSmartFolderSelection(selection) ? selection : "all";
  uiState.tool = "notes";
  uiState.noteFilter = "";
}

export function openSearchWorkspace(
  options: { query?: string; scope?: SearchScope; exactTag?: string | null } = {},
): void {
  const replacesSearch = options.query !== undefined
    || options.scope !== undefined
    || options.exactTag !== undefined;
  if (options.query !== undefined) {
    searchState.query = options.query;
  }
  if (options.scope !== undefined) {
    searchState.scope = options.scope;
  }
  if (replacesSearch) {
    searchState.exactTag = options.exactTag ?? null;
  }
  uiState.commandOpen = false;
  uiState.tool = "search";
  searchState.focusRequest += 1;
}

export function openQuickSearch(query = ""): void {
  searchState.quickQuery = query;
  uiState.commandOpen = true;
}

export function setZoom(zoom: number): void {
  uiState.zoom = clampZoom(zoom);
}

export function zoomIn(): void {
  setZoom(uiState.zoom + ZOOM_STEP);
}

export function zoomOut(): void {
  setZoom(uiState.zoom - ZOOM_STEP);
}

export function resetZoom(): void {
  setZoom(1);
}

export function createNote(folderId?: string | null, title = "Untitled note", content?: string): Note {
  const now = Date.now();
  const note: Note = {
    id: createId("note"),
    title: uniqueNoteTitle(title.trim() || "Untitled note"),
    content: content ?? "# Untitled note\n\n",
    relativePath: "",
    folderId: folderId === undefined ? currentFolderId() : folderId,
    tags: [],
    pinned: false,
    createdAt: now,
    updatedAt: now,
  };
  if (content === undefined) {
    note.content = `# ${note.title}\n\n`;
  }
  vaultState.notes.unshift(note);
  selectNote(note.id);
  vaultState.selectedFolderId = "all";
  uiState.tool = "notes";
  uiState.noteFilter = "";
  notify("New note created", "success");

  return note;
}

export function createLinkedNote(target: string): Note {
  const cleanTarget = target.replace(/\.md$/i, "").split("/").pop()?.trim() || "Untitled note";
  const existing = resolveWikiLink(cleanTarget, vaultState.notes, activeNote.value);
  if (existing) {
    selectNote(existing.id);

    return existing;
  }

  return createNote(activeNote.value?.folderId ?? currentFolderId(), cleanTarget);
}

export function updateNote(id: string, patch: Partial<Pick<Note, "title" | "content" | "folderId" | "tags" | "pinned">>): void {
  const note = vaultState.notes.find((candidate) => candidate.id === id);
  if (!note) {
    return;
  }
  const locationChanged = (patch.title !== undefined && patch.title !== note.title)
    || (patch.folderId !== undefined && patch.folderId !== note.folderId);
  if (patch.title !== undefined) {
    note.title = patch.title;
  }
  if (patch.content !== undefined) {
    note.content = patch.content;
  }
  if (patch.folderId !== undefined) {
    note.folderId = patch.folderId;
  }
  if (patch.tags !== undefined) {
    note.tags = patch.tags;
  }
  if (patch.pinned !== undefined) {
    note.pinned = patch.pinned;
  }
  if (locationChanged) {
    note.relativePath = "";
  }
  note.updatedAt = Date.now();
}

export function moveNoteToFolder(noteId: string, folderId: string | null): boolean {
  const note = vaultState.notes.find((candidate) => candidate.id === noteId);
  if (!note) {
    notify("Could not move that note", "warning");

    return false;
  }

  const folder = folderId === null
    ? null
    : vaultState.folders.find((candidate) => candidate.id === folderId);
  if (folderId !== null && !folder) {
    notify("That folder is no longer available", "warning");

    return false;
  }
  if (note.folderId === folderId) {
    return false;
  }
  const duplicateNote = vaultState.notes.some(
    (candidate) => candidate.id !== noteId
      && candidate.folderId === folderId
      && noteStemKey(candidate) === noteStemKey(note),
  );
  const noteFileNames = noteFileNameKeys(note);
  const duplicateFolder = vaultState.folders.some(
    (candidate) => candidate.parentId === folderId
      && noteFileNames.has(folderNameKey(candidate.name)),
  );
  if (duplicateNote || duplicateFolder) {
    notify("A file with that name already exists there", "warning");

    return false;
  }

  updateNote(noteId, { folderId });
  notify(`Moved to ${folder?.name ?? "Vault root"}`, "success");

  return true;
}

export function deleteNote(id: string): void {
  const index = vaultState.notes.findIndex((note) => note.id === id);
  if (index < 0) {
    return;
  }

  const wasActive = vaultState.activeNoteId === id;
  vaultState.notes.splice(index, 1);
  deleteNoteEditorPosition(currentEditorPositionVaultId(), id);
  removeRecentNote(id);
  removeNoteFromNavigation(id);
  if (wasActive) {
    const fallbackNote = vaultState.notes[Math.min(index, vaultState.notes.length - 1)];
    noteNavigationState.forward.splice(0);
    if (fallbackNote) {
      activateNote(fallbackNote.id);
    } else {
      vaultState.activeNoteId = null;
    }
  }
  notify("Note deleted", "neutral");
}

export function togglePinned(id: string): void {
  const note = vaultState.notes.find((candidate) => candidate.id === id);
  if (note) {
    updateNote(id, { pinned: !note.pinned });
  }
}

export function createFolder(name: string, parentId: string | null = null): Folder | undefined {
  const cleanName = name.trim().replace(/[\\/]/g, " ");
  if (!cleanName) {
    return undefined;
  }
  const duplicate = vaultState.folders.some(
    (folder) => folder.parentId === parentId && folderNameKey(folder.name) === folderNameKey(cleanName),
  ) || vaultState.notes.some(
    (note) => note.folderId === parentId && folderConflictsWithNote(cleanName, note),
  );
  if (duplicate) {
    notify("A file or folder with that name already exists here", "warning");

    return undefined;
  }
  const folder: Folder = {
    id: createId("folder"),
    name: cleanName,
    parentId,
    createdAt: Date.now(),
  };
  vaultState.folders.push(folder);
  if (!isSmartFolderSelection(vaultState.selectedFolderId)) {
    vaultState.selectedFolderId = "all";
  }
  notify(`Created ${cleanName}`, "success");

  return folder;
}

export function renameFolder(id: string, name: string): void {
  const folder = vaultState.folders.find((candidate) => candidate.id === id);
  const cleanName = name.trim().replace(/[\\/]/g, " ");
  if (!folder || !cleanName || folder.name === cleanName) {
    return;
  }

  const duplicate = vaultState.folders.some(
    (candidate) => candidate.id !== id
      && candidate.parentId === folder.parentId
      && folderNameKey(candidate.name) === folderNameKey(cleanName),
  ) || vaultState.notes.some(
    (note) => note.folderId === folder.parentId && folderConflictsWithNote(cleanName, note),
  );
  if (duplicate) {
    notify("A file or folder with that name already exists here", "warning");

    return;
  }

  const affectedFolders = new Set([id, ...descendantFolderIds(id)]);
  folder.name = cleanName;
  for (const note of vaultState.notes) {
    if (note.folderId && affectedFolders.has(note.folderId)) {
      note.relativePath = "";
    }
  }
}

export function moveFolder(folderId: string, parentId: string | null): boolean {
  const folder = vaultState.folders.find((candidate) => candidate.id === folderId);
  if (!folder) {
    notify("Could not move that folder", "warning");

    return false;
  }

  const parent = parentId === null
    ? null
    : vaultState.folders.find((candidate) => candidate.id === parentId);
  if (parentId !== null && !parent) {
    notify("That folder is no longer available", "warning");

    return false;
  }
  if (folder.parentId === parentId) {
    return false;
  }

  const affectedFolders = new Set([folderId, ...descendantFolderIds(folderId)]);
  if (parentId !== null && affectedFolders.has(parentId)) {
    notify("A folder cannot be moved inside itself", "warning");

    return false;
  }

  const duplicate = vaultState.folders.some(
    (candidate) => candidate.id !== folderId
      && candidate.parentId === parentId
      && folderNameKey(candidate.name) === folderNameKey(folder.name),
  ) || vaultState.notes.some(
    (note) => note.folderId === parentId && folderConflictsWithNote(folder.name, note),
  );
  if (duplicate) {
    notify("A file or folder with that name already exists there", "warning");

    return false;
  }

  folder.parentId = parentId;
  for (const note of vaultState.notes) {
    if (note.folderId && affectedFolders.has(note.folderId)) {
      note.relativePath = "";
    }
  }
  notify(`Moved ${folder.name} to ${parent?.name ?? "Vault root"}`, "success");

  return true;
}

export function deleteFolder(id: string): void {
  const folder = vaultState.folders.find((candidate) => candidate.id === id);
  if (!folder) {
    return;
  }
  const affectedFolders = new Set([id, ...descendantFolderIds(id)]);
  const children = vaultState.folders.filter((candidate) => candidate.parentId === id);
  const destinationFolders = vaultState.folders.filter(
    (candidate) => candidate.id !== id && candidate.parentId === folder.parentId,
  );
  const destinationNotes = vaultState.notes.filter((note) => note.folderId === folder.parentId);
  const folderCollision = children.some((child) => (
    destinationFolders.some((candidate) => folderNameKey(candidate.name) === folderNameKey(child.name))
    || destinationNotes.some((note) => folderConflictsWithNote(child.name, note))
  ));
  const noteCollision = vaultState.notes
    .filter((note) => note.folderId === id)
    .some((note) => (
      destinationNotes.some((candidate) => noteStemKey(candidate) === noteStemKey(note))
      || destinationFolders.some((candidate) => folderConflictsWithNote(candidate.name, note))
    ));
  if (folderCollision || noteCollision) {
    notify("Move or rename conflicting items before removing this folder", "warning");

    return;
  }
  for (const child of children) {
    child.parentId = folder.parentId;
  }
  for (const note of vaultState.notes) {
    if (!note.folderId || !affectedFolders.has(note.folderId)) {
      continue;
    }
    note.relativePath = "";
    if (note.folderId === id) {
      note.folderId = folder.parentId;
    }
  }
  vaultState.folders.splice(vaultState.folders.indexOf(folder), 1);
  if (vaultState.selectedFolderId === id) {
    vaultState.selectedFolderId = "all";
  }
  notify("Folder removed; its contents moved up one level", "neutral");
}

export function createFromTemplate(templateId: string, requestedTitle?: string): Note | undefined {
  const template = vaultState.templates.find((candidate) => candidate.id === templateId);
  if (!template) {
    return undefined;
  }
  const now = new Date();
  const date = new Intl.DateTimeFormat("en", {
    month: "long",
    day: "numeric",
    year: "numeric",
  }).format(now);
  const time = new Intl.DateTimeFormat("en", { hour: "numeric", minute: "2-digit" }).format(now);
  const title = requestedTitle?.trim() || replaceTemplateTokens(template.titlePattern, { date, time, title: template.name });
  const uniqueTitle = uniqueNoteTitle(title || template.name);
  const content = replaceTemplateTokens(template.content, { date, time, title: uniqueTitle });

  return createNote(currentFolderId(), uniqueTitle, content);
}

export function saveTemplate(template: Partial<NoteTemplate> & Pick<NoteTemplate, "name" | "content">): NoteTemplate {
  const existing = template.id
    ? vaultState.templates.find((candidate) => candidate.id === template.id)
    : undefined;
  if (existing) {
    Object.assign(existing, template);

    return existing;
  }
  const created: NoteTemplate = {
    id: createId("template"),
    name: template.name.trim() || "Untitled template",
    description: template.description?.trim() || "A custom note structure.",
    titlePattern: template.titlePattern?.trim() || "Untitled note",
    content: template.content,
    glyph: template.glyph || "file-text",
    createdAt: Date.now(),
  };
  vaultState.templates.push(created);

  return created;
}

export function saveSnippet(snippet: Partial<CssSnippet> & Pick<CssSnippet, "name" | "css">): CssSnippet {
  const existing = snippet.id
    ? vaultState.snippets.find((candidate) => candidate.id === snippet.id)
    : undefined;
  if (existing) {
    Object.assign(existing, snippet);

    return existing;
  }
  const created: CssSnippet = {
    id: createId("snippet"),
    name: snippet.name.trim() || "Untitled snippet",
    description: snippet.description?.trim() || "A custom interface style.",
    css: snippet.css,
    enabled: snippet.enabled ?? true,
    createdAt: Date.now(),
  };
  vaultState.snippets.push(created);

  return created;
}

export function deleteSnippet(id: string): void {
  const index = vaultState.snippets.findIndex((snippet) => snippet.id === id);
  if (index >= 0) {
    vaultState.snippets.splice(index, 1);
  }
}

export async function mergeImportedVault(
  result: ImportResult,
  replace = false,
): Promise<{ noteCount: number; saved: boolean }> {
  if (!(await flushVault())) {
    return { noteCount: result.notes.length, saved: false };
  }
  clearTimeout(persistTimer);
  const previousVault = snapshotVault();
  const previousSavedVersion = savedVersion;
  const previousActiveNoteId = vaultState.activeNoteId;
  const previousNoteNavigation = snapshotNoteNavigation();
  if (replace) {
    vaultState.notes.splice(0);
    vaultState.folders.splice(0);
  }

  const now = Date.now();
  let firstImportedNoteId: string | null = null;
  for (const imported of result.notes) {
    const folderId = ensureFolderPath(imported.folderPath);
    const title = uniqueNoteTitle(imported.title || "Untitled note");
    const note: Note = {
      id: createId("note"),
      title,
      content: imported.content,
      relativePath: "",
      folderId,
      tags: imported.tags,
      pinned: false,
      createdAt: now,
      updatedAt: now,
    };
    firstImportedNoteId ??= note.id;
    vaultState.notes.push(note);
  }

  for (const imported of result.snippets) {
    const existing = vaultState.snippets.find(
      (snippet) => snippet.name.toLocaleLowerCase() === imported.name.toLocaleLowerCase(),
    );
    if (existing) {
      continue;
    }
    vaultState.snippets.push({
      id: createId("snippet"),
      name: imported.name,
      description: "Imported from an Obsidian CSS snippet.",
      css: imported.css,
      enabled: imported.enabled,
      createdAt: now,
    });
  }

  vaultState.activeNoteId = firstImportedNoteId ?? (replace ? null : previousActiveNoteId);
  vaultState.selectedFolderId = "all";
  if (replace) {
    vaultState.recentNoteIds.splice(0);
    resetNoteNavigation();
  }
  if (firstImportedNoteId) {
    touchRecentNote(firstImportedNoteId);
  }
  if (!replace && firstImportedNoteId) {
    recordDirectNoteNavigation(previousActiveNoteId, firstImportedNoteId);
  }
  const saved = await flushVault();
  if (!saved) {
    hydrateVault(previousVault);
    restoreNoteNavigation(previousNoteNavigation);
    dirtyVersion = previousSavedVersion;
    savedVersion = previousSavedVersion;
  }
  pruneNoteEditorPositions(currentEditorPositionVaultId(), vaultState.notes);
  notify(
    saved
      ? `Imported ${result.notes.length} Markdown ${result.notes.length === 1 ? "note" : "notes"}`
      : "Import applied, but not saved",
    saved ? "success" : "warning",
  );

  return { noteCount: result.notes.length, saved };
}

export function buildExportPayload(): {
  notes: ExportNote[];
  templates: ExportTemplate[];
  snippets: ExportSnippet[];
} {
  return {
    notes: vaultState.notes.map((note) => ({
      title: note.title,
      content: note.content,
      folderPath: note.folderId ? folderPath(note.folderId) : "",
      tags: note.tags,
    })),
    templates: vaultState.templates.map((template) => ({
      name: template.name,
      content: template.content,
    })),
    snippets: vaultState.snippets.map((snippet) => ({
      name: snippet.name,
      css: snippet.css,
      enabled: snippet.enabled,
    })),
  };
}

export function folderPath(id: string | null): string {
  if (!id) {
    return "";
  }
  const parts: string[] = [];
  const seen = new Set<string>();
  let cursor = folderById.value.get(id);
  while (cursor && !seen.has(cursor.id)) {
    parts.unshift(cursor.name);
    seen.add(cursor.id);
    cursor = cursor.parentId ? folderById.value.get(cursor.parentId) : undefined;
  }

  return parts.join("/");
}

export function noteCountForFolder(id: string): number {
  const ids = new Set([id, ...descendantFolderIds(id)]);

  return vaultState.notes.filter((note) => note.folderId && ids.has(note.folderId)).length;
}

export function folderChildren(parentId: string | null): Folder[] {
  return vaultState.folders
    .filter((folder) => folder.parentId === parentId)
    .sort((a, b) => a.name.localeCompare(b.name));
}

export function notify(message: string, tone: ToastTone = "neutral"): void {
  clearTimeout(toastTimer);
  uiState.toast = { id: Date.now(), message, tone };
  toastTimer = setTimeout(() => {
    uiState.toast = null;
  }, 3200);
}

export async function clearVault(): Promise<boolean> {
  if (!(await flushVault())) {
    return false;
  }
  clearTimeout(persistTimer);
  const previousVault = snapshotVault();
  const previousSavedVersion = savedVersion;
  const previousNoteNavigation = snapshotNoteNavigation();
  vaultState.notes.splice(0);
  vaultState.folders.splice(0);
  vaultState.activeNoteId = null;
  vaultState.recentNoteIds.splice(0);
  resetNoteNavigation();
  vaultState.selectedFolderId = "all";
  uiState.noteFilter = "";
  uiState.commandOpen = false;
  resetSearchState();
  uiState.contextOpen = false;
  uiState.explorerOpen = true;
  const saved = await flushVault();
  if (!saved) {
    hydrateVault(previousVault);
    restoreNoteNavigation(previousNoteNavigation);
    dirtyVersion = previousSavedVersion;
    savedVersion = previousSavedVersion;
  }
  pruneNoteEditorPositions(currentEditorPositionVaultId(), vaultState.notes);
  notify(saved ? "Vault cleared" : "Vault cleared, but not saved", saved ? "success" : "warning");

  return saved;
}

function activateNote(id: string): boolean {
  if (!noteExists(id)) {
    return false;
  }

  const wasVisible = visibleNotes.value.some((note) => note.id === id);
  vaultState.activeNoteId = id;
  touchRecentNote(id);
  if (!wasVisible) {
    vaultState.selectedFolderId = "all";
    uiState.noteFilter = "";
  }

  return true;
}

function touchRecentNote(id: string): void {
  if (vaultState.recentNoteIds[0] === id) {
    return;
  }

  const recentNoteIds = [id, ...vaultState.recentNoteIds.filter((noteId) => noteId !== id)]
    .slice(0, RECENT_NOTE_LIMIT);
  vaultState.recentNoteIds.splice(0, vaultState.recentNoteIds.length, ...recentNoteIds);
}

function removeRecentNote(id: string): void {
  vaultState.recentNoteIds = vaultState.recentNoteIds.filter((noteId) => noteId !== id);
}

function recordDirectNoteNavigation(previousId: string | null, nextId: string): void {
  if (previousId === nextId || !noteExists(nextId)) {
    return;
  }
  if (previousId && noteExists(previousId)) {
    pushNoteNavigationEntry(noteNavigationState.back, previousId);
  }
  noteNavigationState.forward.splice(0);
}

function traverseNoteNavigation(source: string[], destination: string[]): boolean {
  while (source.length) {
    const targetId = source.pop();
    if (!targetId || targetId === vaultState.activeNoteId || !noteExists(targetId)) {
      continue;
    }

    const previousId = vaultState.activeNoteId;
    if (!activateNote(targetId)) {
      continue;
    }
    if (previousId && noteExists(previousId)) {
      pushNoteNavigationEntry(destination, previousId);
    }

    return true;
  }

  return false;
}

function pushNoteNavigationEntry(stack: string[], id: string): void {
  if (stack[stack.length - 1] === id) {
    return;
  }
  stack.push(id);
  if (stack.length > NOTE_NAVIGATION_LIMIT) {
    stack.splice(0, stack.length - NOTE_NAVIGATION_LIMIT);
  }
}

function findNoteNavigationTarget(stack: string[]): Note | undefined {
  for (let index = stack.length - 1; index >= 0; index -= 1) {
    const id = stack[index];
    if (id === vaultState.activeNoteId) {
      continue;
    }
    const note = vaultState.notes.find((candidate) => candidate.id === id);
    if (note) {
      return note;
    }
  }

  return undefined;
}

function removeNoteFromNavigation(id: string): void {
  noteNavigationState.back = noteNavigationState.back.filter((noteId) => noteId !== id);
  noteNavigationState.forward = noteNavigationState.forward.filter((noteId) => noteId !== id);
}

function pruneNoteNavigation(): void {
  const noteIds = new Set(vaultState.notes.map((note) => note.id));
  noteNavigationState.back = noteNavigationState.back.filter((id) => noteIds.has(id));
  noteNavigationState.forward = noteNavigationState.forward.filter((id) => noteIds.has(id));
}

function resetNoteNavigation(): void {
  noteNavigationState.back.splice(0);
  noteNavigationState.forward.splice(0);
}

function snapshotNoteNavigation(): NoteNavigationState {
  return {
    back: [...noteNavigationState.back],
    forward: [...noteNavigationState.forward],
  };
}

function restoreNoteNavigation(snapshot: NoteNavigationState): void {
  noteNavigationState.back.splice(0, noteNavigationState.back.length, ...snapshot.back);
  noteNavigationState.forward.splice(0, noteNavigationState.forward.length, ...snapshot.forward);
}

function noteExists(id: string): boolean {
  return vaultState.notes.some((note) => note.id === id);
}

function currentFolderId(): string | null {
  return activeNote.value?.folderId ?? null;
}

function isSmartFolderSelection(selection: unknown): selection is SmartFolderSelection {
  return selection === "all" || selection === "favorites" || selection === "recent";
}

function folderNameKey(name: string): string {
  return name.trim().toLowerCase();
}

function noteStemKey(note: Note): string {
  return safeNoteStem(note.title).toLowerCase();
}

function noteFileNameKeys(note: Note): Set<string> {
  const stem = noteStemKey(note);

  return new Set([`${stem}.md`, `${stem}.markdown`]);
}

function folderConflictsWithNote(folderName: string, note: Note): boolean {
  return noteFileNameKeys(note).has(folderNameKey(folderName));
}

function safeNoteStem(title: string): string {
  const encoder = new TextEncoder();
  let result = "";
  let byteLength = 0;
  let previousWasReplacement = false;
  for (const character of title.trim()) {
    const forbidden = /[\u0000-\u001f\u007f-\u009f/\\:*?"<>|]/u.test(character);
    const addition = forbidden ? (previousWasReplacement ? "" : "-") : character;
    if (addition) {
      result += addition;
      byteLength += encoder.encode(addition).length;
    }
    previousWasReplacement = forbidden;
    if (byteLength >= 120) {
      break;
    }
  }

  result = result.replace(/^[ .]+|[ .]+$/g, "") || "Untitled note";
  const windowsBase = result.split(".")[0]?.toUpperCase();
  if (["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"].includes(windowsBase)) {
    return `_${result}`;
  }

  return result;
}

function uniqueNoteTitle(base: string): string {
  const normalized = new Set(vaultState.notes.map((note) => note.title.toLocaleLowerCase()));
  if (!normalized.has(base.toLocaleLowerCase())) {
    return base;
  }
  let suffix = 2;
  while (normalized.has(`${base} ${suffix}`.toLocaleLowerCase())) {
    suffix += 1;
  }

  return `${base} ${suffix}`;
}

function ensureFolderPath(path: string): string | null {
  const parts = path.split(/[\\/]/).map((part) => part.trim()).filter(Boolean);
  let parentId: string | null = null;
  for (const part of parts) {
    let folder = vaultState.folders.find(
      (candidate) => candidate.parentId === parentId && candidate.name.toLocaleLowerCase() === part.toLocaleLowerCase(),
    );
    if (!folder) {
      folder = {
        id: createId("folder"),
        name: part,
        parentId,
        createdAt: Date.now(),
      };
      vaultState.folders.push(folder);
    }
    parentId = folder.id;
  }

  return parentId;
}

function descendantFolderIds(id: string): string[] {
  const result: string[] = [];
  const queue = [id];
  while (queue.length) {
    const parent = queue.shift();
    for (const folder of vaultState.folders) {
      if (folder.parentId === parent && !result.includes(folder.id)) {
        result.push(folder.id);
        queue.push(folder.id);
      }
    }
  }

  return result;
}

function replaceTemplateTokens(value: string, tokens: Record<string, string>): string {
  return value.replace(/{{\s*(date|time|title)\s*}}/gi, (_, key: string) => tokens[key.toLocaleLowerCase()] ?? "");
}

function createId(prefix: string): string {
  const random = typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;

  return `${prefix}-${random}`;
}

interface StoredVault {
  raw: string;
  vault: VaultData;
  needsRewrite: boolean;
}

function readStoredVault(): StoredVault | null {
  const raw = safeStorageGet(STORAGE_KEY);
  if (!raw) {
    return null;
  }
  try {
    const parsed = JSON.parse(raw) as Partial<VaultData>;
    if (!Array.isArray(parsed.notes) || !Array.isArray(parsed.folders)) {
      return null;
    }
    const vault = normalizeVault(parsed);

    return {
      raw,
      vault,
      needsRewrite: parsed.selectedFolderId !== vault.selectedFolderId
        || !stringArraysMatch(parsed.recentNoteIds, vault.recentNoteIds),
    };
  } catch {
    return null;
  }
}

function normalizeVault(input: Partial<VaultData>): VaultData {
  const fallback = createSeedVault();
  const rawNotes = Array.isArray(input.notes) ? input.notes : fallback.notes;
  const notes: Note[] = rawNotes.map((note) => ({
    ...note,
    relativePath: typeof note.relativePath === "string" ? note.relativePath : "",
    tags: Array.isArray(note.tags) ? note.tags : [],
  }));
  const folders = Array.isArray(input.folders) ? input.folders : fallback.folders;

  const currentBuiltInSnippets = new Map(
    fallback.snippets
      .filter((snippet) => snippet.builtIn)
      .map((snippet) => [snippet.id, snippet]),
  );
  const snippets = (Array.isArray(input.snippets) ? input.snippets : fallback.snippets)
    .map((snippet) => {
      const current = currentBuiltInSnippets.get(snippet.id);
      if (snippet.builtIn && current) {
        return {
          ...snippet,
          name: current.name,
          description: current.description,
          css: current.css,
        };
      }

      return snippet;
    });

  const activeNoteId = typeof input.activeNoteId === "string"
    && notes.some((note) => note.id === input.activeNoteId)
    ? input.activeNoteId
    : notes[0]?.id ?? null;
  const recentNoteIds = normalizeRecentNoteIds(input.recentNoteIds, notes, activeNoteId);
  const selectedFolderId = normalizeFolderSelection(
    input.selectedFolderId,
    notes,
    recentNoteIds,
  );

  return {
    name: typeof input.name === "string" && input.name.trim() ? input.name : fallback.name,
    notes,
    folders,
    templates: Array.isArray(input.templates) && input.templates.length
      ? input.templates
      : fallback.templates,
    snippets,
    activeNoteId,
    recentNoteIds,
    selectedFolderId,
  };
}

function normalizeFolderSelection(
  selection: unknown,
  notes: Note[],
  recentNoteIds: string[],
): SmartFolderSelection {
  if (selection === "recent" && recentNoteIds.length) {
    return "recent";
  }
  if (selection === "favorites" && notes.some((note) => note.pinned)) {
    return "favorites";
  }

  return "all";
}

function normalizeRecentNoteIds(
  value: unknown,
  notes: Note[],
  activeNoteId: string | null,
): string[] {
  const noteIds = new Set(notes.map((note) => note.id));
  const recentNoteIds: string[] = [];
  const addNote = (id: unknown): void => {
    if (
      typeof id === "string"
      && noteIds.has(id)
      && !recentNoteIds.includes(id)
      && recentNoteIds.length < RECENT_NOTE_LIMIT
    ) {
      recentNoteIds.push(id);
    }
  };

  addNote(activeNoteId);
  if (Array.isArray(value)) {
    value.forEach(addNote);
  }

  return recentNoteIds;
}

function stringArraysMatch(value: unknown, expected: string[]): boolean {
  return Array.isArray(value)
    && value.length === expected.length
    && value.every((item, index) => item === expected[index]);
}

function snapshotVault(): VaultData {
  return JSON.parse(JSON.stringify(vaultState)) as VaultData;
}

function hydrateVault(vault: Partial<VaultData>): void {
  suppressPersistence += 1;
  try {
    Object.assign(vaultState, normalizeVault(vault));
  } finally {
    suppressPersistence -= 1;
  }
}

function applyWorkspace(workspace: WorkspaceLoad, recentVaults = vaultSession.recentVaults): void {
  const previousPath = vaultSession.path;
  sessionGeneration += 1;
  hydrateVault({ ...workspace.vault, name: workspace.descriptor.name });
  initializeNoteEditorPositions(
    "native",
    workspace.descriptor.path,
    vaultState.notes,
    workspace.editorPositions,
    workspace.editorPositionsWritable,
    workspace.editorPositionsRevision,
  );
  if (previousPath === workspace.descriptor.path) {
    pruneNoteNavigation();
  } else {
    resetNoteNavigation();
  }
  vaultSession.phase = "ready";
  vaultSession.path = workspace.descriptor.path;
  vaultSession.revision = workspace.revision;
  vaultSession.error = null;
  vaultSession.conflict = false;
  vaultSession.warnings = workspace.warnings;
  vaultSession.recentVaults = mergeRecentVaults(workspace.descriptor, recentVaults);
  dirtyVersion = 0;
  savedVersion = 0;
  uiState.saveStatus = "saved";
  uiState.lastSavedAt = Date.now();
  uiState.noteFilter = "";
  uiState.commandOpen = false;
  resetSearchState();
  uiState.vaultChooserOpen = false;
  if (workspace.warnings.length) {
    notify(`${workspace.warnings.length} ${workspace.warnings.length === 1 ? "file warning" : "file warnings"} while opening the vault`, "warning");
  }
}

function resetSearchState(): void {
  searchState.query = "";
  searchState.scope = "all";
  searchState.exactTag = null;
  searchState.quickQuery = "";
  searchState.focusRequest += 1;
}

function mergeRecentVaults(
  current: VaultDescriptor,
  recentVaults: VaultDescriptor[],
): VaultDescriptor[] {
  const merged = [current, ...recentVaults.filter((vault) => vault.path !== current.path)];

  return merged.slice(0, 12);
}

function currentEditorPositionVaultId(): string {
  return editorPositionVaultId(vaultSession.backend, vaultSession.path);
}

async function flushBeforeVaultChange(): Promise<boolean> {
  if (savedVersion < dirtyVersion) {
    if (vaultSession.phase !== "ready") {
      vaultSession.error = "Choose a vault before saving changes.";

      return false;
    }
    if (!(await flushVault())) {
      vaultSession.error = "Save the current changes before switching vaults.";

      return false;
    }
  }
  const positionsSaved = await flushNoteEditorPositions(currentEditorPositionVaultId());
  if (!positionsSaved) {
    vaultSession.error = "Save the current document position before switching vaults.";
  }

  return positionsSaved;
}

function persistBrowserVault(vault: VaultData): boolean {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(vault));
    uiState.saveStatus = "saved";
    uiState.lastSavedAt = Date.now();

    return true;
  } catch {
    uiState.saveStatus = "error";

    return false;
  }
}

function installVaultLifecycleHandlers(): void {
  if (typeof window === "undefined" || externalCheckTimer) {
    return;
  }

  window.addEventListener("blur", () => void flushApplicationState());
  window.addEventListener("focus", () => void refreshWorkspaceFromDisk());
  window.addEventListener("beforeunload", () => {
    void flushVault();
    void flushNoteEditorPositions();
  });
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") {
      void flushApplicationState();
    } else {
      void refreshWorkspaceFromDisk();
    }
  });

  if (vaultSession.backend === "native") {
    void installNativeCloseHandler();
  }

  externalCheckTimer = setInterval(
    () => void refreshWorkspaceFromDisk(),
    EXTERNAL_CHECK_DELAY,
  );
}

async function flushApplicationState(): Promise<void> {
  await flushVault();
  await flushNoteEditorPositions();
}

async function installNativeCloseHandler(): Promise<void> {
  if (closeHandlerInstalled) {
    return;
  }
  closeHandlerInstalled = true;
  const appWindow = getCurrentWindow();
  try {
    await appWindow.onCloseRequested(async (event) => {
      if (closingAfterSave) {
        return;
      }
      if (vaultSession.busy) {
        event.preventDefault();
        notify("Wait for the current vault action to finish before closing", "warning");

        return;
      }
      if (
        savedVersion >= dirtyVersion
        && !saveInFlight
        && !hasPendingNoteEditorPositions()
      ) {
        return;
      }
      event.preventDefault();
      const saved = await flushVault();
      if (!saved) {
        notify(vaultSession.error || "Save the current changes before closing", "warning");

        return;
      }
      const positionsSaved = await flushNoteEditorPositions();
      if (!positionsSaved) {
        notify("Notes are saved, but document positions could not be saved", "warning");
      }
      closingAfterSave = true;
      await appWindow.destroy();
    });
  } catch (error) {
    closeHandlerInstalled = false;
    vaultSession.error = errorMessage(error, "Could not install the safe-close handler.");
  }
}

async function refreshWorkspaceFromDisk(): Promise<void> {
  const path = vaultSession.path;
  if (
    vaultSession.backend !== "native"
    || vaultSession.phase !== "ready"
    || !path
    || vaultSession.busy
    || uiState.vaultChooserOpen
    || checkingExternalChanges
    || document.visibilityState === "hidden"
    || saveInFlight
    || dirtyVersion > savedVersion
  ) return;

  checkingExternalChanges = true;
  const generation = sessionGeneration;
  try {
    const revision = await getWorkspaceRevision(path);
    if (revision === vaultSession.revision) {
      return;
    }
    await flushNoteEditorPositions(currentEditorPositionVaultId());
    const workspace = await openWorkspace(path, createEmptyVault());
    if (
      generation !== sessionGeneration
      || path !== vaultSession.path
      || dirtyVersion > savedVersion
    ) return;
    applyWorkspace(workspace);
    notify("Reloaded changes from the vault folder", "neutral");
  } catch (error) {
    if (generation === sessionGeneration && path === vaultSession.path) {
      vaultSession.error = errorMessage(error, "The vault folder could not be checked for changes.");
    }
  } finally {
    checkingExternalChanges = false;
  }
}

function setVaultError(error: unknown, fallback: string): void {
  vaultSession.error = errorMessage(error, fallback);
  vaultSession.conflict = false;
}

function isRevisionConflict(message: string): boolean {
  const normalized = message.toLocaleLowerCase();

  return normalized.includes("changed")
    && (normalized.includes("vault") || normalized.includes("file") || normalized.includes("disk"));
}

function errorMessage(error: unknown, fallback: string): string {
  if (typeof error === "string" && error.trim()) {
    return error;
  }
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }

  return fallback;
}

function safeStorageGet(key: string): string | null {
  if (typeof localStorage === "undefined") {
    return null;
  }
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function safeStorageSet(key: string, value: string): void {
  if (typeof localStorage === "undefined") {
    return;
  }
  try {
    localStorage.setItem(key, value);
  } catch {
    // Local preferences are non-critical when browser storage is unavailable.
  }
}

function readStoredZoom(): number {
  const storedZoom = Number.parseFloat(safeStorageGet(APP_ZOOM_KEY) ?? "");

  return Number.isFinite(storedZoom) ? clampZoom(storedZoom) : 1;
}

function clampZoom(zoom: number): number {
  const roundedZoom = Number((Math.round(zoom / ZOOM_STEP) * ZOOM_STEP).toFixed(2));

  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, roundedZoom));
}

function storageFingerprint(value: string): string {
  let hash = 2_166_136_261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16_777_619);
  }

  return `v1-${value.length}-${(hash >>> 0).toString(16)}`;
}

function applyEnabledSnippets(): void {
  if (typeof document === "undefined") {
    return;
  }
  let style = document.querySelector<HTMLStyleElement>("#obsidian-at-home-user-snippets");
  if (!style) {
    style = document.createElement("style");
    style.id = "obsidian-at-home-user-snippets";
    document.head.appendChild(style);
  }
  style.textContent = vaultState.snippets
    .filter((snippet) => snippet.enabled)
    .map((snippet) => `/* ${snippet.name} */\n${snippet.css}`)
    .join("\n\n");
}
