<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import { useLiveMarkdownRegionLayout } from "../composables/useLiveMarkdownRegionLayout";
import {
  activeLiveMarkdownBlocks,
  parseLiveMarkdownBlocks,
  setLiveMarkdownTaskChecked,
} from "../lib/liveMarkdown";
import {
  indexLiveMarkdownCodeFenceLines,
  parseLiveMarkdownCodeFences,
  setLiveMarkdownCodeLanguage,
} from "../lib/liveMarkdownCode";
import { parseLiveMarkdownInline } from "../lib/liveMarkdownInline";
import {
  indexLiveMarkdownTableLines,
  parseLiveMarkdownTables,
} from "../lib/liveMarkdownTable";
import { navigateLiveMarkdownTable } from "../lib/liveMarkdownTableNavigation";
import { toggleInlineFormatting, wrapInlineCode } from "../lib/markdownFormatting";
import { normalizeWikiTarget, wikiTargetTitle } from "../lib/wikiLinks";
import type { LiveMarkdownBlock } from "../lib/liveMarkdown";
import type {
  LiveMarkdownCodeFence,
  LiveMarkdownCodeFenceLine,
} from "../lib/liveMarkdownCode";
import type { LiveMarkdownInlineSegment } from "../lib/liveMarkdownInline";
import type {
  LiveMarkdownTable as LiveMarkdownTableModel,
  LiveMarkdownTableLine,
} from "../lib/liveMarkdownTable";
import type { LiveMarkdownTableNavigationEdit } from "../lib/liveMarkdownTableNavigation";
import AppIcon from "./AppIcon.vue";
import LiveMarkdownCodeBlock from "./LiveMarkdownCodeBlock.vue";
import LiveMarkdownInline from "./LiveMarkdownInline.vue";
import LiveMarkdownTable from "./LiveMarkdownTable.vue";

const props = defineProps<{
  modelValue: string;
  noteTitles: string[];
}>();

const emit = defineEmits<{
  openLink: [href: string];
  openWiki: [target: string];
  "update:modelValue": [value: string];
  scroll: [element: HTMLTextAreaElement];
}>();

const textarea = ref<HTMLTextAreaElement>();
const gutter = ref<HTMLElement>();
const visualLayer = ref<HTMLElement>();
const editorFocused = ref(false);
const selectionStart = ref(0);
const selectionEnd = ref(0);
const suggestionIndex = ref(0);
const suggestionQuery = ref<string | null>(null);
const INDENT = "  ";
const LIVE_CODE_BLOCK_SELECTOR = [
  ".live-markdown-block.is-code-opening",
  ".live-markdown-block.is-code-content",
  ".live-markdown-block.is-code-closing",
].join(", ");
const LIVE_TABLE_BLOCK_SELECTOR = [
  ".live-markdown-block.is-table-header",
  ".live-markdown-block.is-table-delimiter",
  ".live-markdown-block.is-table-body",
].join(", ");
const UNORDERED_LIST_MARKERS = ["•", "◦", "▪"] as const;
const ROMAN_NUMERALS: ReadonlyArray<readonly [number, string]> = [
  [1_000, "m"],
  [900, "cm"],
  [500, "d"],
  [400, "cd"],
  [100, "c"],
  [90, "xc"],
  [50, "l"],
  [40, "xl"],
  [10, "x"],
  [9, "ix"],
  [5, "v"],
  [4, "iv"],
  [1, "i"],
];

interface ListLine {
  indent: string;
  ordered: boolean;
  number: number;
  delimiter: "." | ")";
  bullet: "-" | "+" | "*";
  spacing: string;
  prefixLength: number;
}

interface TextEdit {
  start: number;
  removed: number;
  added: number;
}

