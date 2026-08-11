import {
  computed,
  nextTick,
  onBeforeUnmount,
  ref,
  shallowRef,
  watch,
} from "vue";
import {
  DOCUMENT_SEARCH_MIN_LENGTH,
  findDocumentTextMatches,
  normalizeDocumentSearchQuery,
  stepDocumentSearchIndex,
} from "../lib/documentSearch";
import type { Ref } from "vue";
import type { DocumentSearchDirection } from "../lib/documentSearch";

const ACTIVE_HIGHLIGHT_NAME = "document-search-active";
const MATCH_HIGHLIGHT_NAME = "document-search-match";
const SEARCH_DEBOUNCE_MS = 140;
const SEARCH_IGNORED_SELECTOR = [
  ".live-list-control",
  ".live-list-indent",
  ".live-quote-prefix",
  ".live-task-control",
  ".live-task-indent",
].join(", ");

interface UseDocumentSearchOptions {
  content: () => string;
  scrollElement: Ref<HTMLTextAreaElement | undefined>;
  visualLayer: Ref<HTMLElement | undefined>;
}

interface PositionedTextNode {
  from: number;
  node: Text;
  to: number;
}

export function useDocumentSearch(options: UseDocumentSearchOptions) {
  const isOpen = ref(false);
  const isPending = ref(false);
  const query = ref("");
  const searchInput = ref<HTMLInputElement>();
  const matchRanges = shallowRef<Range[]>([]);
  const activeMatchIndex = ref(-1);
  let searchTimer: number | undefined;
  let visualLayerObserver: MutationObserver | undefined;

  const normalizedQuery = computed(() => normalizeDocumentSearchQuery(query.value));
  const matchCount = computed(() => matchRanges.value.length);
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

  watch(options.content, () => {
    if (isOpen.value && normalizedQuery.value) {
      scheduleSearch(false, 0);
    }
  });

  watch(
    options.visualLayer,
    (container) => observeVisualLayer(container),
    { flush: "post" },
  );

  onBeforeUnmount(() => {
    cancelScheduledSearch();
    visualLayerObserver?.disconnect();
    clearHighlightRegistry();
  });

  async function openSearch(): Promise<void> {
    isOpen.value = true;
    await nextTick();
    searchInput.value?.focus();
    searchInput.value?.select();
    scheduleSearch(true, 0);
  }

  async function closeSearch(): Promise<void> {
    isOpen.value = false;
    cancelScheduledSearch();
    clearResults();
    await nextTick();
    options.scrollElement.value?.focus();
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
    updateHighlightRegistry();
    scrollToActiveMatch();
  }

  function scheduleSearch(resetActiveMatch: boolean, delay = SEARCH_DEBOUNCE_MS): void {
    cancelScheduledSearch();
    if (!normalizedQuery.value) {
      clearResults();

      return;
    }

    isPending.value = true;
    if (resetActiveMatch) {
      matchRanges.value = [];
      activeMatchIndex.value = -1;
      clearHighlightRegistry();
    }
    searchTimer = window.setTimeout(() => {
      searchTimer = undefined;
      performSearch(resetActiveMatch);
    }, delay);
  }

  function performSearch(resetActiveMatch: boolean): void {
    const container = options.visualLayer.value;
    const searchQuery = normalizedQuery.value;
    if (!isOpen.value || !container || !searchQuery) {
      clearResults();

      return;
    }

    matchRanges.value = collectSearchRanges(container, searchQuery);
    if (
      resetActiveMatch ||
      activeMatchIndex.value < 0 ||
      activeMatchIndex.value >= matchCount.value
    ) {
      activeMatchIndex.value = matchCount.value ? 0 : -1;
    }
    isPending.value = false;
    updateHighlightRegistry();
    if (activeMatchIndex.value >= 0) {
      scrollToActiveMatch();
    }
  }

  function scrollToActiveMatch(): void {
    const range = matchRanges.value[activeMatchIndex.value];
    const container = options.visualLayer.value;
    const scrollElement = options.scrollElement.value;
    const matchRect = range?.getClientRects()[0];
    if (!range || !container || !scrollElement || !matchRect) {
      return;
    }

    const containerRect = container.getBoundingClientRect();
    const matchCenter = matchRect.top - containerRect.top +
      container.scrollTop + matchRect.height / 2;
    const nextScrollTop = Math.max(0, matchCenter - scrollElement.clientHeight / 2);

    scrollElement.scrollTop = nextScrollTop;
    container.scrollTop = nextScrollTop;
  }

  function clearResults(): void {
    matchRanges.value = [];
    activeMatchIndex.value = -1;
    isPending.value = false;
    clearHighlightRegistry();
  }

  function cancelScheduledSearch(): void {
    if (searchTimer === undefined) {
      return;
    }

    window.clearTimeout(searchTimer);
    searchTimer = undefined;
  }

  function observeVisualLayer(container: HTMLElement | undefined): void {
    visualLayerObserver?.disconnect();
    visualLayerObserver = undefined;
    if (!container) {
      return;
    }

    visualLayerObserver = new MutationObserver(() => {
      if (isOpen.value && normalizedQuery.value) {
        scheduleSearch(false, 0);
      }
    });
    visualLayerObserver.observe(container, {
      characterData: true,
      childList: true,
      subtree: true,
    });
  }

  function updateHighlightRegistry(): void {
    clearHighlightRegistry();
    if (!highlightApiIsAvailable() || !matchRanges.value.length) {
      return;
    }

    const matches = new Highlight();
    matches.priority = 1;
    for (const range of matchRanges.value) {
      matches.add(range);
    }
    CSS.highlights.set(MATCH_HIGHLIGHT_NAME, matches);

    const activeRange = matchRanges.value[activeMatchIndex.value];
    if (activeRange) {
      const activeMatch = new Highlight(activeRange);
      activeMatch.priority = 2;
      CSS.highlights.set(ACTIVE_HIGHLIGHT_NAME, activeMatch);
    }
  }

  function clearHighlightRegistry(): void {
    if (!highlightApiIsAvailable()) {
      return;
    }

    CSS.highlights.delete(ACTIVE_HIGHLIGHT_NAME);
    CSS.highlights.delete(MATCH_HIGHLIGHT_NAME);
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
    searchInput,
    statusText,
  };
}

