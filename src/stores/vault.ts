import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { computed, watch } from 'vue';
import { createEmptyVault, createSeedVault } from '../data/seed';
import { findBacklinks, parseWikiLinks, resolveWikiLink, searchNotes } from '../lib';
import { resolveMarkdownImagePath } from '../lib/imageEmbeds';
import {
  formatMarkdownImage,
  parseMarkdownImages,
  relativeImageDestination
} from '../lib/markdownImages';
import {
  formatMarkdownAttachment,
  markdownAttachmentIsArchive,
  markdownAttachmentIsExecutable,
  parseMarkdownAttachments,
  relativeAttachmentDestination
} from '../lib/markdownAttachments';
import {
  compareRecentlyDeletedNotes,
  readBrowserWorkspace,
  RECENTLY_DELETED_LIMIT,
  RECENTLY_DELETED_RETENTION,
  writeBrowserWorkspace
} from '../services/browserWorkspace';
import type { StoredBrowserWorkspace } from '../services/browserWorkspace';
import {
  captureNoteEditorPosition,
  deleteNoteEditorPosition,
  editorPositionVaultId,
  flushNoteEditorPositions,
  hasPendingNoteEditorPositions,
  initializeNoteEditorPositions,
  pruneNoteEditorPositions,
  setNoteEditorPosition
} from './editorPositions';
import {
  deleteNoteEditorHistory,
  pruneNoteEditorHistories
} from './editorHistories';
import {
  applyMarkdownReplacements,
  folderContainsVaultAssets,
  isSafeVaultAttachmentFileName,
  isSafeVaultImageFileName,
  rebuildVaultAssetFolders,
  rewriteVaultAssetDestinationsForNotePath,
  rewriteVaultAttachmentReferences,
  rewriteVaultImageReferences,
  upsertVaultAttachmentFile,
  upsertVaultImageFile
} from './vaultAssets';
import {
  createId,
  descendantFolderIds as vaultDescendantFolderIds,
  ensureFolderPath as ensureVaultFolderPath,
  folderConflictsWithNote,
  folderPathFromFolders,
  noteStemKey,
  projectedNoteRelativePath,
  safeNoteStem,
  uniqueNoteTitle as uniqueVaultNoteTitle
} from './vaultModel';
import { createVaultContent } from './vaultContent';
import {
  clampZoom,
  cloneValue,
  errorMessage,
  isRevisionConflict,
  mergeRecentVaults,
  normalizeVault,
  persistStoredZoom,
  readStoredZoom,
  safeStorageGet,
  safeStorageSet,
  zoomStep
} from './vaultPersistence';
import {
  createVaultNavigation,
  type NoteNavigationState
} from './vaultNavigation';
import {
  recentlyDeletedState,
  uiState,
  vaultAttachmentInsertRequest,
  vaultImageInsertRequest,
  vaultSession,
  vaultState,
  vaultTreeRevealTarget,
  type ToastAction,
  type ToastTone,
  type WorkspaceUiSnapshot
} from './vaultState';
export {
  recentlyDeletedState,
  searchState,
  treeDragState,
  uiState,
  vaultAttachmentInsertRequest,
  vaultImageInsertRequest,
  vaultSession,
  vaultState,
  vaultTreeRevealTarget
} from './vaultState';
export { MAX_ZOOM, MIN_ZOOM } from './vaultPersistence';
import {
  archiveWorkspaceNote,
  bootstrapWorkspace,
  createWorkspace,
  deleteRecentlyDeletedNotes,
  forgetWorkspace,
  getWorkspaceRevision,
  importWorkspaceAssets,
  isTauri,
  locateWorkspaceVaultItem,
  openWorkspaceAttachment,
  openWorkspace,
  pickFolder,
  pruneRecentlyDeletedNotes,
  relocateWorkspaceAttachment,
  relocateWorkspaceImage,
  restoreRecentlyDeletedNote as restoreRecentlyDeletedNoteNative,
  saveWorkspace,
  saveWorkspaceAttachmentCopy,
  saveWorkspaceWithImageImport,
  showWorkspaceVaultItemInFolder,
  type WorkspaceVaultItemKind
} from '../services/native';
import type {
  ExportNote,
  ExportSnippet,
  ExportTemplate,
  Folder,
  ImportResult,
  Note,
  RecentlyDeletedNote,
  VaultData,
  VaultAttachmentFile,
  VaultImageFile,
  WorkspaceEmbedAttachmentResult,
  WorkspaceExternalAssetDiscardResult,
  WorkspaceEmbedImageResult,
  WorkspaceAttachmentNoteUpdate,
  WorkspaceImageNoteUpdate,
  WorkspaceLoad,
  WorkspaceRelocateImageResult,
  WorkspaceRelocateAttachmentResult,
  WorkspaceSaveResult
} from '../types';

const LEGACY_MIGRATED_KEY = 'obsidian-at-home.vault.filesystem-migrated.v1';
const PERSIST_DELAY = 220;
const EXTERNAL_CHECK_DELAY = 3_000;
const RECENTLY_DELETED_RETRY_INITIAL_DELAY = 5_000;
const RECENTLY_DELETED_RETRY_MAX_DELAY = 5 * 60_000;

export const NOTE_DRAG_MIME = 'application/x-obsidian-at-home-note-id';
export const FOLDER_DRAG_MIME = 'application/x-obsidian-at-home-folder-id';

let vaultTreeRevealOperation = 0;
uiState.zoom = readStoredZoom();
let persistTimer: ReturnType<typeof setTimeout> | undefined;
let toastTimer: ReturnType<typeof setTimeout> | undefined;
let externalCheckTimer: ReturnType<typeof setInterval> | undefined;
let recentlyDeletedTimer: ReturnType<typeof setTimeout> | undefined;
let recentlyDeletedRetryDelay = RECENTLY_DELETED_RETRY_INITIAL_DELAY;
let initialized = false;
let suppressPersistence = 0;
let dirtyVersion = 0;
let savedVersion = 0;
let sessionGeneration = 0;
let saveInFlight: Promise<boolean> | null = null;
let recoverySaveInFlight: Promise<boolean> | null = null;
let checkingExternalChanges = false;
let initializePromise: Promise<void> | null = null;
let closeHandlerInstalled = false;
let closingAfterSave = false;
const pendingNoteOriginalPaths = new Map<string, string>();

watch(
  vaultState,
  () => {
    if ( !initialized || suppressPersistence ) {
      return;
    }
    dirtyVersion += 1;
    uiState.saveStatus = 'saving';
    clearTimeout( persistTimer );
    persistTimer = setTimeout( () => void flushApplicationState(), PERSIST_DELAY );
  },
  { deep: true, flush: 'sync' }
);

watch(
  () => vaultState.snippets.map( ( snippet ) => [ snippet.id, snippet.enabled, snippet.css ]),
  applyEnabledSnippets,
  { deep: true, immediate: true }
);

watch(
  () => uiState.zoom,
  persistStoredZoom,
  { flush: 'sync' }
);

export function initializeVault(): Promise<void> {
  if ( initializePromise ) {
    return initializePromise;
  }
  initializePromise = initializeVaultStorage();

  return initializePromise;
}

async function initializeVaultStorage(): Promise<void> {
  vaultSession.error = null;
  vaultSession.phase = 'loading';

  if ( !isTauri() ) {
    let storedVault: StoredBrowserWorkspace | null;
    try {
      storedVault = readStoredVault();
    } catch ( error ) {
      hydrateVault( createEmptyVault() );
      hydrateRecentlyDeletedNotes([]);
      resetNoteNavigation();
      vaultSession.backend = 'browser';
      vaultSession.phase = 'error';
      vaultSession.error = errorMessage( error, 'Saved browser notes could not be read safely.' );
      initialized = true;
      installVaultLifecycleHandlers();

      return;
    }
    const browserVault = storedVault?.vault ?? createSeedVault();
    hydrateVault( browserVault );
    hydrateRecentlyDeletedNotes( storedVault?.recentlyDeletedNotes ?? []);
    initializeNoteEditorPositions( 'browser', null, vaultState.notes );
    resetNoteNavigation();
    vaultSession.backend = 'browser';
    vaultSession.phase = 'ready';
    vaultSession.path = null;
    vaultSession.recentVaults = [];
    vaultSession.legacyAvailable = false;
    vaultSession.revision = 0;
    vaultSession.conflict = false;
    vaultSession.warnings = [];
    initialized = true;
    savedVersion = dirtyVersion;
    if ( storedVault?.needsRewrite ) {
      persistBrowserWorkspace( snapshotVault(), snapshotRecentlyDeletedNotes() );
    }
    scheduleRecentlyDeletedExpiry();
    void pruneExpiredRecentlyDeletedNotes();
    installVaultLifecycleHandlers();

    return;
  }

  vaultSession.backend = 'native';
  let legacy: StoredBrowserWorkspace | null = null;
  try {
    legacy = readStoredVault();
  } catch {
    // A newer browser workspace remains untouched and unavailable for migration
  }
  vaultSession.legacyAvailable = Boolean(
    legacy && safeStorageGet( LEGACY_MIGRATED_KEY ) !== legacy.migrationFingerprint
  );

  try {
    const result = await bootstrapWorkspace( createEmptyVault() );
    vaultSession.recentVaults = result.recentVaults;
    if ( result.workspace ) {
      applyWorkspace( result.workspace, result.recentVaults );
    } else {
      hydrateVault( createEmptyVault() );
      hydrateRecentlyDeletedNotes([]);
      resetNoteNavigation();
      vaultSession.phase = 'needs-vault';
      vaultSession.path = null;
      vaultSession.revision = 0;
      vaultSession.conflict = false;
      vaultSession.warnings = [];
      uiState.vaultChooserOpen = true;
    }
  } catch ( error ) {
    hydrateVault( createEmptyVault() );
    hydrateRecentlyDeletedNotes([]);
    resetNoteNavigation();
    vaultSession.phase = 'error';
    vaultSession.error = errorMessage( error, 'The vault list could not be opened.' );
    uiState.vaultChooserOpen = true;
  } finally {
    initialized = true;
    savedVersion = dirtyVersion;
    installVaultLifecycleHandlers();
  }
}

export async function createFilesystemVault( name: string, useLegacy = false ): Promise<boolean> {
  if ( vaultSession.backend !== 'native' || vaultSession.busy ) {
    return false;
  }
  const cleanName = name.trim();
  if ( !cleanName ) {
    return false;
  }

  vaultSession.busy = true;
  vaultSession.error = null;
  try {
    if ( !( await flushBeforeVaultChange() ) ) {
      return false;
    }

    const parentPath = await pickFolder();
    if ( !parentPath ) {
      return false;
    }
    const legacy = useLegacy ? readStoredVault() : null;
    if ( useLegacy && !legacy ) {
      throw new Error( 'The previous notes could not be read from app storage.' );
    }
    const initial = legacy?.vault ?? createSeedVault();
    const workspace = await createWorkspace( parentPath, cleanName, initial );
    applyWorkspace( workspace );

    if ( useLegacy && legacy ) {
      safeStorageSet( LEGACY_MIGRATED_KEY, legacy.migrationFingerprint );
      vaultSession.legacyAvailable = false;
      notify( `Saved ${ legacy.vault.notes.length } ${ legacy.vault.notes.length === 1 ? 'note' : 'notes' } as Markdown files`, 'success' );
    } else {
      notify( `Created ${ workspace.descriptor.name }`, 'success' );
    }

    return true;
  } catch ( error ) {
    setVaultError( error, 'The vault could not be created.' );

    return false;
  } finally {
    vaultSession.busy = false;
    scheduleRecentlyDeletedExpiry();
  }
}

