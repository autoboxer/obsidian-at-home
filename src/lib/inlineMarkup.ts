export type InlineMarkupKind = "emphasis" | "strikethrough" | "strong";

export type InlineMarkupMarker = "*" | "**" | "_" | "__" | "~~";

export interface InlineMarkupRange {
  from: number;
  to: number;
}

export interface PairedInlineMarkup extends InlineMarkupRange {
  contentFrom: number;
  contentTo: number;
  kind: InlineMarkupKind;
  marker: InlineMarkupMarker;
}

interface InlineMarkupDelimiterRoles {
  canClose: boolean;
  canOpen: boolean;
}

const MARKER_KINDS: Readonly<Record<InlineMarkupMarker, InlineMarkupKind>> = {
  "*": "emphasis",
  "**": "strong",
  _: "emphasis",
  __: "strong",
  "~~": "strikethrough",
};

export function parsePairedInlineMarkup(
  value: string,
  excludedRanges: readonly InlineMarkupRange[] = [],
): PairedInlineMarkup[] {
  const openings = new Map<InlineMarkupMarker, number>();
  const exclusions = mergeRanges(excludedRanges);
  const spans: PairedInlineMarkup[] = [];
  let exclusionIndex = 0;

  for (let index = 0; index < value.length;) {
    if (value[index] === "\n") {
      openings.clear();
      index += 1;

      continue;
    }

    if (characterIsEscaped(value, index)) {
      index += 1;

      continue;
    }

    const marker = inlineMarkupMarkerAt(value, index);
    if (!marker) {
      index += 1;

      continue;
    }

    const markerTo = index + marker.length;
    const markerRange = { from: index, to: markerTo };
    while (
      exclusionIndex < exclusions.length &&
      exclusions[exclusionIndex]!.to <= markerRange.from
    ) {
      exclusionIndex += 1;
    }
    if (
      exclusionIndex < exclusions.length &&
      rangesOverlap(markerRange, exclusions[exclusionIndex]!)
    ) {
      index = markerTo;

      continue;
    }

    const roles = inlineMarkupDelimiterRoles(value, marker, index);
    const opening = openings.get(marker);
    if (opening === undefined) {
      if (roles.canOpen) {
        openings.set(marker, index);
      }
      index = markerTo;

      continue;
    }

    if (
      roles.canClose &&
      inlineMarkupPairCanFormat(value, marker, opening, index)
    ) {
      spans.push({
        contentFrom: opening + marker.length,
        contentTo: index,
        from: opening,
        kind: MARKER_KINDS[marker],
        marker,
        to: markerTo,
      });
      openings.delete(marker);
    } else if (roles.canOpen) {
      openings.set(marker, index);
    }

    index = markerTo;
  }

  return removeCrossingSpans(spans);
}

export function findClosingInlineMarkupDelimiter(
  value: string,
  marker: InlineMarkupMarker,
  openingFrom: number,
  start: number,
): number {
  let cursor = start;

  while (cursor < value.length) {
    const index = value.indexOf(marker, cursor);
    if (index < 0) {
      return -1;
    }
    if (characterIsEscaped(value, index)) {
      cursor = index + marker.length;

      continue;
    }

    const roles = inlineMarkupDelimiterRoles(value, marker, index);
    if (
      roles.canClose &&
      inlineMarkupPairCanFormat(value, marker, openingFrom, index)
    ) {
      return index;
    }
    if (roles.canOpen) {
      return -1;
    }

    cursor = index + marker.length;
  }

  return -1;
}

export function inlineMarkupPairCanFormat(
  value: string,
  marker: InlineMarkupMarker,
  openingFrom: number,
  closingFrom: number,
): boolean {
  const contentFrom = openingFrom + marker.length;
  const content = value.slice(contentFrom, closingFrom);
  if (!content.trim()) {
    return false;
  }

  if (
    marker.startsWith("_") &&
    (
      isIntrawordDelimiter(value, openingFrom, marker.length) ||
      isIntrawordDelimiter(value, closingFrom, marker.length)
    )
  ) {
    return false;
  }

  return marker !== "*" || !looksLikeNumericMultiplication(
    value,
    content,
    openingFrom,
    closingFrom + marker.length,
  );
}