const liveBlocks = computed(() => parseLiveMarkdownBlocks(props.modelValue));
const codeFences = computed(() => parseLiveMarkdownCodeFences(props.modelValue));
const codeFenceLines = computed(() => indexLiveMarkdownCodeFenceLines(codeFences.value));
const tables = computed(() => parseLiveMarkdownTables(
  props.modelValue,
  liveBlocks.value,
));
const tableLines = computed(() => indexLiveMarkdownTableLines(tables.value));
const { heightForRegion: codeFenceHeight } = useLiveMarkdownRegionLayout({
  blockSelector: LIVE_CODE_BLOCK_SELECTOR,
  container: visualLayer,
  regions: codeFences,
});
const { heightForRegion: tableHeight } = useLiveMarkdownRegionLayout({
  blockSelector: LIVE_TABLE_BLOCK_SELECTOR,
  container: visualLayer,
  regions: tables,
});
const lineCount = computed(() => liveBlocks.value.length);
const lineNumbers = computed(() => Array.from({ length: lineCount.value }, (_, index) => index + 1).join("\n"));
const activeBlockLines = computed(() => {
  const lines = new Set(
    editorFocused.value
      ? activeLiveMarkdownBlocks(
        liveBlocks.value,
        selectionStart.value,
        selectionEnd.value,
      ).map((block) => block.lineNumber)
      : [],
  );

  for (const lineNumber of [...lines]) {
    const regionLines = codeFenceLines.value.get(lineNumber)?.fence.lineNumbers ??
      tableLines.value.get(lineNumber)?.table.lineNumbers ??
      [];
    for (const regionLine of regionLines) {
      lines.add(regionLine);
    }
  }

  return lines;
});
const normalizedNoteTitles = computed(() => new Set(
  props.noteTitles.flatMap((title) => [
    normalizeInlineLinkTarget(title),
    normalizeInlineLinkTarget(wikiTargetTitle(title)),
  ]),
));
const inlineSegmentsByLine = computed(() => {
  const segments = new Map<number, LiveMarkdownInlineSegment[]>();

  for (const block of liveBlocks.value) {
    if (!blockRendersInline(block) || tableLines.value.has(block.lineNumber)) {
      continue;
    }

    segments.set(
      block.lineNumber,
      parseLiveMarkdownInline(blockInlineSource(block), {
        resolveWikiLink: (target) => inlineWikiLinkIsResolved(target),
      }),
    );
  }

  return segments;
});
const suggestions = computed(() => {
  if (suggestionQuery.value === null) {
    return [];
  }
  const query = suggestionQuery.value.toLocaleLowerCase();

  return props.noteTitles
    .filter((title) => !query || title.toLocaleLowerCase().includes(query))
    .sort((a, b) => {
      const aStarts = a.toLocaleLowerCase().startsWith(query);
      const bStarts = b.toLocaleLowerCase().startsWith(query);

      return Number(bStarts) - Number(aStarts) || a.localeCompare(b);
    })
    .slice(0, 6);
});

function onInput(event: Event): void {
  const element = event.target as HTMLTextAreaElement;
  updateSelection(element);
  emit("update:modelValue", element.value);
  updateSuggestions(element);
}

function onSelection(): void {
  if (!textarea.value) {
    return;
  }
  updateSelection(textarea.value);
  updateSuggestions(textarea.value);
}

function onFocus(): void {
  editorFocused.value = true;
  onSelection();
}

function onBlur(): void {
  editorFocused.value = false;
}

function onScroll(): void {
  if (!textarea.value || !gutter.value) {
    return;
  }
  gutter.value.scrollTop = textarea.value.scrollTop;
  if (visualLayer.value) {
    visualLayer.value.scrollTop = textarea.value.scrollTop;
    visualLayer.value.scrollLeft = textarea.value.scrollLeft;
  }
  emit("scroll", textarea.value);
}

