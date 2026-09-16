import { computed } from 'vue';
import {
  compareRecentlyDeletedNotes,
  RECENTLY_DELETED_LIMIT,
  RECENTLY_DELETED_RETENTION
} from '../services/browserWorkspace';
import {
  archiveWorkspaceNote,
  deleteRecentlyDeletedNotes,
  pruneRecentlyDeletedNotes,
  restoreRecentlyDeletedNote as restoreRecentlyDeletedNoteNative
} from '../services/native';
import type { Note, RecentlyDeletedNote, VaultData, WorkspaceLoad, WorkspaceSaveResult } from '../types';
import {
  captureNoteEditorPosition,
  deleteNoteEditorPosition,
  flushNoteEditorPositions,
  setNoteEditorPosition
} from './editorPositions';
import { deleteNoteEditorHistory } from './editorHistories';
import { createId, folderConflictsWithNote, noteStemKey, safeNoteStem } from './vaultModel';
import type { createVaultNavigation, NoteNavigationState } from './vaultNavigation';
import { cloneValue, normalizeNote } from './vaultPersistence';
import {
  canEditVault,
  recentlyDeletedState,
  uiState,
  vaultSession,
  vaultState,
  type ToastTone,
  type WorkspaceUiSnapshot
} from './vaultState';

const RECENTLY_DELETED_RETRY_INITIAL_DELAY = 5_000;
const RECENTLY_DELETED_RETRY_MAX_DELAY = 5 * 60_000;

type RecoveryNavigation = Pick<ReturnType<typeof createVaultNavigation>,
  | 'activateNote'
  | 'activateNoteAfterDeletion'
  | 'noteDeletionFallback'
  | 'noteExists'
  | 'recordDirectNoteNavigation'
  | 'removeNoteFromNavigation'
  | 'removeRecentNote'
  | 'restoreNoteNavigation'
  | 'snapshotNoteNavigation'
>;

interface VaultRecoveryDependencies {
  acknowledgeVaultChanges: () => void;
  applyVaultMutation: ( mutation: () => void ) => void;
  applyWorkspaceSaveResult: ( result: WorkspaceSaveResult ) => void;
  currentEditorPositionVaultId: () => string;
  flushVault: () => Promise<boolean>;
  folderPath: ( id: string | null ) => string;
  hydrateVault: ( vault: Partial<VaultData> ) => void;
  navigation: RecoveryNavigation;
  notify: ( message: string, tone: ToastTone ) => void;
  performNativeRecoverySave: <T extends WorkspaceSaveResult>(
    request: () => Promise<T>,
    onSuccess: ( result: T ) => void,
    onFailure: () => Promise<boolean>,
    fallbackError: string
  ) => Promise<boolean>;
  persistBrowserWorkspace: ( vault: VaultData, notes: RecentlyDeletedNote[]) => boolean;
  reconcileNativeWorkspace: ( path: string ) => Promise<WorkspaceLoad | null>;
  snapshotVault: () => VaultData;
}

export function createVaultRecovery( dependencies: VaultRecoveryDependencies ) {
  const {
    acknowledgeVaultChanges,
    applyVaultMutation,
    applyWorkspaceSaveResult,
    currentEditorPositionVaultId,
    flushVault,
    folderPath,
    hydrateVault,
    navigation,
    notify,
    performNativeRecoverySave,
    persistBrowserWorkspace,
    reconcileNativeWorkspace,
    snapshotVault
  } = dependencies;
  const {
    activateNote,
    activateNoteAfterDeletion,
    noteDeletionFallback,
    noteExists,
    recordDirectNoteNavigation,
    removeNoteFromNavigation,
    removeRecentNote,
    restoreNoteNavigation,
    snapshotNoteNavigation
  } = navigation;
  let recentlyDeletedTimer: ReturnType<typeof setTimeout> | undefined;
  let recentlyDeletedRetryDelay = RECENTLY_DELETED_RETRY_INITIAL_DELAY;

  const recentlyDeletedNotes = computed<RecentlyDeletedNote[]>( () =>
    [ ...recentlyDeletedState.notes ].sort( compareRecentlyDeletedNotes )
  );

  async function deleteNote( id: string ): Promise<boolean> {
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
        acknowledgeVaultChanges();
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

  async function restoreRecentlyDeletedNote( id: string ): Promise<boolean> {
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
        acknowledgeVaultChanges();
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

  async function permanentlyDeleteRecentlyDeletedNote( id: string ): Promise<boolean> {
    return removeRecentlyDeletedNotes([ id ], 'Note deleted permanently' );
  }

  async function emptyRecentlyDeletedNotes(): Promise<boolean> {
    const ids = recentlyDeletedState.notes.map( ( entry ) => entry.id );
    if ( !ids.length ) {
      return true;
    }

    return removeRecentlyDeletedNotes( ids, 'Recently Deleted emptied' );
  }

  async function runRecoveryOperation( operation: () => Promise<boolean> ): Promise<boolean> {
    if (
      recentlyDeletedState.busy
      || vaultSession.busy
      || !canEditVault.value
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
        acknowledgeVaultChanges();
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
    if ( !canEditVault.value ) {
      return false;
    }
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
        acknowledgeVaultChanges();
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
    if ( !canEditVault.value ) {
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
    if ( !recentlyDeletedState.notes.length || !canEditVault.value ) {
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

    return normalizeNote({
      ...cloneValue( deletedNote.note ),
      id: noteExists( deletedNote.note.id ) ? createId( 'note' ) : deletedNote.note.id,
      title,
      relativePath,
      folderId
    });
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

  function snapshotRecentlyDeletedNotes(): RecentlyDeletedNote[] {
    return cloneValue( recentlyDeletedState.notes );
  }

  function hydrateRecentlyDeletedNotes( notes: RecentlyDeletedNote[]): void {
    recentlyDeletedState.notes = cloneValue( notes ).sort( compareRecentlyDeletedNotes );
    if ( !notes.length ) {
      clearTimeout( recentlyDeletedTimer );
      recentlyDeletedTimer = undefined;
      uiState.notesView = 'editor';
    }
  }

  return {
    deleteNote,
    emptyRecentlyDeletedNotes,
    hydrateRecentlyDeletedNotes,
    permanentlyDeleteRecentlyDeletedNote,
    pruneExpiredRecentlyDeletedNotes,
    recentlyDeletedNotes,
    restoreRecentlyDeletedNote,
    scheduleRecentlyDeletedExpiry,
    snapshotRecentlyDeletedNotes
  };
}
