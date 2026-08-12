import {
  computed,
  nextTick,
  onBeforeUnmount,
  ref,
  shallowRef,
  watch,
} from "vue";
import { StateEffect, StateField } from "@codemirror/state";
import { Decoration, EditorView } from "@codemirror/view";
import {
  DOCUMENT_SEARCH_MIN_LENGTH,
  findDocumentTextMatches,
  normalizeDocumentSearchQuery,
  stepDocumentSearchIndex,
} from "../lib/documentSearch";
import type { Ref } from "vue";
import type { Extension } from "@codemirror/state";
import type { DecorationSet } from "@codemirror/view";
import type { DocumentSearchDirection, DocumentTextMatch } from "../lib/documentSearch";

const SEARCH_DEBOUNCE_MS = 140;

interface SearchDecorationState {
  activeIndex: number;
  matches: readonly DocumentTextMatch[];
}

const setSearchDecorations = StateEffect.define<SearchDecorationState>();

export const codeMirrorDocumentSearchExtension: Extension = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(decorations, transaction) {
    for (const effect of transaction.effects) {
      if (effect.is(setSearchDecorations)) {
        return createSearchDecorations(effect.value);
      }
    }

    return transaction.docChanged ? decorations.map(transaction.changes) : decorations;
  },
  provide: (field) => EditorView.decorations.from(field),
});

export function useCodeMirrorDocumentSearch(editorView: Ref<EditorView | undefined>) {
  const isOpen = ref(false);
  const isPending = ref(false);
  const query = ref("");
  const searchInput = ref<HTMLInputElement>();
  const matches = shallowRef<readonly DocumentTextMatch[]>([]);
  const activeMatchIndex = ref(-1);
  let searchTimer: number | undefined;

  const normalizedQuery = computed(() => normalizeDocumentSearchQuery(query.value));
  const matchCount = computed(() => matches.value.length);
  const activeMatchNumber = computed(() =>
    activeMatchIndex.value < 0 ? 0 : activeMatchIndex.value + 1
  );
  const statusText = computed(() => {
    if (!query.value.trim()) {
      return `${DOCUMENT_SEARCH_MIN_LENGTH}+ characters`;
    }
    if (!normalizedQuery.value) {
      return `${DOCUMENT_SEARCH_MIN_LENGTH} characters minimum`;
    }
    if (isPending.value) {
      return "Searching…";
    }
    if (!matchCount.value) {
      return "0 matches";
    }

    return `${activeMatchNumber.value} of ${matchCount.value}`;
  });

  watch(query, () => {
    if (isOpen.value) {
      scheduleSearch(true);
    }
  });

  watch(editorView, () => {
    if (isOpen.value && normalizedQuery.value) {
      scheduleSearch(false, 0);
    }
  });

  onBeforeUnmount(cancelScheduledSearch);

  async function openSearch(): Promise<void> {
    isOpen.value = true;
    await nextTick();
    editorView.value?.requestMeasure();
    searchInput.value?.focus();
    searchInput.value?.select();
    scheduleSearch(true, 0);
  }

  async function closeSearch(): Promise<void> {
    isOpen.value = false;
    cancelScheduledSearch();
    clearResults();
    await nextTick();
    editorView.value?.requestMeasure();
    editorView.value?.focus();
  }

  function handleEditorSearchKeydown(event: KeyboardEvent): void {
    if (event.isComposing) {
      return;
    }

    const commandModifier = (event.metaKey || event.ctrlKey) && !event.altKey;
    if (commandModifier && event.key.toLocaleLowerCase() === "f") {
      event.preventDefault();
      event.stopPropagation();
      void openSearch();

      return;
    }
    if (!isOpen.value) {
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      void closeSearch();

      return;
    }
    if (event.key === "F3") {
      event.preventDefault();
      event.stopPropagation();
      moveToMatch(event.shiftKey ? "previous" : "next");
    }
  }

  function handleSearchInputKeydown(event: KeyboardEvent): void {
    const modifiedTab = event.key === "Tab" && (
      event.altKey || event.ctrlKey || event.metaKey
    );
    if (
      event.isComposing ||
      modifiedTab ||
      (event.key !== "Tab" && event.key !== "Enter")
    ) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();
    moveToMatch(event.shiftKey ? "previous" : "next");
  }

  function moveToMatch(direction: DocumentSearchDirection): void {
    activeMatchIndex.value = stepDocumentSearchIndex(
      activeMatchIndex.value,
      matchCount.value,
      direction,
    );
    updateDecorations();
    scrollToActiveMatch();
  }

  function refreshSearch(): void {
    if (isOpen.value && normalizedQuery.value) {
      scheduleSearch(false, 0);
    }
  }

  function scheduleSearch(resetActiveMatch: boolean, delay = SEARCH_DEBOUNCE_MS): void {
    cancelScheduledSearch();
    if (!normalizedQuery.value) {
      clearResults();

      return;
    }

    isPending.value = true;
    if (resetActiveMatch) {
      matches.value = [];
      activeMatchIndex.value = -1;
      updateDecorations();
    }
    searchTimer = window.setTimeout(() => {
      searchTimer = undefined;
      performSearch(resetActiveMatch);
    }, delay);
  }

  function performSearch(resetActiveMatch: boolean): void {
    const view = editorView.value;
    const searchQuery = normalizedQuery.value;
    if (!isOpen.value || !view || !searchQuery) {
      clearResults();

      return;
    }

    matches.value = findDocumentTextMatches(
      view.state.doc.toString(),
      searchQuery,
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
    if (activeMatchIndex.value >= 0) {
      scrollToActiveMatch();
    }
  }

  function scrollToActiveMatch(): void {
    const view = editorView.value;
    const match = matches.value[activeMatchIndex.value];
    if (!view || !match) {
      return;
    }

    view.dispatch({
      effects: EditorView.scrollIntoView(match.from, { y: "center" }),
    });
  }

  function clearResults(): void {
    matches.value = [];
    activeMatchIndex.value = -1;
    isPending.value = false;
    updateDecorations();
  }

  function updateDecorations(): void {
    editorView.value?.dispatch({
      effects: setSearchDecorations.of({
        activeIndex: activeMatchIndex.value,
        matches: matches.value,
      }),
    });
  }

  function cancelScheduledSearch(): void {
    if (searchTimer === undefined) {
      return;
    }

    window.clearTimeout(searchTimer);
    searchTimer = undefined;
  }

  return {
    closeSearch,
    handleEditorSearchKeydown,
    handleSearchInputKeydown,
    isOpen,
    matchCount,
    moveToMatch,
    openSearch,
    query,
    refreshSearch,
    searchInput,
    statusText,
  };
}

function createSearchDecorations(state: SearchDecorationState): DecorationSet {
  return Decoration.set(
    state.matches.flatMap((match, index) => {
      const ranges = [
        Decoration.mark({ class: "document-search-match" }).range(match.from, match.to),
      ];
      if (index === state.activeIndex) {
        ranges.push(
          Decoration.mark({ class: "document-search-active" }).range(match.from, match.to),
        );
      }

      return ranges;
    }),
    true,
  );
}
