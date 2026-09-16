import { EditorSelection, EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { documentSearchMatches } from './codeMirrorDocumentSearch';
import { rangesOverlap } from './liveMarkdownDecorationModel';
import {
  liveMarkdownDocumentModel,
  liveMarkdownDocumentModelField,
  liveMarkdownDocumentModelForText
} from './liveMarkdownDocumentModel';
import { inlineMarkupSpans } from './liveMarkdownInlineDecorations';
import type { SelectionRange } from '@codemirror/state';
import type { MouseSelectionStyle } from '@codemirror/view';
import type { LiveMarkdownBlock } from './liveMarkdown';
import type { LiveConstruct } from './liveMarkdownDecorationModel';
import type { InlineMarkupSpan } from './liveMarkdownInlineDecorations';

const inlineMarkupFirstClick = new WeakMap<
  EditorView,
  { x: number; y: number }
>();

export const tableCellCaretAssociation = EditorState.transactionFilter.of(
  ( transaction ) => {
    if ( !transaction.selection && !transaction.docChanged ) {
      return transaction;
    }

    const documentModel = transaction.docChanged
      ? liveMarkdownDocumentModelForText( transaction.newDoc )
      : liveMarkdownDocumentModel( transaction.startState );
    const ranges = transaction.newSelection.ranges.map( ( range ) => {
      if (
        !range.empty ||
        range.assoc < 0 ||
        !documentModel.tableCellCaretEnds.has( range.head )
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
        range.goalColumn
      );
    });
    if ( ranges.every( ( range, index ) =>
      range === transaction.newSelection.ranges[ index ]
    ) ) {
      return transaction;
    }

    return [
      transaction,
      {
        selection: EditorSelection.create(
          ranges,
          transaction.newSelection.mainIndex
        ),
        sequential: true
      }
    ];
  }
);

export const selectionRendering = EditorView.editorAttributes.compute(
  [ liveMarkdownDocumentModelField, 'selection' ],
  ( state ): Record<string, string> => selectionNeedsDrawnHighlight( state )
    ? { class: 'live-drawn-selection' }
    : {}
);

function selectionNeedsDrawnHighlight( state: EditorState ): boolean {
  // The browser can only paint the primary selection.
  if ( state.selection.ranges.length > 1 ) {
    return true;
  }
  const selectedLineBoundaries = new Set<number>();
  for ( const range of state.selection.ranges ) {
    if ( range.empty ) {
      continue;
    }
    for ( const endpoint of [ range.from, range.to ]) {
      if ( state.doc.lineAt( endpoint ).from === endpoint ) {
        selectedLineBoundaries.add( endpoint );
      }
    }
  }
  if ( !selectedLineBoundaries.size ) {
    return false;
  }

  return liveMarkdownDocumentModel( state ).blocks.some( ( block ) =>
    ( block.type === 'list' || block.type === 'task' ) &&
    selectedLineBoundaries.has( block.from )
  );
}

export function inlineMarkupMouseSelection(
  view: EditorView,
  event: MouseEvent
): MouseSelectionStyle | null {
  if (
    event.button !== 0 ||
    event.altKey ||
    event.ctrlKey ||
    event.metaKey
  ) {
    inlineMarkupFirstClick.delete( view );
    return null;
  }

  if ( event.detail === 2 ) {
    const firstClick = inlineMarkupFirstClick.get( view );
    inlineMarkupFirstClick.delete( view );
    const samePosition = firstClick !== undefined &&
      Math.abs( firstClick.x - event.clientX ) <= 4 &&
      Math.abs( firstClick.y - event.clientY ) <= 4;
    const range = samePosition
      ? inlineMarkupDoubleClickRange( view, event )
      : undefined;
    return range ? fixedMouseSelection( view, range ) : null;
  }
  if ( event.detail !== 1 ) {
    inlineMarkupFirstClick.delete( view );
    return null;
  }

  const target = event.target instanceof Element ? event.target : null;
  if ( target?.closest( '.live-inline-segment' ) ) {
    inlineMarkupFirstClick.set( view, { x: event.clientX, y: event.clientY });
  } else {
    inlineMarkupFirstClick.delete( view );
  }

  let startSelection = view.state.selection;
  let start = inlineMarkupPointerPosition( view, event );

  return {
    get( currentEvent, extend, multiple ) {
      const current = inlineMarkupPointerPosition( view, currentEvent );
      const range = start.pos === current.pos
        ? EditorSelection.cursor( current.pos, current.assoc )
        : EditorSelection.range(
          start.pos,
          current.pos,
          undefined,
          undefined,
          current.assoc
        );

      if ( extend ) {
        return startSelection.replaceRange(
          startSelection.main.extend( range.from, range.to, range.assoc )
        );
      }
      if ( multiple ) {
        return startSelection.addRange( range );
      }

      return EditorSelection.create([ range ]);
    },
    update( update ) {
      if ( !update.docChanged ) {
        return false;
      }

      start = {
        pos: update.changes.mapPos( start.pos, start.assoc ),
        assoc: start.assoc
      };
      startSelection = startSelection.map( update.changes );

      return false;
    }
  };
}

function inlineMarkupDoubleClickRange(
  view: EditorView,
  event: MouseEvent
): SelectionRange | undefined {
  const native = view.posAndSideAtCoords({
    x: event.clientX,
    y: event.clientY
  }, false );
  const line = view.state.doc.lineAt( native.pos );
  // The first click may reveal a span's delimiters and move them under the
  // pointer. Resolve the second click from the source span so the resulting
  // word range is independent of that intervening render.
  const spans = inlineMarkupSpans( view.state, line.from, line.to )
    .filter( ( span ) => native.pos >= span.from && native.pos <= span.to )
    .sort( ( left, right ) =>
      ( left.to - left.from ) - ( right.to - right.from )
    );

  for ( const span of spans ) {
    const probe = Math.max(
      span.contentFrom,
      Math.min( native.pos, span.contentTo )
    );
    const word = view.state.wordAt( probe ) ??
      ( probe > span.contentFrom ? view.state.wordAt( probe - 1 ) : null );
    if ( !word ) {
      continue;
    }

    const from = Math.max( word.from, span.contentFrom );
    const to = Math.min( word.to, span.contentTo );
    if ( from < to ) {
      return EditorSelection.range( from, to, undefined, undefined, -1 );
    }
  }

  return undefined;
}

function fixedMouseSelection(
  view: EditorView,
  initialRange: SelectionRange
): MouseSelectionStyle {
  let startSelection = view.state.selection;
  let range = initialRange;

  return {
    get( _currentEvent, extend, multiple ) {
      if ( extend ) {
        return startSelection.replaceRange(
          startSelection.main.extend( range.from, range.to, range.assoc )
        );
      }
      if ( multiple ) {
        return startSelection.addRange( range );
      }

      return EditorSelection.create([ range ]);
    },
    update( update ) {
      if ( !update.docChanged ) {
        return false;
      }

      range = range.map( update.changes );
      startSelection = startSelection.map( update.changes );

      return false;
    }
  };
}

function inlineMarkupPointerPosition(
  view: EditorView,
  event: MouseEvent
): { assoc: -1 | 1; pos: number } {
  const native = listBoundaryPointerPosition( view, event ) ??
    tableCellPointerPosition( view, event ) ??
    view.posAndSideAtCoords({
      x: event.clientX,
      y: event.clientY
    }, false );
  const line = view.state.doc.lineAt( native.pos );
  let opening: number | undefined;
  let closing: number | undefined;

  for ( const span of inlineMarkupSpans( view.state, line.from, line.to ) ) {
    if ( inlineMarkupSpanIsRevealed( view, span ) ) {
      continue;
    }

    const start = view.coordsAtPos( span.contentFrom, 1 );
    const end = view.coordsAtPos( span.contentTo, -1 );
    if ( !start || !end ) {
      continue;
    }

    const startX = ( start.left + start.right ) / 2;
    const endX = ( end.left + end.right ) / 2;
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
        : Math.min( opening, span.from );
    }
    if (
      afterContent &&
      native.pos >= span.contentTo &&
      native.pos <= span.to
    ) {
      closing = closing === undefined
        ? span.to
        : Math.max( closing, span.to );
    }
  }

  if ( opening !== undefined && closing !== undefined && opening === closing ) {
    return { pos: opening, assoc: native.assoc };
  }
  if ( opening !== undefined ) {
    return { pos: opening, assoc: 1 };
  }
  if ( closing !== undefined ) {
    return { pos: closing, assoc: -1 };
  }

  return native;
}