export async function openFilesystemVault(): Promise<boolean> {
  if ( vaultSession.backend !== 'native' || vaultSession.busy ) {
    return false;
  }

  vaultSession.busy = true;
  vaultSession.error = null;
  try {
    if ( !( await flushBeforeVaultChange() ) ) {
      return false;
    }

    const path = await pickFolder();
    if ( !path ) {
      return false;
    }
    const workspace = await openWorkspace( path, createEmptyVault() );
    applyWorkspace( workspace );
    notify( `Opened ${ workspace.descriptor.name }`, 'success' );

    return true;
  } catch ( error ) {
    setVaultError( error, 'That folder could not be opened as a vault.' );

    return false;
  } finally {
    vaultSession.busy = false;
    scheduleRecentlyDeletedExpiry();
  }
}

export async function switchFilesystemVault( path: string ): Promise<boolean> {
  if ( vaultSession.backend !== 'native' || vaultSession.busy || path === vaultSession.path ) {
    return path === vaultSession.path;
  }

  vaultSession.busy = true;
  vaultSession.error = null;
  try {
    if ( !( await flushBeforeVaultChange() ) ) {
      return false;
    }

    const workspace = await openWorkspace( path, createEmptyVault() );
    applyWorkspace( workspace );
    notify( `Switched to ${ workspace.descriptor.name }`, 'success' );

    return true;
  } catch ( error ) {
    setVaultError( error, 'That recent vault is no longer available.' );

    return false;
  } finally {
    vaultSession.busy = false;
    scheduleRecentlyDeletedExpiry();
  }
}

export async function forgetCurrentVault(): Promise<boolean> {
  const path = vaultSession.path;
  if ( vaultSession.backend !== 'native' || !path || vaultSession.busy ) {
    return false;
  }

  vaultSession.busy = true;
  vaultSession.error = null;
  try {
    if ( !( await flushBeforeVaultChange() ) ) {
      return false;
    }

    const recentVaults = await forgetWorkspace( path );
    sessionGeneration += 1;
    vaultSession.recentVaults = recentVaults;
    vaultSession.path = null;
    vaultSession.revision = 0;
    vaultSession.conflict = false;
    vaultSession.warnings = [];
    vaultSession.phase = 'needs-vault';
    hydrateVault( createEmptyVault() );
    hydrateRecentlyDeletedNotes([]);
    resetNoteNavigation();
    dirtyVersion = 0;
    savedVersion = 0;
    uiState.vaultChooserOpen = true;
    notify( 'Vault forgotten; its files are still on disk', 'neutral' );

    return true;
  } catch ( error ) {
    setVaultError( error, 'The vault could not be removed from the recent list.' );

    return false;
  } finally {
    vaultSession.busy = false;
    scheduleRecentlyDeletedExpiry();
  }
}

export async function showCurrentVaultInFolder(): Promise<void> {
  if ( !vaultSession.path || vaultSession.backend !== 'native' ) {
    return;
  }
  try {
    await revealItemInDir( vaultSession.path );
  } catch ( error ) {
    setVaultError( error, 'The vault folder could not be shown.' );
    throw error;
  }
}

export async function reloadFilesystemVault(): Promise<boolean> {
  const path = vaultSession.path;
  if ( vaultSession.backend !== 'native' || !path || vaultSession.busy ) {
    return false;
  }

  vaultSession.busy = true;
  try {
    await flushNoteEditorPositions( currentEditorPositionVaultId() );
    const workspace = await openWorkspace( path, createEmptyVault() );
    applyWorkspace( workspace );
    notify( 'Reloaded the vault from disk', 'success' );

    return true;
  } catch ( error ) {
    setVaultError( error, 'The vault could not be reloaded from disk.' );

    return false;
  } finally {
    vaultSession.busy = false;
    scheduleRecentlyDeletedExpiry();
  }
}

export async function overwriteFilesystemVault(): Promise<boolean> {
  const path = vaultSession.path;
  if ( vaultSession.backend !== 'native' || !path || vaultSession.busy ) {
    return false;
  }
  vaultSession.busy = true;
  clearTimeout( persistTimer );
  try {
    const targetVersion = dirtyVersion;
    const currentRevision = await getWorkspaceRevision( path );
    const result = await saveWorkspace( path, snapshotVaultForSave(), currentRevision );
    applySavedNotePaths( result.notePaths );
    vaultSession.revision = result.revision;
    vaultSession.error = null;
    vaultSession.conflict = false;
    vaultSession.warnings = result.warnings;
    savedVersion = targetVersion;
    uiState.saveStatus = 'saved';
    uiState.lastSavedAt = result.savedAt || Date.now();
    notify( 'Saved the app version over the changed files', 'success' );

    return true;
  } catch ( error ) {
    const message = errorMessage( error, 'The app version could not be saved.' );
    vaultSession.error = message;
    vaultSession.conflict = isRevisionConflict( message );
    uiState.saveStatus = 'error';

    return false;
  } finally {
    vaultSession.busy = false;
    scheduleRecentlyDeletedExpiry();
  }
}

export async function flushVault( imageImportTransactionId?: string ): Promise<boolean> {
  clearTimeout( persistTimer );

  if ( recoverySaveInFlight ) {
    const saved = await recoverySaveInFlight;
    if ( !saved ) {
      return false;
    }

    return savedVersion < dirtyVersion
      ? flushVault( imageImportTransactionId )
      : true;
  }

  if ( !initialized || ( savedVersion >= dirtyVersion && !imageImportTransactionId ) ) {
    return true;
  }

  if ( saveInFlight ) {
    const saved = await saveInFlight;
    if ( !saved ) {
      return false;
    }

    return savedVersion < dirtyVersion
      ? flushVault( imageImportTransactionId )
      : true;
  }

  const targetVersion = dirtyVersion;
  const generation = sessionGeneration;
  const path = vaultSession.path;
  const snapshot = snapshotVaultForSave();
  const recentlyDeletedSnapshot = snapshotRecentlyDeletedNotes();
  uiState.saveStatus = 'saving';

  const operation = ( async (): Promise<boolean> => {
    if ( vaultSession.backend === 'browser' ) {
      const saved = persistBrowserWorkspace( snapshot, recentlyDeletedSnapshot );
      if ( saved && generation === sessionGeneration ) {
        savedVersion = targetVersion;
      }

      return saved;
    }

    if ( vaultSession.phase !== 'ready' || !path ) {
      uiState.saveStatus = 'error';

      return false;
    }

    try {
      let result: WorkspaceSaveResult;
      if ( imageImportTransactionId ) {
        const importResult = await saveWorkspaceWithImageImport(
          path,
          snapshot,
          vaultSession.revision,
          imageImportTransactionId
        );
        if ( !importResult.saved ) {
          if ( generation !== sessionGeneration || path !== vaultSession.path ) {
            return false;
          }
          vaultSession.revision = importResult.revision;
          vaultSession.warnings = importResult.warnings;
          uiState.saveStatus = 'error';
          const message = importResult.error || 'The imported notes could not be saved.';
          vaultSession.error = message;
          vaultSession.conflict = isRevisionConflict( message );
          uiState.commandOpen = false;
          notify( message, 'warning' );

          return false;
        }
        result = importResult;
      } else {
        result = await saveWorkspace( path, snapshot, vaultSession.revision );
      }
      if ( generation !== sessionGeneration || path !== vaultSession.path ) {
        return true;
      }
      applySavedNotePaths( result.notePaths );
      vaultSession.revision = result.revision;
      vaultSession.error = null;
      vaultSession.conflict = false;
      vaultSession.warnings = result.warnings;
      savedVersion = targetVersion;
      uiState.saveStatus = 'saved';
      uiState.lastSavedAt = result.savedAt || Date.now();
      if ( result.warnings.length ) {
        notify( result.warnings[ 0 ], 'warning' );
      }

      return true;
    } catch ( error ) {
      if ( generation !== sessionGeneration ) {
        return false;
      }
      uiState.saveStatus = 'error';
      const message = errorMessage( error, 'Changes could not be written to the vault folder.' );
      vaultSession.error = message;
      vaultSession.conflict = isRevisionConflict( message );
      uiState.commandOpen = false;
      uiState.vaultChooserOpen = true;
      notify( message, 'warning' );

      return false;
    }
  })();

  saveInFlight = operation;
  const saved = await operation;
  if ( saveInFlight === operation ) {
    saveInFlight = null;
  }
  if ( saved && generation === sessionGeneration && savedVersion < dirtyVersion ) {
    return flushVault();
  }

  return saved;
}

export const activeNote = computed<Note | undefined>( () =>
  vaultState.notes.find( ( note ) => note.id === vaultState.activeNoteId )
);

export const recentNotes = computed<Note[]>( () => {
  const notesById = new Map( vaultState.notes.map( ( note ) => [ note.id, note ]) );

  return vaultState.recentNoteIds.flatMap( ( id ) => {
    const note = notesById.get( id );

    return note ? [ note ] : [];
  });
});

export const recentlyDeletedNotes = computed<RecentlyDeletedNote[]>( () =>
  [ ...recentlyDeletedState.notes ].sort( compareRecentlyDeletedNotes )
);

export const folderById = computed( () =>
  new Map( vaultState.folders.map( ( folder ) => [ folder.id, folder ]) )
);

export const folderNameMap = computed( () => {
  const names: Record<string, string> = {};
  for ( const folder of vaultState.folders ) {
    names[ folder.id ] = folderPath( folder.id );
  }

  return names;
});

export const visibleNotes = computed( () => {
  let notes = vaultState.selectedFolderId === 'recent'
    ? [ ...recentNotes.value ]
    : [ ...vaultState.notes ];

  if ( vaultState.selectedFolderId === 'favorites' ) {
    notes = notes.filter( ( note ) => note.pinned );
  }

  const filter = uiState.noteFilter.trim();
  if ( filter ) {
    const matchingIds = new Set(
      searchNotes( notes, filter, { folderNames: folderNameMap.value, limit: notes.length })
        .map( ( result ) => result.note.id )
    );
    notes = notes.filter( ( note ) => matchingIds.has( note.id ) );
  }

  return vaultState.selectedFolderId === 'recent'
    ? notes
    : notes.sort( ( a, b ) => Number( b.pinned ) - Number( a.pinned ) || b.updatedAt - a.updatedAt );
});

const vaultNavigation = createVaultNavigation({
  folderPath,
  isNoteVisible: ( id ) => visibleNotes.value.some( ( note ) => note.id === id )
});

export const {
  backNavigationNote,
  canNavigateBack,
  canNavigateForward,
  forwardNavigationNote,
  navigateBack,
  navigateForward,
  openQuickSearch,
  openRecentlyDeletedWorkspace,
  openSearchWorkspace,
  selectFolder,
  selectNote
} = vaultNavigation;

const {
  activateNote,
  activateNoteAfterDeletion,
  currentFolderId,
  noteDeletionFallback,
  noteExists,
  pruneNoteNavigation,
  recordDirectNoteNavigation,
  removeNoteFromNavigation,
  removeRecentNote,
  resetNoteNavigation,
  resetSearchState,
  restoreNoteNavigation,
  snapshotNoteNavigation,
  touchRecentNote
} = vaultNavigation;

const vaultContent = createVaultContent({
  activeNote: () => activeNote.value,
  currentFolderId,
  flushVault,
  folderContainsAssets,
  notify,
  rememberNoteOriginalPath,
  selectNote
});

export const {
  createFolder,
  createFromTemplate,
  createLinkedNote,
  createNote,
  deleteFolder,
  deleteSnippet,
  moveFolder,
  moveNoteToFolder,
  renameFolder,
  saveSnippet,
  saveTemplate,
  togglePinned,
  updateNote
} = vaultContent;

export const outgoingLinks = computed( () => {
  if ( !activeNote.value ) {
    return [];
  }

  return parseWikiLinks( activeNote.value.content ).map( ( link ) => ({
    link,
    note: resolveWikiLink( link, vaultState.notes, activeNote.value )
  }) );
});

export const backlinks = computed( () =>
  activeNote.value ? findBacklinks( activeNote.value, vaultState.notes ) : []
);

