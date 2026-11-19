import { computed, reactive, watch } from 'vue';
import { createId } from './vaultModel';
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

export interface NoteTab {
  id: string;
  noteId: string | null;
  back: string[];
  forward: string[];
  frontmatterVisible: boolean;
}

export interface NoteNavigationState {
  tabs: NoteTab[];
  activeTabId: string;
}

interface VaultNavigationDependencies {
  batchChanges: <T>( mutation: () => T ) => T;
  folderPath: ( id: string | null ) => string;
  isNoteVisible: ( id: string ) => boolean;
}

export function createVaultNavigation(
  dependencies: VaultNavigationDependencies
) {
  const initialTab = newTab( vaultState.activeNoteId );
  const noteTabsState = reactive<NoteNavigationState>({
    tabs: [ initialTab ],
    activeTabId: initialTab.id
  });
  const activeNoteTab = computed( () => noteTabsState.tabs.find(
    ( tab ) => tab.id === noteTabsState.activeTabId
  )! );

  // Existing load/recovery paths still own the persisted active note. Reflect
  // their changes in the active session tab without creating navigation entries.
  watch( () => vaultState.activeNoteId, ( id ) => {
    activeNoteTab.value.noteId = id;
  }, { flush: 'sync' });
  watch( () => uiState.frontmatterVisible, ( visible ) => {
    activeNoteTab.value.frontmatterVisible = visible;
  }, { flush: 'sync' });

  function newTab( noteId: string | null ): NoteTab {
    return { id: createId( 'tab' ), noteId, back: [], forward: [], frontmatterVisible: false };
  }

  function createNoteTab( noteId: string | null = null ): string | undefined {
    if ( noteId !== null && !noteExists( noteId ) ) {
      return undefined;
    }
    const tab = newTab( noteId );
    noteTabsState.tabs.push( tab );
    activateNoteTab( tab.id );

    return tab.id;
  }

  function activateNoteTab( id: string ): boolean {
    const tab = noteTabsState.tabs.find( ( candidate ) => candidate.id === id );
    if ( !tab ) {
      return false;
    }
    noteTabsState.activeTabId = id;
    uiState.frontmatterVisible = tab.frontmatterVisible;
    if ( !tab.noteId || !activateNote( tab.noteId ) ) {
      vaultState.activeNoteId = null;
    }
    uiState.tool = 'notes';
    uiState.notesView = 'editor';

    return true;
  }

  function closeNoteTab( id: string ): void {
    const index = noteTabsState.tabs.findIndex( ( tab ) => tab.id === id );
    if ( index < 0 ) {
      return;
    }
    const wasActive = noteTabsState.activeTabId === id;
    // Switch before removing the active tab so synchronous observers always
    // have a current owner for the active note and its view preferences.
    if ( wasActive ) {
      const neighbor = noteTabsState.tabs[ index - 1 ] ?? noteTabsState.tabs[ index + 1 ];
      if ( neighbor ) {
        activateNoteTab( neighbor.id );
      } else {
        createNoteTab();
      }
    }
    noteTabsState.tabs.splice( index, 1 );
  }

  const backNavigationNote = computed<Note | undefined>( () =>
    findNoteNavigationTarget( activeNoteTab.value.back )
  );

  const forwardNavigationNote = computed<Note | undefined>( () =>
    findNoteNavigationTarget( activeNoteTab.value.forward )
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
      activeNoteTab.value.back,
      activeNoteTab.value.forward
    );
    if ( navigated ) {
      uiState.tool = 'notes';
      uiState.notesView = 'editor';
    }

    return navigated;
  }

  function navigateForward(): boolean {
    const navigated = traverseNoteNavigation(
      activeNoteTab.value.forward,
      activeNoteTab.value.back
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
    dependencies.batchChanges( () => {
      vaultState.activeNoteId = id;
      touchRecentNote( id );
      if ( !wasVisible ) {
        vaultState.selectedFolderId = 'all';
        uiState.noteFilter = '';
      }
    });

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
      pushNoteNavigationEntry( activeNoteTab.value.back, previousId );
    }
    activeNoteTab.value.forward.splice( 0 );
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
    for ( const tab of noteTabsState.tabs ) {
      tab.back = tab.back.filter( ( noteId ) => noteId !== id );
      tab.forward = tab.forward.filter( ( noteId ) => noteId !== id );
      if ( tab.noteId === id ) {
        tab.noteId = null;
      }
    }
  }

  function noteDeletionFallback( id: string ): string | undefined {
    const recentId = vaultState.recentNoteIds.find(
      ( noteId ) => noteId !== id && noteExists( noteId )
    );
    if ( recentId ) {
      return recentId;
    }

    const navigationTarget = findNoteNavigationTarget( activeNoteTab.value.forward )
      ?? findNoteNavigationTarget( activeNoteTab.value.back );
    if ( navigationTarget ) {
      return navigationTarget.id;
    }

    const orderedNotes = [ ...vaultState.notes ].sort( compareNotesByLocation );
    const noteIndex = orderedNotes.findIndex( ( note ) => note.id === id );

    return orderedNotes[ noteIndex + 1 ]?.id ?? orderedNotes[ noteIndex - 1 ]?.id;
  }

  function activateNoteAfterDeletion( id: string ): boolean {
    if (
      findNoteNavigationTarget( activeNoteTab.value.forward )?.id === id
      && traverseNoteNavigation(
        activeNoteTab.value.forward,
        activeNoteTab.value.back
      )
    ) {
      return true;
    }
    if (
      findNoteNavigationTarget( activeNoteTab.value.back )?.id === id
      && traverseNoteNavigation(
        activeNoteTab.value.back,
        activeNoteTab.value.forward
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
    for ( const tab of noteTabsState.tabs ) {
      tab.back = tab.back.filter( ( id ) => noteIds.has( id ) );
      tab.forward = tab.forward.filter( ( id ) => noteIds.has( id ) );
      if ( tab.noteId && !noteIds.has( tab.noteId ) ) {
        tab.noteId = null;
      }
    }
  }

  function resetNoteNavigation(): void {
    const tab = newTab( vaultState.activeNoteId );
    noteTabsState.tabs = [ tab ];
    noteTabsState.activeTabId = tab.id;
    uiState.frontmatterVisible = tab.frontmatterVisible;
  }

  function snapshotNoteNavigation(): NoteNavigationState {
    return {
      activeTabId: noteTabsState.activeTabId,
      tabs: noteTabsState.tabs.map( ( tab ) => ({ ...tab, back: [ ...tab.back ], forward: [ ...tab.forward ] }) )
    };
  }

  function restoreNoteNavigation( snapshot: NoteNavigationState ): void {
    noteTabsState.tabs = snapshot.tabs.map( ( tab ) => ({ ...tab, back: [ ...tab.back ], forward: [ ...tab.forward ] }) );
    noteTabsState.activeTabId = snapshot.activeTabId;
    pruneNoteNavigation();
    vaultState.activeNoteId = activeNoteTab.value.noteId;
    uiState.frontmatterVisible = activeNoteTab.value.frontmatterVisible;
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
    activeNoteTab,
    activateNoteTab,
    closeNoteTab,
    createNoteTab,
    noteTabsState,
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