function listBoundaryPointerPosition(
  view: EditorView,
  event: MouseEvent
): { assoc: -1 | 1; pos: number } | undefined {
  const target = event.target instanceof Element ? event.target : null;
  if ( target?.closest( '.live-task-checkbox' ) ) {
    return undefined;
  }

  const line = target?.closest<HTMLElement>( '.cm-line.is-rendered-list' ) ??
    [ ...view.contentDOM.querySelectorAll<HTMLElement>(
      '.cm-line.is-rendered-list'
    ) ].find( ( candidate ) => {
      const bounds = candidate.getBoundingClientRect();
      return event.clientY >= bounds.top && event.clientY <= bounds.bottom;
    });
  if ( !line ) {
    return undefined;
  }

  const linePosition = view.posAtDOM( line, 0 );
  const sourceLine = view.state.doc.lineAt( linePosition );
  const block = liveMarkdownDocumentModel( view.state ).blocks.find( ( candidate ) =>
    candidate.from === sourceLine.from &&
    ( candidate.type === 'list' || candidate.type === 'task' )
  );
  if ( !block ) {
    return undefined;
  }

  const control = line.querySelector<HTMLElement>(
    block.type === 'task' ? '.live-task-checkbox' : '.live-list-marker'
  );
  if ( !control ) {
    return undefined;
  }

  const controlBounds = renderedListControlBounds( control, block.type );
  const lineBounds = line.getBoundingClientRect();
  const lineHeight = Number.parseFloat( getComputedStyle( line ).lineHeight );
  const firstLineBottom = Number.isFinite( lineHeight )
    ? lineBounds.top + lineHeight
    : controlBounds.bottom;
  if (
    event.clientY < lineBounds.top ||
    event.clientY > firstLineBottom
  ) {
    return undefined;
  }

  const contentStart = view.coordsAtPos( block.content.from, 1 );
  const contentBounds = view.contentDOM.getBoundingClientRect();
  let position: number;
  let assoc: -1 | 1;
  if (
    event.clientX >= contentBounds.left &&
    event.clientX < controlBounds.left
  ) {
    position = block.from;
    assoc = -1;
  } else if (
    contentStart &&
    event.clientX > controlBounds.right &&
    event.clientX <= contentStart.left
  ) {
    position = block.content.from;
    assoc = 1;
  } else {
    return undefined;
  }

  return { assoc, pos: position };
}

