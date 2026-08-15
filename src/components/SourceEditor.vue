<script setup lang="ts">
import {
  computed,
  onBeforeUnmount,
  onMounted,
  ref,
  shallowRef,
  watch,
} from "vue";
import {
  defaultKeymap,
  history,
  historyKeymap,
  insertNewline,
} from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import {
  Annotation,
  Compartment,
  EditorSelection,
  EditorState,
  Prec,
  Transaction,
} from "@codemirror/state";
import {
  drawSelection,
  dropCursor,
  EditorView,
  highlightSpecialChars,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import {
  codeMirrorDocumentSearchExtension,
  useCodeMirrorDocumentSearch,
} from "../composables/useCodeMirrorDocumentSearch";
import { normalizeOrderedListMarkers } from "../lib/liveMarkdown";
import {
  liveMarkdownExtension,
  refreshLiveMarkdownEffect,
} from "../lib/liveMarkdownCodeMirror";
import {
  joinLeadingFrontmatter,
  leadingFrontmatterEnd,
  markdownBodyStart,
  splitLeadingFrontmatter,
} from "../lib/frontmatter";
import { registerNoteEditorPositionCapture } from "../stores/editorPositions";
import { parseLiveMarkdownTables } from "../lib/liveMarkdownTable";
import {
  insertLiveMarkdownTableLineBreak,
  insertLiveMarkdownTableRow,
  navigateLiveMarkdownTable,
} from "../lib/liveMarkdownTableNavigation";
import { toggleInlineFormatting, wrapInlineCode } from "../lib/markdownFormatting";
import { normalizeWikiTarget, wikiTargetTitle } from "../lib/wikiLinks";
import type { Extension, SelectionRange } from "@codemirror/state";
import type { Command, ViewUpdate } from "@codemirror/view";
import type { MarkdownSelectionEdit } from "../lib/markdownFormatting";
import type { LiveMarkdownTextEdit } from "../lib/liveMarkdown";
import type { NoteEditorPosition } from "../types";
import AppIcon from "./AppIcon.vue";

const props = defineProps<{
  initialPosition?: NoteEditorPosition;
  modelValue: string;
  noteId: string;
  noteTitles: string[];
  showFrontmatter: boolean;
  vaultId: string;
}>();

const emit = defineEmits<{
  editorPosition: [vaultId: string, noteId: string, position: NoteEditorPosition];
  openLink: [href: string];
  openWiki: [target: string];
  "update:modelValue": [value: string];
}>();

const editorHost = ref<HTMLElement>();
const editorView = shallowRef<EditorView>();
const suggestionIndex = ref(0);
const suggestionQuery = ref<string | null>(null);
const externalUpdate = Annotation.define<boolean>();
const historyCompartment = new Compartment();
const lineNumbersCompartment = new Compartment();
const INDENT = "  ";
const VIEWPORT_ANCHOR_MARGIN = 8;
const VIRTUALIZED_VIEWPORT_THRESHOLD = 200;
let outputLineEnding = preferredLineEnding(props.modelValue);
let frontmatterHistoryChanged = false;
let frontmatterLineOffset = 0;
let frontmatterPrefix = "";
let positionCaptureEnabled = false;
let removePositionCapture: (() => boolean) | undefined;
let viewportRestoreFrame: number | undefined;

const positionCaptureKey = {};
const viewportRestoreKey = {};

interface ListLine {
  indent: string;
  ordered: boolean;
  number: number;
  delimiter: "." | ")";
  bullet: "-" | "+" | "*";
  spacing: string;
  contentOffset: number;
  task: boolean;
}

interface TextEdit {
  start: number;
  removed: number;
  added: number;
}

const normalizedNoteTitles = computed(() => new Set(
  props.noteTitles.flatMap((title) => [
    normalizeInlineLinkTarget(title),
    normalizeInlineLinkTarget(wikiTargetTitle(title)),
  ]),
));

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

const {
  closeSearch: closeDocumentSearch,
  handleEditorSearchKeydown: handleDocumentSearchKeydown,
  handleSearchInputKeydown: handleDocumentSearchInputKeydown,
  isOpen: documentSearchOpen,
  matchCount: documentSearchMatchCount,
  moveToMatch: moveToDocumentSearchMatch,
  openSearch: openDocumentSearch,
  query: documentSearchQuery,
  refreshSearch: refreshDocumentSearch,
  searchInput: documentSearchInput,
  statusText: documentSearchStatus,
} = useCodeMirrorDocumentSearch(editorView);

const suggestionDown: Command = (view) => {
  if (view.composing || !suggestions.value.length) {
    return false;
  }
  suggestionIndex.value = (suggestionIndex.value + 1) % suggestions.value.length;

  return true;
};

const suggestionUp: Command = (view) => {
  if (view.composing || !suggestions.value.length) {
    return false;
  }
  suggestionIndex.value = (
    suggestionIndex.value - 1 + suggestions.value.length
  ) % suggestions.value.length;

  return true;
};

const acceptSuggestion: Command = (view) => {
  if (view.composing || !suggestions.value.length) {
    return false;
  }

  insertSuggestion(suggestions.value[suggestionIndex.value]!);

  return true;
};

const closeSuggestions: Command = (view) => {
  if (view.composing || suggestionQuery.value === null) {
    return false;
  }
  suggestionQuery.value = null;

  return true;
};

const handleEnter: Command = (view) => {
  if (view.composing) {
    return false;
  }

  const value = view.state.doc.toString();
  const selection = view.state.selection.main;
  const tableEdit = insertLiveMarkdownTableRow(
    value,
    parseLiveMarkdownTables(value),
    selection.head,
  );
  if (tableEdit) {
    applyFullDocumentEdit(
      view,
      tableEdit.value,
      tableEdit.selectionStart,
      tableEdit.selectionEnd,
      "input.table",
    );

    return true;
  }
  if (handleSmartEnter(view)) {
    return true;
  }

  return insertNewline(view);
};

const handleShiftEnter: Command = (view) => {
  if (view.composing) {
    return false;
  }

  const value = view.state.doc.toString();
  const selection = view.state.selection.main;
  const tableEdit = insertLiveMarkdownTableLineBreak(
    value,
    parseLiveMarkdownTables(value),
    selection.anchor,
    selection.head,
  );
  if (!tableEdit) {
    return false;
  }

  applyFullDocumentEdit(
    view,
    tableEdit.value,
    tableEdit.selectionStart,
    tableEdit.selectionEnd,
    "input.table",
  );

  return true;
};

const handleTab: Command = (view) => {
  if (view.composing) {
    return false;
  }

  const value = view.state.doc.toString();
  const selection = view.state.selection.main;
  const tableEdit = navigateLiveMarkdownTable(
    value,
    parseLiveMarkdownTables(value),
    selection.from,
    selection.to,
    "next-cell",
  );
  if (tableEdit) {
    applyFullDocumentEdit(
      view,
      tableEdit.value,
      tableEdit.selectionStart,
      tableEdit.selectionEnd,
      "select.table",
    );

    return true;
  }
  if (adjustSelectedLines(view, false)) {
    return true;
  }

  view.dispatch({
    changes: { from: selection.from, to: selection.to, insert: INDENT },
    selection: EditorSelection.cursor(selection.from + INDENT.length),
    scrollIntoView: true,
    userEvent: "input",
  });

  return true;
};

const handleShiftTab: Command = (view) => {
  if (view.composing) {
    return false;
  }

  const value = view.state.doc.toString();
  const selection = view.state.selection.main;
  const tableEdit = navigateLiveMarkdownTable(
    value,
    parseLiveMarkdownTables(value),
    selection.from,
    selection.to,
    "previous-cell",
  );
  if (tableEdit) {
    applyFullDocumentEdit(
      view,
      tableEdit.value,
      tableEdit.selectionStart,
      tableEdit.selectionEnd,
      "select.table",
    );

    return true;
  }

  adjustSelectedLines(view, true);

  return true;
};

const toggleBold: Command = (view) => toggleSelectionFormatting(view, "**", ["__"]);
const toggleItalic: Command = (view) => toggleSelectionFormatting(view, "*", ["_"]);
const toggleStrikethrough: Command = (view) => toggleSelectionFormatting(view, "~~");
const insertLiteralApostrophe: Command = (view) => {
  if (view.composing) {
    return false;
  }

  view.dispatch(
    view.state.replaceSelection("'"),
    {
      scrollIntoView: true,
      userEvent: "input.type",
    },
  );

  return true;
};
const insertLiteralHyphen: Command = (view) => {
  if (view.composing) {
    return false;
  }

  view.dispatch(
    view.state.replaceSelection("-"),
    {
      scrollIntoView: true,
      userEvent: "input.type",
    },
  );

  return true;
};
const wrapSelectionAsInlineCode: Command = (view) => {
  if (view.composing) {
    return false;
  }

  const selection = view.state.selection.main;
  if (selection.empty) {
    return false;
  }

  return applyMarkdownSelectionEdit(
    view,
    wrapInlineCode(
      view.state.doc.toString(),
      selection.from,
      selection.to,
    ),
  );
};

const moveToRenderedListTextStart: Command = (view) => setRenderedListTextStart(view, false);
const selectRenderedListTextStart: Command = (view) => setRenderedListTextStart(view, true);
const revealRenderedListSourceFromRight: Command = (view) => {
  if (view.composing || view.state.selection.ranges.length !== 1) {
    return false;
  }

  const selection = view.state.selection.main;
  if (!selection.empty) {
    return false;
  }

  const line = view.state.doc.lineAt(selection.head);
  const textOffset = renderedListTextOffset(line.text);
  if (textOffset === undefined) {
    return false;
  }

  const textStart = line.from + textOffset;
  if (selection.head !== textStart || textStart <= line.from) {
    return false;
  }

  view.dispatch({
    selection: EditorSelection.cursor(textStart - 1),
    scrollIntoView: true,
    userEvent: "select",
  });

  return true;
};

function setRenderedListTextStart(
  view: EditorView,
  extendSelection: boolean,
): boolean {
  if (view.composing || view.state.selection.ranges.length !== 1) {
    return false;
  }

  const selection = view.state.selection.main;
  const line = view.state.doc.lineAt(selection.head);
  const textOffset = renderedListTextOffset(line.text);
  if (textOffset === undefined) {
    return false;
  }

  const textStart = line.from + textOffset;
  if (selection.head < textStart) {
    return false;
  }
  if (
    selection.head > textStart
    && previousLineBoundary(view, selection) > textStart
  ) {
    return false;
  }
  if (
    selection.head === textStart
    && (extendSelection || selection.empty)
  ) {
    return true;
  }

  view.dispatch({
    selection: extendSelection
      ? EditorSelection.range(selection.anchor, textStart)
      : EditorSelection.cursor(textStart),
    scrollIntoView: true,
    userEvent: "select",
  });

  return true;
}

onMounted(() => {
  const host = editorHost.value;
  if (!host) {
    return;
  }

  const editableDocument = projectEditableDocument(
    normalizeDocumentText(props.modelValue),
    props.showFrontmatter,
  );
  frontmatterLineOffset = editableDocument.lineNumberOffset;
  frontmatterPrefix = editableDocument.prefix;
  const initialPosition = normalizeInitialPosition(
    props.initialPosition,
    editableDocument.body.length,
    editableDocument.bodyStart,
  );
  const view = new EditorView({
    parent: host,
    state: EditorState.create({
      doc: editableDocument.body,
      selection: initialPosition
        ? EditorSelection.range(initialPosition.selection.anchor, initialPosition.selection.head)
        : undefined,
      extensions: [
        lineNumbersCompartment.of(editorLineNumbers(frontmatterLineOffset)),
        highlightSpecialChars(),
        historyCompartment.of(history()),
        drawSelection(),
        dropCursor(),
        EditorState.tabSize.of(4),
        EditorView.lineWrapping,
        EditorView.contentAttributes.of({
          "aria-label": "Markdown source",
          autocapitalize: "sentences",
          class: "source-textarea",
          spellcheck: "true",
        }),
        markdown({
          addKeymap: false,
          base: markdownLanguage,
          completeHTMLTags: false,
          pasteURLAsLink: false,
        }),
        liveMarkdownExtension({
          openLink: openLiveMarkdownLink,
          openWiki: openLiveMarkdownWikiLink,
          wikiLinkIsResolved: inlineWikiLinkIsResolved,
        }),
        codeMirrorDocumentSearchExtension,
        Prec.high(keymap.of([
          { key: "ArrowDown", run: suggestionDown },
          { key: "ArrowUp", run: suggestionUp },
          { key: "Enter", run: acceptSuggestion },
          { key: "Escape", run: closeSuggestions },
          { key: "Shift-Enter", run: handleShiftEnter },
          { key: "Enter", run: handleEnter },
          { key: "Tab", run: handleTab },
          { key: "Shift-Tab", run: handleShiftTab },
          { key: "Mod-b", run: toggleBold },
          { key: "Mod-i", run: toggleItalic },
          { key: "Mod-Shift-x", run: toggleStrikethrough },
          { key: "'", run: insertLiteralApostrophe },
          { key: "-", run: insertLiteralHyphen },
          { key: "`", run: wrapSelectionAsInlineCode },
          { key: "ArrowLeft", run: revealRenderedListSourceFromRight },
          {
            key: "Home",
            run: moveToRenderedListTextStart,
            shift: selectRenderedListTextStart,
          },
          {
            mac: "Cmd-ArrowLeft",
            run: moveToRenderedListTextStart,
            shift: selectRenderedListTextStart,
          },
        ])),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        EditorView.updateListener.of((update) => {
          const localDocumentChange = update.docChanged && update.transactions.some(
            (transaction) => transaction.docChanged && !transaction.annotation(externalUpdate),
          );
          if (localDocumentChange) {
            if (
              props.showFrontmatter
              && changeTouchesLeadingFrontmatter(update)
            ) {
              frontmatterHistoryChanged = true;
            }
            emit(
              "update:modelValue",
              restoreLineEndings(
                joinLeadingFrontmatter(
                  frontmatterPrefix,
                  update.state.doc.toString(),
                ),
                outputLineEnding,
              ),
            );
            refreshDocumentSearch();
          }
          if (update.docChanged || update.selectionSet) {
            updateSuggestions(update.view);
            schedulePositionCapture(update.view);
          }
        }),
      ],
    }),
    scrollTo: initialPosition
      ? EditorView.scrollIntoView(initialPosition.viewport.anchor, { x: "start", y: "start" })
      : undefined,
  });
  editorView.value = view;
  removePositionCapture = registerNoteEditorPositionCapture(
    props.vaultId,
    props.noteId,
    () => positionCaptureEnabled ? captureEditorPosition(view) : undefined,
  );
  view.scrollDOM.addEventListener("scroll", handleEditorScroll, { passive: true });
  updateSuggestions(view);
  view.focus();

  if (initialPosition) {
    scheduleViewportRestore(view, initialPosition);
  } else {
    positionCaptureEnabled = true;
    schedulePositionCapture(view);
  }
  window.requestAnimationFrame(() => {
    if (editorView.value !== view || !view.dom.isConnected) {
      return;
    }

    const scrollLeft = view.scrollDOM.scrollLeft;
    const scrollTop = view.scrollDOM.scrollTop;
    view.focus();
    view.scrollDOM.scrollLeft = scrollLeft;
    view.scrollDOM.scrollTop = scrollTop;
  });
});

onBeforeUnmount(() => {
  const view = editorView.value;
  const captureWasActive = removePositionCapture?.() ?? false;
  removePositionCapture = undefined;
  if (viewportRestoreFrame !== undefined) {
    window.cancelAnimationFrame(viewportRestoreFrame);
    viewportRestoreFrame = undefined;
  }
  if (view) {
    view.scrollDOM.removeEventListener("scroll", handleEditorScroll);
    if (positionCaptureEnabled && captureWasActive) {
      emitEditorPosition(captureEditorPosition(view));
    }
    view.destroy();
  }
  editorView.value = undefined;
});

watch(
  [() => props.modelValue, () => props.showFrontmatter],
  ([value, showFrontmatter], [, previouslyShowingFrontmatter]) => {
    const view = editorView.value;
    outputLineEnding = preferredLineEnding(value);
    const visibilityChanged = showFrontmatter !== previouslyShowingFrontmatter;
    const normalizedValue = visibilityChanged && view
      ? joinLeadingFrontmatter(
          frontmatterPrefix,
          view.state.doc.toString(),
        )
      : normalizeDocumentText(value);

    const editableDocument = projectEditableDocument(
      normalizedValue,
      showFrontmatter,
    );
    const prefixChanged = editableDocument.prefix !== frontmatterPrefix;
    const lineOffsetChanged = (
      editableDocument.lineNumberOffset !== frontmatterLineOffset
    );
    if (!view) {
      frontmatterPrefix = editableDocument.prefix;
      frontmatterLineOffset = editableDocument.lineNumberOffset;

      return;
    }

    const currentBody = view.state.doc.toString();
    const currentBodyStart = markdownBodyStart(frontmatterPrefix, currentBody);
    const bodyChanged = editableDocument.body !== currentBody;
    const coordinateSpaceChanged = bodyChanged
      && editableDocument.bodyStart !== currentBodyStart;
    const resetHistory = coordinateSpaceChanged && (
      !visibilityChanged
      || (previouslyShowingFrontmatter && frontmatterHistoryChanged)
    );
    const selection = visibilityChanged
      ? frontmatterVisibilitySelection(
          view.state.selection.main,
          currentBodyStart,
          editableDocument,
        )
      : undefined;
    frontmatterPrefix = editableDocument.prefix;
    frontmatterLineOffset = editableDocument.lineNumberOffset;
    if (resetHistory) {
      // Projection changes invalidate history entries that target the hidden prefix
      view.dispatch({
        effects: historyCompartment.reconfigure([]),
      });
    }
    if (bodyChanged || lineOffsetChanged) {
      view.dispatch({
        ...(bodyChanged
          ? {
              changes: minimalDocumentChange(currentBody, editableDocument.body),
              annotations: [
                externalUpdate.of(true),
                Transaction.addToHistory.of(false),
              ],
            }
          : {}),
        ...(lineOffsetChanged
          ? {
              effects: lineNumbersCompartment.reconfigure(
                editorLineNumbers(frontmatterLineOffset),
              ),
            }
          : {}),
        ...(selection ? { selection } : {}),
        scrollIntoView: visibilityChanged,
      });
    }
    if (bodyChanged) {
      refreshDocumentSearch();
    }
    if (prefixChanged) {
      schedulePositionCapture(view);
    }
    if (resetHistory) {
      view.dispatch({
        effects: historyCompartment.reconfigure(history()),
      });
    }
    if (visibilityChanged) {
      frontmatterHistoryChanged = false;
      view.focus();
    }
  },
);

watch(
  () => props.noteTitles,
  () => {
    editorView.value?.dispatch({
      effects: refreshLiveMarkdownEffect.of(null),
    });
  },
  { deep: true },
);

function openLiveMarkdownLink(href: string): void {
  emit("openLink", href);
}

function openLiveMarkdownWikiLink(target: string): void {
  emit("openWiki", target);
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

function updateSuggestions(view: EditorView): void {
  const selection = view.state.selection.main;
  if (!selection.empty) {
    suggestionQuery.value = null;

    return;
  }

  const line = view.state.doc.lineAt(selection.head);
  const beforeCursor = view.state.doc.sliceString(line.from, selection.head);
  const match = beforeCursor.match(/\[\[([^\]\n|#]*)$/);
  suggestionQuery.value = match ? match[1] ?? "" : null;
  suggestionIndex.value = 0;
}

function insertSuggestion(title: string): void {
  const view = editorView.value;
  if (!view || suggestionQuery.value === null) {
    return;
  }

  const cursor = view.state.selection.main.head;
  const start = cursor - suggestionQuery.value.length;
  const replacement = `${title}]]`;
  suggestionQuery.value = null;
  view.dispatch({
    changes: { from: start, to: cursor, insert: replacement },
    selection: EditorSelection.cursor(start + replacement.length),
    scrollIntoView: true,
    userEvent: "input.complete",
  });
  view.focus();
}

function toggleSelectionFormatting(
  view: EditorView,
  marker: string,
  alternatives: string[] = [],
): boolean {
  if (view.composing) {
    return false;
  }

  const selection = view.state.selection.main;

  return applyMarkdownSelectionEdit(
    view,
    toggleInlineFormatting(
      view.state.doc.toString(),
      selection.from,
      selection.to,
      marker,
      alternatives,
    ),
  );
}

function applyMarkdownSelectionEdit(
  view: EditorView,
  edit: MarkdownSelectionEdit,
): boolean {
  const value = view.state.doc.toString();
  if (edit.value === value) {
    return true;
  }

  const change = minimalDocumentChange(value, edit.value);
  const backward = view.state.selection.main.anchor > view.state.selection.main.head;
  view.dispatch({
    changes: change,
    selection: backward
      ? EditorSelection.range(edit.selectionEnd, edit.selectionStart)
      : EditorSelection.range(edit.selectionStart, edit.selectionEnd),
    scrollIntoView: true,
    userEvent: "input.format",
  });

  return true;
}

function applyFullDocumentEdit(
  view: EditorView,
  value: string,
  selectionStart: number,
  selectionEnd = selectionStart,
  userEvent = "input",
): void {
  view.dispatch({
    changes: minimalDocumentChange(view.state.doc.toString(), value),
    selection: EditorSelection.range(selectionStart, selectionEnd),
    scrollIntoView: true,
    userEvent,
  });
}

function minimalDocumentChange(
  current: string,
  next: string,
): { from: number; to: number; insert: string } {
  let from = 0;
  const sharedLength = Math.min(current.length, next.length);
  while (from < sharedLength && current[from] === next[from]) {
    from += 1;
  }

  let currentTo = current.length;
  let nextTo = next.length;
  while (
    currentTo > from &&
    nextTo > from &&
    current[currentTo - 1] === next[nextTo - 1]
  ) {
    currentTo -= 1;
    nextTo -= 1;
  }

  return {
    from,
    to: currentTo,
    insert: next.slice(from, nextTo),
  };
}

function preferredLineEnding(value: string): "\n" | "\r" | "\r\n" {
  const lineEnding = value.match(/\r\n|\r|\n/)?.[0];

  return lineEnding === "\r\n" || lineEnding === "\r"
    ? lineEnding
    : "\n";
}

function normalizeDocumentText(value: string): string {
  return value.replace(/\r\n|\r/g, "\n");
}

function projectEditableDocument(
  normalizedMarkdown: string,
  showFrontmatter: boolean,
) {
  if (!showFrontmatter) {
    return splitLeadingFrontmatter(normalizedMarkdown);
  }

  return {
    body: normalizedMarkdown,
    bodyStart: 0,
    lineNumberOffset: 0,
    prefix: "",
  };
}

function frontmatterVisibilitySelection(
  selection: SelectionRange,
  currentBodyStart: number,
  editableDocument: ReturnType<typeof projectEditableDocument>,
): SelectionRange {
  const clamp = (position: number): number => Math.min(
    editableDocument.body.length,
    Math.max(0, position + currentBodyStart - editableDocument.bodyStart),
  );

  return EditorSelection.range(
    clamp(selection.anchor),
    clamp(selection.head),
  );
}

function changeTouchesLeadingFrontmatter(update: ViewUpdate): boolean {
  const previousEnd = leadingFrontmatterEnd(update.startState.doc.toString()) ?? 0;
  const nextEnd = leadingFrontmatterEnd(update.state.doc.toString()) ?? 0;
  let touchesFrontmatter = previousEnd !== nextEnd;
  update.changes.iterChangedRanges((fromA, _toA, fromB) => {
    if (fromA < previousEnd || fromB < nextEnd) {
      touchesFrontmatter = true;
    }
  });

  return touchesFrontmatter;
}

function normalizeInitialPosition(
  position: NoteEditorPosition | undefined,
  documentLength: number,
  bodyStart: number,
): NoteEditorPosition | undefined {
  if (!position) {
    return undefined;
  }

  const clamp = (value: number): number => Math.min(
    documentLength,
    Math.max(0, Math.trunc(value) - bodyStart),
  );

  return {
    selection: {
      anchor: clamp(position.selection.anchor),
      head: clamp(position.selection.head),
    },
    viewport: {
      anchor: clamp(position.viewport.anchor),
      offset: position.viewport.offset,
      left: Math.max(0, position.viewport.left),
    },
  };
}

function handleEditorScroll(): void {
  const view = editorView.value;
  if (view) {
    schedulePositionCapture(view);
  }
}

function schedulePositionCapture(view: EditorView): void {
  if (!positionCaptureEnabled) {
    return;
  }

  view.requestMeasure({
    key: positionCaptureKey,
    read: captureEditorPosition,
    write: emitEditorPosition,
  });
}

function captureEditorPosition(view: EditorView): NoteEditorPosition {
  const selection = view.state.selection.main;
  const bodyStart = markdownBodyStart(
    frontmatterPrefix,
    view.state.doc.toString(),
  );
  const scrollTop = view.scrollDOM.scrollTop;
  const candidate = view.lineBlockAtHeight(scrollTop + VIEWPORT_ANCHOR_MARGIN);
  const firstVisibleBlock = view.viewportLineBlocks[0];
  const viewportBlock = candidate.from >= view.viewport.from
    || !firstVisibleBlock
    || firstVisibleBlock.top - scrollTop > VIRTUALIZED_VIEWPORT_THRESHOLD
    ? candidate
    : firstVisibleBlock;

  return {
    selection: {
      anchor: selection.anchor + bodyStart,
      head: selection.head + bodyStart,
    },
    viewport: {
      anchor: viewportBlock.from + bodyStart,
      offset: viewportBlock.top - scrollTop,
      left: view.scrollDOM.scrollLeft,
    },
  };
}

function editorLineNumbers(offset: number): Extension {
  return lineNumbers({
    formatNumber: (lineNumber) => String(lineNumber + offset),
  });
}

function emitEditorPosition(position: NoteEditorPosition): void {
  emit("editorPosition", props.vaultId, props.noteId, position);
}

function scheduleViewportRestore(
  view: EditorView,
  position: NoteEditorPosition,
): void {
  viewportRestoreFrame = window.requestAnimationFrame(() => {
    viewportRestoreFrame = undefined;
    if (editorView.value !== view) {
      return;
    }

    view.requestMeasure({
      key: viewportRestoreKey,
      read: (measuredView) => {
        const viewportBlock = measuredView.lineBlockAt(position.viewport.anchor);
        const maximumTop = Math.max(
          0,
          measuredView.scrollDOM.scrollHeight - measuredView.scrollDOM.clientHeight,
        );
        const maximumLeft = Math.max(
          0,
          measuredView.scrollDOM.scrollWidth - measuredView.scrollDOM.clientWidth,
        );

        return {
          left: Math.min(maximumLeft, position.viewport.left),
          top: Math.min(
            maximumTop,
            Math.max(0, viewportBlock.top - position.viewport.offset),
          ),
        };
      },
      write: ({ left, top }, measuredView) => {
        measuredView.scrollDOM.scrollLeft = left;
        measuredView.scrollDOM.scrollTop = top;
        positionCaptureEnabled = true;
        schedulePositionCapture(measuredView);
      },
    });
  });
}

function restoreLineEndings(
  value: string,
  lineEnding: "\n" | "\r" | "\r\n",
): string {
  return lineEnding === "\n" ? value : value.replace(/\n/g, lineEnding);
}

function handleSmartEnter(view: EditorView): boolean {
  const selection = view.state.selection.main;
  if (!selection.empty) {
    return false;
  }

  const value = view.state.doc.toString();
  const position = selection.head;
  const line = view.state.doc.lineAt(position);
  const source = line.text;
  const openingFence = source.match(/^([ \t]*)(`{3,}|~{3,})([^\n]*)$/);
  if (
    openingFence &&
    position === line.to &&
    !activeFenceBefore(value, line.from)
  ) {
    const indent = openingFence[1]!;
    const marker = openingFence[2]!;
    const followingLineStart = line.to < value.length ? line.to + 1 : value.length;
    const followingLine = view.state.doc.lineAt(followingLineStart).text;
    const existingClosing = followingLine.match(/^([ \t]*)(`+|~+)\s*$/);
    const closesFence = Boolean(existingClosing
      && existingClosing[2]![0] === marker[0]
      && existingClosing[2]!.length >= marker.length);
    const insertion = closesFence
      ? `\n${indent}`
      : `\n${indent}\n${indent}${marker}`;
    view.dispatch({
      changes: { from: position, insert: insertion },
      selection: EditorSelection.cursor(position + 1 + indent.length),
      scrollIntoView: true,
      userEvent: "input",
    });

    return true;
  }

  if (activeFenceBefore(value, line.from)) {
    return false;
  }

  const item = matchEditableListLine(source);
  if (!item) {
    return false;
  }

  const contentStart = line.from + item.contentOffset;
  const insertionPosition = Math.max(position, contentStart);
  const bodyBefore = value.slice(contentStart, insertionPosition);
  const bodyAfter = value.slice(insertionPosition, line.to);
  const fullBody = `${bodyBefore}${bodyAfter}`;

  if (!fullBody.trim() && insertionPosition === line.to) {
    const replacement = item.indent;
    view.dispatch({
      changes: { from: line.from, to: line.to, insert: replacement },
      selection: EditorSelection.cursor(line.from + replacement.length),
      scrollIntoView: true,
      userEvent: "input",
    });

    return true;
  }

  const marker = item.ordered
    ? `${item.number >= 999_999_999 ? 1 : item.number + 1}${item.delimiter}`
    : item.bullet;
  const taskPrefix = item.task ? "[ ] " : "";
  const continuation = `${item.indent}${marker}${item.spacing}${taskPrefix}`;
  const separatorLength = bodyAfter.match(/^[ \t]+/)?.[0].length ?? 0;
  const next = `${value.slice(0, insertionPosition)}\n${continuation}${value.slice(
    insertionPosition + separatorLength,
  )}`;
  const cursor = insertionPosition + 1 + continuation.length;
  const normalized = item.ordered
    ? normalizeOrderedListMarkers(next)
    : { edits: [], value: next };
  const normalizedCursor = mapPositionThroughLiveMarkdownEdits(
    cursor,
    normalized.edits,
  );
  applyFullDocumentEdit(
    view,
    normalized.value,
    normalizedCursor,
    normalizedCursor,
  );

  return true;
}

function matchEditableListLine(line: string): ListLine | undefined {
  const match = line.match(/^([ \t]*)(?:(\d{1,9})([.)])|([-+*]))([ \t]+)(.*)$/);
  if (!match) {
    return undefined;
  }
  const ordered = Boolean(match[2]);
  const markerLength = ordered ? match[2]!.length + 1 : 1;
  const spacing = match[5]!;
  const listPrefixLength = match[1]!.length + markerLength + spacing.length;
  const taskPrefix = match[6]!.match(/^\[[ xX]\][ \t]+/)?.[0];

  return {
    indent: match[1]!,
    ordered,
    number: ordered ? Number.parseInt(match[2]!, 10) : 1,
    delimiter: ordered ? match[3]! as "." | ")" : ".",
    bullet: ordered ? "-" : match[4]! as "-" | "+" | "*",
    spacing,
    contentOffset: listPrefixLength + (taskPrefix?.length ?? 0),
    task: Boolean(taskPrefix),
  };
}

function renderedListTextOffset(line: string): number | undefined {
  const item = matchEditableListLine(line);
  if (!item) {
    return undefined;
  }

  return item.contentOffset;
}

function previousLineBoundary(
  view: EditorView,
  selection: SelectionRange,
): number {
  const line = view.lineBlockAt(selection.head);
  let boundary = view.moveToLineBoundary(selection, false);
  if (boundary.head === selection.head && boundary.head !== line.from) {
    boundary = view.moveToLineBoundary(selection, false, false);
  }

  return boundary.head;
}

function activeFenceBefore(
  value: string,
  position: number,
): { marker: string; length: number } | undefined {
  const lines = value.slice(0, position).split("\n");
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
    ) {
      active = undefined;
    }
  }

  return active;
}

function adjustSelectedLines(view: EditorView, outdent: boolean): boolean {
  const value = view.state.doc.toString();
  const selection = view.state.selection.main;
  const selectionStart = selection.from;
  const selectionEnd = selection.to;
  const hasSelection = !selection.empty;
  const firstLine = view.state.doc.lineAt(selectionStart);

  if (!hasSelection && !matchEditableListLine(firstLine.text)) {
    if (!outdent || !/^[ \t]/.test(firstLine.text)) {
      return false;
    }
  }

  const selectionEndsAtLineStart = hasSelection
    && selectionEnd > 0
    && value[selectionEnd - 1] === "\n";
  const effectiveEnd = selectionEndsAtLineStart ? selectionEnd - 1 : selectionEnd;
  const lastLine = view.state.doc.lineAt(effectiveEnd);
  const block = value.slice(firstLine.from, lastLine.to);
  const lines = block.split("\n");
  const orderedListChanged = lines.some((line) =>
    matchEditableListLine(line)?.ordered
  );
  const edits: TextEdit[] = [];
  let sourceOffset = firstLine.from;

  const transformed = lines.map((line) => {
    if (!outdent) {
      edits.push({ start: sourceOffset, removed: 0, added: INDENT.length });
      sourceOffset += line.length + 1;

      return `${INDENT}${line}`;
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

  const adjusted = `${value.slice(0, firstLine.from)}${transformed}${value.slice(lastLine.to)}`;
  const mappedStart = mapPositionThroughEdits(selectionStart, edits);
  const mappedEnd = mapPositionThroughEdits(selectionEnd, edits);
  const normalized = orderedListChanged
    ? normalizeOrderedListMarkers(adjusted)
    : { edits: [], value: adjusted };
  const normalizedStart = mapPositionThroughLiveMarkdownEdits(
    mappedStart,
    normalized.edits,
  );
  const normalizedEnd = mapPositionThroughLiveMarkdownEdits(
    mappedEnd,
    normalized.edits,
  );
  const backward = selection.anchor > selection.head;
  view.dispatch({
    changes: minimalDocumentChange(value, normalized.value),
    selection: backward
      ? EditorSelection.range(normalizedEnd, normalizedStart)
      : EditorSelection.range(normalizedStart, normalizedEnd),
    scrollIntoView: true,
    userEvent: "input.indent",
  });

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

function mapPositionThroughLiveMarkdownEdits(
  position: number,
  edits: readonly LiveMarkdownTextEdit[],
): number {
  return mapPositionThroughEdits(
    position,
    edits.map((edit) => ({
      start: edit.from,
      removed: edit.to - edit.from,
      added: edit.insert.length,
    })),
  );
}
</script>

<template>
  <div
    class="source-editor"
    :class="{ 'is-searching': documentSearchOpen }"
    @keydown.capture="handleDocumentSearchKeydown"
  >
    <div ref="editorHost" class="code-mirror-host" />

    <Transition name="popover-fade">
      <form
        v-if="documentSearchOpen"
        class="document-search-bar"
        data-ui-region="document-search"
        role="search"
        aria-label="Find in note"
        @submit.prevent="moveToDocumentSearchMatch('next')"
      >
        <label class="document-search-field">
          <AppIcon name="search" :size="14" />
          <input
            ref="documentSearchInput"
            v-model="documentSearchQuery"
            type="search"
            placeholder="Find in note"
            aria-label="Find in note"
            aria-describedby="document-search-status"
            autocomplete="off"
            autocapitalize="none"
            spellcheck="false"
            @keydown="handleDocumentSearchInputKeydown"
          />
        </label>
        <span
          id="document-search-status"
          class="document-search-status"
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >{{ documentSearchStatus }}</span>
        <button
          type="button"
          class="document-search-button previous"
          :disabled="!documentSearchMatchCount"
          aria-label="Previous match"
          aria-keyshortcuts="Shift+Tab Shift+F3"
          title="Previous match (Shift+Tab)"
          @mousedown.prevent
          @click="moveToDocumentSearchMatch('previous')"
        >
          <AppIcon name="chevron-down" :size="13" />
        </button>
        <button
          type="button"
          class="document-search-button"
          :disabled="!documentSearchMatchCount"
          aria-label="Next match"
          aria-keyshortcuts="Tab F3"
          title="Next match (Tab)"
          @mousedown.prevent
          @click="moveToDocumentSearchMatch('next')"
        >
          <AppIcon name="chevron-down" :size="13" />
        </button>
        <button
          type="button"
          class="document-search-button close"
          aria-label="Close find in note"
          title="Close (Escape)"
          @mousedown.prevent
          @click="closeDocumentSearch"
        >
          <AppIcon name="x" :size="13" />
        </button>
      </form>

      <button
        v-else
        type="button"
        class="document-search-trigger"
        aria-label="Find in note"
        aria-keyshortcuts="Control+F Meta+F"
        title="Find in note"
        @click="openDocumentSearch"
      >
        <AppIcon name="search" :size="14" />
      </button>
    </Transition>

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
