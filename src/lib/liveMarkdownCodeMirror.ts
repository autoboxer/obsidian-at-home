import { syntaxTree } from "@codemirror/language";
import {
  EditorSelection,
  EditorState,
  RangeSet,
  StateEffect,
} from "@codemirror/state";
import {
  Decoration,
  EditorView,
  ViewPlugin,
} from "@codemirror/view";
import { documentSearchMatches } from "./codeMirrorDocumentSearch";
import { parseMarkdownHeadingTarget } from "./headingLinks";
import { highlightCodeRanges } from "./highlight";
import { parsePairedInlineMarkup } from "./inlineMarkup";
import { parseLiveMarkdownCodeFences } from "./liveMarkdownCode";
import {
  liveMarkdownDocumentModel,
  liveMarkdownDocumentModelField,
  liveMarkdownDocumentModelForText,
} from "./liveMarkdownDocumentModel";
import { liveMarkdownHeadingFoldingExtension } from "./liveMarkdownHeadingFolding";
import {
  HorizontalRuleWidget,
  ListMarkerWidget,
  QuoteMarkerWidget,
  renderedListMarker,
  TaskWidget,
  WikiLinkWidget,
} from "./liveMarkdownCodeMirrorWidgets";
import {
  CodeFenceFooterWidget,
  CodeFenceHeaderWidget,
  EmptyTableCellWidget,
  TableCellBreakWidget,
  TableDelimiterWidget,
} from "./liveMarkdownRegionWidgets";
import { sanitizeLinkUrl } from "./markdown";
import { parseWikiLinks } from "./wikiLinks";
import type {
  Extension,
  SelectionRange,
  Text,
} from "@codemirror/state";
import type {
  DecorationSet,
  MouseSelectionStyle,
  ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import type { SyntaxNode, SyntaxNodeRef, Tree } from "@lezer/common";
import type { InlineMarkupKind } from "./inlineMarkup";
import type { LiveMarkdownBlock, LiveMarkdownRange } from "./liveMarkdown";
import type { LiveMarkdownCodeFence } from "./liveMarkdownCode";
import type {
  LiveMarkdownTable,
  LiveMarkdownTableAlignment,
  LiveMarkdownTableRow,
} from "./liveMarkdownTable";

export interface LiveMarkdownOptions {
  readonly documentId: string;
  readonly openLink: (href: string) => void;
  readonly openWiki: (target: string, heading?: string) => void;
  readonly wikiLinkIsResolved: (target: string) => boolean;
}

interface HiddenSyntax extends LiveMarkdownRange {
  widget?: WidgetType;
}

interface LiveConstruct {
  atomicRanges: LiveMarkdownRange[];
  boundaryReveal: "construct" | "none" | "syntax";
  from: number;
  renderedDecorations: StoredDecoration[];
  revealWithinSyntax: boolean;
  to: number;
  syntax: HiddenSyntax[];
}

interface LiveConstructOptions {
  atomicRanges?: readonly LiveMarkdownRange[];
  boundaryReveal?: LiveConstruct["boundaryReveal"];
  revealWithinSyntax?: boolean;
}

interface StoredDecoration extends LiveMarkdownRange {
  decoration: Decoration;
}

interface InlineMarkupSpan extends LiveMarkdownRange {
  contentFrom: number;
  contentTo: number;
  kind: InlineMarkupKind | "code";
  syntax: LiveMarkdownRange[];
}

interface LiveMarkdownModel {
  constructs: LiveConstruct[];
  decorations: StoredDecoration[];
}

const LIST_INDENT_STEP_EM = 1.65;
const inlineMarkupSpanCache = new WeakMap<Text, InlineMarkupSpan[]>();

export const refreshLiveMarkdownEffect = StateEffect.define<null>();

const liveMarkdownPlugin = ViewPlugin.fromClass(
  class LiveMarkdownView {
    atomicRanges: DecorationSet = RangeSet.empty;
    decorations: DecorationSet = Decoration.none;
    private model: LiveMarkdownModel;
    private parsedTree: ReturnType<typeof syntaxTree>;
    private searchMatches: readonly LiveMarkdownRange[];
    private wikiResolutionVersion = 0;

    constructor(
      view: EditorView,
      readonly options: LiveMarkdownOptions,
    ) {
      this.parsedTree = syntaxTree(view.state);
      this.searchMatches = documentSearchMatches(view.state);
      this.model = parseLiveMarkdownModel(view.state, options);
      this.render(view);
    }

    update(update: ViewUpdate): void {
      const nextTree = syntaxTree(update.state);
      const syntaxTreeChanged = nextTree !== this.parsedTree;
      const nextSearchMatches = documentSearchMatches(update.state);
      const searchMatchesChanged = nextSearchMatches !== this.searchMatches;
      const refreshed = update.transactions.some((transaction) =>
        transaction.effects.some((effect) => effect.is(refreshLiveMarkdownEffect))
      );
      if (refreshed) {
        this.wikiResolutionVersion += 1;
      }
      if (update.docChanged || syntaxTreeChanged || refreshed) {
        this.parsedTree = nextTree;
        this.model = parseLiveMarkdownModel(
          update.state,
          this.options,
          this.wikiResolutionVersion,
        );
      }
      this.searchMatches = nextSearchMatches;
      if (
        update.docChanged ||
        update.selectionSet ||
        update.focusChanged ||
        syntaxTreeChanged ||
        searchMatchesChanged ||
        refreshed
      ) {
        this.render(update.view);
      }
    }

    private render(view: EditorView): void {
      const decorations = this.model.decorations.map(({ decoration, from, to }) =>
        decoration.range(from, to)
      );
      const atomicRanges = [];

      for (const construct of this.model.constructs) {
        if (constructIsRevealed(view, construct)) {
          for (const syntax of construct.syntax) {
            decorations.push(
              Decoration.mark({ class: "live-markdown-syntax" }).range(
                syntax.from,
                syntax.to,
              ),
            );
          }

          continue;
        }

        for (const { decoration, from, to } of construct.renderedDecorations) {
          decorations.push(decoration.range(from, to));
        }

        for (const range of construct.atomicRanges) {
          atomicRanges.push(
            Decoration.mark({}).range(range.from, range.to),
          );
        }

        for (const syntax of construct.syntax) {
          decorations.push(
            Decoration.replace({
              inclusive: false,
              ...(syntax.widget ? { widget: syntax.widget } : {}),
            }).range(syntax.from, syntax.to),
          );
          atomicRanges.push(
            Decoration.replace({ inclusive: false }).range(syntax.from, syntax.to),
          );
        }
      }

      this.decorations = Decoration.set(decorations, true);
      this.atomicRanges = Decoration.set(atomicRanges, true);
    }
  },
  {
    decorations: (plugin) => plugin.decorations,
    eventHandlers: {
      click(event, view) {
        const link = renderedMarkdownLink(event.target, view);
        if (!link) {
          return false;
        }

        const rawHref = link.dataset.liveHref ?? "";
        const href = rawHref.startsWith("//") ? `https:${rawHref}` : rawHref;
        if (parseMarkdownHeadingTarget(rawHref)) {
          this.options.openLink(rawHref);
        } else if (!window.__TAURI__) {
          return false;
        } else if (/^(?:https?|mailto):/i.test(href)) {
          this.options.openLink(href);
        }

        event.preventDefault();
        event.stopPropagation();

        return true;
      },
      mousedown(event, view) {
        const link = renderedMarkdownLink(event.target, view);
        if (!link || event.button !== 0) {
          return false;
        }

        const rawHref = link.dataset.liveHref ?? "";
        if (window.__TAURI__ || parseMarkdownHeadingTarget(rawHref)) {
          return true;
        }

        event.stopPropagation();

        return false;
      },
    },
    provide: (plugin) => EditorView.atomicRanges.of((view) =>
      view.plugin(plugin)?.atomicRanges ?? RangeSet.empty
    ),
  },
);

const tableCellCaretAssociation = EditorState.transactionFilter.of(
  (transaction) => {
    if (!transaction.selection && !transaction.docChanged) {
      return transaction;
    }

    const documentModel = transaction.docChanged
      ? liveMarkdownDocumentModelForText(transaction.newDoc)
      : liveMarkdownDocumentModel(transaction.startState);
    const ranges = transaction.newSelection.ranges.map((range) => {
      if (
        !range.empty ||
        range.assoc < 0 ||
        !documentModel.tableCellCaretEnds.has(range.head)
      ) {
        return range;
      }

      return EditorSelection.cursor(
        // At the end of a flex cell, the right-hand coordinates are the cell
        // wall. Associate the caret with its text so CodeMirror uses the
        // actual insertion coordinates on the left.
        range.head,
        -1,
        range.bidiLevel ?? undefined,
        range.goalColumn,
      );
    });
    if (ranges.every((range, index) =>
      range === transaction.newSelection.ranges[index]
    )) {
      return transaction;
    }

    return [
      transaction,
      {
        selection: EditorSelection.create(
          ranges,
          transaction.newSelection.mainIndex,
        ),
        sequential: true,
      },
    ];
  },
);

export function liveMarkdownExtension(options: LiveMarkdownOptions): Extension {
  return [
    liveMarkdownDocumentModelField,
    liveMarkdownHeadingFoldingExtension(options.documentId),
    tableCellCaretAssociation,
    liveMarkdownPlugin.of(options),
    EditorView.mouseSelectionStyle.of(inlineMarkupMouseSelection),
  ];
}

function inlineMarkupMouseSelection(
  view: EditorView,
  event: MouseEvent,
): MouseSelectionStyle | null {
  if (
    event.button !== 0 ||
    event.detail !== 1 ||
    event.altKey ||
    event.ctrlKey ||
    event.metaKey
  ) {
    return null;
  }

  let startSelection = view.state.selection;
  let start = inlineMarkupPointerPosition(view, event);

  return {
    get(currentEvent, extend, multiple) {
      const current = inlineMarkupPointerPosition(view, currentEvent);
      const range = start.pos === current.pos
        ? EditorSelection.cursor(current.pos, current.assoc)
        : EditorSelection.range(
          start.pos,
          current.pos,
          undefined,
          undefined,
          current.assoc,
        );

      if (extend) {
        return startSelection.replaceRange(
          startSelection.main.extend(range.from, range.to, range.assoc),
        );
      }
      if (multiple) {
        return startSelection.addRange(range);
      }

      return EditorSelection.create([range]);
    },
    update(update) {
      if (!update.docChanged) {
        return false;
      }

      start = {
        pos: update.changes.mapPos(start.pos, start.assoc),
        assoc: start.assoc,
      };
      startSelection = startSelection.map(update.changes);

      return false;
    },
  };
}

function inlineMarkupPointerPosition(
  view: EditorView,
  event: MouseEvent,
): { assoc: -1 | 1; pos: number } {
  const native = tableCellPointerPosition(view, event) ?? view.posAndSideAtCoords({
    x: event.clientX,
    y: event.clientY,
  }, false);
  const line = view.state.doc.lineAt(native.pos);
  let opening: number | undefined;
  let closing: number | undefined;

  for (const span of inlineMarkupSpans(view.state, line.from, line.to)) {
    if (inlineMarkupSpanIsRevealed(view, span)) {
      continue;
    }

    const start = view.coordsAtPos(span.contentFrom, 1);
    const end = view.coordsAtPos(span.contentTo, -1);
    if (!start || !end) {
      continue;
    }

    const startX = (start.left + start.right) / 2;
    const endX = (end.left + end.right) / 2;
    const leftToRight = endX >= startX;
    const beforeContent = leftToRight
      ? event.clientX <= startX
      : event.clientX >= startX;
    const afterContent = leftToRight
      ? event.clientX >= endX
      : event.clientX <= endX;

    if (
      beforeContent &&
      native.pos >= span.from &&
      native.pos <= span.contentFrom
    ) {
      opening = opening === undefined
        ? span.from
        : Math.min(opening, span.from);
    }
    if (
      afterContent &&
      native.pos >= span.contentTo &&
      native.pos <= span.to
    ) {
      closing = closing === undefined
        ? span.to
        : Math.max(closing, span.to);
    }
  }

  if (opening !== undefined && closing !== undefined && opening === closing) {
    return { pos: opening, assoc: native.assoc };
  }
  if (opening !== undefined) {
    return { pos: opening, assoc: 1 };
  }
  if (closing !== undefined) {
    return { pos: closing, assoc: -1 };
  }

  return native;
}

function tableCellPointerPosition(
  view: EditorView,
  event: MouseEvent,
): { assoc: -1 | 1; pos: number } | undefined {
  if (!(event.target instanceof Element)) {
    return undefined;
  }

  const cellElement = event.target.closest<HTMLElement>(
    ".live-table-cell[data-column-index]",
  );
  if (!cellElement || !view.dom.contains(cellElement)) {
    return undefined;
  }

  const columnIndex = Number(cellElement.dataset.columnIndex);
  if (!Number.isInteger(columnIndex)) {
    return undefined;
  }

  const native = view.posAndSideAtCoords({
    x: event.clientX,
    y: event.clientY,
  }, false);
  const cell = liveMarkdownDocumentModel(view.state).tables.flatMap((table) =>
    [table.header, ...table.rows].map((row) =>
      native.pos >= row.from && native.pos <= row.to
        ? row.cells[columnIndex]
        : undefined
    )
  ).find((candidate) => candidate !== undefined);
  if (
    !cell ||
    native.pos < cell.to ||
    view.state.sliceDoc(cell.to, cell.editableTo).trim()
  ) {
    return undefined;
  }

  return { pos: cell.to, assoc: -1 };
}

function inlineMarkupSpans(
  state: EditorState,
  from: number,
  to: number,
): InlineMarkupSpan[] {
  const cached = inlineMarkupSpanCache.get(state.doc);
  if (cached) {
    return cached.filter((span) => span.from <= to && span.to >= from);
  }

  const value = state.doc.toString();
  const blockExclusions = liveMarkdownDocumentModel(state).blocks.flatMap(
    (block) => {
      if (
        block.type === "code" ||
        block.type === "frontmatter" ||
        block.type === "horizontal-rule"
      ) {
        return [{ from: block.from, to: block.end }];
      }

      return block.syntax;
    },
  );
  const wikiExclusions = parseWikiLinks(value).map((link) => ({
    from: link.index,
    to: link.index + link.raw.length,
  }));
  const syntaxSpans = syntaxTreeInlineMarkupSpans(state).filter((span) =>
    !rangeIsContainedByAny(span, blockExclusions) &&
    !rangeOverlapsAny(span, wikiExclusions)
  );
  const spans: InlineMarkupSpan[] = [];

  spans.push(
    ...syntaxSpans,
    ...pairedInlineMarkupSpans(
      state,
      value,
      [...blockExclusions, ...wikiExclusions],
      syntaxSpans,
    ),
  );
  inlineMarkupSpanCache.set(state.doc, spans);

  return spans.filter((span) => span.from <= to && span.to >= from);
}

function syntaxTreeInlineMarkupSpans(
  state: EditorState,
): InlineMarkupSpan[] {
  const spans: InlineMarkupSpan[] = [];

  syntaxTree(state).iterate({
    enter(reference) {
      const span = inlineMarkupSpan(reference, state);
      if (span) {
        spans.push(span);
      }

      return undefined;
    },
  });

  return spans;
}

function inlineMarkupSpan(
  node: SyntaxNodeRef,
  state: EditorState,
): InlineMarkupSpan | undefined {
  const markup = node.name === "InlineCode"
    ? { kind: "code" as const, markerName: "CodeMark" }
    : node.name === "StrongEmphasis"
      ? { kind: "strong" as const, markerName: "EmphasisMark" }
      : node.name === "Emphasis"
        ? { kind: "emphasis" as const, markerName: "EmphasisMark" }
        : node.name === "Strikethrough"
          ? { kind: "strikethrough" as const, markerName: "StrikethroughMark" }
          : undefined;
  if (!markup) {
    return undefined;
  }

  const markers = node.node.getChildren(markup.markerName);
  const opening = markers[0];
  const closing = markers.at(-1);
  if (!opening || !closing || opening === closing || opening.to > closing.from) {
    return undefined;
  }

  const syntax: LiveMarkdownRange[] = [
    { from: opening.from, to: opening.to },
    { from: closing.from, to: closing.to },
  ];
  let contentFrom = opening.to;
  let contentTo = closing.from;
  if (node.name === "InlineCode") {
    const rawContent = state.sliceDoc(contentFrom, contentTo);
    if (/\r|\n/.test(rawContent)) {
      return undefined;
    }
    if (/^\s.*\s$/.test(rawContent) && rawContent.trim()) {
      syntax.push(
        { from: contentFrom, to: contentFrom + 1 },
        { from: contentTo - 1, to: contentTo },
      );
      contentFrom += 1;
      contentTo -= 1;
    }
  }

  return {
    from: node.from,
    to: node.to,
    contentFrom,
    contentTo,
    kind: markup.kind,
    syntax,
  };
}

function pairedInlineMarkupSpans(
  state: EditorState,
  value: string,
  excludedRanges: readonly LiveMarkdownRange[],
  syntaxSpans: readonly InlineMarkupSpan[],
): InlineMarkupSpan[] {
  const protectedRanges = [
    ...excludedRanges,
    ...inlineMarkupSyntaxExclusions(state),
  ];

  return parsePairedInlineMarkup(value, protectedRanges)
    .map((span): InlineMarkupSpan => ({
      contentFrom: span.contentFrom,
      contentTo: span.contentTo,
      from: span.from,
      kind: span.kind,
      syntax: [
        { from: span.from, to: span.contentFrom },
        { from: span.contentTo, to: span.to },
      ],
      to: span.to,
    }))
    .filter((span) => !syntaxSpans.some((syntaxSpan) =>
      inlineMarkupSpansMatch(span, syntaxSpan) || rangesCross(span, syntaxSpan)
    ));
}

function inlineMarkupSyntaxExclusions(
  state: EditorState,
): LiveMarkdownRange[] {
  const exclusions: LiveMarkdownRange[] = [];

  syntaxTree(state).iterate({
    enter(reference) {
      if (
        reference.name === "Autolink" ||
        reference.name === "Escape" ||
        reference.name === "HTMLTag" ||
        reference.name === "Image" ||
        reference.name === "InlineCode"
      ) {
        exclusions.push({ from: reference.from, to: reference.to });

        return false;
      }
      if (reference.name !== "Link") {
        return undefined;
      }

      const labelClosing = reference.node.getChildren("LinkMark")[1];
      exclusions.push(labelClosing
        ? { from: labelClosing.from, to: reference.to }
        : { from: reference.from, to: reference.to });

      return undefined;
    },
  });

  return exclusions;
}

function inlineMarkupSpansMatch(
  left: InlineMarkupSpan,
  right: InlineMarkupSpan,
): boolean {
  return left.from === right.from &&
    left.to === right.to &&
    left.kind === right.kind;
}

function inlineMarkupSpanIsRevealed(
  view: EditorView,
  span: InlineMarkupSpan,
): boolean {
  if (documentSearchMatches(view.state).some((match) =>
    span.syntax.some((syntax) => rangesOverlap(match, syntax))
  )) {
    return true;
  }
  if (!view.hasFocus) {
    return false;
  }

  return view.state.selection.ranges.some((selection) => {
    if (selection.empty) {
      return span.syntax.some((syntax) =>
        selection.head >= syntax.from && selection.head <= syntax.to
      );
    }

    return span.syntax.some((syntax) => rangesOverlap(selection, syntax));
  });
}

function parseLiveMarkdownModel(
  state: EditorState,
  options: LiveMarkdownOptions,
  wikiResolutionVersion = 0,
): LiveMarkdownModel {
  const value = state.doc.toString();
  const tree = syntaxTree(state);
  const { blocks, tables } = liveMarkdownDocumentModel(state);
  const codeFences = parseLiveMarkdownCodeFences(value);
  const codeLines = new Set(
    codeFences.flatMap((fence) => fence.lineNumbers),
  );
  const tableLines = new Set(tables.flatMap((table) => table.lineNumbers));
  const model: LiveMarkdownModel = {
    constructs: [],
    decorations: [],
  };
  const excludedRanges: LiveMarkdownRange[] = [];
  let renderedQuotePrefix: { depth: number; source: string } | undefined;

  for (const block of blocks) {
    if (
      codeLines.has(block.lineNumber) ||
      tableLines.has(block.lineNumber)
    ) {
      continue;
    }
    if (block.type === "frontmatter") {
      excludedRanges.push({ from: block.from, to: block.end });

      continue;
    }

    // The line model only sees literal `>` prefixes. The Markdown tree owns
    // container semantics, including unmarked lazy paragraph continuations.
    const literalQuoteDepth = block.quote?.depth ?? 0;
    const quoteDepth = Math.max(
      literalQuoteDepth,
      blockquoteDepthAt(tree, block.from),
    );
    if (!quoteDepth) {
      renderedQuotePrefix = undefined;
    } else if (literalQuoteDepth === quoteDepth) {
      // Keep partially marked lines from replacing the full-width prefix that
      // deeper lazy continuations reuse.
      renderedQuotePrefix = {
        depth: quoteDepth,
        source: value.slice(block.from, block.content.from),
      };
    }

    addBlockDecorations(
      model,
      block,
      value,
      quoteDepth,
      renderedQuotePrefix?.depth === quoteDepth
        ? renderedQuotePrefix.source
        : undefined,
    );
  }

  for (const fence of codeFences) {
    excludedRanges.push({ from: fence.from, to: fence.to });
    addCodeFenceDecorations(model, state, fence);
  }
  for (const table of tables) {
    excludedRanges.push({
      from: table.delimiter.from,
      to: table.delimiter.end,
    });
    addTableDecorations(model, state, table);
  }

  const markdownLinkRanges = supportedMarkdownLinkRanges(
    state,
    excludedRanges,
  );
  const wikiRanges = addWikiLinkDecorations(
    model,
    value,
    excludedRanges,
    markdownLinkRanges,
    options,
    wikiResolutionVersion,
  );
  const inlineMarkupExcludedRanges = [
    ...excludedRanges,
    ...wikiRanges,
    ...blocks.flatMap((block) => block.syntax),
  ];
  addInlineDecorations(
    model,
    state,
    value,
    excludedRanges,
    wikiRanges,
    inlineMarkupExcludedRanges,
  );

  return model;
}

function addBlockDecorations(
  model: LiveMarkdownModel,
  block: LiveMarkdownBlock,
  value: string,
  quoteDepth = block.quote?.depth ?? 0,
  quotePrefix?: string,
): void {
  const classes = [
    "live-markdown-block",
    `is-${quoteDepth ? "blockquote" : block.type}`,
  ];
  if (block.headingLevel) {
    classes.push(`heading-level-${block.headingLevel}`);
  }
  if (block.list) {
    classes.push(`list-depth-${block.list.depth % 3}`);
  }
  if (quoteDepth) {
    classes.push(`quote-depth-${Math.min(quoteDepth, 3)}`);
    if (!block.quote) {
      classes.push("is-blockquote-continuation");
    }
  }
  if (block.task?.checked) {
    classes.push("is-checked");
  }
  addLineDecoration(model, block.from, classes.join(" "));

  if (block.type === "heading") {
    addConstruct(model, block.from, block.to, block.syntax);

    return;
  }
  if (block.type === "horizontal-rule") {
    addConstruct(model, block.from, block.to, [{
      from: block.from,
      to: block.to,
      widget: new HorizontalRuleWidget(block.from, block.to),
    }]);

    return;
  }
  if (block.type === "task" && block.task && block.list) {
    const markerSource = value.slice(block.from, block.content.from);
    addConstruct(model, block.from, block.content.from, [{
      from: block.from,
      to: block.content.from,
      widget: new TaskWidget(
        markerSource,
        block.task.checked,
        block.task.check.from,
        block.from,
        block.content.from,
      ),
    }], [renderedListLineDecoration(block)], { boundaryReveal: "none" });
    if (block.task.checked && block.content.from < block.content.to) {
      addMarkDecoration(model, block.content, "live-task-content");
    }

    return;
  }
  if (block.type === "list" && block.list) {
    const markerSource = value.slice(block.from, block.content.from);
    addConstruct(model, block.from, block.content.from, [{
      from: block.from,
      to: block.content.from,
      widget: new ListMarkerWidget(
        markerSource,
        renderedListMarker(block),
        block.from,
        block.content.from,
      ),
    }], [renderedListLineDecoration(block)], { boundaryReveal: "none" });

    return;
  }
  if (block.type === "blockquote" && block.quote) {
    const prefix = quotePrefix ?? value.slice(block.from, block.content.from);
    addConstruct(model, block.from, block.content.from, [{
      from: block.from,
      to: block.content.from,
      widget: new QuoteMarkerWidget(
        prefix,
        quoteDepth,
        block.from,
        block.content.from,
      ),
    }]);

    return;
  }
  if (quoteDepth) {
    const prefix = quotePrefix ?? "> ".repeat(quoteDepth);
    model.decorations.push({
      from: block.from,
      to: block.from,
      decoration: Decoration.widget({
        side: -1,
        widget: new QuoteMarkerWidget(
          prefix,
          quoteDepth,
          block.from,
          block.from,
        ),
      }),
    });
  }
}

function blockquoteDepthAt(tree: Tree, position: number): number {
  let depth = 0;
  let node: SyntaxNode | null = tree.resolve(position, 1);

  while (node) {
    if (node.name === "Blockquote") {
      depth += 1;
    }
    node = node.parent;
  }

  return depth;
}

function addCodeFenceDecorations(
  model: LiveMarkdownModel,
  state: EditorState,
  fence: LiveMarkdownCodeFence,
): void {
  const opening = state.doc.line(fence.openingLine);
  const openingClasses = [
    "live-markdown-block",
    "is-code-opening",
    ...(fence.lineNumbers.length === 1 ? ["is-code-last"] : []),
  ];
  addConstruct(
    model,
    opening.from,
    opening.to,
    [{
      from: opening.from,
      to: opening.to,
      widget: new CodeFenceHeaderWidget(fence, opening.from, opening.to),
    }],
    [lineDecoration(opening.from, openingClasses.join(" "))],
  );

  const finalContentLine = fence.closingLine === undefined
    ? fence.lineNumbers.at(-1)
    : fence.closingLine - 1;
  for (
    let lineNumber = fence.openingLine + 1;
    lineNumber <= (finalContentLine ?? fence.openingLine);
    lineNumber += 1
  ) {
    const line = state.doc.line(lineNumber);
    const classes = ["live-markdown-block", "is-code-content"];
    if (fence.closingLine === undefined && lineNumber === finalContentLine) {
      classes.push("is-code-last");
    }
    addLineDecoration(model, line.from, classes.join(" "));
  }

  addCodeHighlightDecorations(model, state, fence);

  if (fence.closingLine !== undefined) {
    const closing = state.doc.line(fence.closingLine);
    addConstruct(
      model,
      closing.from,
      closing.to,
      [{
        from: closing.from,
        to: closing.to,
        widget: new CodeFenceFooterWidget(closing.from, closing.to),
      }],
      [lineDecoration(
        closing.from,
        "live-markdown-block is-code-closing is-code-last",
      )],
    );
  }
}

function addCodeHighlightDecorations(
  model: LiveMarkdownModel,
  state: EditorState,
  fence: LiveMarkdownCodeFence,
): void {
  if (!fence.code || !fence.language || fence.openingLine >= state.doc.lines) {
    return;
  }

  const codeFrom = state.doc.line(fence.openingLine + 1).from;
  for (const range of highlightCodeRanges(fence.code, fence.language)) {
    addMultilineMarkDecoration(
      model,
      state,
      codeFrom + range.from,
      codeFrom + range.to,
      range.className,
    );
  }
}

function addMultilineMarkDecoration(
  model: LiveMarkdownModel,
  state: EditorState,
  from: number,
  to: number,
  className: string,
): void {
  let cursor = from;
  while (cursor < to) {
    const line = state.doc.lineAt(cursor);
    const segmentTo = Math.min(to, line.to);
    if (cursor < segmentTo) {
      addMarkDecoration(model, { from: cursor, to: segmentTo }, className);
    }
    cursor = segmentTo < to ? line.to + 1 : to;
  }
}

function addTableDecorations(
  model: LiveMarkdownModel,
  state: EditorState,
  table: LiveMarkdownTable,
): void {
  addTableRowDecorations(model, state, table, table.header, "header", false);

  const delimiterLast = table.rows.length === 0;
  addConstruct(
    model,
    table.delimiter.from,
    table.delimiter.to,
    [{
      from: table.delimiter.from,
      to: table.delimiter.to,
      widget: new TableDelimiterWidget(
        table.delimiter.from,
        table.delimiter.to,
      ),
    }],
    [lineDecoration(
      table.delimiter.from,
      [
        "live-markdown-block",
        "is-table-delimiter",
        ...(delimiterLast ? ["is-table-last"] : []),
      ].join(" "),
    )],
  );

  table.rows.forEach((row, index) => {
    addTableRowDecorations(
      model,
      state,
      table,
      row,
      "body",
      index === table.rows.length - 1,
    );
  });
}

function addTableRowDecorations(
  model: LiveMarkdownModel,
  state: EditorState,
  table: LiveMarkdownTable,
  row: LiveMarkdownTableRow,
  role: "body" | "header",
  last: boolean,
): void {
  const cells = row.cells.slice(0, table.columnCount);
  const syntax = [
    ...tableRowSyntax(row, cells),
    ...tableCellBreakSyntax(state, cells),
  ].sort((left, right) => left.from - right.from || left.to - right.to);
  const classes = [
    "live-markdown-block",
    "is-table-row",
    `is-table-${role}`,
    ...(last ? ["is-table-last"] : []),
  ];
  const renderedDecorations: StoredDecoration[] = [lineDecoration(
    row.from,
    classes.join(" "),
  )];

  for (let index = 0; index < table.columnCount; index += 1) {
    const cell = cells[index];
    const cellLast = index === table.columnCount - 1;
    const className = tableCellClass(
      table.alignments[index],
      cellLast,
    );
    if (cell && cell.editableFrom < cell.editableTo) {
      renderedDecorations.push({
        from: cell.editableFrom,
        to: cell.editableTo,
        decoration: Decoration.mark({
          attributes: { "data-column-index": String(index) },
          class: [className, ...(!cell.source ? ["is-empty"] : [])]
            .join(" "),
          inclusive: true,
        }),
      });
    } else {
      const position = cell?.editableFrom ?? row.to;
      renderedDecorations.push({
        from: position,
        to: position,
        decoration: Decoration.widget({
          side: index + 1,
          widget: new EmptyTableCellWidget(position, index, className),
        }),
      });
    }
  }

  addConstruct(
    model,
    row.from,
    row.to,
    syntax,
    renderedDecorations,
    {
      atomicRanges: tableCellTrailingPadding(state, cells),
      boundaryReveal: "construct",
      revealWithinSyntax: false,
    },
  );
}

function tableCellTrailingPadding(
  state: EditorState,
  cells: readonly LiveMarkdownTableRow["cells"][number][],
): LiveMarkdownRange[] {
  return cells.flatMap((cell) => {
    if (
      cell.to >= cell.editableTo ||
      !/\s/.test(state.sliceDoc(cell.editableTo - 1, cell.editableTo))
    ) {
      return [];
    }

    return [{ from: cell.editableTo - 1, to: cell.editableTo }];
  });
}

function tableRowSyntax(
  row: LiveMarkdownTableRow,
  cells: readonly LiveMarkdownTableRow["cells"][number][],
): HiddenSyntax[] {
  const syntax: HiddenSyntax[] = [];
  let cursor = row.from;

  for (const cell of cells) {
    if (cursor < cell.editableFrom) {
      syntax.push({ from: cursor, to: cell.editableFrom });
    }
    cursor = Math.max(cursor, cell.editableTo);
  }
  if (cursor < row.to) {
    syntax.push({ from: cursor, to: row.to });
  }

  return syntax;
}

function tableCellBreakSyntax(
  state: EditorState,
  cells: readonly LiveMarkdownTableRow["cells"][number][],
): HiddenSyntax[] {
  const syntax: HiddenSyntax[] = [];
  const firstCell = cells[0];
  const lastCell = cells.at(-1);
  if (!firstCell || !lastCell) {
    return syntax;
  }

  syntaxTree(state).iterate({
    from: firstCell.editableFrom,
    to: lastCell.editableTo,
    enter(reference) {
      if (reference.name !== "HTMLTag") {
        return undefined;
      }

      const cell = cells.find((candidate) =>
        reference.from >= candidate.editableFrom &&
        reference.to <= candidate.editableTo
      );
      const source = state.sliceDoc(reference.from, reference.to);
      if (
        !cell ||
        !/^<br[ \t]*\/?>$/i.test(source) ||
        characterIsEscaped(
          state.sliceDoc(cell.editableFrom, cell.editableTo),
          reference.from - cell.editableFrom,
        )
      ) {
        return false;
      }

      syntax.push({
        from: reference.from,
        to: reference.to,
        widget: new TableCellBreakWidget(),
      });

      return false;
    },
  });

  return syntax;
}

function characterIsEscaped(value: string, index: number): boolean {
  let backslashes = 0;
  for (let cursor = index - 1; cursor >= 0 && value[cursor] === "\\"; cursor -= 1) {
    backslashes += 1;
  }

  return backslashes % 2 === 1;
}

function tableCellClass(
  alignment: LiveMarkdownTableAlignment | undefined,
  last: boolean,
): string {
  return [
    "live-table-cell",
    ...(alignment ? [`align-${alignment}`] : []),
    ...(last ? ["is-last"] : []),
  ].join(" ");
}

function supportedMarkdownLinkRanges(
  state: EditorState,
  excludedRanges: readonly LiveMarkdownRange[],
): LiveMarkdownRange[] {
  const ranges: LiveMarkdownRange[] = [];

  syntaxTree(state).iterate({
    enter(reference) {
      const range = { from: reference.from, to: reference.to };
      if (rangeIsContainedByAny(range, excludedRanges)) {
        return false;
      }
      if (
        reference.name !== "Autolink" &&
        reference.name !== "Link"
      ) {
        return undefined;
      }

      const markers = reference.node.getChildren("LinkMark");
      const requiredMarkers = reference.name === "Autolink" ? 2 : 4;
      if (
        markers.length >= requiredMarkers &&
        reference.node.getChild("URL")
      ) {
        ranges.push(range);
      }

      return undefined;
    },
  });

  return ranges;
}

function addWikiLinkDecorations(
  model: LiveMarkdownModel,
  value: string,
  excludedRanges: readonly LiveMarkdownRange[],
  markdownLinkRanges: readonly LiveMarkdownRange[],
  options: LiveMarkdownOptions,
  wikiResolutionVersion: number,
): LiveMarkdownRange[] {
  const ranges: LiveMarkdownRange[] = [];

  for (const link of parseWikiLinks(value)) {
    const range = { from: link.index, to: link.index + link.raw.length };
    if (
      rangeOverlapsAny(range, excludedRanges) ||
      rangeOverlapsAny(range, markdownLinkRanges)
    ) {
      continue;
    }

    ranges.push(range);
    addConstruct(model, range.from, range.to, [{
      ...range,
      widget: new WikiLinkWidget(
        link.display || link.heading || link.target,
        link.target,
        link.heading,
        link.embedded,
        options.wikiLinkIsResolved(link.target),
        options.openWiki,
        range.from,
        range.to,
        wikiResolutionVersion,
      ),
    }]);
  }

  return ranges;
}

function addInlineDecorations(
  model: LiveMarkdownModel,
  state: EditorState,
  value: string,
  excludedRanges: readonly LiveMarkdownRange[],
  wikiRanges: readonly LiveMarkdownRange[],
  inlineMarkupExcludedRanges: readonly LiveMarkdownRange[],
): void {
  const syntaxSpans: InlineMarkupSpan[] = [];

  syntaxTree(state).iterate({
    enter(reference) {
      const node = reference;
      const range = { from: node.from, to: node.to };
      if (rangeIsContainedByAny(range, excludedRanges)) {
        return false;
      }

      if (node.name === "Escape") {
        if (!rangeOverlapsAny(range, wikiRanges) && node.to - node.from >= 2) {
          addConstruct(model, node.from, node.to, [{
            from: node.from,
            to: node.from + 1,
          }]);
        }

        return false;
      }
      if (rangeOverlapsAny(range, wikiRanges)) {
        return undefined;
      }
      const span = inlineMarkupSpan(node, state);
      if (span) {
        syntaxSpans.push(span);
        addInlineMarkupDecoration(model, span);
      } else if (
        node.name === "Autolink" ||
        node.name === "Link"
      ) {
        addMarkdownLinkDecoration(model, node, value);
      }

      return undefined;
    },
  });

  const pairedSpans = pairedInlineMarkupSpans(
    state,
    value,
    inlineMarkupExcludedRanges,
    syntaxSpans,
  );
  inlineMarkupSpanCache.set(state.doc, [...syntaxSpans, ...pairedSpans]);

  for (const span of pairedSpans) {
    addInlineMarkupDecoration(model, span);
  }
}

function addInlineMarkupDecoration(
  model: LiveMarkdownModel,
  span: InlineMarkupSpan,
): void {
  addConstruct(model, span.from, span.to, span.syntax);
  addMarkDecoration(
    model,
    { from: span.contentFrom, to: span.contentTo },
    `live-inline-segment is-${span.kind}`,
  );
}

function addMarkdownLinkDecoration(
  model: LiveMarkdownModel,
  node: SyntaxNodeRef,
  value: string,
): void {
  const markers = node.node.getChildren("LinkMark");
  const urlNode = node.node.getChild("URL");
  const autolink = node.name === "Autolink";
  if ((autolink ? markers.length < 2 : markers.length < 4) || !urlNode) {
    return;
  }

  const labelOpening = markers[0]!;
  const labelClosing = autolink ? markers.at(-1)! : markers[1]!;
  if (labelOpening.to > labelClosing.from) {
    return;
  }

  const rawHref = value.slice(urlNode.from, urlNode.to);
  const href = sanitizeLinkUrl(
    autolink && /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(rawHref)
      ? `mailto:${rawHref}`
      : rawHref,
  );
  const titleNode = node.node.getChild("LinkTitle");
  const title = titleNode
    ? unwrapLinkTitle(value.slice(titleNode.from, titleNode.to))
    : undefined;
  const attributes: Record<string, string> = {};
  if (href) {
    attributes.href = href;
    attributes["data-live-href"] = href;
    attributes.rel = "noopener noreferrer";
    if (/^(?:https?:)?\/\//i.test(href)) {
      attributes.target = "_blank";
    }
    if (title) {
      attributes.title = title;
    }
  }

  addConstruct(model, node.from, node.to, [
    { from: labelOpening.from, to: labelOpening.to },
    {
      from: labelClosing.from,
      to: autolink ? labelClosing.to : node.to,
    },
  ]);
  if (labelOpening.to < labelClosing.from) {
    const className = href
      ? [
          "live-inline-segment",
          "is-link",
          ...(parseMarkdownHeadingTarget(href) ? ["is-heading-link"] : []),
        ].join(" ")
      : "live-inline-segment is-unsafe-link";
    model.decorations.push({
      from: labelOpening.to,
      to: labelClosing.from,
      decoration: Decoration.mark({
        class: className,
        ...(href
          ? {
              attributes,
              tagName: "a",
            }
          : {}),
      }),
    });
  }
}

function addConstruct(
  model: LiveMarkdownModel,
  from: number,
  to: number,
  syntax: readonly HiddenSyntax[],
  renderedDecorations: readonly StoredDecoration[] = [],
  options: LiveConstructOptions = {},
): void {
  const nonemptySyntax = syntax.filter((range) => range.from < range.to);
  if (!nonemptySyntax.length) {
    return;
  }

  model.constructs.push({
    atomicRanges: [...(options.atomicRanges ?? [])],
    boundaryReveal: options.boundaryReveal ?? "syntax",
    from,
    renderedDecorations: [...renderedDecorations],
    revealWithinSyntax: options.revealWithinSyntax ?? true,
    to,
    syntax: nonemptySyntax,
  });
}

function addLineDecoration(
  model: LiveMarkdownModel,
  from: number,
  className: string,
): void {
  model.decorations.push(lineDecoration(from, className));
}

function lineDecoration(
  from: number,
  className: string,
): StoredDecoration {
  return {
    from,
    to: from,
    decoration: Decoration.line({ class: className }),
  };
}

function renderedListLineDecoration(block: LiveMarkdownBlock): StoredDecoration {
  const indentation = ((block.list?.depth ?? 0) + 1) * LIST_INDENT_STEP_EM;

  return {
    from: block.from,
    to: block.from,
    decoration: Decoration.line({
      attributes: {
        style: `--live-list-content-indent: ${indentation}em`,
      },
      class: "is-rendered-list",
    }),
  };
}

function addMarkDecoration(
  model: LiveMarkdownModel,
  range: LiveMarkdownRange,
  className: string,
): void {
  if (range.from >= range.to) {
    return;
  }

  model.decorations.push({
    ...range,
    decoration: Decoration.mark({ class: className }),
  });
}

function constructIsRevealed(
  view: EditorView,
  construct: LiveConstruct,
): boolean {
  if (documentSearchMatches(view.state).some((match) =>
    construct.syntax.some((syntax) => rangesOverlap(match, syntax))
  )) {
    return true;
  }
  if (!view.hasFocus) {
    return false;
  }

  return view.state.selection.ranges.some((selection) =>
    selectionRevealsConstruct(selection, construct)
  );
}

function selectionRevealsConstruct(
  selection: SelectionRange,
  construct: LiveConstruct,
): boolean {
  if (selection.empty) {
    const insideSyntax = construct.syntax.some((syntax) =>
      selection.head > syntax.from && selection.head < syntax.to
    );
    if (insideSyntax && construct.revealWithinSyntax) {
      return true;
    }
    if (construct.boundaryReveal === "syntax") {
      return construct.syntax.some((syntax) =>
        selection.head === syntax.from || selection.head === syntax.to
      );
    }
    if (construct.boundaryReveal === "construct") {
      return selection.head === construct.from || selection.head === construct.to;
    }

    return false;
  }

  return construct.syntax.some((syntax) =>
    selection.from < syntax.to && selection.to > syntax.from
  );
}

function rangeOverlapsAny(
  range: LiveMarkdownRange,
  candidates: readonly LiveMarkdownRange[],
): boolean {
  return candidates.some((candidate) => rangesOverlap(range, candidate));
}

function rangeIsContainedByAny(
  range: LiveMarkdownRange,
  candidates: readonly LiveMarkdownRange[],
): boolean {
  return candidates.some((candidate) =>
    range.from >= candidate.from && range.to <= candidate.to
  );
}

function rangesOverlap(
  first: LiveMarkdownRange,
  second: LiveMarkdownRange,
): boolean {
  return first.from < second.to && first.to > second.from;
}

function rangesCross(
  first: LiveMarkdownRange,
  second: LiveMarkdownRange,
): boolean {
  return (
    first.from < second.from &&
    second.from < first.to &&
    first.to < second.to
  ) || (
    second.from < first.from &&
    first.from < second.to &&
    second.to < first.to
  );
}

function unwrapLinkTitle(value: string): string {
  const opening = value[0];
  const closing = value.at(-1);
  if (
    (opening === '"' && closing === '"') ||
    (opening === "'" && closing === "'") ||
    (opening === "(" && closing === ")")
  ) {
    return value.slice(1, -1);
  }

  return value;
}

function renderedMarkdownLink(
  eventTarget: EventTarget | null,
  view: EditorView,
): HTMLAnchorElement | undefined {
  if (!(eventTarget instanceof Element)) {
    return undefined;
  }

  const link = eventTarget.closest<HTMLAnchorElement>("a[data-live-href]");

  return link && view.dom.contains(link) ? link : undefined;
}
