import { syntaxTree } from '@codemirror/language';
import { RangeSet, StateEffect } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin } from '@codemirror/view';
import { documentSearchMatches } from './codeMirrorDocumentSearch';
import { isMultiCursorGesture } from './codeMirrorMultiCursor';
import {
  addBlockDecorations,
  addCodeFenceDecorations,
  addTableDecorations,
  blockquoteDepthAt,
  renderedListIndentations
} from './liveMarkdownBlockDecorations';
import { parseLiveMarkdownCodeFences } from './liveMarkdownCode';
import {
  liveMarkdownDocumentModel,
  liveMarkdownDocumentModelField
} from './liveMarkdownDocumentModel';
import { liveMarkdownHeadingFoldingExtension } from './liveMarkdownHeadingFolding';
import {
  addInlineDecorations,
  addWikiLinkDecorations,
  supportedMarkdownLinkRanges
} from './liveMarkdownInlineDecorations';
import {
  constructIsRevealed,
  inlineMarkupMouseSelection,
  selectionRendering,
  tableCellCaretAssociation
} from './liveMarkdownSelection';
import { parseMarkdownNoteTarget } from './markdownLinks';
import type { EditorState, Extension } from '@codemirror/state';
import type { DecorationSet, ViewUpdate } from '@codemirror/view';
import type { LiveMarkdownRange } from './liveMarkdown';
import type { LiveMarkdownModel, LiveMarkdownOptions } from './liveMarkdownDecorationModel';

export { orderedListRenumberingExtension } from './liveMarkdownOrderedLists';
export type { LiveMarkdownOptions } from './liveMarkdownDecorationModel';

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
      readonly options: LiveMarkdownOptions
    ) {
      this.parsedTree = syntaxTree( view.state );
      this.searchMatches = documentSearchMatches( view.state );
      this.model = parseLiveMarkdownModel( view.state, options );
      this.render( view );
    }

    update( update: ViewUpdate ): void {
      const nextTree = syntaxTree( update.state );
      const syntaxTreeChanged = nextTree !== this.parsedTree;
      const nextSearchMatches = documentSearchMatches( update.state );
      const searchMatchesChanged = nextSearchMatches !== this.searchMatches;
      const refreshed = update.transactions.some( ( transaction ) =>
        transaction.effects.some( ( effect ) => effect.is( refreshLiveMarkdownEffect ) )
      );
      if ( refreshed ) {
        this.wikiResolutionVersion += 1;
      }
      if ( update.docChanged || syntaxTreeChanged || refreshed ) {
        this.parsedTree = nextTree;
        this.model = parseLiveMarkdownModel(
          update.state,
          this.options,
          this.wikiResolutionVersion
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
        this.render( update.view );
      }
    }

    private render( view: EditorView ): void {
      const decorations = this.model.decorations.map( ({ decoration, from, to }) =>
        decoration.range( from, to )
      );
      const atomicRanges = [];

      for ( const construct of this.model.constructs ) {
        if ( constructIsRevealed( view, construct ) ) {
          for ( const syntax of construct.syntax ) {
            decorations.push(
              Decoration.mark({ class: 'live-markdown-syntax' }).range(
                syntax.from,
                syntax.to
              )
            );
          }

          continue;
        }

        for ( const { decoration, from, to } of construct.renderedDecorations ) {
          decorations.push( decoration.range( from, to ) );
        }

        for ( const range of construct.atomicRanges ) {
          atomicRanges.push(
            Decoration.mark({}).range( range.from, range.to )
          );
        }

        for ( const syntax of construct.syntax ) {
          decorations.push(
            Decoration.replace({
              inclusive: false,
              ...( syntax.widget ? { widget: syntax.widget } : {})
            }).range( syntax.from, syntax.to )
          );
          atomicRanges.push(
            Decoration.replace({ inclusive: false }).range( syntax.from, syntax.to )
          );
        }
      }

      this.decorations = Decoration.set( decorations, true );
      this.atomicRanges = Decoration.set( atomicRanges, true );
    }
  },
  {
    decorations: ( plugin ) => plugin.decorations,
    eventHandlers: {
      click( event, view ) {
        const link = renderedMarkdownLink( event.target, view );
        if ( !link ) {
          return false;
        }
        if ( isMultiCursorGesture( event ) ) {
          event.preventDefault();

          return true;
        }

        const rawHref = link.dataset.liveHref ?? '';
        const href = rawHref.startsWith( '//' ) ? `https:${ rawHref }` : rawHref;
        if ( parseMarkdownNoteTarget( rawHref ) ) {
          this.options.openLink( rawHref );
        } else if ( !window.__TAURI__ ) {
          return false;
        } else if ( /^(?:https?|mailto):/i.test( href ) ) {
          this.options.openLink( href );
        }

        event.preventDefault();
        event.stopPropagation();

        return true;
      },
      mousedown( event, view ) {
        const link = renderedMarkdownLink( event.target, view );
        if ( !link || event.button !== 0 || isMultiCursorGesture( event ) ) {
          return false;
        }

        const rawHref = link.dataset.liveHref ?? '';
        if ( window.__TAURI__ || parseMarkdownNoteTarget( rawHref ) ) {
          return true;
        }

        event.stopPropagation();

        return false;
      }
    },
    provide: ( plugin ) => EditorView.atomicRanges.of( ( view ) =>
      view.plugin( plugin )?.atomicRanges ?? RangeSet.empty
    )
  }
);

