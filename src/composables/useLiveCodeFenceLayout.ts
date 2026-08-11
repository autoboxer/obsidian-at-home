import {
  onBeforeUnmount,
  onMounted,
  shallowRef,
  watch,
} from "vue";
import type { Ref } from "vue";
import type { LiveMarkdownCodeFence } from "../lib/liveMarkdownCode";

interface LiveCodeFenceLayoutOptions {
  container: Readonly<Ref<HTMLElement | undefined>>;
  fences: Readonly<Ref<readonly LiveMarkdownCodeFence[]>>;
}

const LIVE_CODE_BLOCK_SELECTOR = [
  ".live-markdown-block.is-code-opening",
  ".live-markdown-block.is-code-content",
  ".live-markdown-block.is-code-closing",
].join(", ");

export function useLiveCodeFenceLayout(
  options: LiveCodeFenceLayoutOptions,
): {
  heightForFence: (fence: LiveMarkdownCodeFence) => number | undefined;
} {
  const heights = shallowRef(new Map<number, number>());
  const observedBlocks = new Set<Element>();
  let resizeObserver: ResizeObserver | undefined;
  let measurementFrame: number | undefined;

  onMounted(() => {
    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(scheduleMeasurement);
      if (options.container.value) {
        resizeObserver.observe(options.container.value);
      }
      syncObservedBlocks();
    }

    scheduleMeasurement();
  });

  onBeforeUnmount(() => {
    resizeObserver?.disconnect();
    if (measurementFrame !== undefined) {
      window.cancelAnimationFrame(measurementFrame);
    }
  });

  watch(
    options.fences,
    () => {
      syncObservedBlocks();
      scheduleMeasurement();
    },
    { flush: "post" },
  );

  function heightForFence(
    fence: LiveMarkdownCodeFence,
  ): number | undefined {
    return heights.value.get(fence.openingLine);
  }

  function syncObservedBlocks(): void {
    if (!resizeObserver || !options.container.value) {
      return;
    }

    const currentBlocks = new Set(
      options.container.value.querySelectorAll(LIVE_CODE_BLOCK_SELECTOR),
    );
    for (const block of observedBlocks) {
      if (!currentBlocks.has(block)) {
        resizeObserver.unobserve(block);
        observedBlocks.delete(block);
      }
    }
    for (const block of currentBlocks) {
      if (!observedBlocks.has(block)) {
        resizeObserver.observe(block);
        observedBlocks.add(block);
      }
    }
  }

  function scheduleMeasurement(): void {
    if (measurementFrame !== undefined) {
      return;
    }

    measurementFrame = window.requestAnimationFrame(() => {
      measurementFrame = undefined;
      measureHeights();
    });
  }

  function measureHeights(): void {
    if (!options.container.value) {
      return;
    }

    const blocks = new Map<number, HTMLElement>();
    for (const element of options.container.value.querySelectorAll<HTMLElement>(
      LIVE_CODE_BLOCK_SELECTOR,
    )) {
      const lineNumber = Number.parseInt(element.dataset.lineNumber ?? "", 10);
      if (Number.isInteger(lineNumber)) {
        blocks.set(lineNumber, element);
      }
    }

    const nextHeights = new Map<number, number>();
    for (const fence of options.fences.value) {
      const opening = blocks.get(fence.openingLine);
      const closing = blocks.get(fence.lineNumbers.at(-1) ?? fence.openingLine);
      if (!opening || !closing) {
        continue;
      }

      const height = closing.offsetTop + closing.offsetHeight - opening.offsetTop;
      if (height > 0) {
        nextHeights.set(fence.openingLine, height);
      }
    }

    if (!sameHeights(heights.value, nextHeights)) {
      heights.value = nextHeights;
    }
  }

  return { heightForFence };
}

function sameHeights(
  left: ReadonlyMap<number, number>,
  right: ReadonlyMap<number, number>,
): boolean {
  return left.size === right.size &&
    [...left].every(([lineNumber, height]) => right.get(lineNumber) === height);
}