export function setZoom( zoom: number ): void {
  uiState.zoom = clampZoom( zoom );
}

export function zoomIn(): void {
  setZoom( uiState.zoom + zoomStep() );
}

export function zoomOut(): void {
  setZoom( uiState.zoom - zoomStep() );
}

export function resetZoom(): void {
  setZoom( 1 );
}

export async function deleteNote( id: string ): Promise<boolean> {
  return runRecoveryOperation( async () => {
    if ( !( await flushVault() ) ) {
      return false;
    }

    const index = vaultState.notes.findIndex( ( note ) => note.id === id );
    const note = vaultState.notes[ index ];
    if ( !note ) {
      return false;
    }

    const archivedNote = cloneValue( note );
    const originalFolderPath = folderPath( note.folderId );
    const vaultId = currentEditorPositionVaultId();
    const editorPosition = captureNoteEditorPosition( vaultId, note.id, note.content );
    const previousVault = snapshotVault();
    const previousNavigation = snapshotNoteNavigation();
    const previousWorkspaceUi = snapshotWorkspaceUi();

    if ( vaultSession.backend === 'browser' ) {
      if ( recentlyDeletedState.notes.length >= RECENTLY_DELETED_LIMIT ) {
        recentlyDeletedState.error = 'Recently Deleted is full.';
        notify( 'Recently Deleted is full, so the note was not deleted', 'warning' );

        return false;
      }
      const candidateVault = snapshotVaultAfterDeletion( id );
      const deletedAt = Date.now();
      const deletedNote: RecentlyDeletedNote = {
        id: createId( 'deleted' ),
        note: archivedNote,
        originalFolderPath,
        deletedAt,
        expiresAt: deletedAt + RECENTLY_DELETED_RETENTION,
        ...( editorPosition ? { editorPosition } : {})
      };
      const candidateDeletedNotes = [
        deletedNote,
        ...snapshotRecentlyDeletedNotes()
      ].sort( compareRecentlyDeletedNotes );

      if ( !persistBrowserWorkspace( candidateVault, candidateDeletedNotes ) ) {
        recentlyDeletedState.error = 'The note could not be moved to Recently Deleted.';
        notify( 'The note was not deleted because browser storage is full or unavailable', 'warning' );

        return false;
      }

      applyVaultMutation( () => applyNoteDeletion( id ) );
      hydrateRecentlyDeletedNotes( candidateDeletedNotes );
      deleteNoteEditorPosition( vaultId, id );
      deleteNoteEditorHistory( vaultId, id );
      savedVersion = dirtyVersion;
      recentlyDeletedState.error = null;
      notify( 'Note moved to Recently Deleted', 'neutral' );
      scheduleRecentlyDeletedExpiry();

      return true;
    }

    const path = vaultSession.path;
    if ( !path ) {
      return false;
    }

    applyVaultMutation( () => applyNoteDeletion( id ) );
    const candidateVault = snapshotVault();
    const saved = await performNativeRecoverySave(
      () => archiveWorkspaceNote(
        path,
        candidateVault,
        archivedNote,
        originalFolderPath,
        editorPosition,
        vaultSession.revision
      ),
      ( result ) => {
        applyWorkspaceSaveResult( result );
        hydrateRecentlyDeletedNotes([
          result.deletedNote,
          ...recentlyDeletedState.notes
        ]);
        deleteNoteEditorPosition( vaultId, id );
        deleteNoteEditorHistory( vaultId, id );
      },
      async () => {
        const workspace = await reconcileNativeWorkspace( path );
        if ( workspace ) {
          if ( workspace.vault.notes.some( ( candidate ) => candidate.id === id ) ) {
            restoreNoteNavigation( previousNavigation );
            restoreWorkspaceUi( previousWorkspaceUi );
          }

          return true;
        }

        restoreFailedNoteDeletion(
          index,
          archivedNote,
          previousVault,
          previousNavigation,
          previousWorkspaceUi
        );

        return false;
      },
      'The note could not be moved to Recently Deleted.'
    );
    if ( !saved ) {
      return false;
    }

    if ( !( await flushNoteEditorPositions( vaultId ) ) ) {
      addVaultWarning( 'The note was recovered safely, but its old editor position could not be removed.' );
    } else {
      notifyRecoverySuccess( 'Note moved to Recently Deleted', 'neutral' );
    }
    scheduleRecentlyDeletedExpiry();

    return true;
  });
}

export async function restoreRecentlyDeletedNote( id: string ): Promise<boolean> {
  return runRecoveryOperation( async () => {
    if ( !( await flushVault() ) ) {
      return false;
    }

    const deletedNote = recentlyDeletedState.notes.find( ( entry ) => entry.id === id );
    if ( !deletedNote ) {
      return false;
    }
    if ( deletedNote.expiresAt <= Date.now() ) {
      recentlyDeletedState.error = 'That deleted note has expired and can no longer be restored.';
      notify( recentlyDeletedState.error, 'warning' );

      return false;
    }
    const previousActiveNoteId = vaultState.activeNoteId;
    const vaultId = currentEditorPositionVaultId();

    if ( vaultSession.backend === 'browser' ) {
      const restoredNote = buildBrowserRestoredNote( deletedNote );
      const candidateVault = snapshotVaultWithRestoredNote( restoredNote );
      const candidateDeletedNotes = recentlyDeletedState.notes.filter( ( entry ) => entry.id !== id );
      let editorPositionSaved = true;
      if ( deletedNote.editorPosition ) {
        setNoteEditorPosition( vaultId, restoredNote.id, deletedNote.editorPosition );
        editorPositionSaved = await flushNoteEditorPositions( vaultId );
      }
      if ( !persistBrowserWorkspace( candidateVault, candidateDeletedNotes ) ) {
        if ( deletedNote.editorPosition ) {
          deleteNoteEditorPosition( vaultId, restoredNote.id );
          void flushNoteEditorPositions( vaultId );
        }
        recentlyDeletedState.error = 'That note could not be restored.';
        notify( 'The note was not restored because browser storage is full or unavailable', 'warning' );

        return false;
      }

      applyVaultMutation( () => applyRestoredNote( restoredNote, previousActiveNoteId ) );
      hydrateRecentlyDeletedNotes( candidateDeletedNotes );
      savedVersion = dirtyVersion;
      recentlyDeletedState.error = null;
      if ( editorPositionSaved ) {
        notify( `Restored ${ restoredNote.title }`, 'success' );
      } else {
        addVaultWarning( 'The note was restored, but its editor position could not be saved.' );
      }
      scheduleRecentlyDeletedExpiry();

      return true;
    }

    const path = vaultSession.path;
    if ( !path ) {
      return false;
    }
    const saved = await performNativeRecoverySave(
      () => restoreRecentlyDeletedNoteNative( path, id, snapshotVault(), vaultSession.revision ),
      ( result ) => {
        applyWorkspaceSaveResult( result );
        applyVaultMutation( () => applyRestoredNote( result.restoredNote, previousActiveNoteId ) );
        removeRecentlyDeletedEntries([ id ]);
        if ( result.editorPosition ) {
          setNoteEditorPosition( vaultId, result.restoredNote.id, result.editorPosition );
        }
      },
      async () => Boolean( await reconcileNativeWorkspace( path ) ),
      'That note could not be restored.'
    );
    if ( !saved ) {
      return false;
    }

    if ( !( await flushNoteEditorPositions( vaultId ) ) ) {
      addVaultWarning( 'The note was restored, but its editor position could not be saved.' );
    } else {
      const restoredTitle = vaultState.notes.find( ( note ) => note.id === vaultState.activeNoteId )?.title
        ?? deletedNote.note.title;
      notifyRecoverySuccess( `Restored ${ restoredTitle }`, 'success' );
    }
    scheduleRecentlyDeletedExpiry();

    return true;
  });
}

export async function permanentlyDeleteRecentlyDeletedNote( id: string ): Promise<boolean> {
  return removeRecentlyDeletedNotes([ id ], 'Note deleted permanently' );
}

export async function emptyRecentlyDeletedNotes(): Promise<boolean> {
  const ids = recentlyDeletedState.notes.map( ( entry ) => entry.id );
  if ( !ids.length ) {
    return true;
  }

  return removeRecentlyDeletedNotes( ids, 'Recently Deleted emptied' );
}

export async function mergeImportedVault(
  result: ImportResult,
  sourcePath: string,
  replace = false
): Promise<{
  attachmentCount: number;
  imageCount: number;
  noteCount: number;
  saved: boolean;
  warnings: string[];
}> {
  return runExclusiveVaultDataOperation(
    {
      attachmentCount: 0,
      imageCount: 0,
      noteCount: result.notes.length,
      saved: false,
      warnings: []
    },
    () => mergeImportedVaultExclusive( result, sourcePath, replace )
  );
}

