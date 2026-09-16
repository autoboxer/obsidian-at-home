import { computed, watch } from 'vue';
import { createEmptyVault } from '../data/seed';
import { createNoteLinkRewriter, findBacklinks, parseNoteLinks, resolveNoteLink, searchNotes } from '../lib';
import { resolveMarkdownImagePath } from '../lib/imageEmbeds';
import {
  formatMarkdownImage,
  parseMarkdownImages,
  relativeImageDestination
} from '../lib/markdownImages';
import {
  formatMarkdownAttachment,
  parseMarkdownAttachments,
  relativeAttachmentDestination
} from '../lib/markdownAttachments';
import { writeBrowserWorkspace } from '../services/browserWorkspace';
import {
  editorPositionVaultId,
  initializeNoteEditorPositions,
  pruneNoteEditorPositions
} from './editorPositions';
import { pruneNoteEditorHistories } from './editorHistories';
import {
  applyMarkdownReplacements,
  folderContainsVaultAssets,
  rewriteVaultAssetDestinationsForNotePath
} from './vaultAssets';
import {
  createId,
  descendantFolderIds as vaultDescendantFolderIds,
  folderPathFromFolders,
  planImportedNotes,
  projectedNoteRelativePath
} from './vaultModel';
import { createVaultAssetDeletion } from './vaultAssetDeletion';
import { createVaultAssetOperations } from './vaultAssetOperations';
import { createVaultContent } from './vaultContent';
import { createVaultItemActions } from './vaultItemActions';
import { createVaultRecovery } from './vaultRecovery';
import { createVaultLifecycle } from './vaultLifecycle';
import {
  clampZoom,
  cloneValue,
  createVaultChangeTracker,
  errorMessage,
  isRevisionConflict,
  mergeRecentVaults,
  normalizeVault,
  persistStoredZoom,
  readStoredZoom,
  zoomStep
} from './vaultPersistence';
import { createVaultNavigation } from './vaultNavigation';
import {
  canEditVault,
  recentlyDeletedState,
  uiState,
  vaultSession,
  vaultState,
  type ToastAction,
  type ToastTone
} from './vaultState';
export {
  assetDeletionState,
  canEditVault,
  recentlyDeletedState,
  searchState,
  treeDragState,
  uiState,
  vaultSession,
  vaultState,
  vaultTreeRevealTarget
} from './vaultState';
export { MAX_ZOOM, MIN_ZOOM } from './vaultPersistence';
export type { VaultItemLocator } from './vaultItemActions';
import {
  getWorkspaceRevision,
  importWorkspaceAssets,
  openWorkspace,
  saveWorkspace,
  saveWorkspaceChanges,
  saveWorkspaceWithImageImport
} from '../services/native';
import type {
  CssSnippet,
  ExportNote,
  ExportSnippet,
  ExportTemplate,
  Folder,
  ImportResult,
  Note,
  RecentlyDeletedNote,
  VaultData,
  WorkspaceLoad,
  WorkspaceChanges,
  WorkspaceSaveResult
} from '../types';

const PERSIST_DELAY = 220;

export const NOTE_DRAG_MIME = 'application/x-obsidian-at-home-note-id';
export const FOLDER_DRAG_MIME = 'application/x-obsidian-at-home-folder-id';

uiState.zoom = readStoredZoom();
let persistTimer: ReturnType<typeof setTimeout> | undefined;
let toastTimer: ReturnType<typeof setTimeout> | undefined;
let initialized = false;
let suppressPersistence = 0;
let sessionGeneration = 0;
let saveInFlight: Promise<boolean> | null = null;
let recoverySaveInFlight: Promise<boolean> | null = null;
const pendingNoteOriginalPaths = new Map<string, string>();

