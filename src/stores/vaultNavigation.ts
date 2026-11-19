import { computed, reactive, watch } from 'vue';
import { createId } from './vaultModel';
import type { Note, SearchScope, VaultImageFile } from '../types';
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

// Note IDs remain the persisted document identity. Image targets and mixed
// navigation histories belong only to the current tab session.
type TabNavigationTarget = string | VaultImageFile;

export interface NoteTab {
  id: string;
  noteId: string | null;
  image: VaultImageFile | null;
  back: TabNavigationTarget[];
  forward: TabNavigationTarget[];
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
    if ( id ) {
      activeNoteTab.value.image = null;
    }
  }, { flush: 'sync' });
  watch( () => uiState.frontmatterVisible, ( visible ) => {
    activeNoteTab.value.frontmatterVisible = visible;
  }, { flush: 'sync' });

  function newTab( noteId: string | null ): NoteTab {
    return { id: createId( 'tab' ), noteId, image: null, back: [], forward: [], frontmatterVisible: false };
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
    if ( tab.image ) {
      activateImage( tab.image );
    } else if ( !tab.noteId || !activateNote( tab.noteId ) ) {
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

  const activeImage = computed( () => activeNoteTab.value.image
    ? resolveImageTarget( activeNoteTab.value.image ) : undefined );
  const backNavigationTarget = computed( () => findNavigationTarget( activeNoteTab.value.back ) );
  const forwardNavigationTarget = computed( () => findNavigationTarget( activeNoteTab.value.forward ) );
  const backNavigationNote = computed( () => noteForTarget( backNavigationTarget.value ) );
  const forwardNavigationNote = computed( () => noteForTarget( forwardNavigationTarget.value ) );
  const backNavigationTitle = computed( () => navigationTitle( backNavigationTarget.value ) );
  const forwardNavigationTitle = computed( () => navigationTitle( forwardNavigationTarget.value ) );
  const canNavigateBack = computed( () => Boolean( backNavigationTarget.value ) );
  const canNavigateForward = computed( () => Boolean( forwardNavigationTarget.value ) );

  function findImage( target: VaultImageFile ): VaultImageFile | undefined {
    return vaultState.imageFiles.find( ( image ) => target.assetId
      ? image.assetId === target.assetId : image.relativePath === target.relativePath );
  }

  function resolveImageTarget( target: VaultImageFile ): VaultImageFile {
    return findImage( target ) ?? target;
  }

  function currentTarget(): TabNavigationTarget | null {
    return activeNoteTab.value.image ?? vaultState.activeNoteId;
  }

  function targetExists( target: TabNavigationTarget ): boolean {
    return typeof target === 'string' ? noteExists( target ) : Boolean( findImage( target ) );
  }

  function sameTarget( first: TabNavigationTarget | null | undefined, second: TabNavigationTarget | null ): boolean {
    if ( typeof first === 'string' || typeof second === 'string' || !first || !second ) {
      return first === second;
    }

    return first.assetId && second.assetId
      ? first.assetId === second.assetId : first.relativePath === second.relativePath;
  }

  function copyTarget( target: TabNavigationTarget ): TabNavigationTarget {
    return typeof target === 'string' ? target : { ...target };
  }

  function noteForTarget( target: TabNavigationTarget | undefined ): Note | undefined {
    return typeof target === 'string' ? vaultState.notes.find( ( note ) => note.id === target ) : undefined;
  }

  function navigationTitle( target: TabNavigationTarget | undefined ): string | undefined {
    if ( !target ) {
      return undefined;
    }

    return typeof target === 'string'
      ? noteForTarget( target )?.title.trim() || 'Untitled note'
      : resolveImageTarget( target ).relativePath.split( '/' ).at( -1 );
  }

  function activateImage( image: VaultImageFile ): void {
    activeNoteTab.value.image = { ...resolveImageTarget( image ) };
    activeNoteTab.value.noteId = null;
    vaultState.activeNoteId = null;
  }

  function selectVaultImage( image: VaultImageFile ): void {
    const target = findImage( image );
    if ( !target ) {
      return;
    }
    const previous = currentTarget();
    if ( !sameTarget( previous, target ) ) {
      if ( previous && targetExists( previous ) ) {
        pushNavigationEntry( activeNoteTab.value.back, previous );
      }
      activeNoteTab.value.forward.splice( 0 );
    }
    activateImage( target );
    uiState.tool = 'notes';
    uiState.notesView = 'editor';
  }

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
      activeNoteTab.value.image = null;
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
    const previous = activeNoteTab.value.image ?? previousId;
    if ( sameTarget( previous, nextId ) || !noteExists( nextId ) ) {
      return;
    }
    if ( previous && targetExists( previous ) ) {
      pushNavigationEntry( activeNoteTab.value.back, previous );
    }
    activeNoteTab.value.forward.splice( 0 );
  }

  function traverseNoteNavigation(
    source: TabNavigationTarget[],
    destination: TabNavigationTarget[]
  ): boolean {
    while ( source.length ) {
      const target = source.pop();
      if ( !target || sameTarget( target, currentTarget() ) || !targetExists( target ) ) {
        continue;
      }
      const previous = currentTarget();
      if ( typeof target === 'string' ) {
        activateNote( target );
      } else {
        activateImage( target );
      }
      if ( previous && targetExists( previous ) ) {
        pushNavigationEntry( destination, previous );
      }

      return true;
    }

    return false;
  }

  function pushNavigationEntry( stack: TabNavigationTarget[], target: TabNavigationTarget ): void {
    if ( sameTarget( stack[ stack.length - 1 ], target ) ) {
      return;
    }
    stack.push( copyTarget( target ) );
    if ( stack.length > NOTE_NAVIGATION_LIMIT ) {
      stack.splice( 0, stack.length - NOTE_NAVIGATION_LIMIT );
    }
  }

  function findNavigationTarget( stack: TabNavigationTarget[]): TabNavigationTarget | undefined {
    for ( let index = stack.length - 1; index >= 0; index -= 1 ) {
      const target = stack[ index ]!;
      if ( !sameTarget( target, currentTarget() ) && targetExists( target ) ) {
        return target;
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

    const navigationTarget = forwardNavigationNote.value
      ?? backNavigationNote.value;
    if ( navigationTarget ) {
      return navigationTarget.id;
    }

    const orderedNotes = [ ...vaultState.notes ].sort( compareNotesByLocation );
    const noteIndex = orderedNotes.findIndex( ( note ) => note.id === id );

    return orderedNotes[ noteIndex + 1 ]?.id ?? orderedNotes[ noteIndex - 1 ]?.id;
  }

  function activateNoteAfterDeletion( id: string ): boolean {
    if (
      forwardNavigationNote.value?.id === id
      && traverseNoteNavigation(
        activeNoteTab.value.forward,
        activeNoteTab.value.back
      )
    ) {
      return true;
    }
    if (
      backNavigationNote.value?.id === id
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
      tab.back = tab.back.filter( ( target ) => typeof target === 'string' ? noteIds.has( target ) : targetExists( target ) );
      tab.forward = tab.forward.filter( ( target ) => typeof target === 'string' ? noteIds.has( target ) : targetExists( target ) );
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
      tabs: noteTabsState.tabs.map( ( tab ) => ({ ...tab, image: tab.image ? { ...tab.image } : null, back: tab.back.map( copyTarget ), forward: tab.forward.map( copyTarget ) }) )
    };
  }

  function restoreNoteNavigation( snapshot: NoteNavigationState ): void {
    noteTabsState.tabs = snapshot.tabs.map( ( tab ) => ({ ...tab, image: tab.image ? { ...tab.image } : null, back: tab.back.map( copyTarget ), forward: tab.forward.map( copyTarget ) }) );
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
    activeImage,
    backNavigationTitle,
    forwardNavigationTitle,
    resolveImageTarget,
    selectVaultImage,
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