function onRenderedLinkClick(event: MouseEvent): void {
  const target = event.target;
  if (!(target instanceof Element)) {
    return;
  }

  const link = target.closest<HTMLAnchorElement>("a.live-inline-segment");
  if (!link || !visualLayer.value?.contains(link)) {
    return;
  }
  if (link.classList.contains("is-wiki-link")) {
    event.preventDefault();
    const wikiTarget = link.dataset.wikiTarget?.trim();
    if (wikiTarget) {
      emit("openWiki", wikiTarget);
    }

    return;
  }
  if (!window.__TAURI__) {
    return;
  }

  event.preventDefault();
  const rawHref = link.getAttribute("href") ?? "";
  const href = rawHref.startsWith("//") ? `https:${rawHref}` : rawHref;
  if (/^(?:https?|mailto):/i.test(href)) {
    emit("openLink", href);
  }
}

function updateSelection(element: HTMLTextAreaElement): void {
  selectionStart.value = element.selectionStart;
  selectionEnd.value = element.selectionEnd;
}

function blockIsActive(block: LiveMarkdownBlock): boolean {
  return activeBlockLines.value.has(block.lineNumber);
}

function blockSource(block: LiveMarkdownBlock): string {
  return block.source || "\u00a0";
}

function blockRendersInline(block: LiveMarkdownBlock): boolean {
  return block.type === "blockquote" ||
    block.type === "heading" ||
    block.type === "list" ||
    block.type === "task" ||
    block.type === "text";
}

function blockHasLiveRendering(block: LiveMarkdownBlock): boolean {
  return blockRendersInline(block) ||
    block.type === "horizontal-rule" ||
    codeFenceLines.value.has(block.lineNumber) ||
    tableLines.value.has(block.lineNumber);
}

function blockCodeFenceLine(
  block: LiveMarkdownBlock,
): LiveMarkdownCodeFenceLine | undefined {
  return codeFenceLines.value.get(block.lineNumber);
}

function blockCodeFenceClass(block: LiveMarkdownBlock): string | undefined {
  const role = blockCodeFenceLine(block)?.role;

  return role ? `is-code-${role}` : undefined;
}

function blockTableLine(
  block: LiveMarkdownBlock,
): LiveMarkdownTableLine | undefined {
  return tableLines.value.get(block.lineNumber);
}

function blockTableClass(block: LiveMarkdownBlock): string | undefined {
  const role = blockTableLine(block)?.role;

  return role ? `is-table-${role}` : undefined;
}

function openingCodeFence(block: LiveMarkdownBlock): LiveMarkdownCodeFence {
  const line = blockCodeFenceLine(block);
  if (!line || line.role !== "opening") {
    throw new Error("Expected an opening code fence");
  }

  return line.fence;
}

function openingTable(block: LiveMarkdownBlock): LiveMarkdownTableModel {
  const line = blockTableLine(block);
  if (!line || line.role !== "header") {
    throw new Error("Expected a table header");
  }

  return line.table;
}

function changeCodeFenceLanguage(
  fence: LiveMarkdownCodeFence,
  language: string,
): void {
  const next = setLiveMarkdownCodeLanguage(
    props.modelValue,
    fence,
    language,
  );
  if (next !== props.modelValue) {
    emit("update:modelValue", next);
  }
}

function blockInlineSource(block: LiveMarkdownBlock): string {
  return props.modelValue.slice(block.content.from, block.content.to);
}

function blockInlineSegments(block: LiveMarkdownBlock): readonly LiveMarkdownInlineSegment[] {
  return inlineSegmentsByLine.value.get(block.lineNumber) ?? [];
}

function blockPrefix(block: LiveMarkdownBlock): string {
  return props.modelValue.slice(block.from, block.content.from);
}

function blockListPrefix(block: LiveMarkdownBlock): string {
  return block.list
    ? props.modelValue.slice(block.list.marker.from, block.content.from)
    : "";
}

function blockListMarker(block: LiveMarkdownBlock): string {
  if (!block.list) {
    return "";
  }
  if (!block.list.ordered) {
    return UNORDERED_LIST_MARKERS[block.list.depth % 3]!;
  }

  const number = block.list.number ?? 1;
  if (block.list.depth % 3 === 1) {
    return `${alphabeticListMarker(number)}.`;
  }
  if (block.list.depth % 3 === 2) {
    return `${romanListMarker(number)}.`;
  }

  return `${number}.`;
}

