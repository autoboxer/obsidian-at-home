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

export type AssetEmbedLocation =
  | "vault-root"
  | "note-folder"
  | "specified-folder"
  | "specified-folder-mirrored";

export interface AssetEmbedSettings {
  location: AssetEmbedLocation;
  folderPath: string;
}

export type ImageEmbedLocation = AssetEmbedLocation;
export type ImageEmbedSettings = AssetEmbedSettings;
export type AttachmentEmbedLocation = AssetEmbedLocation;
export type AttachmentEmbedSettings = AssetEmbedSettings;

export interface EmbeddedImage {
  id: string;
  relativePath: string;
  mediaType: string;
}

export interface VaultImageFile {
  assetId?: string;
  relativePath: string;
  mediaType: string;
}

export interface EmbeddedAttachment {
  id: string;
  relativePath: string;
  mediaType: string;
  byteLength: number;
  openingDisabled: boolean;
}

export interface VaultAttachmentFile {
  assetId?: string;
  relativePath: string;
  mediaType: string;
  byteLength: number;
  openingDisabled: boolean;
}

export interface AssetInsertionCapture {
  inTable: boolean;
  noteId: string;
  selectedText: string;
  token: string;
}

export type ImageInsertionCapture = AssetInsertionCapture;
export type AttachmentInsertionCapture = AssetInsertionCapture;

export interface VaultData {
  name: string;
  notes: Note[];
  folders: Folder[];
  templates: NoteTemplate[];
  snippets: CssSnippet[];
  activeNoteId: string | null;
  recentNoteIds: string[];
  selectedFolderId: "all" | "favorites" | "recent";
  embeddedImages: EmbeddedImage[];
  imageFiles: VaultImageFile[];
  imageEmbedSettings: ImageEmbedSettings;
  embeddedAttachments: EmbeddedAttachment[];
  attachmentFiles: VaultAttachmentFile[];
  attachmentEmbedSettings: AttachmentEmbedSettings;
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
  notePaths?: Record<string, string>;
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

export interface WorkspaceEmbedImageResult extends WorkspaceSaveResult {
  image: EmbeddedImage;
}

export interface WorkspaceEmbedAttachmentResult extends WorkspaceSaveResult {
  attachment: EmbeddedAttachment;
}

export interface WorkspaceImageNoteUpdate {
  noteId: string;
  relativePath: string;
  expectedContent: string;
  content: string;
}

export type WorkspaceAttachmentNoteUpdate = WorkspaceImageNoteUpdate;

export interface WorkspaceRelocateImageResult extends WorkspaceSaveResult {
  image: EmbeddedImage;
  previousRelativePath: string;
}

export interface WorkspaceRelocateAttachmentResult extends WorkspaceSaveResult {
  attachment: EmbeddedAttachment;
  previousRelativePath: string;
}

export interface WorkspaceAttachmentCopyResult {
  path: string;
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

export interface ImportedImage {
  relativePath: string;
}

export interface ImportedAttachment {
  relativePath: string;
}

export interface ImportedSnippet {
  name: string;
  css: string;
  enabled: boolean;
}

export interface ImportResult {
  vaultName: string;
  images: ImportedImage[];
  attachments: ImportedAttachment[];
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
  imageCount: number;
  attachmentCount: number;
  noteCount: number;
  templateCount: number;
  snippetCount: number;
  warnings: string[];
}

export interface WorkspaceImportAssetsResult extends WorkspaceSaveResult {
  imageCount: number;
  imageFiles: VaultImageFile[];
  attachmentCount: number;
  attachmentFiles: VaultAttachmentFile[];
  pathMappings: Record<string, string>;
  transactionId?: string;
}

export interface WorkspaceImportSaveResult extends WorkspaceSaveResult {
  saved: boolean;
  error?: string;
}
