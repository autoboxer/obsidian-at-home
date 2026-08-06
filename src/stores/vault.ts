import { computed, reactive, watch } from "vue";
import { createEmptyVault, createSeedVault } from "../data/seed";
import { findBacklinks, parseWikiLinks, resolveWikiLink, searchNotes } from "../lib";
import type {
  CssSnippet,
  EditorMode,
  ExportNote,
  ExportSnippet,
  ExportTemplate,
  Folder,
  ImportResult,
  Note,
  NoteTemplate,
  ToolView,
  VaultData,
} from "../types";

const STORAGE_KEY = "obsidian-at-home.vault.v1";
const PERSIST_DELAY = 220;

type FolderSelection = VaultData["selectedFolderId"];
type SaveStatus = "saved" | "saving" | "error";
type ToastTone = "neutral" | "success" | "warning";

interface UiState {
  tool: ToolView;
  noteFilter: string;
  commandOpen: boolean;
  contextOpen: boolean;
  explorerOpen: boolean;
  inspectorTab: "links" | "info";
  saveStatus: SaveStatus;
  lastSavedAt: number;
  toast: { id: number; message: string; tone: ToastTone } | null;
}

function loadVault(): VaultData {
  const fallback = createSeedVault();
  if (typeof localStorage === "undefined") return fallback;

  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return fallback;
    const parsed = JSON.parse(raw) as Partial<VaultData>;
    if (!Array.isArray(parsed.notes) || !Array.isArray(parsed.folders)) return fallback;

    const currentReadingSnippet = fallback.snippets.find(
      (snippet) => snippet.id === "snippet-editor-serif",
    );
    const currentWideSnippet = fallback.snippets.find(
      (snippet) => snippet.id === "snippet-wide-page",
    );
    const snippets = (Array.isArray(parsed.snippets) ? parsed.snippets : fallback.snippets)
      .map((snippet) => {
        if (snippet.id === "snippet-editor-serif" && snippet.builtIn && currentReadingSnippet) {
          return {
            ...snippet,
            name: currentReadingSnippet.name,
            description: currentReadingSnippet.description,
            css: currentReadingSnippet.css,
          };
        }
        if (snippet.id === "snippet-wide-page" && snippet.builtIn && currentWideSnippet) {
          return {
            ...snippet,
            name: currentWideSnippet.name,
            description: currentWideSnippet.description,
          };
        }
        return snippet;
      });

    return {
      name: typeof parsed.name === "string" ? parsed.name : fallback.name,
      notes: parsed.notes,
      folders: parsed.folders,
      templates: Array.isArray(parsed.templates) && parsed.templates.length
        ? parsed.templates
        : fallback.templates,
      snippets,
      activeNoteId: typeof parsed.activeNoteId === "string" || parsed.activeNoteId === null
        ? parsed.activeNoteId
        : parsed.notes[0]?.id ?? null,
      selectedFolderId: parsed.selectedFolderId ?? "all",
      editorMode: ["source", "split", "reading"].includes(parsed.editorMode ?? "")
        ? parsed.editorMode as EditorMode
        : "source",
    };
  } catch {
    return fallback;
  }
}

export const vaultState = reactive<VaultData>(loadVault());

export const uiState = reactive<UiState>({
  tool: "notes",
  noteFilter: "",
  commandOpen: false,
  contextOpen: true,
  explorerOpen: true,
  inspectorTab: "links",
  saveStatus: "saved",
  lastSavedAt: Date.now(),
  toast: null,
});

let persistTimer: ReturnType<typeof setTimeout> | undefined;
let toastTimer: ReturnType<typeof setTimeout> | undefined;

watch(
  vaultState,
  () => {
    uiState.saveStatus = "saving";
    clearTimeout(persistTimer);
    persistTimer = setTimeout(persistVault, PERSIST_DELAY);
  },
  { deep: true },
);

watch(
  () => vaultState.snippets.map((snippet) => [snippet.id, snippet.enabled, snippet.css]),
  applyEnabledSnippets,
  { deep: true, immediate: true },
);

export const activeNote = computed<Note | undefined>(() =>
  vaultState.notes.find((note) => note.id === vaultState.activeNoteId),
);

export const folderById = computed(() =>
  new Map(vaultState.folders.map((folder) => [folder.id, folder])),
);