function renderedListControlBounds(
  control: HTMLElement,
  blockType: LiveMarkdownBlock[ 'type' ]
): DOMRect {
  const text = control.firstChild;
  if ( blockType === 'task' || !text ) {
    return control.getBoundingClientRect();
  }

  const range = control.ownerDocument.createRange();
  range.selectNodeContents( control );
  const bounds = range.getBoundingClientRect();

  return bounds.width ? bounds : control.getBoundingClientRect();
}

function tableCellPointerPosition(
  view: EditorView,
  event: MouseEvent
): { assoc: -1 | 1; pos: number } | undefined {
  if ( !( event.target instanceof Element ) ) {
    return undefined;
  }

  const cellElement = event.target.closest<HTMLElement>(
    '.live-table-cell[data-column-index]'
  );
  if ( !cellElement || !view.dom.contains( cellElement ) ) {
    return undefined;
  }

  const columnIndex = Number( cellElement.dataset.columnIndex );
  if ( !Number.isInteger( columnIndex ) ) {
    return undefined;
  }

  const native = view.posAndSideAtCoords({
    x: event.clientX,
    y: event.clientY
  }, false );
  const cell = liveMarkdownDocumentModel( view.state ).tables.flatMap( ( table ) =>
    [ table.header, ...table.rows ].map( ( row ) =>
      native.pos >= row.from && native.pos <= row.to
        ? row.cells[ columnIndex ]
        : undefined
    )
  ).find( ( candidate ) => candidate !== undefined );
  if (
    !cell ||
    native.pos < cell.to ||
    view.state.sliceDoc( cell.to, cell.editableTo ).trim()
  ) {
    return undefined;
  }

  return { pos: cell.to, assoc: -1 };
}

function inlineMarkupSpanIsRevealed(
  view: EditorView,
  span: InlineMarkupSpan
): boolean {
  if ( documentSearchMatches( view.state ).some( ( match ) =>
    span.syntax.some( ( syntax ) => rangesOverlap( match, syntax ) )
  ) ) {
    return true;
  }
  if ( !view.hasFocus ) {
    return false;
  }

  return view.state.selection.ranges.some( ( selection ) => {
    if ( selection.empty ) {
      return span.syntax.some( ( syntax ) =>
        selection.head >= syntax.from && selection.head <= syntax.to
      );
    }

    return span.syntax.some( ( syntax ) => rangesOverlap( selection, syntax ) );
  });
}

export function constructIsRevealed(
  view: EditorView,
  construct: LiveConstruct
): boolean {
  if ( documentSearchMatches( view.state ).some( ( match ) =>
    construct.syntax.some( ( syntax ) => rangesOverlap( match, syntax ) )
  ) ) {
    return true;
  }
  if ( !view.hasFocus ) {
    return false;
  }

  return view.state.selection.ranges.some( ( selection ) =>
    selectionRevealsConstruct( selection, construct )
  );
}

function selectionRevealsConstruct(
  selection: SelectionRange,
  construct: LiveConstruct
): boolean {
  if ( selection.empty ) {
    const insideSyntax = construct.syntax.some( ( syntax ) =>
      selection.head > syntax.from && selection.head < syntax.to
    );
    if ( insideSyntax && construct.revealWithinSyntax ) {
      return true;
    }
    if ( construct.boundaryReveal === 'syntax' ) {
      return construct.syntax.some( ( syntax ) =>
        selection.head === syntax.from || selection.head === syntax.to
      );
    }
    if ( construct.boundaryReveal === 'construct' ) {
      return selection.head === construct.from || selection.head === construct.to;
    }

    return false;
  }

  if (
    construct.revealWhenSelectedWithin &&
    selection.from >= construct.from &&
    selection.to <= construct.to
  ) {
    return true;
  }

  return construct.syntax.some( ( syntax ) =>
    selection.from < syntax.to && selection.to > syntax.from
  );
}