async function mergeImportedVaultExclusive(
  result: ImportResult,
  sourcePath: string,
  replace: boolean
): Promise<{
  attachmentCount: number;
  imageCount: number;
  noteCount: number;
  saved: boolean;
  warnings: string[];
}> {
  if ( !( await flushVault() ) ) {
    return {
      attachmentCount: 0,
      imageCount: 0,
      noteCount: result.notes.length,
      saved: false,
      warnings: []
    };
  }
  clearTimeout( persistTimer );
  const previousSavedVersion = savedVersion;
  const previousActiveNoteId = vaultState.activeNoteId;
  const previousNoteNavigation = snapshotNoteNavigation();
  const previousVault = snapshotVault();
  let attachmentCount = 0;
  let imageCount = 0;
  let imageImportTransactionId: string | undefined;
  const warnings: string[] = [];
  const importedImagePaths = new Map(
    result.images.map( ( image ) => [
      image.relativePath.toLowerCase(),
      image.relativePath
    ])
  );
  const importedAttachmentPaths = new Map(
    result.attachments.map( ( attachment ) => [
      attachment.relativePath.toLowerCase(),
      attachment.relativePath
    ])
  );
  if ( result.images.length || result.attachments.length ) {
    if ( vaultSession.backend !== 'native' ) {
      warnings.push( 'The notes were imported, but their asset files could not be copied here.' );
    } else if ( !vaultSession.path || !sourcePath ) {
      const warning = 'The import stopped because its asset source was no longer available.';
      warnings.push( warning );
      notify( warning, 'warning' );

      return {
        attachmentCount: 0,
        imageCount: 0,
        noteCount: 0,
        saved: false,
        warnings
      };
    } else {
      try {
        const assetResult = await importWorkspaceAssets(
          vaultSession.path,
          sourcePath,
          result.images.map( ( image ) => image.relativePath ),
          result.attachments.map( ( attachment ) => attachment.relativePath ),
          vaultSession.revision
        );
        const returnedMappings = new Map(
          Object.entries( assetResult.pathMappings ).map( ([ source, target ]) => [
            source.toLowerCase(),
            target
          ])
        );
        const importedAssetPaths = [
          ...result.images.map( ( image ) => image.relativePath ),
          ...result.attachments.map( ( attachment ) => attachment.relativePath )
        ];
        if ( importedAssetPaths.some( ( path ) => !returnedMappings.has( path.toLowerCase() ) ) ) {
          throw new Error( 'A safe destination could not be reserved for every imported asset.' );
        }
        for ( const [ source, target ] of returnedMappings ) {
          if ( importedImagePaths.has( source ) ) {
            importedImagePaths.set( source, target );
          }
          if ( importedAttachmentPaths.has( source ) ) {
            importedAttachmentPaths.set( source, target );
          }
        }
        applyWorkspaceSaveResult( assetResult );
        applyWorkspaceImageFiles( assetResult.imageFiles );
        applyWorkspaceAttachmentFiles( assetResult.attachmentFiles );
        uiState.imageRefreshToken += 1;
        uiState.attachmentRefreshToken += 1;
        imageCount = assetResult.imageCount;
        attachmentCount = assetResult.attachmentCount;
        imageImportTransactionId = assetResult.transactionId;
        warnings.push( ...assetResult.warnings );
      } catch ( error ) {
        const detail = error instanceof Error ? error.message : String( error );
        const warning = `The import stopped before its notes were added: ${ detail }`;
        warnings.push( warning );
        notify( warning, 'warning' );

        return {
          attachmentCount: 0,
          imageCount: 0,
          noteCount: 0,
          saved: false,
          warnings
        };
      }
    }
  }

  if ( replace ) {
    vaultState.notes.splice( 0 );
    vaultState.folders.splice( 0 );
    rebuildWorkspaceAssetFolders();
  }

  const now = Date.now();
  let firstImportedNoteId: string | null = null;
  for ( const imported of result.notes ) {
    const folderId = ensureFolderPath( imported.folderPath );
    const title = uniqueNoteTitle( imported.title || 'Untitled note' );
    const relativePath = importedNoteTargetPath( title, folderPath( folderId ) );
    const note: Note = {
      id: createId( 'note' ),
      title,
      content: rewriteImportedAssetReferences(
        imported.content,
        imported.relativePath,
        relativePath,
        importedImagePaths,
        importedAttachmentPaths
      ),
      relativePath,
      folderId,
      tags: imported.tags,
      pinned: false,
      createdAt: now,
      updatedAt: now
    };
    firstImportedNoteId ??= note.id;
    vaultState.notes.push( note );
  }

  for ( const imported of result.snippets ) {
    const existing = vaultState.snippets.find(
      ( snippet ) => snippet.name.toLocaleLowerCase() === imported.name.toLocaleLowerCase()
    );
    if ( existing ) {
      continue;
    }
    vaultState.snippets.push({
      id: createId( 'snippet' ),
      name: imported.name,
      description: 'Imported from an Obsidian CSS snippet.',
      css: imported.css,
      enabled: imported.enabled,
      createdAt: now
    });
  }

  vaultState.activeNoteId = firstImportedNoteId ?? ( replace ? null : previousActiveNoteId );
  vaultState.selectedFolderId = 'all';
  if ( replace ) {
    vaultState.recentNoteIds.splice( 0 );
    resetNoteNavigation();
  }
  if ( firstImportedNoteId ) {
    touchRecentNote( firstImportedNoteId );
  }
  if ( !replace && firstImportedNoteId ) {
    recordDirectNoteNavigation( previousActiveNoteId, firstImportedNoteId );
  }
  const saved = await flushVault( imageImportTransactionId );
  if ( !saved ) {
    hydrateVault( previousVault );
    restoreNoteNavigation( previousNoteNavigation );
    dirtyVersion = previousSavedVersion;
    savedVersion = previousSavedVersion;
    uiState.imageRefreshToken += 1;
    uiState.attachmentRefreshToken += 1;
  }
  pruneNoteEditorPositions( currentEditorPositionVaultId(), vaultState.notes );
  pruneNoteEditorHistories( currentEditorPositionVaultId(), vaultState.notes );
  notify(
    saved
      ? `Imported ${ result.notes.length } Markdown ${
        result.notes.length === 1 ? 'note' : 'notes'
      }${ imageCount ? ` and ${ imageCount } ${ imageCount === 1 ? 'image' : 'images' }` : '' }${
        attachmentCount
          ? ` and ${ attachmentCount } ${ attachmentCount === 1 ? 'attachment' : 'attachments' }`
          : ''
      }`
      : 'Import could not be completed',
    saved && !warnings.length ? 'success' : 'warning'
  );

  return {
    attachmentCount: saved ? attachmentCount : 0,
    imageCount: saved ? imageCount : 0,
    noteCount: saved ? result.notes.length : 0,
    saved,
    warnings
  };
}

function rewriteImportedAssetReferences(
  content: string,
  sourceNotePath: string,
  targetNotePath: string,
  imagePaths: ReadonlyMap<string, string>,
  attachmentPaths: ReadonlyMap<string, string>
): string {
  const replacements: Array<{ from: number; to: number; value: string }> = [];
  for ( const image of parseMarkdownImages( content ) ) {
    const sourceImagePath = resolveMarkdownImagePath(
      sourceNotePath,
      image.destination
    );
    const targetImagePath = sourceImagePath
      ? imagePaths.get( sourceImagePath.toLowerCase() )
      : undefined;
    if ( !targetImagePath ) {
      continue;
    }
    replacements.push({
      from: image.start,
      to: image.end + 1,
      value: formatMarkdownImage({
        alt: image.alt,
        destination: relativeImageDestination( targetNotePath, targetImagePath ),
        ...( image.width ? { width: image.width } : {}),
        ...( image.height ? { height: image.height } : {}),
        ...( image.title !== undefined ? { title: image.title } : {}),
        inTable: image.raw.includes( '\\|' )
      })
    });
  }
  const isKnownExtensionlessAttachment = ( destination: string ): boolean => {
    const sourcePath = resolveMarkdownImagePath( sourceNotePath, destination );

    return Boolean( sourcePath && attachmentPaths.has( sourcePath.toLowerCase() ) );
  };
  for ( const attachment of parseMarkdownAttachments( content, {
    acceptExtensionless: isKnownExtensionlessAttachment
  }) ) {
    const sourceAttachmentPath = resolveMarkdownImagePath(
      sourceNotePath,
      attachment.destination
    );
    const targetAttachmentPath = sourceAttachmentPath
      ? attachmentPaths.get( sourceAttachmentPath.toLowerCase() )
      : undefined;
    if ( !targetAttachmentPath ) {
      continue;
    }
    const sourceName = sourceAttachmentPath?.split( '/' ).at( -1 ) || 'Attachment';
    const targetName = targetAttachmentPath.split( '/' ).at( -1 ) || 'Attachment';
    replacements.push({
      from: attachment.start,
      to: attachment.end + 1,
      value: formatMarkdownAttachment({
        label: attachment.label === sourceName ? targetName : attachment.label,
        destination: relativeAttachmentDestination( targetNotePath, targetAttachmentPath ),
        ...( attachment.title !== undefined ? { title: attachment.title } : {}),
        inTable: attachment.raw.includes( '\\|' )
      })
    });
  }

  return applyMarkdownReplacements( content, replacements );
}

function importedNoteTargetPath( title: string, folder: string ): string {
  const fileName = `${ safeImportedNoteStem( title ) }.md`;

  return folder ? `${ folder }/${ fileName }` : fileName;
}