export const folderNameMap = computed(() => {
  const names: Record<string, string> = {};
  for (const folder of vaultState.folders) names[folder.id] = folderPath(folder.id);
  return names;
});

export const visibleNotes = computed(() => {
  let notes = [...vaultState.notes];
  const selected = vaultState.selectedFolderId;

  if (selected === "favorites") notes = notes.filter((note) => note.pinned);
  else if (selected === "unfiled") notes = notes.filter((note) => note.folderId === null);
  else if (selected !== "all") {
    const folderIds = new Set([selected, ...descendantFolderIds(selected)]);
    notes = notes.filter((note) => note.folderId && folderIds.has(note.folderId));
  }

  const filter = uiState.noteFilter.trim();
  if (filter) {
    const matchingIds = new Set(
      searchNotes(notes, filter, { folderNames: folderNameMap.value, limit: notes.length })
        .map((result) => result.note.id),
    );
    notes = notes.filter((note) => matchingIds.has(note.id));
  }

  return notes.sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updatedAt - a.updatedAt);
});

export const outgoingLinks = computed(() => {
  if (!activeNote.value) return [];
  return parseWikiLinks(activeNote.value.content).map((link) => ({
    link,
    note: resolveWikiLink(link, vaultState.notes, activeNote.value),
  }));
});

export const backlinks = computed(() =>
  activeNote.value ? findBacklinks(activeNote.value, vaultState.notes) : [],
);

export function selectNote(id: string): void {
  if (!vaultState.notes.some((note) => note.id === id)) return;
  vaultState.activeNoteId = id;
}

export function selectFolder(selection: FolderSelection): void {
  vaultState.selectedFolderId = selection;
  uiState.tool = "notes";
  uiState.noteFilter = "";
}

export function setEditorMode(mode: EditorMode): void {
  vaultState.editorMode = mode;
}

export function createNote(folderId?: string | null, title = "Untitled note", content?: string): Note {
  const now = Date.now();
  const note: Note = {
    id: createId("note"),
    title: uniqueNoteTitle(title.trim() || "Untitled note"),
    content: content ?? "# Untitled note\n\n",
    folderId: folderId === undefined ? currentFolderId() : folderId,
    tags: [],
    pinned: false,
    createdAt: now,
    updatedAt: now,
  };
  if (content === undefined) note.content = `# ${note.title}\n\n`;
  vaultState.notes.unshift(note);
  vaultState.activeNoteId = note.id;
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
  if (!note) return;
  if (patch.title !== undefined) note.title = patch.title;
  if (patch.content !== undefined) note.content = patch.content;
  if (patch.folderId !== undefined) note.folderId = patch.folderId;
  if (patch.tags !== undefined) note.tags = patch.tags;
  if (patch.pinned !== undefined) note.pinned = patch.pinned;
  note.updatedAt = Date.now();
}

export function deleteNote(id: string): void {
  const index = vaultState.notes.findIndex((note) => note.id === id);
  if (index < 0) return;
  vaultState.notes.splice(index, 1);
  if (vaultState.activeNoteId === id) {
    vaultState.activeNoteId = vaultState.notes[Math.min(index, vaultState.notes.length - 1)]?.id ?? null;
  }
  notify("Note deleted", "neutral");
}

export function togglePinned(id: string): void {
  const note = vaultState.notes.find((candidate) => candidate.id === id);
  if (note) updateNote(id, { pinned: !note.pinned });
}

export function createFolder(name: string, parentId: string | null = null): Folder | undefined {
  const cleanName = name.trim().replace(/[\\/]/g, " ");
  if (!cleanName) return undefined;
  const duplicate = vaultState.folders.some(
    (folder) => folder.parentId === parentId && folder.name.toLocaleLowerCase() === cleanName.toLocaleLowerCase(),
  );
  if (duplicate) {
    notify("That folder already exists here", "warning");
    return undefined;
  }
  const folder: Folder = {
    id: createId("folder"),
    name: cleanName,
    parentId,
    createdAt: Date.now(),
  };
  vaultState.folders.push(folder);
  selectFolder(folder.id);
  notify(`Created ${cleanName}`, "success");
  return folder;
}

export function renameFolder(id: string, name: string): void {
  const folder = vaultState.folders.find((candidate) => candidate.id === id);
  const cleanName = name.trim().replace(/[\\/]/g, " ");
  if (folder && cleanName) folder.name = cleanName;
}