const vaultChanges = createVaultChangeTracker(
  vaultState,
  () => initialized && !suppressPersistence && canEditVault.value,
  () => {
    uiState.saveStatus = 'saving';
    clearTimeout( persistTimer );
    persistTimer = setTimeout( () => void flushApplicationState(), PERSIST_DELAY );
  }
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

export async function overwriteFilesystemVault(): Promise<boolean> {
  if ( !canEditVault.value ) {
    return false;
  }
  const path = vaultSession.path;
  if ( vaultSession.backend !== 'native' || !path || vaultSession.busy ) {
    return false;
  }
  vaultSession.busy = true;
  clearTimeout( persistTimer );
  try {
    const { version: targetVersion } = vaultChanges.capture();
    const currentRevision = await getWorkspaceRevision( path );
    const result = await saveWorkspace( path, snapshotVaultForSave(), currentRevision );
    applySavedNotePaths( result.notePaths );
    vaultSession.revision = result.revision;
    vaultSession.error = null;
    vaultSession.conflict = false;
    vaultSession.warnings = result.warnings;
    vaultChanges.acknowledge( targetVersion );
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
  vaultChanges.capture();
  clearTimeout( persistTimer );
  if ( vaultSession.access.mode === 'read-only' ) {
    return !vaultChanges.hasChanges && !imageImportTransactionId;
  }

  if ( recoverySaveInFlight ) {
    const saved = await recoverySaveInFlight;
    if ( !saved ) {
      return false;
    }

    return vaultChanges.hasChanges
      ? flushVault( imageImportTransactionId )
      : true;
  }

  if ( !initialized || ( !vaultChanges.hasChanges && !imageImportTransactionId ) ) {
    return true;
  }

  if ( saveInFlight ) {
    const saved = await saveInFlight;
    if ( !saved ) {
      return false;
    }

    return vaultChanges.hasChanges
      ? flushVault( imageImportTransactionId )
      : true;
  }

  const changes = vaultChanges.capture();
  const targetVersion = changes.version;
  const generation = sessionGeneration;
  const path = vaultSession.path;
  const fullSnapshot = vaultSession.backend === 'browser' || imageImportTransactionId
    ? snapshotVaultForSave()
    : undefined;
  const patch = fullSnapshot ? undefined : snapshotWorkspaceChanges( changes );
  const recentlyDeletedSnapshot = vaultSession.backend === 'browser' ? snapshotRecentlyDeletedNotes() : [];
  uiState.saveStatus = 'saving';

  const operation = ( async (): Promise<boolean> => {
    if ( vaultSession.backend === 'browser' ) {
      const saved = persistBrowserWorkspace( fullSnapshot!, recentlyDeletedSnapshot );
      if ( saved && generation === sessionGeneration ) {
        vaultChanges.acknowledge( targetVersion );
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
          fullSnapshot!,
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
        result = await saveWorkspaceChanges( path, patch!, vaultSession.revision );
      }
      if ( generation !== sessionGeneration || path !== vaultSession.path ) {
        return true;
      }
      applySavedNotePaths( result.notePaths );
      vaultSession.revision = result.revision;
      vaultSession.error = null;
      vaultSession.conflict = false;
      vaultSession.warnings = result.warnings;
      vaultChanges.acknowledge( targetVersion );
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
  if ( saved && generation === sessionGeneration && vaultChanges.hasChanges ) {
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
  batchChanges: vaultChanges.batch,
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
  currentFolderId,
  pruneNoteNavigation,
  recordDirectNoteNavigation,
  resetNoteNavigation,
  resetSearchState,
  restoreNoteNavigation,
  snapshotNoteNavigation,
  touchRecentNote
} = vaultNavigation;

export const noteLinkPaths = computed( () => new Map(
  vaultState.notes.map( ( note ) => [
    note.id,
    projectedNoteRelativePath( note, vaultState.folders, note.relativePath )
  ])
) );

const vaultContent = createVaultContent({
  batchChanges: vaultChanges.batch,
  activeNote: () => activeNote.value,
  currentFolderId,
  flushVault,
  folderContainsAssets,
  notify,
  rememberNoteOriginalPath,
  noteLinkPaths: () => noteLinkPaths.value,
  selectNote
});

export const {
  createFolder,
  createFromTemplate,
  createLinkedNote,
  createNote,
  deleteFolder,
  deleteSnippet,
  discardSnippetDraft,
  moveFolder,
  moveNoteToFolder,
  renameFolder,
  saveSnippet,
  saveTemplate,
  selectSnippet,
  snippetDraftConflict,
  snippetDraftIsDirty,
  snippetLibrary,
  snippetWorkspace,
  togglePinned,
  updateNote,
  updateSnippetDraft
} = vaultContent;

const vaultAssetOperations = createVaultAssetOperations({
  applyVaultMutation,
  applyWorkspaceSaveResult,
  flushVault,
  folderPath,
  notify,
  runExclusiveVaultDataOperation
});

export const {
  applyEmbeddedAttachmentResult,
  applyEmbeddedImageResult,
  applyExternalAssetDiscardResult,
  moveVaultAttachmentToFolder,
  moveVaultImageToFolder,
  renameVaultAttachment,
  renameVaultImage
} = vaultAssetOperations;

const {
  applyWorkspaceAttachmentFiles,
  applyWorkspaceImageFiles
} = vaultAssetOperations;

const vaultAssetDeletion = createVaultAssetDeletion({
  applyVaultMutation,
  applyWorkspaceSaveResult,
  currentEditorPositionVaultId,
  flushVault,
  getSessionGeneration: () => sessionGeneration,
  notify,
  runExclusiveVaultDataOperation
});

export const {
  cancelVaultAssetDeletion,
  confirmVaultAssetDeletion,
  requestVaultAssetDeletion
} = vaultAssetDeletion;

const { clearAssetDeletionRequest } = vaultAssetDeletion;

const vaultItemActions = createVaultItemActions({ flushVault, folderPath, notify });

export const {
  activateVaultAttachment,
  locateVaultItem,
  revealVaultItemInTree,
  showVaultItemInFolder,
  vaultTreeItemIsRevealed,
  vaultTreeRevealIncludesFolder
} = vaultItemActions;

const vaultRecovery = createVaultRecovery({
  acknowledgeVaultChanges: () => vaultChanges.acknowledge( vaultChanges.version ),
  applyVaultMutation,
  applyWorkspaceSaveResult,
  currentEditorPositionVaultId,
  flushVault,
  folderPath,
  hydrateVault,
  navigation: vaultNavigation,
  notify,
  performNativeRecoverySave,
  persistBrowserWorkspace,
  reconcileNativeWorkspace,
  snapshotVault
});

export const {
  deleteNote,
  emptyRecentlyDeletedNotes,
  permanentlyDeleteRecentlyDeletedNote,
  recentlyDeletedNotes,
  restoreRecentlyDeletedNote
} = vaultRecovery;

const {
  hydrateRecentlyDeletedNotes,
  scheduleRecentlyDeletedExpiry,
  snapshotRecentlyDeletedNotes
} = vaultRecovery;

const vaultLifecycle = createVaultLifecycle({
  advanceSessionGeneration: () => {
    sessionGeneration += 1;
  },
  applyWorkspace,
  currentEditorPositionVaultId,
  flushVault,
  getSessionGeneration: () => sessionGeneration,
  hasRecoverySaveInFlight: () => recoverySaveInFlight !== null,
  hasSaveInFlight: () => saveInFlight !== null,
  hydrateVault,
  markVaultInitialized: () => {
    initialized = true;
  },
  notify,
  persistBrowserWorkspace,
  recovery: vaultRecovery,
  resetNoteNavigation,
  snapshotVault,
  vaultChanges
});

export const {
  createFilesystemVault,
  forgetCurrentVault,
  initializeVault,
  openFilesystemVault,
  reloadFilesystemVault,
  showCurrentVaultInFolder,
  switchFilesystemVault
} = vaultLifecycle;

const { flushApplicationState } = vaultLifecycle;

export async function saveSnippetDraft( id: string, asCopy = false ): Promise<boolean> {
  const workspace = snippetWorkspace.value;
  const draft = workspace?.drafts.get( id );
  if ( !workspace || !draft || !snippetDraftIsDirty( id ) || draft.saving ) {
    return false;
  }

  return runExclusiveVaultDataOperation( false, async () => {
    const generation = sessionGeneration;
    draft.saving = true;
    draft.error = null;
    try {
      if ( !( await flushVault() ) ) {
        draft.error = vaultSession.error || 'Save the pending vault changes before saving this draft.';

        return false;
      }
      if ( generation !== sessionGeneration || workspace !== snippetWorkspace.value || !canEditVault.value ) {
        return false;
      }
      if ( !asCopy && snippetDraftConflict( id ) ) {
        return false;
      }
      const saved = vaultState.snippets.find( ( snippet ) => snippet.id === id );
      if ( !asCopy && !saved ) {
        return false;
      }
      const values = cloneValue( draft.values );
      let snippets = cloneValue( vaultState.snippets );
      let nextSnippet: CssSnippet;
      if ( asCopy ) {
        const names = new Set( snippets.map( ( snippet ) => snippet.name.toLocaleLowerCase() ) );
        const baseName = values.name.trim() || 'Untitled snippet';
        let name = baseName;
        for ( let suffix = 1; names.has( name.toLocaleLowerCase() ); suffix += 1 ) {
          name = `${ baseName } (copy${ suffix === 1 ? '' : ` ${ suffix }` })`;
        }
        nextSnippet = { ...values, name, id: createId( 'snippet' ), enabled: false, createdAt: Date.now() };
        snippets.push( nextSnippet );
      } else {
        nextSnippet = { ...cloneValue( saved! ), ...values };
        snippets = snippets.map( ( snippet ) => snippet.id === id ? nextSnippet : snippet );
      }

      // Persist a candidate first. A failed save must not apply CSS, mark the
      // draft clean, or leave it queued for an unrelated autosave to retry.
      let result: WorkspaceSaveResult | undefined;
      if ( vaultSession.backend === 'native' ) {
        if ( !vaultSession.path ) {
          throw new Error( 'Open a vault before saving this draft.' );
        }
        result = await saveWorkspaceChanges(
          vaultSession.path,
          { notes: [], removedNoteIds: [], snippets },
          vaultSession.revision
        );
      } else if ( !persistBrowserWorkspace({ ...snapshotVaultForSave(), snippets }, snapshotRecentlyDeletedNotes() ) ) {
        throw new Error( vaultSession.error || 'The snippet could not be saved.' );
      }
      if ( generation !== sessionGeneration || workspace !== snippetWorkspace.value ) {
        return false;
      }
      applyVaultMutation( () => {
        if ( asCopy ) {
          vaultState.snippets.push( nextSnippet );
        } else {
          Object.assign( saved!, values );
        }
      });
      if ( result ) {
        applyWorkspaceSaveResult( result );
      }
      draft.saving = false;
      discardSnippetDraft( id );
      selectSnippet( nextSnippet.id );
      const warning = result?.warnings[ 0 ];
      notify( warning || ( asCopy ? 'Draft saved as a new, disabled snippet' : 'CSS snippet saved' ), warning ? 'warning' : 'success' );

      return true;
    } catch ( error ) {
      const message = errorMessage( error, 'The snippet could not be saved. Your draft is still available.' );
      draft.error = message;
      if ( generation === sessionGeneration ) {
        vaultSession.error = message;
        vaultSession.conflict = isRevisionConflict( message );
        uiState.saveStatus = 'error';
        uiState.vaultChooserOpen = vaultSession.backend === 'native';
        notify( message, 'warning' );
      }

      return false;
    } finally {
      draft.saving = false;
    }
  });
}

export const outgoingLinks = computed( () => {
  if ( !activeNote.value ) {
    return [];
  }

  return parseNoteLinks( activeNote.value.content ).map( ( link ) => ({
    link,
    note: resolveNoteLink( link, vaultState.notes, activeNote.value, noteLinkPaths.value )
  }) );
});

export const backlinks = computed( () =>
  activeNote.value ? findBacklinks( activeNote.value, vaultState.notes, noteLinkPaths.value ) : []
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
  const previousSavedVersion = vaultChanges.savedVersion;
  const previousActiveNoteId = vaultState.activeNoteId;
  const previousNoteNavigation = snapshotNoteNavigation();
  const previousVault = snapshotVault();
  let plan: ReturnType<typeof planImportedNotes>;
  try {
    plan = planImportedNotes( vaultState, result.notes, replace );
  } catch ( error ) {
    const warning = errorMessage( error, 'The imported note paths could not be reserved.' );
    notify( warning, 'warning' );

    return { attachmentCount: 0, imageCount: 0, noteCount: 0, saved: false, warnings: [ warning ] };
  }
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
        // Copied assets can reserve different folders from their source paths.
        plan = planImportedNotes( vaultState, result.notes, replace );
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

  const now = Date.now();
  const importedNotes: Note[] = plan.notes.map( ({ source, title, folderId, relativePath }) => ({
    id: createId( 'note' ),
    title,
    content: source.content,
    relativePath,
    folderId,
    tags: source.tags,
    pinned: false,
    createdAt: now,
    updatedAt: now
  }) );
  const sourcePaths = new Map( importedNotes.map( ( note, index ) => [ note.id, plan.notes[ index ]!.source.relativePath ]) );
  const existingNotes = replace ? [] : vaultState.notes;
  const destinationNotes = [ ...existingNotes, ...importedNotes ];
  const destinationPaths = new Map([
    ...noteLinkPaths.value,
    ...importedNotes.map( ( note ) => [ note.id, note.relativePath ] as const )
  ]);
  const rewriteOptions = { destinationNotes, preserveUnresolvedPaths: false };
  const rewriteExistingNote = createNoteLinkRewriter( existingNotes, noteLinkPaths.value, destinationPaths, rewriteOptions );
  vaultChanges.batch( () => {
    for ( const note of existingNotes ) {
      const content = rewriteExistingNote( note );
      if ( content !== note.content ) {
        note.content = content;
        note.updatedAt = now;
      }
    }
    const rewriteImportedNote = createNoteLinkRewriter( importedNotes, sourcePaths, destinationPaths, rewriteOptions );
    for ( const note of importedNotes ) {
      const content = rewriteImportedNote( note );
      note.content = rewriteImportedAssetReferences(
        content, sourcePaths.get( note.id )!, note.relativePath, importedImagePaths, importedAttachmentPaths
      );
    }
    vaultState.folders.splice( 0, vaultState.folders.length, ...plan.folders );
    vaultState.notes.splice( 0, vaultState.notes.length, ...destinationNotes );
    const firstImportedNoteId = importedNotes[ 0 ]?.id ?? null;

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
  });
  const saved = await flushVault( imageImportTransactionId );
  if ( !saved ) {
    hydrateVault( previousVault );
    restoreNoteNavigation( previousNoteNavigation );
    vaultChanges.reset( previousSavedVersion );
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
  const previousSavedVersion = vaultChanges.savedVersion;
  const previousNoteNavigation = snapshotNoteNavigation();
  vaultChanges.batch( () => {
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
  });
  const saved = await flushVault();
  if ( !saved ) {
    hydrateVault( previousVault );
    restoreNoteNavigation( previousNoteNavigation );
    vaultChanges.reset( previousSavedVersion );
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
  if ( vaultSession.busy || !canEditVault.value ) {
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
  if ( writesMayResume && vaultChanges.hasChanges ) {
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
  uiState.saveStatus = vaultChanges.hasChanges ? 'saving' : 'saved';
  uiState.lastSavedAt = result.savedAt || Date.now();
}

function applySavedNotePaths( notePaths: Record<string, string> | undefined ): void {
  if ( !notePaths ) {
    return;
  }
  applyVaultMutation( () => {
    for ( const [ id, relativePath ] of Object.entries( notePaths ) ) {
      const note = vaultContent.noteById( id );
      if ( note && relativePath ) {
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
    for ( const noteId of pendingNoteOriginalPaths.keys() ) {
      if ( !vaultContent.noteById( noteId ) ) {
        pendingNoteOriginalPaths.delete( noteId );
      }
    }
  });
}

function applyVaultMutation( mutation: () => void ): void {
  suppressPersistence += 1;
  try {
    mutation();
  } finally {
    suppressPersistence -= 1;
  }
}

function descendantFolderIds( id: string ): string[] {
  return vaultDescendantFolderIds( vaultState, id );
}

function snapshotVault(): VaultData {
  return cloneValue( vaultState );
}

function snapshotWorkspaceChanges(
  changes: ReturnType<typeof vaultChanges.capture>
): WorkspaceChanges {
  const patch: WorkspaceChanges = { notes: [], removedNoteIds: [] };
  for ( const id of changes.noteIds ) {
    const note = vaultContent.noteById( id );
    if ( !note ) {
      patch.removedNoteIds.push( id );
      continue;
    }
    const snapshot = cloneValue( note );
    const originalPath = pendingNoteOriginalPaths.get( id );
    if ( originalPath ) {
      snapshot.content = rewriteAssetDestinationsForNotePath(
        snapshot.content,
        originalPath,
        projectedNoteRelativePath( snapshot, vaultState.folders, originalPath )
      );
    }
    patch.notes.push( snapshot );
  }
  const fields = new Set( changes.fields );
  if ( fields.has( 'folders' ) ) {
    patch.folders = cloneValue( vaultState.folders );
  }
  if ( fields.has( 'name' ) ) {
    patch.name = vaultState.name;
  }
  if ( fields.has( 'templates' ) ) {
    patch.templates = cloneValue( vaultState.templates );
  }
  if ( fields.has( 'snippets' ) ) {
    patch.snippets = cloneValue( vaultState.snippets );
  }
  if ( fields.has( 'imageEmbedSettings' ) ) {
    patch.imageEmbedSettings = cloneValue( vaultState.imageEmbedSettings );
  }
  if ( fields.has( 'attachmentEmbedSettings' ) ) {
    patch.attachmentEmbedSettings = cloneValue( vaultState.attachmentEmbedSettings );
  }
  if ( fields.has( 'activeNoteId' ) || fields.has( 'recentNoteIds' ) || fields.has( 'selectedFolderId' ) ) {
    patch.navigation = {
      activeNoteId: vaultState.activeNoteId,
      recentNoteIds: [ ...vaultState.recentNoteIds ],
      selectedFolderId: vaultState.selectedFolderId
    };
  }

  return patch;
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

function hydrateVault( vault: Partial<VaultData> ): void {
  pendingNoteOriginalPaths.clear();
  suppressPersistence += 1;
  try {
    Object.assign( vaultState, normalizeVault( vault ) );
  } finally {
    suppressPersistence -= 1;
  }
}

function applyWorkspace( workspace: WorkspaceLoad, recentVaults = vaultSession.recentVaults ): void {
  clearAssetDeletionRequest();
  const previousPath = vaultSession.path;
  sessionGeneration += 1;
  clearTimeout( persistTimer );
  vaultSession.access = workspace.access;
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
  vaultChanges.reset();
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
