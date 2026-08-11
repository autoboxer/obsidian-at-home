export interface MarkdownSelectionEdit {
  value: string;
  selectionStart: number;
  selectionEnd: number;
}

export function toggleInlineFormatting(
  value: string,
  start: number,
  end: number,
  marker: string,
  alternatives: readonly string[] = [],
): MarkdownSelectionEdit {
  const selected = value.slice(start, end);
  const recognizedMarkers = [marker, ...alternatives];
  const includedMarker = recognizedMarkers.find((candidate) =>
    selectionIncludesFormatting(value, start, end, candidate),
  );

  if (includedMarker) {
    const unwrapped = selected.slice(includedMarker.length, -includedMarker.length);

    return {
      value: `${value.slice(0, start)}${unwrapped}${value.slice(end)}`,
      selectionStart: start,
      selectionEnd: start + unwrapped.length,
    };
  }

  const surroundingMarker = recognizedMarkers.find((candidate) =>
    selectionHasSurroundingFormatting(value, start, end, candidate),
  );

  if (surroundingMarker) {
    const unwrappedStart = start - surroundingMarker.length;

    return {
      value: `${value.slice(0, unwrappedStart)}${selected}${value.slice(end + surroundingMarker.length)}`,
      selectionStart: unwrappedStart,
      selectionEnd: unwrappedStart + selected.length,
    };
  }

  if (
    start === end
    && recognizedMarkers.some((candidate) => markersSurroundSelection(value, start, end, candidate))
  ) {
    return unchangedSelection(value, start, end);
  }

  const wrappingMarker = recognizedMarkers.find((candidate) =>
    markerCanWrapSelection(value, start, end, candidate),
  );

  return wrappingMarker
    ? wrapSelection(value, start, end, wrappingMarker, wrappingMarker)
    : unchangedSelection(value, start, end);
}

export function wrapInlineCode(
  value: string,
  start: number,
  end: number,
): MarkdownSelectionEdit {
  if (value[start - 1] === "`" || value[end] === "`") {
    return unchangedSelection(value, start, end);
  }

  const selected = value.slice(start, end);
  const backtickRuns = selected.match(/`+/g) ?? [];
  const longestRun = Math.max(0, ...backtickRuns.map((run) => run.length));
  const delimiter = "`".repeat(longestRun + 1);
  const needsPadding = selected.startsWith("`")
    || selected.endsWith("`")
    || (/^\s/.test(selected) && /\s$/.test(selected) && Boolean(selected.trim()));
  const padding = needsPadding ? " " : "";

  return wrapSelection(
    value,
    start,
    end,
    `${delimiter}${padding}`,
    `${padding}${delimiter}`,
  );
}

function wrapSelection(
  value: string,
  start: number,
  end: number,
  before: string,
  after: string,
): MarkdownSelectionEdit {
  const selected = value.slice(start, end);
  const selectionStart = start + before.length;

  return {
    value: `${value.slice(0, start)}${before}${selected}${after}${value.slice(end)}`,
    selectionStart,
    selectionEnd: selectionStart + selected.length,
  };
}

function selectionIncludesFormatting(
  value: string,
  start: number,
  end: number,
  marker: string,
): boolean {
  const closingMarkerStart = end - marker.length;

  return end - start > marker.length * 2
    && value.slice(start, start + marker.length) === marker
    && value.slice(closingMarkerStart, end) === marker
    && isFormattingOpening(value, start, marker)
    && isFormattingClosing(value, closingMarkerStart, marker)
    && findClosingFormatting(value, marker, start + marker.length) === closingMarkerStart;
}

function selectionHasSurroundingFormatting(
  value: string,
  start: number,
  end: number,
  marker: string,
): boolean {
  const openingMarkerStart = start - marker.length;

  if (start === end || !markersSurroundSelection(value, start, end, marker)) {
    return false;
  }

  return isFormattingOpening(value, openingMarkerStart, marker)
    && isFormattingClosing(value, end, marker)
    && findClosingFormatting(value, marker, start) === end;
}

function markersSurroundSelection(
  value: string,
  start: number,
  end: number,
  marker: string,
): boolean {
  return start >= marker.length
    && value.slice(start - marker.length, start) === marker
    && value.slice(end, end + marker.length) === marker;
}

function markerCanWrapSelection(
  value: string,
  start: number,
  end: number,
  marker: string,
): boolean {
  const delimiter = marker[0]!;

  return !value.slice(start, end).includes(marker)
    && value[start - 1] !== delimiter
    && value[end] !== delimiter;
}

function unchangedSelection(value: string, start: number, end: number): MarkdownSelectionEdit {
  return {
    value,
    selectionStart: start,
    selectionEnd: end,
  };
}

function isFormattingOpening(value: string, start: number, marker: string): boolean {
  if (value[start - 1] === "\\") {
    return false;
  }
  if (marker !== "*" && marker !== "_") {
    return true;
  }

  const before = value[start - 1] ?? "";
  const after = value[start + 1] ?? "";
  if (before === marker || after === marker) {
    return false;
  }

  return marker !== "_"
    || !(/[\p{L}\p{N}]/u.test(before) && /[\p{L}\p{N}]/u.test(after));
}

function isFormattingClosing(value: string, start: number, marker: string): boolean {
  if (marker !== "*" && marker !== "_") {
    return true;
  }

  return value[start - 1] !== marker && value[start + 1] !== marker;
}

function findClosingFormatting(value: string, marker: string, start: number): number {
  let cursor = start;

  while (cursor < value.length) {
    const index = value.indexOf(marker, cursor);
    if (index < 0) {
      return -1;
    }
    if (value[index - 1] !== "\\") {
      return index;
    }
    cursor = index + marker.length;
  }

  return -1;
}
