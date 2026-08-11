import {
  onBeforeUnmount,
  onMounted,
  shallowRef,
  watch,
} from "vue";
import type { Ref } from "vue";

export interface LiveMarkdownRegion {
  lineNumbers: readonly number[];
}

interface LiveMarkdownRegionLayoutOptions<Region extends LiveMarkdownRegion> {
  blockSelector: string;
  container: Readonly<Ref<HTMLElement | undefined>>;
  regions: Readonly<Ref<readonly Region[]>>;
}

export function useLiveMarkdownRegionLayout<Region extends LiveMarkdownRegion>(
  options: LiveMarkdownRegionLayoutOptions<Region>,
): {
  heightForRegion: (region: Region) => number | undefined;
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
    options.regions,
    () => {
      syncObservedBlocks();
      scheduleMeasurement();
    },
    { flush: "post" },
  );

  function heightForRegion(region: Region): number | undefined {
    const openingLine = region.lineNumbers[0];

    return openingLine === undefined
      ? undefined
      : heights.value.get(openingLine);
  }

  function syncObservedBlocks(): void {
    if (!resizeObserver || !options.container.value) {
      return;
    }

    const currentBlocks = new Set(
      options.container.value.querySelectorAll(options.blockSelector),
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
      options.blockSelector,
    )) {
      const lineNumber = Number.parseInt(element.dataset.lineNumber ?? "", 10);
      if (Number.isInteger(lineNumber)) {
        blocks.set(lineNumber, element);
      }
    }

    const nextHeights = new Map<number, number>();
    for (const region of options.regions.value) {
      const openingLine = region.lineNumbers[0];
      const closingLine = region.lineNumbers.at(-1);
      if (openingLine === undefined || closingLine === undefined) {
        continue;
      }

      const opening = blocks.get(openingLine);
      const closing = blocks.get(closingLine);
      if (!opening || !closing) {
        continue;
      }

      const height = closing.offsetTop + closing.offsetHeight - opening.offsetTop;
      if (height > 0) {
        nextHeights.set(openingLine, height);
      }
    }

    if (!sameHeights(heights.value, nextHeights)) {
      heights.value = nextHeights;
    }
  }

  return { heightForRegion };
}

function sameHeights(
  left: ReadonlyMap<number, number>,
  right: ReadonlyMap<number, number>,
): boolean {
  return left.size === right.size &&
    [...left].every(([lineNumber, height]) => right.get(lineNumber) === height);
}