export function liveMarkdownExtension( options: LiveMarkdownOptions ): Extension {
  return [
    liveMarkdownDocumentModelField,
    liveMarkdownHeadingFoldingExtension( options.documentId ),
    tableCellCaretAssociation,
    selectionRendering,
    liveMarkdownPlugin.of( options ),
    EditorView.mouseSelectionStyle.of( inlineMarkupMouseSelection )
  ];
}

function parseLiveMarkdownModel(
  state: EditorState,
  options: LiveMarkdownOptions,
  wikiResolutionVersion = 0
): LiveMarkdownModel {
  const value = state.doc.toString();
  const tree = syntaxTree( state );
  const { blocks, tables } = liveMarkdownDocumentModel( state );
  const listIndentations = renderedListIndentations( blocks );
  const codeFences = parseLiveMarkdownCodeFences( value );
  const codeLines = new Set(
    codeFences.flatMap( ( fence ) => fence.lineNumbers )
  );
  const tableLines = new Set( tables.flatMap( ( table ) => table.lineNumbers ) );
  const model: LiveMarkdownModel = {
    constructs: [],
    decorations: []
  };
  const excludedRanges: LiveMarkdownRange[] = [];
  let renderedQuotePrefix: { depth: number; source: string } | undefined;

  for ( const block of blocks ) {
    if (
      codeLines.has( block.lineNumber ) ||
      tableLines.has( block.lineNumber )
    ) {
      continue;
    }
    if ( block.type === 'frontmatter' ) {
      excludedRanges.push({ from: block.from, to: block.end });

      continue;
    }

    // The line model only sees literal `>` prefixes. The Markdown tree owns
    // container semantics, including unmarked lazy paragraph continuations.
    const literalQuoteDepth = block.quote?.depth ?? 0;
    const quoteDepth = Math.max(
      literalQuoteDepth,
      blockquoteDepthAt( tree, block.from )
    );
    if ( !quoteDepth ) {
      renderedQuotePrefix = undefined;
    } else if ( literalQuoteDepth === quoteDepth ) {
      // Keep partially marked lines from replacing the full-width prefix that
      // deeper lazy continuations reuse.
      renderedQuotePrefix = {
        depth: quoteDepth,
        source: value.slice( block.from, block.content.from )
      };
    }

    addBlockDecorations(
      model,
      block,
      value,
      options,
      quoteDepth,
      renderedQuotePrefix?.depth === quoteDepth
        ? renderedQuotePrefix.source
        : undefined,
      listIndentations.get( block.from )
    );
  }

  for ( const fence of codeFences ) {
    excludedRanges.push({ from: fence.from, to: fence.to });
    addCodeFenceDecorations( model, state, fence );
  }
  for ( const table of tables ) {
    excludedRanges.push({
      from: table.delimiter.from,
      to: table.delimiter.end
    });
    addTableDecorations( model, state, table );
  }

  const markdownLinkRanges = supportedMarkdownLinkRanges(
    state,
    excludedRanges
  );
  const wikiRanges = addWikiLinkDecorations(
    model,
    value,
    excludedRanges,
    markdownLinkRanges,
    options,
    wikiResolutionVersion
  );
  const inlineMarkupExcludedRanges = [
    ...excludedRanges,
    ...wikiRanges,
    ...blocks.flatMap( ( block ) => block.syntax )
  ];
  addInlineDecorations(
    model,
    state,
    value,
    excludedRanges,
    wikiRanges,
    inlineMarkupExcludedRanges,
    options,
    wikiResolutionVersion
  );

  return model;
}

function renderedMarkdownLink(
  eventTarget: EventTarget | null,
  view: EditorView
): HTMLAnchorElement | undefined {
  if ( !( eventTarget instanceof Element ) ) {
    return undefined;
  }

  const link = eventTarget.closest<HTMLAnchorElement>( 'a[data-live-href]' );

  return link && view.dom.contains( link ) ? link : undefined;
}
