import { computed, nextTick, onBeforeUnmount, ref, shallowRef, watch } from 'vue';
import { EditorSelection } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { codeMirrorDocumentSearchExtension, setDocumentSearchMatches } from '../lib/codeMirrorDocumentSearch';
import {
  DOCUMENT_SEARCH_MIN_LENGTH,
  findDocumentTextMatches,
  normalizeDocumentSearchQuery,
  stepDocumentSearchIndex
} from '../lib/documentSearch';
import type { Ref } from 'vue';
import type { DocumentSearchDirection, DocumentTextMatch } from '../lib/documentSearch';

// coalesce typing before rebuilding match decorations across the document
const SEARCH_DEBOUNCE_MS = 140;

export { codeMirrorDocumentSearchExtension };

export function useCodeMirrorDocumentSearch( editorView: Ref<EditorView | undefined> ) {
  const isOpen = ref( false );
  const isPending = ref( false );
  const query = ref( '' );
  const searchInput = ref<HTMLInputElement>();
  const matches = shallowRef<readonly DocumentTextMatch[]>([]);
  const activeMatchIndex = ref( -1 );
  let searchTimer: number | undefined;

  const normalizedQuery = computed( () => normalizeDocumentSearchQuery( query.value ) );
  const matchCount = computed( () => matches.value.length );

  const activeMatchNumber = computed( () =>
    activeMatchIndex.value < 0 ? 0 : activeMatchIndex.value + 1
  );

  const statusText = computed( () => {
    if ( !query.value.trim() ) {
      return `${ DOCUMENT_SEARCH_MIN_LENGTH }+ characters`;
    }

    if ( !normalizedQuery.value ) {
      return `${ DOCUMENT_SEARCH_MIN_LENGTH } characters minimum`;
    }

    if ( isPending.value ) {
      return 'Searching…';
    }

    if ( !matchCount.value ) {
      return '0 matches';
    }

    return `${ activeMatchNumber.value } of ${ matchCount.value }`;
  });

  watch( query, () => {
    if ( isOpen.value ) {
      scheduleSearch( true );
    }
  }, { flush: 'sync' });

  watch( editorView, () => {
    if ( isOpen.value && normalizedQuery.value ) {
      scheduleSearch( false, 0 );
    }
  });

  onBeforeUnmount( cancelScheduledSearch );

  async function openSearch(): Promise<void> {
    isOpen.value = true;
    const view = editorView.value;

    if ( view && view.state.selection.ranges.length > 1 ) {
      view.dispatch({
        selection: EditorSelection.create([ view.state.selection.main ]),
        userEvent: 'select.search'
      });
    }

    await nextTick();
    editorView.value?.requestMeasure();
    searchInput.value?.focus();
    searchInput.value?.select();
    scheduleSearch( true, 0 );
  }

  async function closeSearch( focusEditor = true ): Promise<void> {
    const view = editorView.value;

    isOpen.value = false;
    cancelScheduledSearch();
    resetResults();
    await nextTick();

    if ( !view || view !== editorView.value || isOpen.value ) {
      return;
    }

    view.requestMeasure();
    view.dispatch({
      effects: setDocumentSearchMatches.of({
        activeIndex: activeMatchIndex.value,
        matches: matches.value
      })
    });

    if ( focusEditor ) {
      view.focus();
    }
  }

  function acceptSearch(): void {
    if ( !isOpen.value ) {
      return;
    }

    if ( isPending.value ) {
      cancelScheduledSearch();
      performSearch( false );
    }

    selectActiveMatch();
    void closeSearch();
  }

  function handleEditorClick(): void {
    if ( isOpen.value ) {
      // let the completed pointer gesture keep its selection and focus,
      // including clicks on controls embedded in the document
      void closeSearch( false );
    }
  }

  function handleEditorSearchKeydown( event: KeyboardEvent ): void {
    if ( event.isComposing ) {
      return;
    }

    const commandModifier = ( event.metaKey || event.ctrlKey ) && !event.altKey;

    if ( commandModifier && !event.shiftKey && event.key.toLocaleLowerCase() === 'f' ) {
      event.preventDefault();
      event.stopPropagation();
      void openSearch();

      return;
    }

    if ( !isOpen.value ) {
      return;
    }

    if ( event.key === 'Escape' ) {
      event.preventDefault();
      event.stopPropagation();
      void closeSearch();

      return;
    }

    if ( event.key === 'F3' ) {
      event.preventDefault();
      event.stopPropagation();
      moveToMatch( event.shiftKey ? 'previous' : 'next' );
    }
  }

  function handleSearchInputKeydown( event: KeyboardEvent ): void {
    // account for WebKit reporting Shift+Tab as an unidentified key on Linux
    const key = event.key === 'Unidentified' && event.code === 'Tab'
      ? 'Tab'
      : event.key;

    const modifiedTab = key === 'Tab' && (
      event.altKey || event.ctrlKey || event.metaKey
    );

    if (
      event.isComposing ||
      modifiedTab ||
      ( key !== 'Tab' && key !== 'Enter' )
    ) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();

    if ( key === 'Enter' && !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey ) {
      acceptSearch();

      return;
    }

    moveToMatch( event.shiftKey ? 'previous' : 'next' );
  }

  function moveToMatch( direction: DocumentSearchDirection ): void {
    // step from the prior index, before refreshing initializes a pending query
    const currentIndex = activeMatchIndex.value;

    if ( isPending.value ) {
      cancelScheduledSearch();
      performSearch( false );
    }

    activeMatchIndex.value = stepDocumentSearchIndex(
      currentIndex,
      matchCount.value,
      direction
    );
    updateDecorations();
    selectActiveMatch();
  }

  function refreshSearch(): void {
    if ( isOpen.value && normalizedQuery.value ) {
      scheduleSearch( false, 0 );
    }
  }

  function scheduleSearch( resetActiveMatch: boolean, delay = SEARCH_DEBOUNCE_MS ): void {
    cancelScheduledSearch();

    if ( !normalizedQuery.value ) {
      clearResults();

      return;
    }

    isPending.value = true;

    if ( resetActiveMatch ) {
      matches.value = [];
      activeMatchIndex.value = -1;
      updateDecorations();
    }

    searchTimer = window.setTimeout( () => {
      searchTimer = undefined;
      performSearch( resetActiveMatch );
    }, delay );
  }

  function performSearch( resetActiveMatch: boolean ): void {
    const view = editorView.value;
    const searchQuery = normalizedQuery.value;

    if ( !isOpen.value || !view || !searchQuery ) {
      clearResults();

      return;
    }

    matches.value = findDocumentTextMatches(
      view.state.doc.toString(),
      searchQuery
    );

    if (
      resetActiveMatch ||
      activeMatchIndex.value < 0 ||
      activeMatchIndex.value >= matchCount.value
    ) {
      activeMatchIndex.value = matchCount.value ? 0 : -1;
    }

    isPending.value = false;
    updateDecorations();

    // refreshing highlights after an edit must not move the user's caret or
    // viewport. A query change may navigate only while its input still has focus
    if ( resetActiveMatch && searchInput.value === document.activeElement ) {
      selectActiveMatch();
    }
  }

  function selectActiveMatch(): void {
    const view = editorView.value;
    const match = matches.value[ activeMatchIndex.value ];

    if ( !view || !match ) {
      return;
    }

    view.dispatch({
      selection: EditorSelection.single( match.from, match.to ),
      effects: EditorView.scrollIntoView( match.from, { y: 'center' }),
      userEvent: 'select.search'
    });
  }

  function clearResults(): void {
    resetResults();
    updateDecorations();
  }

  function resetResults(): void {
    matches.value = [];
    activeMatchIndex.value = -1;
    isPending.value = false;
  }

  function updateDecorations(): void {
    editorView.value?.dispatch({
      effects: setDocumentSearchMatches.of({
        activeIndex: activeMatchIndex.value,
        matches: matches.value
      })
    });
  }

  function cancelScheduledSearch(): void {
    if ( searchTimer === undefined ) {
      return;
    }

    window.clearTimeout( searchTimer );
    searchTimer = undefined;
  }

  return {
    acceptSearch,
    closeSearch,
    handleEditorClick,
    handleEditorSearchKeydown,
    handleSearchInputKeydown,
    isOpen,
    matchCount,
    moveToMatch,
    openSearch,
    query,
    refreshSearch,
    searchInput,
    statusText
  };
}