function alphabeticListMarker(number: number): string {
  if (number < 1) {
    return String(number);
  }

  let value = number;
  let marker = "";

  while (value > 0) {
    value -= 1;
    marker = String.fromCharCode(97 + (value % 26)) + marker;
    value = Math.floor(value / 26);
  }

  return marker;
}

function romanListMarker(number: number): string {
  if (number < 1 || number > 3_999) {
    return String(number);
  }

  let remaining = number;
  let marker = "";

  for (const [value, numeral] of ROMAN_NUMERALS) {
    while (remaining >= value) {
      marker += numeral;
      remaining -= value;
    }
  }

  return marker;
}

function normalizeInlineLinkTarget(value: string): string {
  return normalizeWikiTarget(value)
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLocaleLowerCase()
    .replace(/\s+/g, " ")
    .trim();
}

function inlineWikiLinkIsResolved(target: string): boolean {
  if (!target) {
    return true;
  }

  return normalizedNoteTitles.value.has(normalizeInlineLinkTarget(target)) ||
    normalizedNoteTitles.value.has(normalizeInlineLinkTarget(wikiTargetTitle(target)));
}

function blockIndent(block: LiveMarkdownBlock): string {
  const markerFrom = block.task?.marker.from ?? block.list?.marker.from;

  return markerFrom === undefined
    ? ""
    : props.modelValue.slice(block.from, markerFrom);
}

function blockTaskMarker(block: LiveMarkdownBlock): string {
  return block.task
    ? props.modelValue.slice(block.task.marker.from, block.task.marker.to)
    : "";
}

function toggleLiveTask(block: LiveMarkdownBlock): void {
  const element = textarea.value;
  if (!block.task || !element) {
    return;
  }

  const selectionStart = element.selectionStart;
  const selectionEnd = element.selectionEnd;
  const selectionDirection = element.selectionDirection;
  const next = setLiveMarkdownTaskChecked(
    props.modelValue,
    block,
    !block.task.checked,
  );
  if (next !== props.modelValue) {
    applyEdit(next, selectionStart, selectionEnd, selectionDirection);
  }
}

function getScrollElement(): HTMLTextAreaElement | undefined {
  return textarea.value;
}

defineExpose({ getScrollElement });

function onKeydown(event: KeyboardEvent): void {
  const element = textarea.value;
  if (!element || event.isComposing) {
    return;
  }

  if (suggestions.value.length) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      suggestionIndex.value = (suggestionIndex.value + 1) % suggestions.value.length;

      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      suggestionIndex.value = (suggestionIndex.value - 1 + suggestions.value.length) % suggestions.value.length;

      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      insertSuggestion(suggestions.value[suggestionIndex.value]!);

      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      suggestionQuery.value = null;

      return;
    }
  }

  const commandModifier = event.metaKey || event.ctrlKey || event.altKey;

  if (event.key === "Enter" && !commandModifier && !event.shiftKey) {
    const tableEdit = navigateLiveMarkdownTable(
      element.value,
      tables.value,
      element.selectionStart,
      element.selectionEnd,
      "next-row",
    );
    if (tableEdit) {
      event.preventDefault();
      applyTableNavigation(element, tableEdit);

      return;
    }
    if (handleSmartEnter(element)) {
      event.preventDefault();

      return;
    }
  }

  if (event.key === "Tab" && !commandModifier) {
    const tableEdit = navigateLiveMarkdownTable(
      element.value,
      tables.value,
      element.selectionStart,
      element.selectionEnd,
      event.shiftKey ? "previous-cell" : "next-cell",
    );
    event.preventDefault();
    if (tableEdit) {
      applyTableNavigation(element, tableEdit);

      return;
    }
    if (!adjustSelectedLines(element, event.shiftKey) && !event.shiftKey) {
      replaceSelection(INDENT);
    }

    return;
  }

  const modifier = event.metaKey || event.ctrlKey;
  const formattingModifier = modifier && !event.altKey;
  const key = event.key.toLocaleLowerCase();

  if (formattingModifier && !event.shiftKey && key === "b") {
    event.preventDefault();
    toggleSelectionFormatting("**", ["__"]);
  } else if (formattingModifier && !event.shiftKey && key === "i") {
    event.preventDefault();
    toggleSelectionFormatting("*", ["_"]);
  } else if (formattingModifier && event.shiftKey && key === "x") {
    event.preventDefault();
    toggleSelectionFormatting("~~");
  } else if (event.key === "`" && !commandModifier && element.selectionStart !== element.selectionEnd) {
    event.preventDefault();
    wrapSelectionAsInlineCode();
  }
}

