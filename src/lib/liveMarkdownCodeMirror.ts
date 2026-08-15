import { syntaxTree } from "@codemirror/language";
import { RangeSet, StateEffect } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  ViewPlugin,
} from "@codemirror/view";
import { documentSearchMatches } from "./codeMirrorDocumentSearch";
import { highlightCodeRanges } from "./highlight";
import { parseLiveMarkdownBlocks } from "./liveMarkdown";
import { parseLiveMarkdownCodeFences } from "./liveMarkdownCode";
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
  TableDelimiterWidget,
} from "./liveMarkdownRegionWidgets";
import { parseLiveMarkdownTables } from "./liveMarkdownTable";
import { sanitizeLinkUrl } from "./markdown";
import { parseWikiLinks } from "./wikiLinks";
import type { EditorState, Extension, SelectionRange } from "@codemirror/state";
import type { DecorationSet, ViewUpdate, WidgetType } from "@codemirror/view";
import type { SyntaxNodeRef } from "@lezer/common";
import type { LiveMarkdownBlock, LiveMarkdownRange } from "./liveMarkdown";
import type { LiveMarkdownCodeFence } from "./liveMarkdownCode";
import type {
  LiveMarkdownTable,
  LiveMarkdownTableAlignment,
  LiveMarkdownTableRow,
} from "./liveMarkdownTable";

export interface LiveMarkdownOptions {
  readonly openLink: (href: string) => void;
  readonly openWiki: (target: string) => void;
  readonly wikiLinkIsResolved: (target: string) => boolean;
}

interface HiddenSyntax extends LiveMarkdownRange {
  widget?: WidgetType;
}

interface LiveConstruct {
  boundaryReveal: "construct" | "none" | "syntax";
  from: number;
  renderedDecorations: StoredDecoration[];
  revealWithinSyntax: boolean;
  to: number;
  syntax: HiddenSyntax[];
}

interface LiveConstructOptions {
  boundaryReveal?: LiveConstruct["boundaryReveal"];
  revealWithinSyntax?: boolean;
}

interface StoredDecoration extends LiveMarkdownRange {
  decoration: Decoration;
}

interface LiveMarkdownModel {
  constructs: LiveConstruct[];
  decorations: StoredDecoration[];
}

const LIST_INDENT_STEP_EM = 1.65;

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

        if (!window.__TAURI__) {
          return false;
        }

        const rawHref = link.dataset.liveHref ?? "";
        const href = rawHref.startsWith("//") ? `https:${rawHref}` : rawHref;
        if (/^(?:https?|mailto):/i.test(href)) {
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

        if (window.__TAURI__) {
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

export function liveMarkdownExtension(options: LiveMarkdownOptions): Extension {
  return liveMarkdownPlugin.of(options);
}

function parseLiveMarkdownModel(
  state: EditorState,
  options: LiveMarkdownOptions,
  wikiResolutionVersion = 0,
): LiveMarkdownModel {
  const value = state.doc.toString();
  const blocks = parseLiveMarkdownBlocks(value);
  const codeFences = parseLiveMarkdownCodeFences(value);
  const codeLines = new Set(
    codeFences.flatMap((fence) => fence.lineNumbers),
  );
  const tables = parseLiveMarkdownTables(value, blocks);
  const tableLines = new Set(tables.flatMap((table) => table.lineNumbers));
  const model: LiveMarkdownModel = {
    constructs: [],
    decorations: [],
  };
  const excludedRanges: LiveMarkdownRange[] = [];

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

    addBlockDecorations(model, block, value);
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
    addTableDecorations(model, table);
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
  addInlineDecorations(
    model,
    state,
    value,
    excludedRanges,
    wikiRanges,
  );

  return model;
}

function addBlockDecorations(
  model: LiveMarkdownModel,
  block: LiveMarkdownBlock,
  value: string,
): void {
  const classes = ["live-markdown-block", `is-${block.type}`];
  if (block.headingLevel) {
    classes.push(`heading-level-${block.headingLevel}`);
  }
  if (block.list) {
    classes.push(`list-depth-${block.list.depth % 3}`);
  }
  if (block.quote) {
    classes.push(`quote-depth-${Math.min(block.quote.depth, 3)}`);
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
    const prefix = value.slice(block.from, block.content.from);
    addConstruct(model, block.from, block.content.from, [{
      from: block.from,
      to: block.content.from,
      widget: new QuoteMarkerWidget(
        prefix,
        block.quote.depth,
        block.from,
        block.content.from,
      ),
    }]);
  }
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
  table: LiveMarkdownTable,
): void {
  addTableRowDecorations(model, table, table.header, "header", false);

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
      table,
      row,
      "body",
      index === table.rows.length - 1,
    );
  });
}