export function deleteFolder(id: string): void {
  const folder = vaultState.folders.find((candidate) => candidate.id === id);
  if (!folder) return;
  const children = vaultState.folders.filter((candidate) => candidate.parentId === id);
  for (const child of children) child.parentId = folder.parentId;
  for (const note of vaultState.notes.filter((candidate) => candidate.folderId === id)) {
    note.folderId = null;
  }
  vaultState.folders.splice(vaultState.folders.indexOf(folder), 1);
  if (vaultState.selectedFolderId === id) vaultState.selectedFolderId = "all";
  notify("Folder removed; its notes are now unfiled", "neutral");
}

export function createFromTemplate(templateId: string, requestedTitle?: string): Note | undefined {
  const template = vaultState.templates.find((candidate) => candidate.id === templateId);
  if (!template) return undefined;
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
  if (index >= 0) vaultState.snippets.splice(index, 1);
}

export function mergeImportedVault(
  result: ImportResult,
  replace = false,
): { noteCount: number; saved: boolean } {
  clearTimeout(persistTimer);
  if (replace) {
    vaultState.notes.splice(0);
    vaultState.folders.splice(0);
    vaultState.name = result.vaultName || "Imported vault";
  }

  const now = Date.now();
  for (const imported of result.notes) {
    const folderId = ensureFolderPath(imported.folderPath);
    vaultState.notes.push({
      id: createId("note"),
      title: imported.title || "Untitled note",
      content: imported.content,
      folderId,
      tags: imported.tags,
      pinned: false,
      createdAt: now,
      updatedAt: now,
    });
  }

  for (const imported of result.snippets) {
    const existing = vaultState.snippets.find(
      (snippet) => snippet.name.toLocaleLowerCase() === imported.name.toLocaleLowerCase(),
    );
    if (existing) continue;
    vaultState.snippets.push({
      id: createId("snippet"),
      name: imported.name,
      description: "Imported from an Obsidian CSS snippet.",
      css: imported.css,
      enabled: imported.enabled,
      createdAt: now,
    });
  }

  vaultState.activeNoteId = result.notes.length
    ? vaultState.notes[vaultState.notes.length - result.notes.length]?.id ?? vaultState.notes[0]?.id ?? null
    : replace
      ? null
      : vaultState.activeNoteId;
  vaultState.selectedFolderId = "all";
  const saved = persistVault();
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
  if (!id) return "";
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

export function clearVault(): boolean {
  clearTimeout(persistTimer);
  vaultState.notes.splice(0);
  vaultState.folders.splice(0);
  vaultState.activeNoteId = null;
  vaultState.selectedFolderId = "all";
  uiState.noteFilter = "";
  uiState.commandOpen = false;
  uiState.contextOpen = false;
  uiState.explorerOpen = true;
  const saved = persistVault();
  notify(saved ? "Vault cleared" : "Vault cleared, but not saved", saved ? "success" : "warning");
  return saved;
}

export function deleteVault(): boolean {
  clearTimeout(persistTimer);
  Object.assign(vaultState, createEmptyVault());
  uiState.noteFilter = "";
  uiState.commandOpen = false;
  uiState.contextOpen = false;
  uiState.explorerOpen = true;
  const saved = persistVault();
  notify(saved ? "Vault deleted" : "Vault deleted, but not saved", saved ? "success" : "warning");
  return saved;
}

function currentFolderId(): string | null {
  const selected = vaultState.selectedFolderId;
  return typeof selected === "string" && !["all", "favorites", "unfiled"].includes(selected)
    ? selected
    : activeNote.value?.folderId ?? null;
}

function uniqueNoteTitle(base: string): string {
  const normalized = new Set(vaultState.notes.map((note) => note.title.toLocaleLowerCase()));
  if (!normalized.has(base.toLocaleLowerCase())) return base;
  let suffix = 2;
  while (normalized.has(`${base} ${suffix}`.toLocaleLowerCase())) suffix += 1;
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

function persistVault(): boolean {
  if (typeof localStorage === "undefined") return false;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(vaultState));
    uiState.saveStatus = "saved";
    uiState.lastSavedAt = Date.now();
    return true;
  } catch {
    uiState.saveStatus = "error";
    return false;
  }
}

function applyEnabledSnippets(): void {
  if (typeof document === "undefined") return;
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