function replaceSelection(value: string): void {
  const element = textarea.value;
  if (!element) {
    return;
  }
  const start = element.selectionStart;
  const end = element.selectionEnd;
  const next = `${element.value.slice(0, start)}${value}${element.value.slice(end)}`;
  applyEdit(next, start + value.length);
}

function wrapSelectionAsInlineCode(): void {
  const element = textarea.value;
  if (!element) {
    return;
  }

  const edit = wrapInlineCode(
    element.value,
    element.selectionStart,
    element.selectionEnd,
  );
  applyEdit(edit.value, edit.selectionStart, edit.selectionEnd, element.selectionDirection);
}

function toggleSelectionFormatting(marker: string, alternatives: string[] = []): void {
  const element = textarea.value;
  if (!element) {
    return;
  }

  const edit = toggleInlineFormatting(
    element.value,
    element.selectionStart,
    element.selectionEnd,
    marker,
    alternatives,
  );
  if (edit.value === element.value) {
    return;
  }

  applyEdit(edit.value, edit.selectionStart, edit.selectionEnd, element.selectionDirection);
}

function updateSuggestions(element: HTMLTextAreaElement): void {
  const beforeCursor = element.value.slice(0, element.selectionStart);
  const match = beforeCursor.match(/\[\[([^\]\n|#]*)$/);
  suggestionQuery.value = match ? match[1] ?? "" : null;
  suggestionIndex.value = 0;
}

function insertSuggestion(title: string): void {
  const element = textarea.value;
  if (!element || suggestionQuery.value === null) {
    return;
  }
  const queryLength = suggestionQuery.value.length;
  const start = element.selectionStart - queryLength;
  const replacement = `${title}]]`;
  const next = `${element.value.slice(0, start)}${replacement}${element.value.slice(element.selectionStart)}`;
  suggestionQuery.value = null;
  applyEdit(next, start + replacement.length);
}

function applyEdit(
  value: string,
  selectionStart: number,
  selectionEnd = selectionStart,
  direction: "forward" | "backward" | "none" | null = "none",
): void {
  emit("update:modelValue", value);
  nextTick(() => {
    const element = textarea.value;
    if (!element) {
      return;
    }
    element.focus();
    element.setSelectionRange(selectionStart, selectionEnd, direction ?? "none");
    onSelection();
  });
}

function applyTableNavigation(
  element: HTMLTextAreaElement,
  edit: LiveMarkdownTableNavigationEdit,
): void {
  if (edit.value !== element.value) {
    applyEdit(edit.value, edit.selectionStart, edit.selectionEnd);

    return;
  }

  element.setSelectionRange(edit.selectionStart, edit.selectionEnd);
  onSelection();
}

function handleSmartEnter(element: HTMLTextAreaElement): boolean {
  if (element.selectionStart !== element.selectionEnd) {
    return false;
  }

  const value = element.value;
  const position = element.selectionStart;
  const lineStart = value.lastIndexOf("\n", position - 1) + 1;
  const nextNewline = value.indexOf("\n", position);
  const lineEnd = nextNewline < 0 ? value.length : nextNewline;
  const line = value.slice(lineStart, lineEnd);

  const openingFence = line.match(/^([ \t]*)(`{3,}|~{3,})([^\n]*)$/);
  if (
    openingFence &&
    position === lineEnd &&
    !activeFenceBefore(value, lineStart)
  ) {
    const indent = openingFence[1]!;
    const marker = openingFence[2]!;
    const followingLineStart = lineEnd < value.length ? lineEnd + 1 : value.length;
    const followingLineEndIndex = value.indexOf("\n", followingLineStart);
    const followingLineEnd = followingLineEndIndex < 0 ? value.length : followingLineEndIndex;
    const followingLine = value.slice(followingLineStart, followingLineEnd);
    const existingClosing = followingLine.match(/^([ \t]*)(`+|~+)\s*$/);
    const closesFence = Boolean(existingClosing
      && existingClosing[2]![0] === marker[0]
      && existingClosing[2]!.length >= marker.length);
    const insertion = closesFence
      ? `\n${indent}`
      : `\n${indent}\n${indent}${marker}`;
    const next = `${value.slice(0, position)}${insertion}${value.slice(position)}`;
    applyEdit(next, position + 1 + indent.length);

    return true;
  }

  if (activeFenceBefore(value, lineStart)) {
    return false;
  }

  const item = matchEditableListLine(line);
  if (!item || position < lineStart + item.prefixLength) {
    return false;
  }

  const bodyStart = lineStart + item.prefixLength;
  const bodyBefore = value.slice(bodyStart, position);
  const bodyAfter = value.slice(position, lineEnd);
  const fullBody = `${bodyBefore}${bodyAfter}`;
  const contentWithoutTask = fullBody.replace(/^\[[ xX]\][ \t]*/, "");

  if (!contentWithoutTask.trim() && position === lineEnd) {
    const next = `${value.slice(0, lineStart)}${item.indent}${value.slice(lineEnd)}`;
    applyEdit(next, lineStart + item.indent.length);

    return true;
  }

  const marker = item.ordered
    ? `${item.number >= 999_999_999 ? 1 : item.number + 1}${item.delimiter}`
    : item.bullet;
  const taskPrefix = /^\[[ xX]\][ \t]+/.test(bodyBefore) ? "[ ] " : "";
  const continuation = `${item.indent}${marker}${item.spacing}${taskPrefix}`;
  const next = `${value.slice(0, position)}\n${continuation}${value.slice(position)}`;
  applyEdit(next, position + 1 + continuation.length);

  return true;
}

function matchEditableListLine(line: string): ListLine | undefined {
  const match = line.match(/^([ \t]*)(?:(\d{1,9})([.)])|([-+*]))([ \t]+)(.*)$/);
  if (!match) {
    return undefined;
  }
  const ordered = Boolean(match[2]);
  const indent = match[1]!;
  const markerLength = ordered ? match[2]!.length + 1 : 1;
  const spacing = match[5]!;

  return {
    indent,
    ordered,
    number: ordered ? Number.parseInt(match[2]!, 10) : 1,
    delimiter: ordered ? match[3]! as "." | ")" : ".",
    bullet: ordered ? "-" : match[4]! as "-" | "+" | "*",
    spacing,
    prefixLength: indent.length + markerLength + spacing.length,
  };
}

function activeFenceBefore(value: string, position: number): { marker: string; length: number } | undefined {
  const before = value.slice(0, position);
  const lines = before.split("\n");
  lines.pop();
  let active: { marker: string; length: number } | undefined;

  for (const line of lines) {
    if (!active) {
      const opening = line.match(/^[ \t]*(`{3,}|~{3,})[^\n]*$/);
      if (opening) {
        active = { marker: opening[1]![0]!, length: opening[1]!.length };
      }
      continue;
    }

    const closing = line.match(/^[ \t]*(`+|~+)\s*$/);
    if (
      closing &&
      closing[1]![0] === active.marker &&
      closing[1]!.length >= active.length
    ) active = undefined;
  }

  return active;
}

function adjustSelectedLines(element: HTMLTextAreaElement, outdent: boolean): boolean {
  const value = element.value;
  const selectionStart = element.selectionStart;
  const selectionEnd = element.selectionEnd;
  const hasSelection = selectionStart !== selectionEnd;
  const firstLineStart = value.lastIndexOf("\n", selectionStart - 1) + 1;
  const firstLineEndIndex = value.indexOf("\n", selectionStart);
  const firstLineEnd = firstLineEndIndex < 0 ? value.length : firstLineEndIndex;
  const firstLine = value.slice(firstLineStart, firstLineEnd);

  if (!hasSelection && !matchEditableListLine(firstLine)) {
    if (!outdent || !/^[ \t]/.test(firstLine)) {
      return false;
    }
  }

  const selectionEndsAtLineStart = hasSelection && selectionEnd > 0 && value[selectionEnd - 1] === "\n";
  const effectiveEnd = selectionEndsAtLineStart ? selectionEnd - 1 : selectionEnd;
  const lastLineEndIndex = value.indexOf("\n", effectiveEnd);
  const blockEnd = lastLineEndIndex < 0 ? value.length : lastLineEndIndex;
  const block = value.slice(firstLineStart, blockEnd);
  const lines = block.split("\n");
  const edits: TextEdit[] = [];
  let sourceOffset = firstLineStart;

  const transformed = lines.map((line) => {
    if (!outdent) {
      edits.push({ start: sourceOffset, removed: 0, added: INDENT.length });
      const orderedMarker = line.match(/^([ \t]*)(\d{1,9})([.)])/);
      if (orderedMarker && orderedMarker[2]!.length !== 1) {
        edits.push({
          start: sourceOffset + orderedMarker[1]!.length,
          removed: orderedMarker[2]!.length,
          added: 1,
        });
      }
      sourceOffset += line.length + 1;
      const indentedLine = orderedMarker
        ? `${orderedMarker[1]}1${orderedMarker[3]}${line.slice(orderedMarker[0].length)}`
        : line;

      return `${INDENT}${indentedLine}`;
    }

    const removable = line.startsWith("\t") ? 1 : line.match(/^ {1,2}/)?.[0].length ?? 0;
    if (removable) {
      edits.push({ start: sourceOffset, removed: removable, added: 0 });
    }
    sourceOffset += line.length + 1;

    return line.slice(removable);
  }).join("\n");

  if (!edits.length) {
    return true;
  }

  const next = `${value.slice(0, firstLineStart)}${transformed}${value.slice(blockEnd)}`;
  applyEdit(
    next,
    mapPositionThroughEdits(selectionStart, edits),
    mapPositionThroughEdits(selectionEnd, edits),
    element.selectionDirection,
  );

  return true;
}

function mapPositionThroughEdits(position: number, edits: TextEdit[]): number {
  let delta = 0;
  for (const edit of edits) {
    if (position < edit.start) {
      break;
    }
    if (position <= edit.start + edit.removed) {
      return edit.start + delta + edit.added;
    }
    delta += edit.added - edit.removed;
  }

  return position + delta;
}
</script>

<template>
  <div class="source-editor">
    <div ref="gutter" class="editor-gutter" aria-hidden="true">
      <pre>{{ lineNumbers }}</pre>
    </div>
    <div
      ref="visualLayer"
      class="live-markdown-layer"
      @click="onRenderedLinkClick"
    >
      <div
        v-for="block in liveBlocks"
        :key="`${block.lineNumber}:${block.from}`"
        class="live-markdown-block"
        :data-line-number="block.lineNumber"
        :class="[
          `is-${block.type}`,
          block.headingLevel ? `heading-level-${block.headingLevel}` : undefined,
          block.list ? `list-depth-${block.list.depth % 3}` : undefined,
          block.quote ? `quote-depth-${Math.min(block.quote.depth, 3)}` : undefined,
          blockCodeFenceClass(block),
          blockTableClass(block),
          { 'is-active': blockIsActive(block), 'is-checked': block.task?.checked },
        ]"
      >
        <template v-if="blockIsActive(block) || !blockHasLiveRendering(block)">
          <span aria-hidden="true">{{ blockSource(block) }}</span>
        </template>
        <template v-else>
          <span class="live-markdown-layout" aria-hidden="true">{{ blockSource(block) }}</span>
          <span
            class="live-markdown-content"
            :class="{ 'is-heading': block.type === 'heading' }"
          >
            <LiveMarkdownTable
              v-if="blockTableLine(block)?.role === 'header'"
              :table="openingTable(block)"
              :height="tableHeight(openingTable(block))"
              :resolve-wiki-link="inlineWikiLinkIsResolved"
            />
            <span
              v-else-if="blockTableLine(block)"
              aria-hidden="true"
            />
            <LiveMarkdownCodeBlock
              v-else-if="blockCodeFenceLine(block)?.role === 'opening'"
              :fence="openingCodeFence(block)"
              :height="codeFenceHeight(openingCodeFence(block))"
              @update:language="changeCodeFenceLanguage(openingCodeFence(block), $event)"
            />
            <span
              v-else-if="block.type === 'code'"
              aria-hidden="true"
            />
            <span
              v-else-if="block.type === 'horizontal-rule'"
              class="live-horizontal-rule"
              aria-hidden="true"
            />
            <template v-else-if="block.type === 'task'">
              <span class="live-task-indent" aria-hidden="true">{{ blockIndent(block) }}</span>
              <span class="live-task-control">
                <span class="live-task-marker" aria-hidden="true">{{ blockTaskMarker(block) }}</span>
                <button
                  type="button"
                  class="live-task-checkbox"
                  :aria-label="block.task?.checked ? 'Mark task incomplete' : 'Mark task complete'"
                  :aria-pressed="block.task?.checked"
                  tabindex="-1"
                  @mousedown.prevent
                  @click.stop="toggleLiveTask(block)"
                >
                  <AppIcon v-if="block.task?.checked" name="check" :size="9" :stroke-width="2.4" />
                </button>
              </span>
              <LiveMarkdownInline
                class="live-task-content"
                :segments="blockInlineSegments(block)"
              />
            </template>
            <template v-else-if="block.type === 'list'">
              <span class="live-list-indent" aria-hidden="true">{{ blockIndent(block) }}</span>
              <span class="live-list-control">
                <span class="live-list-prefix" aria-hidden="true">{{ blockListPrefix(block) }}</span>
                <span class="live-list-marker" aria-hidden="true">{{ blockListMarker(block) }}</span>
              </span>
              <LiveMarkdownInline :segments="blockInlineSegments(block)" />
            </template>
            <template v-else-if="block.type === 'blockquote'">
              <span class="live-quote-prefix" aria-hidden="true">{{ blockPrefix(block) }}</span>
              <LiveMarkdownInline :segments="blockInlineSegments(block)" />
            </template>
            <LiveMarkdownInline
              v-else
              :segments="blockInlineSegments(block)"
            />
          </span>
        </template>
      </div>
    </div>
    <textarea
      ref="textarea"
      class="source-textarea"
      :value="modelValue"
      spellcheck="true"
      aria-label="Markdown source"
      @blur="onBlur"
      @focus="onFocus"
      @input="onInput"
      @click="onSelection"
      @keyup="onSelection"
      @scroll="onScroll"
      @keydown="onKeydown"
    />

    <Transition name="popover-fade">
      <div v-if="suggestions.length" class="wiki-suggestions" role="listbox">
        <div class="suggestion-kicker">Link a note</div>
        <button
          v-for="(title, index) in suggestions"
          :key="title"
          type="button"
          class="wiki-suggestion"
          :class="{ active: index === suggestionIndex }"
          @mousedown.prevent="insertSuggestion(title)"
        >
          <span class="suggestion-icon"><AppIcon name="link" :size="14" /></span>
          <span>{{ title }}</span>
          <AppIcon v-if="index === suggestionIndex" name="enter" :size="13" />
        </button>
      </div>
    </Transition>

    <div class="editor-language-pill">MD</div>
  </div>
</template>
