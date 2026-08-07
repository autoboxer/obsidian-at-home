import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { EditorMode } from "../types";

type ScrollPane = "source" | "preview";

interface SourceEditorScrollApi {
  getScrollElement: () => HTMLTextAreaElement | undefined;
}

interface SplitScrollSyncOptions {
  mode: () => EditorMode;
  noteId: () => string | undefined;
  content: () => string | undefined;
}

const PANE_RESIZE_SETTLE_MS = 240;

export function useSplitScrollSync(options: SplitScrollSyncOptions) {
  const editorCanvas = ref<HTMLElement>();
  const sourceEditor = ref<SourceEditorScrollApi>();
  const previewPane = ref<HTMLElement>();

  let lastScrollSource: ScrollPane = "source";
  let queuedScrollSource: ScrollPane | undefined;
  let scrollSyncFrame: number | undefined;
  // Direct scrollTop assignments also emit scroll events. Track their expected
  // landing points so those events cannot feed back into the opposite pane.
  let expectedScrollTops = new WeakMap<HTMLElement, number>();
  let resizeObserver: ResizeObserver | undefined;
  let preservedModeProgress: number | undefined;
  let modeScrollTimer: number | undefined;

  function handleSourceScroll(element: HTMLTextAreaElement): void {
    if (consumeExpectedScroll(element)) {
      return;
    }

    cancelModeScrollPreservation();
    if (options.mode() !== "split") {
      return;
    }

    lastScrollSource = "source";
    queueScrollSync("source");
  }

  function handlePreviewScroll(event: Event): void {
    const element = event.currentTarget as HTMLElement;
    if (consumeExpectedScroll(element)) {
      return;
    }

    cancelModeScrollPreservation();
    if (options.mode() !== "split") {
      return;
    }

    lastScrollSource = "preview";
    queueScrollSync("preview");
  }

  function claimScrollPane(pane: ScrollPane): void {
    cancelModeScrollPreservation();
    if (options.mode() === "split") {
      lastScrollSource = pane;
    }
  }

  function consumeExpectedScroll(element: HTMLElement): boolean {
    const expected = expectedScrollTops.get(element);
    expectedScrollTops.delete(element);

    if (preservedModeProgress !== undefined) {
      return true;
    }

    return expected !== undefined && Math.abs(element.scrollTop - expected) <= 1;
  }

  function queueScrollSync(source: ScrollPane): void {
    if (options.mode() !== "split") {
      return;
    }

    queuedScrollSource = visibleScrollSource(source);
    lastScrollSource = queuedScrollSource;
    if (scrollSyncFrame !== undefined) {
      return;
    }

    scrollSyncFrame = window.requestAnimationFrame(() => {
      scrollSyncFrame = undefined;

      const latestSource = queuedScrollSource;
      queuedScrollSource = undefined;
      if (latestSource && options.mode() === "split") {
        synchronizeScroll(latestSource);
      }
    });
  }

  function visibleScrollSource(preferred: ScrollPane): ScrollPane {
    const sourceElement = sourceEditor.value?.getScrollElement();
    const previewElement = previewPane.value;
    const preferredElement = preferred === "source" ? sourceElement : previewElement;
    const fallback = preferred === "source" ? "preview" : "source";
    const fallbackElement = fallback === "source" ? sourceElement : previewElement;

    if (preferredElement && preferredElement.clientWidth > 0) {
      return preferred;
    }
    if (fallbackElement && fallbackElement.clientWidth > 0) {
      return fallback;
    }

    return preferred;
  }

  function synchronizeScroll(source: ScrollPane): void {
    const sourceElement = sourceEditor.value?.getScrollElement();
    const previewElement = previewPane.value;
    if (!sourceElement || !previewElement) {
      return;
    }

    const origin = source === "source" ? sourceElement : previewElement;
    const destination = source === "source" ? previewElement : sourceElement;
    if (
      origin.clientWidth <= 0 ||
      origin.clientHeight <= 0 ||
      destination.clientWidth <= 0 ||
      destination.clientHeight <= 0
    ) {
      return;
    }

    const originRange = scrollRange(origin);
    const destinationRange = scrollRange(destination);
    // A non-scrollable pane has no meaningful progress to impose on a longer one.
    if (originRange <= 0 && destinationRange > 0) {
      return;
    }

    const progress = originRange > 0
      ? Math.min(1, Math.max(0, origin.scrollTop / originRange))
      : 0;

    setScrollTop(destination, progress * destinationRange);
  }

  function scrollRange(element: HTMLElement): number {
    return Math.max(0, element.scrollHeight - element.clientHeight);
  }

  function setScrollTop(element: HTMLElement, scrollTop: number): void {
    if (Math.abs(element.scrollTop - scrollTop) <= 0.5) {
      return;
    }

    expectedScrollTops.set(element, scrollTop);
    element.scrollTop = scrollTop;
    expectedScrollTops.set(element, element.scrollTop);
  }

  function getScrollProgress(pane: ScrollPane): number | undefined {
    const element = pane === "source"
      ? sourceEditor.value?.getScrollElement()
      : previewPane.value;
    if (!element || element.clientWidth <= 0 || element.clientHeight <= 0) {
      return undefined;
    }

    const range = scrollRange(element);

    return range > 0
      ? Math.min(1, Math.max(0, element.scrollTop / range))
      : 0;
  }

  function preserveScrollThroughModeChange(progress: number): void {
    cancelModeScrollPreservation();
    preservedModeProgress = progress;
    restoreModeScroll(progress);

    // The pane width transition re-wraps both documents for 190 ms. Keep the
    // normalized position stable until their final scroll ranges have settled.
    modeScrollTimer = window.setTimeout(() => {
      const finalProgress = preservedModeProgress;
      preservedModeProgress = undefined;
      modeScrollTimer = undefined;
      if (finalProgress === undefined) {
        return;
      }

      restoreModeScroll(finalProgress);
      queueScrollSync(lastScrollSource);
    }, PANE_RESIZE_SETTLE_MS);
  }

  function restoreModeScroll(progress: number): void {
    const sourceElement = sourceEditor.value?.getScrollElement();
    const previewElement = previewPane.value;
    const mode = options.mode();
    const visiblePanes: HTMLElement[] = [];

    if (sourceElement && mode !== "reading") {
      visiblePanes.push(sourceElement);
    }
    if (previewElement && mode !== "source") {
      visiblePanes.push(previewElement);
    }

    for (const element of visiblePanes) {
      if (element.clientWidth > 0 && element.clientHeight > 0) {
        setScrollTop(element, progress * scrollRange(element));
      }
    }
  }

  function cancelModeScrollPreservation(): void {
    preservedModeProgress = undefined;
    if (modeScrollTimer !== undefined) {
      window.clearTimeout(modeScrollTimer);
      modeScrollTimer = undefined;
    }
  }

  function resetScrollPositions(): void {
    cancelQueuedScrollSync();
    cancelModeScrollPreservation();
    expectedScrollTops = new WeakMap();
    lastScrollSource = "source";

    const sourceElement = sourceEditor.value?.getScrollElement();
    if (sourceElement) {
      setScrollTop(sourceElement, 0);
    }
    if (previewPane.value) {
      setScrollTop(previewPane.value, 0);
    }
  }

  function cancelQueuedScrollSync(): void {
    queuedScrollSource = undefined;
    if (scrollSyncFrame !== undefined) {
      window.cancelAnimationFrame(scrollSyncFrame);
      scrollSyncFrame = undefined;
    }
  }

  function refreshObservers(): void {
    if (!resizeObserver) {
      return;
    }

    resizeObserver.disconnect();

    const sourceElement = sourceEditor.value?.getScrollElement();
    const previewElement = previewPane.value;
    const previewContent = previewElement?.querySelector<HTMLElement>(".markdown-preview");
    for (const element of [editorCanvas.value, sourceElement, previewElement, previewContent]) {
      if (element) {
        resizeObserver.observe(element);
      }
    }
  }

  watch(
    options.mode,
    async (_mode, previousMode) => {
      let previousPane: ScrollPane = "source";
      if (previousMode === "split") {
        previousPane = lastScrollSource;
      } else if (previousMode === "reading") {
        previousPane = "preview";
      }
      const previousProgress = getScrollProgress(previousPane);

      cancelQueuedScrollSync();
      cancelModeScrollPreservation();
      expectedScrollTops = new WeakMap();

      if (previousMode === "reading") {
        lastScrollSource = "preview";
      } else if (previousMode === "source") {
        lastScrollSource = "source";
      }

      await nextTick();
      refreshObservers();
      if (previousProgress === undefined) {
        queueScrollSync(lastScrollSource);
      } else {
        preserveScrollThroughModeChange(previousProgress);
      }
    },
  );

  watch(
    () => [options.noteId(), options.content()] as const,
    async ([noteId], [previousNoteId]) => {
      await nextTick();

      if (noteId !== previousNoteId) {
        refreshObservers();
        resetScrollPositions();

        return;
      }

      queueScrollSync(lastScrollSource);
    },
    { flush: "post" },
  );

  onMounted(() => {
    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(() => {
        if (preservedModeProgress === undefined) {
          queueScrollSync(lastScrollSource);
        } else {
          restoreModeScroll(preservedModeProgress);
        }
      });
    }

    nextTick(() => {
      refreshObservers();
      queueScrollSync(lastScrollSource);
    });
  });

  onBeforeUnmount(() => {
    cancelQueuedScrollSync();
    cancelModeScrollPreservation();
    resizeObserver?.disconnect();
    resizeObserver = undefined;
  });

  return {
    editorCanvas,
    sourceEditor,
    previewPane,
    handleSourceScroll,
    handlePreviewScroll,
    claimScrollPane,
  };
}