function addTableRowDecorations(
  model: LiveMarkdownModel,
  table: LiveMarkdownTable,
  row: LiveMarkdownTableRow,
  role: "body" | "header",
  last: boolean,
): void {
  const cells = row.cells.slice(0, table.columnCount);
  const syntax = tableRowSyntax(row, cells);
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
    if (cell && cell.from < cell.to) {
      renderedDecorations.push({
        from: cell.from,
        to: cell.to,
        decoration: Decoration.mark({
          attributes: { "data-column-index": String(index) },
          class: className,
          inclusive: true,
        }),
      });
    } else {
      const position = cell?.from ?? row.to;
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
      boundaryReveal: "construct",
      revealWithinSyntax: false,
    },
  );
}

function tableRowSyntax(
  row: LiveMarkdownTableRow,
  cells: readonly LiveMarkdownTableRow["cells"][number][],
): HiddenSyntax[] {
  const syntax: HiddenSyntax[] = [];
  let cursor = row.from;

  for (const cell of cells) {
    if (cursor < cell.from) {
      syntax.push({ from: cursor, to: cell.from });
    }
    cursor = Math.max(cursor, cell.to);
  }
  if (cursor < row.to) {
    syntax.push({ from: cursor, to: row.to });
  }

  return syntax;
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
): void {
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
      if (node.name === "StrongEmphasis") {
        addDelimitedMark(model, node, "EmphasisMark", "is-strong");
      } else if (node.name === "Emphasis") {
        addDelimitedMark(model, node, "EmphasisMark", "is-emphasis");
      } else if (node.name === "Strikethrough") {
        addDelimitedMark(
          model,
          node,
          "StrikethroughMark",
          "is-strikethrough",
        );
      } else if (node.name === "InlineCode") {
        addInlineCodeDecoration(model, node, value);
      } else if (
        node.name === "Autolink" ||
        node.name === "Link"
      ) {
        addMarkdownLinkDecoration(model, node, value);
      }

      return undefined;
    },
  });
}

function addDelimitedMark(
  model: LiveMarkdownModel,
  node: SyntaxNodeRef,
  markerName: string,
  className: string,
): void {
  const markers = node.node.getChildren(markerName);
  const opening = markers[0];
  const closing = markers.at(-1);
  if (!opening || !closing || opening === closing || opening.to > closing.from) {
    return;
  }

  addConstruct(model, node.from, node.to, [
    { from: opening.from, to: opening.to },
    { from: closing.from, to: closing.to },
  ]);
  if (opening.to < closing.from) {
    addMarkDecoration(
      model,
      { from: opening.to, to: closing.from },
      `live-inline-segment ${className}`,
    );
  }
}

function addInlineCodeDecoration(
  model: LiveMarkdownModel,
  node: SyntaxNodeRef,
  value: string,
): void {
  const markers = node.node.getChildren("CodeMark");
  const opening = markers[0];
  const closing = markers.at(-1);
  if (!opening || !closing || opening === closing || opening.to > closing.from) {
    return;
  }

  const rawContent = value.slice(opening.to, closing.from);
  if (/\r|\n/.test(rawContent)) {
    return;
  }

  const syntax: HiddenSyntax[] = [
    { from: opening.from, to: opening.to },
    { from: closing.from, to: closing.to },
  ];
  let contentFrom = opening.to;
  let contentTo = closing.from;
  if (/^\s.*\s$/.test(rawContent) && rawContent.trim()) {
    syntax.push(
      { from: contentFrom, to: contentFrom + 1 },
      { from: contentTo - 1, to: contentTo },
    );
    contentFrom += 1;
    contentTo -= 1;
  }

  addConstruct(model, node.from, node.to, syntax);
  if (contentFrom < contentTo) {
    addMarkDecoration(
      model,
      { from: contentFrom, to: contentTo },
      "live-inline-segment is-code",
    );
  }
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
    model.decorations.push({
      from: labelOpening.to,
      to: labelClosing.from,
      decoration: Decoration.mark({
        class: href
          ? "live-inline-segment is-link"
          : "live-inline-segment is-unsafe-link",
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
