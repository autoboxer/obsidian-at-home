export type SearchScope = "all" | "titles" | "content" | "tags";
export type ToolView = "notes" | "search" | "templates" | "snippets" | "settings";

export interface Note {
  id: string;
  title: string;
  content: string;
  relativePath: string;
  folderId: string | null;
  tags: string[];
  pinned: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface NoteEditorPosition {
  selection: {
    anchor: number;
    head: number;
  };
  viewport: {
    anchor: number;
    offset: number;
    left: number;
  };
}

export interface RecentlyDeletedNote {
  id: string;
  note: Note;
  originalFolderPath: string;
  deletedAt: number;
  expiresAt: number;
  editorPosition?: NoteEditorPosition;
}

export interface Folder {
  id: string;
  name: string;
  parentId: string | null;
  createdAt: number;
}

export interface NoteTemplate {
  id: string;
  name: string;
  description: string;
  titlePattern: string;
  content: string;
  glyph: string;
  createdAt: number;
  builtIn?: boolean;
}

export interface CssSnippet {
  id: string;
  name: string;
  description: string;
  css: string;
  enabled: boolean;
  createdAt: number;
  builtIn?: boolean;
}

export interface VaultData {
  name: string;
  notes: Note[];
  folders: Folder[];
  templates: NoteTemplate[];
  snippets: CssSnippet[];
  activeNoteId: string | null;
  recentNoteIds: string[];
  selectedFolderId: "all" | "favorites" | "recent";
}

export interface VaultDescriptor {
  name: string;
  path: string;
  lastOpenedAt: number;
}

export interface WorkspaceLoad {
  vault: VaultData;
  descriptor: VaultDescriptor;
  recentlyDeletedNotes: RecentlyDeletedNote[];
  editorPositions: Record<string, NoteEditorPosition>;
  editorPositionsRevision: string | null;
  editorPositionsWritable: boolean;
  revision: number;
  warnings: string[];
}

export interface WorkspaceBootstrap {
  workspace: WorkspaceLoad | null;
  recentVaults: VaultDescriptor[];
}

export interface WorkspaceSaveResult {
  revision: number;
  savedAt: number;
  warnings: string[];
}

export interface WorkspaceArchiveResult extends WorkspaceSaveResult {
  deletedNote: RecentlyDeletedNote;
}

export interface WorkspaceRestoreResult extends WorkspaceSaveResult {
  restoredNote: Note;
  editorPosition?: NoteEditorPosition;
}

export interface WorkspaceRecoveryMutationResult extends WorkspaceSaveResult {
  removedIds: string[];
}

export interface VaultSessionState {
  phase: "loading" | "needs-vault" | "ready" | "error";
  backend: "native" | "browser";
  path: string | null;
  recentVaults: VaultDescriptor[];
  error: string | null;
  busy: boolean;
  legacyAvailable: boolean;
  revision: number;
  conflict: boolean;
  warnings: string[];
}

export interface SearchResult {
  note: Note;
  score: number;
  snippet: string;
  reason: "title" | "content" | "tag" | "folder";
}

export interface WikiLink {
  raw: string;
  target: string;
  display: string;
  heading?: string;
  embedded: boolean;
  index: number;
}

export interface Backlink {
  note: Note;
  link: WikiLink;
  excerpt: string;
}

export interface ImportedNote {
  title: string;
  content: string;
  folderPath: string;
  relativePath: string;
  tags: string[];
}

export interface ImportedSnippet {
  name: string;
  css: string;
  enabled: boolean;
}

export interface ImportResult {
  vaultName: string;
  notes: ImportedNote[];
  snippets: ImportedSnippet[];
  warnings: string[];
}

export interface ExportNote {
  title: string;
  content: string;
  folderPath: string;
  tags: string[];
}

export interface ExportTemplate {
  name: string;
  content: string;
}

export interface ExportSnippet {
  name: string;
  css: string;
  enabled: boolean;
}

export interface ExportResult {
  path: string;
  noteCount: number;
  templateCount: number;
  snippetCount: number;
  warnings: string[];
}