function inlineMarkupDelimiterRoles(
  value: string,
  marker: InlineMarkupMarker,
  from: number,
): InlineMarkupDelimiterRoles {
  if (
    marker.startsWith("_") &&
    isIntrawordDelimiter(value, from, marker.length)
  ) {
    return { canClose: false, canOpen: false };
  }

  return {
    canClose: from > 0 && value[from - 1] !== "\n",
    canOpen: from + marker.length < value.length &&
      value[from + marker.length] !== "\n",
  };
}

function inlineMarkupMarkerAt(
  value: string,
  index: number,
): InlineMarkupMarker | undefined {
  const character = value[index];
  if (
    (character !== "*" && character !== "_" && character !== "~") ||
    (
      value[index - 1] === character &&
      !characterIsEscaped(value, index - 1)
    )
  ) {
    return undefined;
  }

  let runEnd = index + 1;
  while (
    value[runEnd] === character &&
    !characterIsEscaped(value, runEnd)
  ) {
    runEnd += 1;
  }

  const length = runEnd - index;
  if (character === "~") {
    return length === 2 ? "~~" : undefined;
  }
  if (length !== 1 && length !== 2) {
    return undefined;
  }

  return character.repeat(length) as InlineMarkupMarker;
}

function isIntrawordDelimiter(
  value: string,
  from: number,
  length: number,
): boolean {
  return isWordCharacter(value[from - 1]) &&
    isWordCharacter(value[from + length]);
}

function isWordCharacter(character: string | undefined): boolean {
  return Boolean(character && /[\p{L}\p{N}]/u.test(character));
}

function looksLikeNumericMultiplication(
  value: string,
  content: string,
  before: number,
  after: number,
): boolean {
  if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)$/.test(content.trim())) {
    return false;
  }

  return /\d/.test(nearestNonWhitespace(value, before - 1, -1) ?? "") &&
    /\d/.test(nearestNonWhitespace(value, after, 1) ?? "");
}

function nearestNonWhitespace(
  value: string,
  start: number,
  direction: -1 | 1,
): string | undefined {
  for (
    let index = start;
    index >= 0 && index < value.length;
    index += direction
  ) {
    if (!/\s/u.test(value[index]!)) {
      return value[index];
    }
  }

  return undefined;
}

export function characterIsEscaped(value: string, index: number): boolean {
  let backslashes = 0;
  for (
    let cursor = index - 1;
    cursor >= 0 && value[cursor] === "\\";
    cursor -= 1
  ) {
    backslashes += 1;
  }

  return backslashes % 2 === 1;
}

function mergeRanges(
  ranges: readonly InlineMarkupRange[],
): InlineMarkupRange[] {
  const ordered = ranges
    .filter((range) => range.from < range.to)
    .map((range) => ({ ...range }))
    .sort((left, right) => left.from - right.from || left.to - right.to);
  const merged: InlineMarkupRange[] = [];

  for (const range of ordered) {
    const previous = merged.at(-1);
    if (!previous || range.from > previous.to) {
      merged.push(range);
    } else {
      previous.to = Math.max(previous.to, range.to);
    }
  }

  return merged;
}

function removeCrossingSpans(
  spans: readonly PairedInlineMarkup[],
): PairedInlineMarkup[] {
  const accepted: PairedInlineMarkup[] = [];
  const ordered = [...spans].sort((left, right) =>
    left.from - right.from || right.to - left.to
  );

  for (const span of ordered) {
    if (!accepted.some((candidate) => rangesCross(candidate, span))) {
      accepted.push(span);
    }
  }

  return accepted;
}

function rangesCross(
  left: InlineMarkupRange,
  right: InlineMarkupRange,
): boolean {
  return (
    left.from < right.from && right.from < left.to && left.to < right.to
  ) || (
    right.from < left.from && left.from < right.to && right.to < left.to
  );
}

function rangesOverlap(
  left: InlineMarkupRange,
  right: InlineMarkupRange,
): boolean {
  return left.from < right.to && right.from < left.to;
}