function collectSearchRanges(container: HTMLElement, query: string): Range[] {
  const ranges: Range[] = [];

  for (const unit of searchableUnits(container)) {
    const nodes = positionedTextNodes(unit);
    const text = nodes.map(({ node }) => node.data).join("");

    for (const match of findDocumentTextMatches(text, query)) {
      const range = rangeForMatch(nodes, match.from, match.to);
      if (range) {
        ranges.push(range);
      }
    }
  }

  return ranges;
}

function searchableUnits(container: HTMLElement): HTMLElement[] {
  const units: HTMLElement[] = [];
  const blocks = container.querySelectorAll<HTMLElement>(".live-markdown-block");

  for (const block of blocks) {
    const tableCells = block.querySelectorAll<HTMLElement>(
      ".live-table-block th, .live-table-block td",
    );
    if (tableCells.length) {
      units.push(...tableCells);

      continue;
    }

    const codeBody = block.querySelector<HTMLElement>(".live-code-body");
    if (codeBody) {
      units.push(codeBody);

      continue;
    }

    const content = Array.from(block.children).find((element) =>
      element.classList.contains("live-markdown-content")
    );
    units.push(content instanceof HTMLElement ? content : block);
  }

  return units;
}

function positionedTextNodes(container: HTMLElement): PositionedTextNode[] {
  const nodes: PositionedTextNode[] = [];
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
  let offset = 0;
  let current = walker.nextNode();

  while (current) {
    if (current instanceof Text && !textNodeIsIgnored(current, container)) {
      nodes.push({
        from: offset,
        node: current,
        to: offset + current.data.length,
      });
      offset += current.data.length;
    }
    current = walker.nextNode();
  }

  return nodes;
}

function textNodeIsIgnored(node: Text, boundary: HTMLElement): boolean {
  let element = node.parentElement;

  while (element) {
    if (element.matches(SEARCH_IGNORED_SELECTOR)) {
      return true;
    }
    if (element === boundary) {
      return false;
    }
    element = element.parentElement;
  }

  return false;
}

function rangeForMatch(
  nodes: PositionedTextNode[],
  from: number,
  to: number,
): Range | undefined {
  const first = nodes.find((entry) => from >= entry.from && from < entry.to);
  const last = nodes.find((entry) => to > entry.from && to <= entry.to);
  if (!first || !last) {
    return undefined;
  }

  const range = document.createRange();
  range.setStart(first.node, from - first.from);
  range.setEnd(last.node, to - last.from);

  return range;
}

function highlightApiIsAvailable(): boolean {
  return typeof CSS !== "undefined" &&
    "highlights" in CSS &&
    typeof Highlight !== "undefined";
}
