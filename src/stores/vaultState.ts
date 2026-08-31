import { reactive } from 'vue';
import { createEmptyVault } from '../data/seed';
import { isTauri, type WorkspaceVaultItemKind } from '../services/native';
import type {
  RecentlyDeletedNote,
  SearchScope,
  ToolView,
  VaultData,
  VaultSessionState
} from '../types';

export type FolderSelection = VaultData[ 'selectedFolderId' ];
export type SmartFolderSelection = 'all' | 'favorites' | 'recent';
export type SaveStatus = 'saved' | 'saving' | 'error';
export type ToastTone = 'neutral' | 'success' | 'warning';

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface UiState {
  tool: ToolView;
  notesView: 'editor' | 'recently-deleted';
  noteFilter: string;
  commandOpen: boolean;
  contextOpen: boolean;
  explorerOpen: boolean;
  frontmatterVisible: boolean;
  vaultChooserOpen: boolean;
  inspectorTab: 'links' | 'info';
  attachmentRefreshToken: number;
  imageRefreshToken: number;
  saveStatus: SaveStatus;
  lastSavedAt: number;
  zoom: number;
  toast: { action?: ToastAction; id: number; message: string; tone: ToastTone } | null;
}

export interface SearchState {
  query: string;
  scope: SearchScope;
  exactTag: string | null;
  quickQuery: string;
  focusRequest: number;
}

export interface WorkspaceUiSnapshot {
  tool: ToolView;
  notesView: UiState[ 'notesView' ];
  noteFilter: string;
}

export interface RecentlyDeletedState {
  notes: RecentlyDeletedNote[];
  busy: boolean;
  error: string | null;
}

export const treeDragState = reactive<{
  attachmentPath: string | null;
  noteId: string | null;
  folderId: string | null;
  imagePath: string | null;
}>({
  attachmentPath: null,
  noteId: null,
  folderId: null,
  imagePath: null
});

export const vaultImageInsertRequest = reactive({
  id: 0,
  relativePath: ''
});

export const vaultAttachmentInsertRequest = reactive({
  id: 0,
  relativePath: ''
});

export const vaultTreeRevealTarget = reactive<{
  assetId: string | null;
  kind: WorkspaceVaultItemKind | null;
  relativePath: string;
  requestId: number;
  vaultKey: string;
}>({
  assetId: null,
  kind: null,
  relativePath: '',
  requestId: 0,
  vaultKey: ''
});

export const vaultState = reactive<VaultData>( createEmptyVault() );

export const recentlyDeletedState = reactive<RecentlyDeletedState>({
  notes: [],
  busy: false,
  error: null
});

export const vaultSession = reactive<VaultSessionState>({
  phase: 'loading',
  backend: isTauri() ? 'native' : 'browser',
  path: null,
  recentVaults: [],
  error: null,
  busy: false,
  legacyAvailable: false,
  revision: 0,
  conflict: false,
  warnings: []
});

export const uiState = reactive<UiState>({
  tool: 'notes',
  notesView: 'editor',
  noteFilter: '',
  commandOpen: false,
  contextOpen: true,
  explorerOpen: true,
  frontmatterVisible: false,
  vaultChooserOpen: false,
  inspectorTab: 'links',
  attachmentRefreshToken: 0,
  imageRefreshToken: 0,
  saveStatus: 'saved',
  lastSavedAt: Date.now(),
  zoom: 1,
  toast: null
});

export const searchState = reactive<SearchState>({
  query: '',
  scope: 'all',
  exactTag: null,
  quickQuery: '',
  focusRequest: 0
});
