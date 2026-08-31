import { computed, reactive } from 'vue';
import type { Note, SearchScope } from '../types';
import {
  recentlyDeletedState,
  searchState,
  uiState,
  vaultState,
  type FolderSelection,
  type SmartFolderSelection
} from './vaultState';

const NOTE_NAVIGATION_LIMIT = 100;
const RECENT_NOTE_LIMIT = 10;

export interface NoteNavigationState {
  back: string[];
  forward: string[];
}

interface VaultNavigationDependencies {
  folderPath: ( id: string | null ) => string;
  isNoteVisible: ( id: string ) => boolean;
}

export function createVaultNavigation(
  dependencies: VaultNavigationDependencies
) {
  const noteNavigationState = reactive<NoteNavigationState>({
    back: [],
    forward: []
  });

  const backNavigationNote = computed<Note | undefined>( () =>
    findNoteNavigationTarget( noteNavigationState.back )
  );

  const forwardNavigationNote = computed<Note | undefined>( () =>
    findNoteNavigationTarget( noteNavigationState.forward )
  );

  const canNavigateBack = computed( () => Boolean( backNavigationNote.value ) );
  const canNavigateForward = computed( () => Boolean( forwardNavigationNote.value ) );

  function selectNote( id: string ): void {
    if ( !noteExists( id ) ) {
      return;
    }

    if ( id !== vaultState.activeNoteId ) {
      recordDirectNoteNavigation( vaultState.activeNoteId, id );
    }
    activateNote( id );
    uiState.tool = 'notes';
    uiState.notesView = 'editor';
  }

  function navigateBack(): boolean {
    const navigated = traverseNoteNavigation(
      noteNavigationState.back,
      noteNavigationState.forward
    );
    if ( navigated ) {
      uiState.tool = 'notes';
      uiState.notesView = 'editor';
    }

    return navigated;
  }

  function navigateForward(): boolean {
    const navigated = traverseNoteNavigation(
      noteNavigationState.forward,
      noteNavigationState.back
    );
    if ( navigated ) {
      uiState.tool = 'notes';
      uiState.notesView = 'editor';
    }

    return navigated;
  }

  function selectFolder( selection: FolderSelection ): void {
    vaultState.selectedFolderId = isSmartFolderSelection( selection )
      ? selection
      : 'all';
    uiState.tool = 'notes';
    uiState.notesView = 'editor';
    uiState.noteFilter = '';
  }

  function openRecentlyDeletedWorkspace(): boolean {
    if ( !recentlyDeletedState.notes.length ) {
      return false;
    }

    uiState.commandOpen = false;
    uiState.tool = 'notes';
    uiState.notesView = 'recently-deleted';

    return true;
  }

  function openSearchWorkspace(
    options: { query?: string; scope?: SearchScope; exactTag?: string | null } = {}
  ): void {
    const replacesSearch = options.query !== undefined
      || options.scope !== undefined
      || options.exactTag !== undefined;
    if ( options.query !== undefined ) {
      searchState.query = options.query;
    }
    if ( options.scope !== undefined ) {
      searchState.scope = options.scope;
    }
    if ( replacesSearch ) {
      searchState.exactTag = options.exactTag ?? null;
    }
    uiState.commandOpen = false;
    uiState.tool = 'search';
    searchState.focusRequest += 1;
  }

  function openQuickSearch( query = '' ): void {
    searchState.quickQuery = query;
    uiState.commandOpen = true;
  }

  function activateNote( id: string ): boolean {
    if ( !noteExists( id ) ) {
      return false;
    }

    const wasVisible = dependencies.isNoteVisible( id );
    vaultState.activeNoteId = id;
    touchRecentNote( id );
    if ( !wasVisible ) {
      vaultState.selectedFolderId = 'all';
      uiState.noteFilter = '';
    }

    return true;
  }

  function touchRecentNote( id: string ): void {
    if ( vaultState.recentNoteIds[ 0 ] === id ) {
      return;
    }

    const recentNoteIds = [
      id,
      ...vaultState.recentNoteIds.filter( ( noteId ) => noteId !== id )
    ].slice( 0, RECENT_NOTE_LIMIT );
    vaultState.recentNoteIds.splice(
      0,
      vaultState.recentNoteIds.length,
      ...recentNoteIds
    );
  }

  function removeRecentNote( id: string ): void {
    vaultState.recentNoteIds = vaultState.recentNoteIds.filter(
      ( noteId ) => noteId !== id
    );
  }

  function recordDirectNoteNavigation(
    previousId: string | null,
    nextId: string
  ): void {
    if ( previousId === nextId || !noteExists( nextId ) ) {
      return;
    }
    if ( previousId && noteExists( previousId ) ) {
      pushNoteNavigationEntry( noteNavigationState.back, previousId );
    }
    noteNavigationState.forward.splice( 0 );
  }

  function traverseNoteNavigation(
    source: string[],
    destination: string[]
  ): boolean {
    while ( source.length ) {
      const targetId = source.pop();
      if (
        !targetId
        || targetId === vaultState.activeNoteId
        || !noteExists( targetId )
      ) {
        continue;
      }

      const previousId = vaultState.activeNoteId;
      if ( !activateNote( targetId ) ) {
        continue;
      }
      if ( previousId && noteExists( previousId ) ) {
        pushNoteNavigationEntry( destination, previousId );
      }

      return true;
    }

    return false;
  }

  function pushNoteNavigationEntry( stack: string[], id: string ): void {
    if ( stack[ stack.length - 1 ] === id ) {
      return;
    }
    stack.push( id );
    if ( stack.length > NOTE_NAVIGATION_LIMIT ) {
      stack.splice( 0, stack.length - NOTE_NAVIGATION_LIMIT );
    }
  }

  function findNoteNavigationTarget( stack: string[]): Note | undefined {
    for ( let index = stack.length - 1; index >= 0; index -= 1 ) {
      const id = stack[ index ];
      if ( id === vaultState.activeNoteId ) {
        continue;
      }
      const note = vaultState.notes.find( ( candidate ) => candidate.id === id );
      if ( note ) {
        return note;
      }
    }

    return undefined;
  }

  function removeNoteFromNavigation( id: string ): void {
    noteNavigationState.back = noteNavigationState.back.filter(
      ( noteId ) => noteId !== id
    );
    noteNavigationState.forward = noteNavigationState.forward.filter(
      ( noteId ) => noteId !== id
    );
  }

  function noteDeletionFallback( id: string ): string | undefined {
    const recentId = vaultState.recentNoteIds.find(
      ( noteId ) => noteId !== id && noteExists( noteId )
    );
    if ( recentId ) {
      return recentId;
    }

    const navigationTarget = findNoteNavigationTarget( noteNavigationState.forward )
      ?? findNoteNavigationTarget( noteNavigationState.back );
    if ( navigationTarget ) {
      return navigationTarget.id;
    }

    const orderedNotes = [ ...vaultState.notes ].sort( compareNotesByLocation );
    const noteIndex = orderedNotes.findIndex( ( note ) => note.id === id );

    return orderedNotes[ noteIndex + 1 ]?.id ?? orderedNotes[ noteIndex - 1 ]?.id;
  }

  function activateNoteAfterDeletion( id: string ): boolean {
    if (
      findNoteNavigationTarget( noteNavigationState.forward )?.id === id
      && traverseNoteNavigation(
        noteNavigationState.forward,
        noteNavigationState.back
      )
    ) {
      return true;
    }
    if (
      findNoteNavigationTarget( noteNavigationState.back )?.id === id
      && traverseNoteNavigation(
        noteNavigationState.back,
        noteNavigationState.forward
      )
    ) {
      return true;
    }

    return activateNote( id );
  }

  function compareNotesByLocation( first: Note, second: Note ): number {
    return dependencies.folderPath( first.folderId ).localeCompare(
      dependencies.folderPath( second.folderId ),
      undefined,
      { sensitivity: 'base', numeric: true }
    ) || first.title.localeCompare(
      second.title,
      undefined,
      { sensitivity: 'base', numeric: true }
    ) || first.id.localeCompare( second.id );
  }

  function pruneNoteNavigation(): void {
    const noteIds = new Set( vaultState.notes.map( ( note ) => note.id ) );
    noteNavigationState.back = noteNavigationState.back.filter(
      ( id ) => noteIds.has( id )
    );
    noteNavigationState.forward = noteNavigationState.forward.filter(
      ( id ) => noteIds.has( id )
    );
  }

  function resetNoteNavigation(): void {
    noteNavigationState.back.splice( 0 );
    noteNavigationState.forward.splice( 0 );
  }

  function snapshotNoteNavigation(): NoteNavigationState {
    return {
      back: [ ...noteNavigationState.back ],
      forward: [ ...noteNavigationState.forward ]
    };
  }

  function restoreNoteNavigation( snapshot: NoteNavigationState ): void {
    noteNavigationState.back.splice(
      0,
      noteNavigationState.back.length,
      ...snapshot.back
    );
    noteNavigationState.forward.splice(
      0,
      noteNavigationState.forward.length,
      ...snapshot.forward
    );
  }

  function noteExists( id: string ): boolean {
    return vaultState.notes.some( ( note ) => note.id === id );
  }

  function currentFolderId(): string | null {
    return vaultState.notes.find(
      ( note ) => note.id === vaultState.activeNoteId
    )?.folderId ?? null;
  }

  function resetSearchState(): void {
    searchState.query = '';
    searchState.scope = 'all';
    searchState.exactTag = null;
    searchState.quickQuery = '';
    searchState.focusRequest += 1;
  }

  return {
    activateNote,
    activateNoteAfterDeletion,
    backNavigationNote,
    canNavigateBack,
    canNavigateForward,
    currentFolderId,
    forwardNavigationNote,
    navigateBack,
    navigateForward,
    noteDeletionFallback,
    noteExists,
    openQuickSearch,
    openRecentlyDeletedWorkspace,
    openSearchWorkspace,
    pruneNoteNavigation,
    recordDirectNoteNavigation,
    removeNoteFromNavigation,
    removeRecentNote,
    resetNoteNavigation,
    resetSearchState,
    restoreNoteNavigation,
    selectFolder,
    selectNote,
    snapshotNoteNavigation,
    touchRecentNote
  };
}

export function isSmartFolderSelection(
  selection: unknown
): selection is SmartFolderSelection {
  return selection === 'all' || selection === 'favorites' || selection === 'recent';
}