function safeImportedNoteStem( value: string ): string {
  const encoder = new TextEncoder();
  let result = '';
  let previousWasReplacement = false;
  for ( const character of value.trim() ) {
    if ( /[\p{Cc}/\\:*?"<>|]/u.test( character ) ) {
      if ( !previousWasReplacement ) {
        result += '-';
        previousWasReplacement = true;
      }
    } else {
      result += character;
      previousWasReplacement = false;
    }
    if ( encoder.encode( result ).length >= 120 ) {
      break;
    }
  }
  result = result.replace( /^[ .]+|[ .]+$/g, '' ) || 'Untitled note';

  return /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/iu.test( result )
    ? `_${ result }`
    : result;
}

export function buildExportPayload(): {
  notes: ExportNote[];
  templates: ExportTemplate[];
  snippets: ExportSnippet[];
} {
  return {
    notes: vaultState.notes.map( ( note ) => ({
      title: note.title,
      content: note.content,
      folderPath: note.folderId ? folderPath( note.folderId ) : '',
      tags: note.tags
    }) ),
    templates: vaultState.templates.map( ( template ) => ({
      name: template.name,
      content: template.content
    }) ),
    snippets: vaultState.snippets.map( ( snippet ) => ({
      name: snippet.name,
      css: snippet.css,
      enabled: snippet.enabled
    }) )
  };
}

export function folderPath( id: string | null ): string {
  return folderPathFromFolders( id, vaultState.folders );
}

function rememberNoteOriginalPath( note: Note ): void {
  if (
    vaultSession.backend === 'native'
    && note.relativePath
    && !pendingNoteOriginalPaths.has( note.id )
  ) {
    pendingNoteOriginalPaths.set( note.id, note.relativePath );
  }
}

function folderContainsAssets( folderId: string ): boolean {
  return folderContainsVaultAssets( vaultState, folderPath( folderId ) );
}

export function requestInsertVaultImage( image: VaultImageFile ): void {
  if ( vaultSession.backend !== 'native' || !vaultSession.path || !activeNote.value ) {
    notify( 'Open a note in a desktop vault before inserting an image', 'warning' );

    return;
  }
  vaultImageInsertRequest.relativePath = image.relativePath;
  vaultImageInsertRequest.id += 1;
}

export async function renameVaultImage(
  image: VaultImageFile,
  fileName: string
): Promise<boolean> {
  const parent = image.relativePath.split( '/' ).slice( 0, -1 ).join( '/' );

  return relocateVaultImage( image, parent, fileName );
}

export async function moveVaultImageToFolder(
  image: VaultImageFile,
  folderId: string | null
): Promise<boolean> {
  const fileName = image.relativePath.split( '/' ).at( -1 ) || 'Image.png';

  return relocateVaultImage( image, folderPath( folderId ), fileName );
}

async function relocateVaultImage(
  image: VaultImageFile,
  targetFolderPath: string,
  requestedFileName: string
): Promise<boolean> {
  const fileName = requestedFileName.trim();
  if ( !isSafeVaultImageFileName( fileName ) ) {
    notify( 'Enter a safe image file name with a supported extension', 'warning' );

    return false;
  }
  const targetRelativePath = targetFolderPath
    ? `${ targetFolderPath }/${ fileName }`
    : fileName;
  if ( targetRelativePath === image.relativePath ) {
    return false;
  }
  const targetKey = targetRelativePath.toLocaleLowerCase();
  if ( vaultState.imageFiles.some( ( candidate ) =>
    candidate.relativePath.toLocaleLowerCase() === targetKey
    && candidate.relativePath.toLocaleLowerCase() !== image.relativePath.toLocaleLowerCase()
  ) ) {
    notify( 'An image with that name already exists there', 'warning' );

    return false;
  }
  if ( vaultSession.backend !== 'native' || !vaultSession.path ) {
    notify( 'Image files can be reorganized in the desktop app', 'warning' );

    return false;
  }

  return runExclusiveVaultDataOperation( false, async () => {
    if ( !( await flushVault() ) ) {
      return false;
    }
    const currentImage = vaultState.imageFiles.find( ( candidate ) =>
      ( image.assetId && candidate.assetId === image.assetId )
      || candidate.relativePath.toLocaleLowerCase() === image.relativePath.toLocaleLowerCase()
    );
    if ( !currentImage ) {
      notify( 'That image can no longer be moved', 'warning' );

      return false;
    }

    const assetId = currentImage.assetId || createId( 'image' );
    const noteUpdates = vaultState.notes.flatMap( ( note ): WorkspaceImageNoteUpdate[] => {
      const content = rewriteVaultImageReferences(
        vaultState,
        note.content,
        note.relativePath,
        currentImage.relativePath,
        targetRelativePath,
        currentImage.assetId,
        assetId
      );

      return content === note.content ? [] : [{
        noteId: note.id,
        relativePath: note.relativePath,
        expectedContent: note.content,
        content
      }];
    });

    try {
      const result = await relocateWorkspaceImage(
        vaultSession.path!,
        currentImage.relativePath,
        targetRelativePath,
        assetId,
        noteUpdates,
        vaultSession.revision
      );
      applyRelocatedImageResult( result, noteUpdates );
      uiState.imageRefreshToken += 1;
      notify(
        targetFolderPath
          ? `Moved image to ${ targetFolderPath }`
          : 'Moved image to Vault root',
        'success'
      );

      return true;
    } catch ( error ) {
      const message = errorMessage( error, 'The image could not be moved.' );
      vaultSession.error = message;
      vaultSession.conflict = isRevisionConflict( message );
      notify( message, 'warning' );

      return false;
    }
  });
}

function applyRelocatedImageResult(
  result: WorkspaceRelocateImageResult,
  noteUpdates: WorkspaceImageNoteUpdate[]
): void {
  const updatesById = new Map( noteUpdates.map( ( update ) => [ update.noteId, update.content ]) );
  applyVaultMutation( () => {
    for ( const note of vaultState.notes ) {
      const content = updatesById.get( note.id );
      if ( content !== undefined ) {
        note.content = content;
        note.updatedAt = Date.now();
      }
    }
    const oldPathKey = result.previousRelativePath.toLocaleLowerCase();
    const oldFileIndex = vaultState.imageFiles.findIndex( ( candidate ) =>
      candidate.assetId === result.image.id
      || candidate.relativePath.toLocaleLowerCase() === oldPathKey
    );
    if ( oldFileIndex >= 0 ) {
      vaultState.imageFiles.splice( oldFileIndex, 1 );
    }
    const oldEmbeddedIndex = vaultState.embeddedImages.findIndex( ( candidate ) =>
      candidate.id === result.image.id
      || candidate.relativePath.toLocaleLowerCase() === oldPathKey
    );
    if ( oldEmbeddedIndex >= 0 ) {
      vaultState.embeddedImages.splice( oldEmbeddedIndex, 1 );
    }
    vaultState.embeddedImages.push( result.image );
    upsertWorkspaceImageFile({
      assetId: result.image.id,
      relativePath: result.image.relativePath,
      mediaType: result.image.mediaType
    });
  });
  applyWorkspaceSaveResult( result );
}

export function requestInsertVaultAttachment( attachment: VaultAttachmentFile ): void {
  if ( vaultSession.backend !== 'native' || !vaultSession.path || !activeNote.value ) {
    notify( 'Open a note in a desktop vault before inserting a file', 'warning' );

    return;
  }
  vaultAttachmentInsertRequest.relativePath = attachment.relativePath;
  vaultAttachmentInsertRequest.id += 1;
}

export async function renameVaultAttachment(
  attachment: Pick<VaultAttachmentFile, 'assetId' | 'relativePath'>,
  fileName: string
): Promise<boolean> {
  return relocateVaultAttachment( attachment, {
    fileName: fileName.trim(),
    kind: 'rename'
  });
}

export async function moveVaultAttachmentToFolder(
  attachment: Pick<VaultAttachmentFile, 'assetId' | 'relativePath'>,
  folderId: string | null
): Promise<boolean> {
  return relocateVaultAttachment( attachment, {
    kind: 'move',
    targetFolderId: folderId
  });
}

type VaultAttachmentRelocation = {
  fileName: string;
  kind: 'rename';
} | {
  kind: 'move';
  targetFolderId: string | null;
};

async function relocateVaultAttachment(
  attachment: Pick<VaultAttachmentFile, 'assetId' | 'relativePath'>,
  relocation: VaultAttachmentRelocation
): Promise<boolean> {
  if (
    relocation.kind === 'rename'
    && !isSafeVaultAttachmentFileName( relocation.fileName )
  ) {
    notify( 'Enter a safe non-Markdown, non-image file name', 'warning' );

    return false;
  }
  if ( vaultSession.backend !== 'native' || !vaultSession.path ) {
    notify( 'Attachment files can be reorganized in the desktop app', 'warning' );

    return false;
  }

  return runExclusiveVaultDataOperation( false, async () => {
    if ( !( await flushVault() ) ) {
      return false;
    }
    const currentAttachment = resolveCurrentVaultAttachment( attachment );
    if ( !currentAttachment ) {
      notify( 'That attachment could not be uniquely found in the vault', 'warning' );

      return false;
    }
    const currentFileName = currentAttachment.relativePath.split( '/' ).at( -1 )
      || 'Attachment';
    const fileName = relocation.kind === 'rename'
      ? relocation.fileName
      : currentFileName;
    const targetFolder = relocation.kind === 'move' && relocation.targetFolderId
      ? vaultState.folders.find( ( folder ) => folder.id === relocation.targetFolderId )
      : null;
    if ( relocation.kind === 'move' && relocation.targetFolderId && !targetFolder ) {
      notify( 'That destination folder could not be found', 'warning' );

      return false;
    }
    const targetFolderPath = relocation.kind === 'rename'
      ? currentAttachment.relativePath.split( '/' ).slice( 0, -1 ).join( '/' )
      : folderPath( targetFolder?.id ?? null );
    const targetRelativePath = targetFolderPath
      ? `${ targetFolderPath }/${ fileName }`
      : fileName;
    if ( targetRelativePath === currentAttachment.relativePath ) {
      return false;
    }
    const targetKey = targetRelativePath.toLocaleLowerCase();
    if ( vaultState.attachmentFiles.some( ( candidate ) =>
      candidate.relativePath.toLocaleLowerCase() === targetKey
      && candidate.relativePath.toLocaleLowerCase()
        !== currentAttachment.relativePath.toLocaleLowerCase()
    ) ) {
      notify( 'A file with that name already exists there', 'warning' );

      return false;
    }

    const assetId = currentAttachment.assetId || createId( 'attachment' );
    const noteUpdates = vaultState.notes.flatMap( ( note ): WorkspaceAttachmentNoteUpdate[] => {
      const content = rewriteVaultAttachmentReferences(
        vaultState,
        note.content,
        note.relativePath,
        currentAttachment.relativePath,
        targetRelativePath,
        currentAttachment.assetId,
        assetId
      );

      return content === note.content ? [] : [{
        noteId: note.id,
        relativePath: note.relativePath,
        expectedContent: note.content,
        content
      }];
    });

    try {
      const result = await relocateWorkspaceAttachment(
        vaultSession.path!,
        currentAttachment.relativePath,
        targetRelativePath,
        assetId,
        noteUpdates,
        vaultSession.revision
      );
      applyRelocatedAttachmentResult( result, noteUpdates );
      uiState.attachmentRefreshToken += 1;
      notify(
        relocation.kind === 'rename'
          ? `Renamed attachment to ${ fileName }`
          : targetFolderPath
            ? `Moved attachment to ${ targetFolderPath }`
            : 'Moved attachment to Vault root',
        'success'
      );

      return true;
    } catch ( error ) {
      const message = errorMessage( error, 'The attachment could not be renamed or moved.' );
      vaultSession.error = message;
      vaultSession.conflict = isRevisionConflict( message );
      notify( message, 'warning' );

      return false;
    }
  });
}

function resolveCurrentVaultAttachment(
  attachment: Pick<VaultAttachmentFile, 'assetId' | 'relativePath'>
): VaultAttachmentFile | undefined {
  const matches = attachment.assetId
    ? vaultState.attachmentFiles.filter( ( candidate ) =>
      candidate.assetId === attachment.assetId
    )
    : vaultState.attachmentFiles.filter( ( candidate ) =>
      candidate.relativePath.toLocaleLowerCase()
        === attachment.relativePath.toLocaleLowerCase()
    );

  return matches.length === 1 ? matches[ 0 ] : undefined;
}

function applyRelocatedAttachmentResult(
  result: WorkspaceRelocateAttachmentResult,
  noteUpdates: WorkspaceAttachmentNoteUpdate[]
): void {
  const updatesById = new Map( noteUpdates.map( ( update ) => [ update.noteId, update.content ]) );
  applyVaultMutation( () => {
    for ( const note of vaultState.notes ) {
      const content = updatesById.get( note.id );
      if ( content !== undefined ) {
        note.content = content;
        note.updatedAt = Date.now();
      }
    }
    const oldPathKey = result.previousRelativePath.toLocaleLowerCase();
    const oldFileIndex = vaultState.attachmentFiles.findIndex( ( candidate ) =>
      candidate.assetId === result.attachment.id
      || candidate.relativePath.toLocaleLowerCase() === oldPathKey
    );
    if ( oldFileIndex >= 0 ) {
      vaultState.attachmentFiles.splice( oldFileIndex, 1 );
    }
    const oldEmbeddedIndex = vaultState.embeddedAttachments.findIndex( ( candidate ) =>
      candidate.id === result.attachment.id
      || candidate.relativePath.toLocaleLowerCase() === oldPathKey
    );
    if ( oldEmbeddedIndex >= 0 ) {
      vaultState.embeddedAttachments.splice( oldEmbeddedIndex, 1 );
    }
    vaultState.embeddedAttachments.push( result.attachment );
    upsertWorkspaceAttachmentFile({
      assetId: result.attachment.id,
      relativePath: result.attachment.relativePath,
      mediaType: result.attachment.mediaType,
      byteLength: result.attachment.byteLength,
      openingDisabled: result.attachment.openingDisabled
    });
  });
  applyWorkspaceSaveResult( result );
}

const ARCHIVE_COPY_DIRECTORY_KEY = 'obsidian-at-home.archive-copy-directory.v1';

export async function activateVaultAttachment(
  attachment: Pick<
    VaultAttachmentFile,
    'assetId' | 'mediaType' | 'openingDisabled' | 'relativePath'
  >
): Promise<void> {
  if ( vaultSession.backend !== 'native' || !vaultSession.path ) {
    notify( 'Attachment files can be opened in the desktop app', 'warning' );

    return;
  }
  if ( markdownAttachmentIsExecutable(
    attachment.relativePath,
    attachment.openingDisabled
  ) ) {
    notify( 'Opening executable or installer attachments is not supported', 'warning' );

    return;
  }
  try {
    if ( markdownAttachmentIsArchive( attachment.relativePath, attachment.mediaType ) ) {
      let preferredDirectory: string | undefined;
      try {
        preferredDirectory = window.localStorage.getItem( ARCHIVE_COPY_DIRECTORY_KEY )
          || undefined;
      } catch {
        // A Downloads default remains available when browser storage is unavailable.
      }
      const result = await saveWorkspaceAttachmentCopy(
        vaultSession.path,
        attachment.relativePath,
        attachment.assetId,
        preferredDirectory
      );
      if ( !result ) {
        return;
      }
      const directory = parentSystemPath( result.path );
      if ( directory ) {
        try {
          window.localStorage.setItem( ARCHIVE_COPY_DIRECTORY_KEY, directory );
        } catch {
          // Remembering the folder is helpful but not required for a successful copy.
        }
      }
      notify( 'Saved the archive outside the vault', 'success', {
        label: 'Reveal archive',
        run: () => {
          void revealItemInDir( result.path ).catch( ( error ) =>
            notify( errorMessage( error, 'The saved archive could not be revealed.' ), 'warning' )
          );
        }
      });

      return;
    }
    await openWorkspaceAttachment(
      vaultSession.path,
      attachment.relativePath,
      attachment.assetId
    );
  } catch ( error ) {
    notify( errorMessage( error, 'The attachment could not be opened.' ), 'warning' );
  }
}

export interface VaultItemLocator {
  assetId?: string;
  itemId?: string;
  kind: WorkspaceVaultItemKind;
  relativePath: string;
}

export async function locateVaultItem( locator: VaultItemLocator ): Promise<string | undefined> {
  if ( vaultSession.backend !== 'native' || !vaultSession.path ) {
    return locator.relativePath;
  }
  const sourcePath = vaultSession.path;
  let relativePath = locator.relativePath;
  if ( locator.kind === 'note' || locator.kind === 'folder' ) {
    if ( !( await flushVault() ) ) {
      return undefined;
    }
    if ( vaultSession.backend !== 'native' || vaultSession.path !== sourcePath ) {
      return undefined;
    }
    if ( locator.kind === 'note' ) {
      const note = locator.itemId
        ? vaultState.notes.find( ( candidate ) => candidate.id === locator.itemId )
        : vaultState.notes.find( ( candidate ) => candidate.relativePath === locator.relativePath );
      relativePath = note?.relativePath ?? '';
    } else {
      const folder = locator.itemId
        ? vaultState.folders.find( ( candidate ) => candidate.id === locator.itemId )
        : vaultState.folders.find( ( candidate ) => folderPath( candidate.id ) === locator.relativePath );
      relativePath = folder ? folderPath( folder.id ) : '';
    }
    if ( !relativePath ) {
      notify( 'The vault item is no longer available.', 'warning' );

      return undefined;
    }
  }
  try {
    return await locateWorkspaceVaultItem(
      sourcePath,
      locator.kind,
      relativePath,
      locator.assetId
    );
  } catch ( error ) {
    notify( errorMessage( error, 'The vault item could not be located.' ), 'warning' );

    return undefined;
  }
}

export async function revealVaultItemInTree( locator: VaultItemLocator ): Promise<boolean> {
  const operation = ++vaultTreeRevealOperation;
  const sourceVaultKey = currentVaultTreeKey();
  const locatedPath = await locateVaultItem( locator );
  if (
    !locatedPath
    || operation !== vaultTreeRevealOperation
    || sourceVaultKey !== currentVaultTreeKey()
  ) {
    return false;
  }

  const target = currentVaultTreeItem( locator, locatedPath );
  if ( !target ) {
    notify( "The vault item could not be found in the app's file tree.", 'warning' );

    return false;
  }

  uiState.commandOpen = false;
  uiState.tool = 'notes';
  uiState.notesView = 'editor';
  uiState.explorerOpen = true;
  uiState.noteFilter = '';
  vaultState.selectedFolderId = 'all';
  vaultTreeRevealTarget.assetId = target.assetId ?? null;
  vaultTreeRevealTarget.kind = locator.kind;
  vaultTreeRevealTarget.relativePath = target.relativePath;
  vaultTreeRevealTarget.vaultKey = sourceVaultKey;
  vaultTreeRevealTarget.requestId += 1;

  return true;
}

export function vaultTreeItemIsRevealed( locator: VaultItemLocator ): boolean {
  if (
    !vaultTreeRevealTarget.requestId
    || vaultTreeRevealTarget.vaultKey !== currentVaultTreeKey()
    || vaultTreeRevealTarget.kind !== locator.kind
  ) {
    return false;
  }
  if ( vaultTreeRevealTarget.assetId ) {
    return locator.assetId === vaultTreeRevealTarget.assetId;
  }

  return locator.relativePath === vaultTreeRevealTarget.relativePath;
}

export function vaultTreeRevealIncludesFolder( relativePath: string ): boolean {
  if (
    !vaultTreeRevealTarget.requestId
    || vaultTreeRevealTarget.vaultKey !== currentVaultTreeKey()
  ) {
    return false;
  }

  return vaultTreeRevealTarget.relativePath === relativePath
    || vaultTreeRevealTarget.relativePath.startsWith( `${ relativePath }/` );
}

export async function showVaultItemInFolder( locator: VaultItemLocator ): Promise<void> {
  if ( vaultSession.backend !== 'native' || !vaultSession.path ) {
    notify( 'Showing vault files in a system folder is available in the desktop app', 'warning' );

    return;
  }
  const sourcePath = vaultSession.path;
  const relativePath = await locateVaultItem( locator );
  if (
    !relativePath
    || vaultSession.backend !== 'native'
    || vaultSession.path !== sourcePath
  ) {
    return;
  }
  try {
    await showWorkspaceVaultItemInFolder(
      sourcePath,
      locator.kind,
      relativePath,
      locator.assetId
    );
  } catch ( error ) {
    notify( errorMessage( error, 'The vault item could not be shown in its folder.' ), 'warning' );
  }
}

function currentVaultTreeKey(): string {
  return `${ vaultSession.backend }\u0000${ vaultSession.path ?? vaultState.name }`;
}

function currentVaultTreeItem(
  locator: VaultItemLocator,
  locatedPath: string
): { assetId?: string; relativePath: string } | undefined {
  if ( locator.kind === 'attachment' ) {
    const attachment = locator.assetId
      ? vaultState.attachmentFiles.find( ( candidate ) => candidate.assetId === locator.assetId )
      : vaultState.attachmentFiles.find( ( candidate ) => candidate.relativePath === locatedPath );

    return attachment
      ? {
        ...( attachment.assetId ? { assetId: attachment.assetId } : {}),
        relativePath: attachment.relativePath
      }
      : undefined;
  }
  if ( locator.kind === 'image' ) {
    const image = locator.assetId
      ? vaultState.imageFiles.find( ( candidate ) => candidate.assetId === locator.assetId )
      : vaultState.imageFiles.find( ( candidate ) => candidate.relativePath === locatedPath );

    return image
      ? {
        ...( image.assetId ? { assetId: image.assetId } : {}),
        relativePath: image.relativePath
      }
      : undefined;
  }
  if ( locator.kind === 'note' ) {
    const note = vaultState.notes.find( ( candidate ) => candidate.relativePath === locatedPath );

    return note ? { relativePath: note.relativePath } : undefined;
  }
  const folder = vaultState.folders.find( ( candidate ) => folderPath( candidate.id ) === locatedPath );

  return folder ? { relativePath: locatedPath } : undefined;
}

function parentSystemPath( path: string ): string | undefined {
  const index = Math.max( path.lastIndexOf( '/' ), path.lastIndexOf( '\\' ) );

  return index > 0 ? path.slice( 0, index ) : undefined;
}

function rewriteAssetDestinationsForNotePath(
  content: string,
  sourceNotePath: string,
  targetNotePath: string
): string {
  return rewriteVaultAssetDestinationsForNotePath(
    vaultState,
    content,
    sourceNotePath,
    targetNotePath
  );
}

export function noteCountForFolder( id: string ): number {
  const ids = new Set([ id, ...descendantFolderIds( id ) ]);

  return vaultState.notes.filter( ( note ) => note.folderId && ids.has( note.folderId ) ).length;
}

export function folderChildren( parentId: string | null ): Folder[] {
  return vaultState.folders
    .filter( ( folder ) => folder.parentId === parentId )
    .sort( ( a, b ) => a.name.localeCompare( b.name ) );
}

export function notify(
  message: string,
  tone: ToastTone = 'neutral',
  action?: ToastAction
): void {
  clearTimeout( toastTimer );
  uiState.toast = { id: Date.now(), message, tone, ...( action ? { action } : {}) };
  toastTimer = setTimeout( () => {
    uiState.toast = null;
  }, 3200 );
}

export async function clearVault(): Promise<boolean> {
  return runExclusiveVaultDataOperation( false, clearVaultExclusive );
}

async function clearVaultExclusive(): Promise<boolean> {
  if ( !( await flushVault() ) ) {
    return false;
  }
  clearTimeout( persistTimer );
  const previousVault = snapshotVault();
  const previousSavedVersion = savedVersion;
  const previousNoteNavigation = snapshotNoteNavigation();
  vaultState.notes.splice( 0 );
  vaultState.folders.splice( 0 );
  vaultState.activeNoteId = null;
  vaultState.recentNoteIds.splice( 0 );
  resetNoteNavigation();
  vaultState.selectedFolderId = 'all';
  uiState.noteFilter = '';
  uiState.commandOpen = false;
  resetSearchState();
  uiState.contextOpen = false;
  uiState.explorerOpen = true;
  const saved = await flushVault();
  if ( !saved ) {
    hydrateVault( previousVault );
    restoreNoteNavigation( previousNoteNavigation );
    dirtyVersion = previousSavedVersion;
    savedVersion = previousSavedVersion;
  }
  pruneNoteEditorPositions( currentEditorPositionVaultId(), vaultState.notes );
  pruneNoteEditorHistories( currentEditorPositionVaultId(), vaultState.notes );
  notify( saved ? 'Vault cleared' : 'Vault cleared, but not saved', saved ? 'success' : 'warning' );

  return saved;
}

async function runExclusiveVaultDataOperation<T>(
  fallback: T,
  operation: () => Promise<T>
): Promise<T> {
  if ( vaultSession.busy || vaultSession.phase !== 'ready' ) {
    return fallback;
  }

  vaultSession.busy = true;
  try {
    return await operation();
  } finally {
    vaultSession.busy = false;
    scheduleRecentlyDeletedExpiry();
  }
}

async function runRecoveryOperation( operation: () => Promise<boolean> ): Promise<boolean> {
  if (
    recentlyDeletedState.busy
    || vaultSession.busy
    || vaultSession.phase !== 'ready'
  ) {
    return false;
  }

  recentlyDeletedState.busy = true;
  recentlyDeletedState.error = null;
  vaultSession.busy = true;
  uiState.commandOpen = false;
  try {
    return await operation();
  } finally {
    recentlyDeletedState.busy = false;
    vaultSession.busy = false;
    scheduleRecentlyDeletedExpiry();
  }
}

async function performNativeRecoverySave<T extends WorkspaceSaveResult>(
  request: () => Promise<T>,
  onSuccess: ( result: T ) => void,
  onFailure: () => Promise<boolean>,
  fallbackError: string
): Promise<boolean> {
  clearTimeout( persistTimer );
  const generation = sessionGeneration;
  const path = vaultSession.path;
  const originalSessionIsActive = (): boolean => (
    generation === sessionGeneration && path === vaultSession.path
  );
  let writesMayResume = true;
  const operation = ( async (): Promise<boolean> => {
    try {
      const result = await request();
      if ( !originalSessionIsActive() ) {
        writesMayResume = false;

        return false;
      }
      onSuccess( result );
      recentlyDeletedState.error = null;

      return true;
    } catch ( error ) {
      if ( !originalSessionIsActive() ) {
        writesMayResume = false;

        return false;
      }
      let reconciled: boolean;
      try {
        reconciled = await onFailure();
      } catch {
        reconciled = false;
      }
      if ( vaultSession.path !== path ) {
        writesMayResume = false;

        return false;
      }
      writesMayResume = reconciled;
      const message = errorMessage( error, fallbackError );
      recentlyDeletedState.error = message;
      vaultSession.error = message;
      vaultSession.conflict = !reconciled;
      uiState.vaultChooserOpen = !reconciled;
      uiState.saveStatus = reconciled ? 'saved' : 'error';
      notify( message, 'warning' );

      return false;
    }
  })();

  recoverySaveInFlight = operation;
  let saved: boolean;
  try {
    saved = await operation;
  } finally {
    if ( recoverySaveInFlight === operation ) {
      recoverySaveInFlight = null;
    }
  }
  if ( writesMayResume && savedVersion < dirtyVersion ) {
    persistTimer = setTimeout( () => void flushVault(), 0 );
  }

  return saved;
}

async function reconcileNativeWorkspace( path: string ): Promise<WorkspaceLoad | null> {
  try {
    const workspace = await openWorkspace( path, createEmptyVault() );
    applyWorkspace( workspace );

    return workspace;
  } catch {
    return null;
  }
}

function applyWorkspaceSaveResult( result: WorkspaceSaveResult ): void {
  applySavedNotePaths( result.notePaths );
  vaultSession.revision = result.revision;
  vaultSession.error = null;
  vaultSession.conflict = false;
  vaultSession.warnings = result.warnings;
  uiState.saveStatus = savedVersion < dirtyVersion ? 'saving' : 'saved';
  uiState.lastSavedAt = result.savedAt || Date.now();
}

function applySavedNotePaths( notePaths: Record<string, string> | undefined ): void {
  if ( !notePaths ) {
    return;
  }
  applyVaultMutation( () => {
    for ( const note of vaultState.notes ) {
      const relativePath = notePaths[ note.id ];
      if ( relativePath ) {
        const originalPath = pendingNoteOriginalPaths.get( note.id );
        if ( originalPath ) {
          note.content = rewriteAssetDestinationsForNotePath(
            note.content,
            originalPath,
            relativePath
          );
          pendingNoteOriginalPaths.delete( note.id );
        }
        note.relativePath = relativePath;
        const nextPath = projectedNoteRelativePath(
          note,
          vaultState.folders,
          relativePath
        );
        if ( nextPath !== relativePath ) {
          pendingNoteOriginalPaths.set( note.id, relativePath );
        }
      }
    }
    const noteIds = new Set( vaultState.notes.map( ( note ) => note.id ) );
    for ( const noteId of pendingNoteOriginalPaths.keys() ) {
      if ( !noteIds.has( noteId ) ) {
        pendingNoteOriginalPaths.delete( noteId );
      }
    }
  });
}

export function applyEmbeddedImageResult( result: WorkspaceEmbedImageResult ): void {
  applyVaultMutation( () => {
    const index = vaultState.embeddedImages.findIndex( ( image ) => image.id === result.image.id );
    if ( index >= 0 ) {
      vaultState.embeddedImages.splice( index, 1, result.image );
    } else {
      vaultState.embeddedImages.push( result.image );
    }
    upsertWorkspaceImageFile({
      assetId: result.image.id,
      relativePath: result.image.relativePath,
      mediaType: result.image.mediaType
    });
  });
  applyWorkspaceSaveResult( result );
  uiState.imageRefreshToken += 1;
}

export function applyEmbeddedAttachmentResult(
  result: WorkspaceEmbedAttachmentResult
): void {
  applyVaultMutation( () => {
    const index = vaultState.embeddedAttachments.findIndex(
      ( attachment ) => attachment.id === result.attachment.id
    );
    if ( index >= 0 ) {
      vaultState.embeddedAttachments.splice( index, 1, result.attachment );
    } else {
      vaultState.embeddedAttachments.push( result.attachment );
    }
    upsertWorkspaceAttachmentFile({
      assetId: result.attachment.id,
      relativePath: result.attachment.relativePath,
      mediaType: result.attachment.mediaType,
      byteLength: result.attachment.byteLength,
      openingDisabled: result.attachment.openingDisabled
    });
  });
  applyWorkspaceSaveResult( result );
  uiState.attachmentRefreshToken += 1;
}

export function applyExternalAssetDiscardResult(
  result: WorkspaceExternalAssetDiscardResult
): void {
  applyWorkspaceSaveResult( result );
}

function applyWorkspaceImageFiles( images: VaultImageFile[]): void {
  applyVaultMutation( () => {
    for ( const image of images ) {
      upsertWorkspaceImageFile( image );
    }
  });
}

function applyWorkspaceAttachmentFiles( attachments: VaultAttachmentFile[]): void {
  applyVaultMutation( () => {
    for ( const attachment of attachments ) {
      upsertWorkspaceAttachmentFile( attachment );
    }
  });
}

function upsertWorkspaceImageFile( image: VaultImageFile ): void {
  upsertVaultImageFile( vaultState, image, () => createId( 'folder' ) );
}

function upsertWorkspaceAttachmentFile( attachment: VaultAttachmentFile ): void {
  upsertVaultAttachmentFile( vaultState, attachment, () => createId( 'folder' ) );
}

function rebuildWorkspaceAssetFolders(): void {
  rebuildVaultAssetFolders( vaultState, () => createId( 'folder' ) );
}

function addVaultWarning( message: string ): void {
  vaultSession.warnings = [ message, ...vaultSession.warnings ].slice( 0, 200 );
  notify( message, 'warning' );
}

function notifyRecoverySuccess( message: string, tone: ToastTone ): void {
  if ( vaultSession.warnings.length ) {
    notify( vaultSession.warnings[ 0 ], 'warning' );
  } else {
    notify( message, tone );
  }
}

async function removeRecentlyDeletedNotes( ids: string[], successMessage: string ): Promise<boolean> {
  return runRecoveryOperation( async () => {
    const uniqueIds = [ ...new Set( ids ) ];
    const availableIds = new Set( recentlyDeletedState.notes.map( ( entry ) => entry.id ) );
    if (
      !uniqueIds.length
      || uniqueIds.some( ( id ) => !availableIds.has( id ) )
    ) {
      return false;
    }
    if ( !( await flushVault() ) ) {
      return false;
    }

    if ( vaultSession.backend === 'browser' ) {
      const removedIds = new Set( uniqueIds );
      const candidateDeletedNotes = recentlyDeletedState.notes.filter(
        ( entry ) => !removedIds.has( entry.id )
      );
      if ( !persistBrowserWorkspace( snapshotVault(), candidateDeletedNotes ) ) {
        recentlyDeletedState.error = 'Recently Deleted could not be updated.';
        notify( 'Recently Deleted was not changed because browser storage is unavailable', 'warning' );

        return false;
      }

      hydrateRecentlyDeletedNotes( candidateDeletedNotes );
      savedVersion = dirtyVersion;
      recentlyDeletedState.error = null;
      notify( successMessage, 'neutral' );
      scheduleRecentlyDeletedExpiry();

      return true;
    }

    const path = vaultSession.path;
    if ( !path ) {
      return false;
    }
    const saved = await performNativeRecoverySave(
      () => deleteRecentlyDeletedNotes( path, uniqueIds, vaultSession.revision ),
      ( result ) => {
        applyWorkspaceSaveResult( result );
        removeRecentlyDeletedEntries( result.removedIds );
      },
      async () => Boolean( await reconcileNativeWorkspace( path ) ),
      'Recently Deleted could not be updated.'
    );
    if ( saved ) {
      notifyRecoverySuccess( successMessage, 'neutral' );
      scheduleRecentlyDeletedExpiry();
    }

    return saved;
  });
}

async function pruneExpiredRecentlyDeletedNotes(): Promise<boolean> {
  const now = Date.now();
  if ( !recentlyDeletedState.notes.some( ( entry ) => entry.expiresAt <= now ) ) {
    scheduleRecentlyDeletedExpiry();

    return true;
  }

  const pruned = await runRecoveryOperation( async () => {
    if ( !( await flushVault() ) ) {
      return false;
    }

    if ( vaultSession.backend === 'browser' ) {
      const candidateDeletedNotes = recentlyDeletedState.notes.filter(
        ( entry ) => entry.expiresAt > Date.now()
      );
      if ( !persistBrowserWorkspace( snapshotVault(), candidateDeletedNotes ) ) {
        recentlyDeletedState.error = 'Expired notes could not be removed safely.';
        addVaultWarning( 'Expired notes remain recoverable because browser storage could not be updated.' );

        return false;
      }

      hydrateRecentlyDeletedNotes( candidateDeletedNotes );
      savedVersion = dirtyVersion;
      recentlyDeletedState.error = null;

      return true;
    }

    const path = vaultSession.path;
    if ( !path ) {
      return false;
    }

    return performNativeRecoverySave(
      () => pruneRecentlyDeletedNotes( path, vaultSession.revision ),
      ( result ) => {
        applyWorkspaceSaveResult( result );
        removeRecentlyDeletedEntries( result.removedIds );
      },
      async () => Boolean( await reconcileNativeWorkspace( path ) ),
      'Expired notes could not be removed safely.'
    );
  });

  const expiredEntriesRemain = recentlyDeletedState.notes.some(
    ( entry ) => entry.expiresAt <= Date.now()
  );
  if ( pruned && !expiredEntriesRemain ) {
    scheduleRecentlyDeletedExpiry();
  } else {
    scheduleRecentlyDeletedExpiryRetry();
  }
  if ( pruned && vaultSession.backend === 'native' && vaultSession.warnings.length ) {
    if ( expiredEntriesRemain ) {
      recentlyDeletedState.error = vaultSession.warnings[ 0 ];
    }
    notify( vaultSession.warnings[ 0 ], 'warning' );
  }

  return pruned;
}

function scheduleRecentlyDeletedExpiry(): void {
  clearTimeout( recentlyDeletedTimer );
  recentlyDeletedTimer = undefined;
  if ( !recentlyDeletedState.notes.length ) {
    recentlyDeletedRetryDelay = RECENTLY_DELETED_RETRY_INITIAL_DELAY;

    return;
  }
  if ( vaultSession.phase !== 'ready' ) {
    return;
  }

  const nextExpiry = recentlyDeletedState.notes.reduce(
    ( earliest, entry ) => Math.min( earliest, entry.expiresAt ),
    Number.POSITIVE_INFINITY
  );
  const delay = Math.max( 0, nextExpiry - Date.now() );
  if ( delay > 0 ) {
    recentlyDeletedRetryDelay = RECENTLY_DELETED_RETRY_INITIAL_DELAY;
  }
  recentlyDeletedTimer = setTimeout(
    () => void pruneExpiredRecentlyDeletedNotes(),
    Math.max( 25, Math.min( delay, 2_147_483_647 ) )
  );
}

function scheduleRecentlyDeletedExpiryRetry(): void {
  clearTimeout( recentlyDeletedTimer );
  recentlyDeletedTimer = undefined;
  if ( !recentlyDeletedState.notes.length || vaultSession.phase !== 'ready' ) {
    return;
  }

  const delay = recentlyDeletedRetryDelay;
  recentlyDeletedRetryDelay = Math.min(
    recentlyDeletedRetryDelay * 2,
    RECENTLY_DELETED_RETRY_MAX_DELAY
  );
  recentlyDeletedTimer = setTimeout(
    () => void pruneExpiredRecentlyDeletedNotes(),
    delay
  );
}

function applyVaultMutation( mutation: () => void ): void {
  suppressPersistence += 1;
  try {
    mutation();
  } finally {
    suppressPersistence -= 1;
  }
}

function applyNoteDeletion( id: string ): void {
  const index = vaultState.notes.findIndex( ( note ) => note.id === id );
  if ( index < 0 ) {
    return;
  }

  const wasActive = vaultState.activeNoteId === id;
  const fallbackId = wasActive ? noteDeletionFallback( id ) : undefined;
  vaultState.notes.splice( index, 1 );
  removeRecentNote( id );
  removeNoteFromNavigation( id );
  if (
    wasActive
    && ( !fallbackId || !activateNoteAfterDeletion( fallbackId ) )
  ) {
    vaultState.activeNoteId = null;
  }
}

function snapshotVaultAfterDeletion( id: string ): VaultData {
  const previousVault = snapshotVault();
  const previousNavigation = snapshotNoteNavigation();
  const previousWorkspaceUi = snapshotWorkspaceUi();
  applyVaultMutation( () => applyNoteDeletion( id ) );
  const candidateVault = snapshotVault();
  hydrateVault( previousVault );
  restoreNoteNavigation( previousNavigation );
  restoreWorkspaceUi( previousWorkspaceUi );

  return candidateVault;
}

function restoreFailedNoteDeletion(
  index: number,
  note: Note,
  previousVault: VaultData,
  previousNavigation: NoteNavigationState,
  previousWorkspaceUi: WorkspaceUiSnapshot
): void {
  applyVaultMutation( () => {
    if ( !noteExists( note.id ) ) {
      vaultState.notes.splice( Math.min( index, vaultState.notes.length ), 0, cloneValue( note ) );
    }
    vaultState.activeNoteId = previousVault.activeNoteId;
    vaultState.recentNoteIds.splice(
      0,
      vaultState.recentNoteIds.length,
      ...previousVault.recentNoteIds
    );
    vaultState.selectedFolderId = previousVault.selectedFolderId;
    restoreNoteNavigation( previousNavigation );
    restoreWorkspaceUi( previousWorkspaceUi );
  });
}

function applyRestoredNote( note: Note, previousActiveNoteId: string | null ): void {
  if ( noteExists( note.id ) ) {
    return;
  }

  vaultState.notes.unshift( cloneValue( note ) );
  recordDirectNoteNavigation( previousActiveNoteId, note.id );
  activateNote( note.id );
  vaultState.selectedFolderId = 'all';
  uiState.tool = 'notes';
  uiState.notesView = 'editor';
  uiState.noteFilter = '';
}

function snapshotVaultWithRestoredNote( note: Note ): VaultData {
  const previousVault = snapshotVault();
  const previousNavigation = snapshotNoteNavigation();
  const previousWorkspaceUi = snapshotWorkspaceUi();
  applyVaultMutation( () => applyRestoredNote( note, vaultState.activeNoteId ) );
  const candidateVault = snapshotVault();
  hydrateVault( previousVault );
  restoreNoteNavigation( previousNavigation );
  restoreWorkspaceUi( previousWorkspaceUi );

  return candidateVault;
}

function buildBrowserRestoredNote( deletedNote: RecentlyDeletedNote ): Note {
  const originalFolderId = folderIdForPath( deletedNote.originalFolderPath );
  const folderId = originalFolderId ?? null;
  const baseTitle = deletedNote.note.title.trim() || 'Untitled note';
  let title = baseTitle;
  let suffix = 2;
  while ( restoredTitleConflicts( title, folderId ) ) {
    title = `${ baseTitle } ${ suffix }`;
    suffix += 1;
  }

  const originalExtension = deletedNote.note.relativePath.toLocaleLowerCase().endsWith( '.markdown' )
    ? 'markdown'
    : 'md';
  const restoredFolderPath = folderId ? folderPath( folderId ) : '';
  const relativePath = `${ restoredFolderPath ? `${ restoredFolderPath }/` : '' }${ safeNoteStem( title ) }.${ originalExtension }`;

  return {
    ...cloneValue( deletedNote.note ),
    id: noteExists( deletedNote.note.id ) ? createId( 'note' ) : deletedNote.note.id,
    title,
    relativePath,
    folderId
  };
}

function folderIdForPath( path: string ): string | undefined {
  if ( !path ) {
    return undefined;
  }

  return vaultState.folders.find( ( folder ) => folderPath( folder.id ) === path )?.id;
}

function restoredTitleConflicts( title: string, folderId: string | null ): boolean {
  const note: Note = {
    id: '',
    title,
    content: '',
    relativePath: '',
    folderId,
    tags: [],
    pinned: false,
    createdAt: 0,
    updatedAt: 0
  };

  return vaultState.notes.some(
    ( candidate ) => candidate.folderId === folderId && noteStemKey( candidate ) === noteStemKey( note )
  ) || vaultState.folders.some(
    ( folder ) => folder.parentId === folderId && folderConflictsWithNote( folder.name, note )
  );
}

function removeRecentlyDeletedEntries( ids: string[]): void {
  const removedIds = new Set( ids );
  hydrateRecentlyDeletedNotes(
    recentlyDeletedState.notes.filter( ( entry ) => !removedIds.has( entry.id ) )
  );
}

function snapshotWorkspaceUi(): WorkspaceUiSnapshot {
  return {
    tool: uiState.tool,
    notesView: uiState.notesView,
    noteFilter: uiState.noteFilter
  };
}

function restoreWorkspaceUi( snapshot: WorkspaceUiSnapshot ): void {
  uiState.tool = snapshot.tool;
  uiState.notesView = snapshot.notesView;
  uiState.noteFilter = snapshot.noteFilter;
}

function uniqueNoteTitle( base: string ): string {
  return uniqueVaultNoteTitle( vaultState, base );
}

function ensureFolderPath( path: string ): string | null {
  return ensureVaultFolderPath( vaultState, path, () => createId( 'folder' ) );
}

function descendantFolderIds( id: string ): string[] {
  return vaultDescendantFolderIds( vaultState, id );
}

function readStoredVault(): StoredBrowserWorkspace | null {
  return readBrowserWorkspace( normalizeVault );
}

function snapshotVault(): VaultData {
  return cloneValue( vaultState );
}

function snapshotVaultForSave(): VaultData {
  const snapshot = snapshotVault();
  for ( const note of snapshot.notes ) {
    const originalPath = pendingNoteOriginalPaths.get( note.id );
    if ( !originalPath ) {
      continue;
    }
    const targetPath = projectedNoteRelativePath( note, snapshot.folders, originalPath );
    note.content = rewriteAssetDestinationsForNotePath(
      note.content,
      originalPath,
      targetPath
    );
  }

  return snapshot;
}

function snapshotRecentlyDeletedNotes(): RecentlyDeletedNote[] {
  return cloneValue( recentlyDeletedState.notes );
}

function hydrateVault( vault: Partial<VaultData> ): void {
  pendingNoteOriginalPaths.clear();
  suppressPersistence += 1;
  try {
    Object.assign( vaultState, normalizeVault( vault ) );
  } finally {
    suppressPersistence -= 1;
  }
}

function hydrateRecentlyDeletedNotes( notes: RecentlyDeletedNote[]): void {
  recentlyDeletedState.notes = cloneValue( notes ).sort( compareRecentlyDeletedNotes );
  if ( !notes.length ) {
    clearTimeout( recentlyDeletedTimer );
    recentlyDeletedTimer = undefined;
    uiState.notesView = 'editor';
  }
}

function applyWorkspace( workspace: WorkspaceLoad, recentVaults = vaultSession.recentVaults ): void {
  const previousPath = vaultSession.path;
  sessionGeneration += 1;
  hydrateVault({ ...workspace.vault, name: workspace.descriptor.name });
  uiState.imageRefreshToken += 1;
  uiState.attachmentRefreshToken += 1;
  hydrateRecentlyDeletedNotes( workspace.recentlyDeletedNotes );
  initializeNoteEditorPositions(
    'native',
    workspace.descriptor.path,
    vaultState.notes,
    workspace.editorPositions,
    workspace.editorPositionsWritable,
    workspace.editorPositionsRevision
  );
  pruneNoteEditorHistories(
    editorPositionVaultId( 'native', workspace.descriptor.path ),
    vaultState.notes
  );
  if ( previousPath === workspace.descriptor.path ) {
    pruneNoteNavigation();
  } else {
    resetNoteNavigation();
    uiState.notesView = 'editor';
  }
  vaultSession.phase = 'ready';
  vaultSession.path = workspace.descriptor.path;
  vaultSession.revision = workspace.revision;
  vaultSession.error = null;
  vaultSession.conflict = false;
  vaultSession.warnings = workspace.warnings;
  vaultSession.recentVaults = mergeRecentVaults( workspace.descriptor, recentVaults );
  dirtyVersion = 0;
  savedVersion = 0;
  uiState.saveStatus = 'saved';
  uiState.lastSavedAt = Date.now();
  uiState.noteFilter = '';
  uiState.commandOpen = false;
  resetSearchState();
  uiState.vaultChooserOpen = false;
  recentlyDeletedState.error = null;
  scheduleRecentlyDeletedExpiry();
  if ( workspace.warnings.length ) {
    notify( `${ workspace.warnings.length } ${ workspace.warnings.length === 1 ? 'file warning' : 'file warnings' } while opening the vault`, 'warning' );
  }
}

function currentEditorPositionVaultId(): string {
  return editorPositionVaultId( vaultSession.backend, vaultSession.path );
}

async function flushBeforeVaultChange(): Promise<boolean> {
  if ( savedVersion < dirtyVersion ) {
    if ( vaultSession.phase !== 'ready' ) {
      vaultSession.error = 'Choose a vault before saving changes.';

      return false;
    }
    if ( !( await flushVault() ) ) {
      vaultSession.error = 'Save the current changes before switching vaults.';

      return false;
    }
  }
  const positionsSaved = await flushNoteEditorPositions( currentEditorPositionVaultId() );
  if ( !positionsSaved ) {
    vaultSession.error = 'Save the current document position before switching vaults.';
  }

  return positionsSaved;
}

function persistBrowserWorkspace(
  vault: VaultData,
  recentlyDeletedNotes: RecentlyDeletedNote[]
): boolean {
  try {
    writeBrowserWorkspace( vault, recentlyDeletedNotes );
    vaultSession.error = null;
    vaultSession.conflict = false;
    uiState.saveStatus = 'saved';
    uiState.lastSavedAt = Date.now();

    return true;
  } catch {
    uiState.saveStatus = 'error';

    return false;
  }
}

function installVaultLifecycleHandlers(): void {
  if ( typeof window === 'undefined' || externalCheckTimer ) {
    return;
  }

  window.addEventListener( 'blur', () => void flushApplicationState() );
  window.addEventListener( 'focus', () => {
    void ( async () => {
      await refreshWorkspaceFromDisk();
      await pruneExpiredRecentlyDeletedNotes();
    })();
  });
  window.addEventListener( 'beforeunload', () => {
    void flushVault();
    void flushNoteEditorPositions();
  });
  document.addEventListener( 'visibilitychange', () => {
    if ( document.visibilityState === 'hidden' ) {
      void flushApplicationState();
    } else {
      void ( async () => {
        await refreshWorkspaceFromDisk();
        await pruneExpiredRecentlyDeletedNotes();
      })();
    }
  });

  if ( vaultSession.backend === 'native' ) {
    void installNativeCloseHandler();
  }

  externalCheckTimer = setInterval(
    () => void refreshWorkspaceFromDisk(),
    EXTERNAL_CHECK_DELAY
  );
}

async function flushApplicationState(): Promise<void> {
  await flushVault();
  await flushNoteEditorPositions();
}

async function installNativeCloseHandler(): Promise<void> {
  if ( closeHandlerInstalled ) {
    return;
  }
  closeHandlerInstalled = true;
  const appWindow = getCurrentWindow();
  try {
    await appWindow.onCloseRequested( async ( event ) => {
      if ( closingAfterSave ) {
        return;
      }
      if ( vaultSession.busy ) {
        event.preventDefault();
        notify( 'Wait for the current vault action to finish before closing', 'warning' );

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
      if ( !saved ) {
        notify( vaultSession.error || 'Save the current changes before closing', 'warning' );

        return;
      }
      const positionsSaved = await flushNoteEditorPositions();
      if ( !positionsSaved ) {
        notify( 'Notes are saved, but document positions could not be saved', 'warning' );
      }
      closingAfterSave = true;
      await appWindow.destroy();
    });
  } catch ( error ) {
    closeHandlerInstalled = false;
    vaultSession.error = errorMessage( error, 'Could not install the safe-close handler.' );
  }
}

async function refreshWorkspaceFromDisk(): Promise<void> {
  const path = vaultSession.path;
  if (
    vaultSession.backend !== 'native'
    || vaultSession.phase !== 'ready'
    || !path
    || vaultSession.busy
    || uiState.vaultChooserOpen
    || checkingExternalChanges
    || document.visibilityState === 'hidden'
    || saveInFlight
    || recoverySaveInFlight
    || dirtyVersion > savedVersion
  ) {
    return;
  }

  checkingExternalChanges = true;
  const generation = sessionGeneration;
  try {
    const revision = await getWorkspaceRevision( path );
    if ( revision === vaultSession.revision ) {
      return;
    }
    await flushNoteEditorPositions( currentEditorPositionVaultId() );
    const workspace = await openWorkspace( path, createEmptyVault() );
    if (
      generation !== sessionGeneration
      || path !== vaultSession.path
      || vaultSession.busy
      || recoverySaveInFlight
      || dirtyVersion > savedVersion
    ) {
      return;
    }
    applyWorkspace( workspace );
    notify( 'Reloaded changes from the vault folder', 'neutral' );
  } catch ( error ) {
    if ( generation === sessionGeneration && path === vaultSession.path ) {
      vaultSession.error = errorMessage( error, 'The vault folder could not be checked for changes.' );
    }
  } finally {
    checkingExternalChanges = false;
  }
}

function setVaultError( error: unknown, fallback: string ): void {
  vaultSession.error = errorMessage( error, fallback );
  vaultSession.conflict = false;
}

function applyEnabledSnippets(): void {
  if ( typeof document === 'undefined' ) {
    return;
  }
  let style = document.querySelector<HTMLStyleElement>( '#obsidian-at-home-user-snippets' );
  if ( !style ) {
    style = document.createElement( 'style' );
    style.id = 'obsidian-at-home-user-snippets';
    document.head.appendChild( style );
  }
  style.textContent = vaultState.snippets
    .filter( ( snippet ) => snippet.enabled )
    .map( ( snippet ) => `/* ${ snippet.name } */\n${ snippet.css }` )
    .join( '\n\n' );
}
