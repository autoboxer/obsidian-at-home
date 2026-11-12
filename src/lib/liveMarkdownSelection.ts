import { EditorSelection, EditorState, StateEffect } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { documentSearchMatches } from './codeMirrorDocumentSearch';
import { rangesOverlap } from './liveMarkdownDecorationModel';
import {
  liveMarkdownDocumentModel,
  liveMarkdownDocumentModelField,
  liveMarkdownDocumentModelForText
} from './liveMarkdownDocumentModel';
import { inlineMarkupSpans } from './liveMarkdownInlineDecorations';
import { wikiLinkPointerPosition } from './liveMarkdownCodeMirrorWidgets';
import type { SelectionRange } from '@codemirror/state';
import type { MouseSelectionStyle, ViewUpdate } from '@codemirror/view';
import type { LiveMarkdownBlock } from './liveMarkdown';
import type { LiveConstruct } from './liveMarkdownDecorationModel';
import type { InlineMarkupSpan } from './liveMarkdownInlineDecorations';
import type { LiveMarkdownTableCell } from './liveMarkdownTable';

const inlineMarkupFirstClick = new WeakMap<
  EditorView,
  {
    x: number;
    y: number;
    doc: EditorState[ 'doc' ];
    cell?: LiveMarkdownTableCell;
    word?: SelectionRange;
  }
>();

export const finishTablePointerSelection = StateEffect.define<null>();

interface TablePointerSelection {
  focused: boolean;
  selection: EditorSelection;
  link: Element | null;
  moved: boolean;
  pending: boolean;
  schedule: ( activate?: () => void ) => void;
  cleanup: () => void;
}

const TABLE_LINK_CLICK_DELAY = 500;
const tablePointerSelections = new WeakMap<EditorView, TablePointerSelection>();

export function tablePointerSelectionActive( view: EditorView ): boolean {
  return tablePointerSelections.has( view );
}

export function clearTablePointerSelection( view: EditorView ): void {
  tablePointerSelections.get( view )?.cleanup();
  tablePointerSelections.delete( view );
}

export function updateTablePointerSelection( update: ViewUpdate ): void {
  const pointer = tablePointerSelections.get( update.view );
  if ( pointer && (
    update.docChanged ||
    ( update.focusChanged && !update.view.hasFocus ) ||
    ( pointer.pending && update.selectionSet &&
      !update.state.selection.eq( update.startState.selection ) )
  ) ) {
    clearTablePointerSelection( update.view );
  }
}

export function handleTableLinkClick(
  view: EditorView,
  event: MouseEvent,
  link: Element,
  activate: () => void
): boolean {
  if ( !link.closest( '.live-table-cell' ) || event.ctrlKey || event.metaKey || event.detail === 0 ) {
    return false;
  }
  const pointer = tablePointerSelections.get( view );
  if ( event.detail === 1 && pointer?.link === link && !pointer.moved && !event.shiftKey && !event.altKey ) {
    pointer.schedule( activate );
  }

  return true;
}

function holdTablePointerPresentation( view: EditorView, link: Element | null ): void {
  const previous = tablePointerSelections.get( view );
  const focused = previous?.focused ?? view.hasFocus;
  const selection = previous?.selection ?? view.state.selection;
  clearTablePointerSelection( view );
  const document = view.dom.ownerDocument;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const finish = () => {
    clearTablePointerSelection( view );
    view.dispatch({ effects: finishTablePointerSelection.of( null ) });
  };
  const pointer: TablePointerSelection = {
    focused,
    selection,
    link,
    moved: false,
    pending: false,
    schedule: ( activate ) => {
      clearTimeout( timer );
      pointer.pending = true;
      timer = setTimeout( () => {
        finish();
        activate?.();
      }, TABLE_LINK_CLICK_DELAY );
    },
    cleanup: () => {
      clearTimeout( timer );
      document.removeEventListener( 'mouseup', release );
      document.removeEventListener( 'mousedown', nextPress, true );
      document.removeEventListener( 'keydown', finish, true );
      document.removeEventListener( 'dragend', finish );
      document.defaultView?.removeEventListener( 'blur', finish );
    }
  };
  const release = () => {
    if ( pointer.link && !pointer.moved ) {
      // Keep the original anchor through click dispatch and the multi-click
      // interval, including when focusing reveals the retained caret's source.
      pointer.schedule();
    } else {
      finish();
    }
  };
  const nextPress = ( event: MouseEvent ) => {
    // Even a press handled by a widget or context menu cancels navigation.
    // Retain the presentation until that gesture can resolve its target.
    clearTimeout( timer );
    pointer.pending = false;
    pointer.link = null;
    if ( !( event.target instanceof Node ) || !view.dom.contains( event.target ) ) {
      finish();
    }
  };
  document.addEventListener( 'mouseup', release );
  document.addEventListener( 'mousedown', nextPress, true );
  document.addEventListener( 'keydown', finish, true );
  document.addEventListener( 'dragend', finish );
  document.defaultView?.addEventListener( 'blur', finish );
  tablePointerSelections.set( view, pointer );
  // Refresh atomic ranges before CodeMirror starts the gesture. A wiki label
  // can then select source characters inside its replacement widget.
  view.dispatch({ effects: finishTablePointerSelection.of( null ) });
}

function presentationSelection( view: EditorView ) {
  // Revealing syntax while dragging moves the text beneath the pointer. Keep
  // the starting presentation until release, then reveal the selected source.
  return tablePointerSelections.get( view ) ?? {
    focused: view.hasFocus,
    selection: view.state.selection
  };
}

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
  const target = event.target instanceof Element ? event.target : null;
  const cell = tableCellAtPointer( view, event );
  const link = cell ? target?.closest( 'a[data-live-href], a[data-wiki-target]' ) ?? null : null;
  const activationLink = event.detail === 1 && !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey
    ? link : null;
  if ( event.button === 0 && ( cell || tablePointerSelectionActive( view ) ) ) {
    holdTablePointerPresentation( view, activationLink );
  }
  if ( event.button !== 0 || event.altKey || event.ctrlKey || event.metaKey ) {
    inlineMarkupFirstClick.delete( view );
    return null;
  }

  if ( event.detail === 2 || event.detail === 3 ) {
    const firstClick = inlineMarkupFirstClick.get( view );
    const samePosition = firstClick !== undefined &&
      firstClick.doc === view.state.doc &&
      Math.abs( firstClick.x - event.clientX ) <= 4 &&
      Math.abs( firstClick.y - event.clientY ) <= 4;
    const selectedCell = samePosition ? firstClick.cell : cell;
    const range = event.detail === 3
      ? selectedCell && EditorSelection.range( selectedCell.from, selectedCell.to )
      : samePosition ? firstClick.word : undefined;
    return range
      ? fixedMouseSelection( view, range, selectedCell ? event : undefined )
      : null;
  }
  if ( event.detail !== 1 ) {
    inlineMarkupFirstClick.delete( view );
    return null;
  }

  if ( cell || target?.closest( '.live-inline-segment' ) ) {
    inlineMarkupFirstClick.set( view, {
      x: event.clientX,
      y: event.clientY,
      doc: view.state.doc,
      cell,
      word: cell
        ? tableWordAtPointer( view, event, cell )
        : inlineMarkupWordAtPointer( view, event )
    });
  } else {
    inlineMarkupFirstClick.delete( view );
  }

  let startSelection = view.state.selection;
  let start = inlineMarkupPointerPosition( view, event );

  return {
    get( currentEvent, extend, multiple ) {
      const pointer = tablePointerSelections.get( view );
      if ( activationLink && pointer && !extend && !multiple ) {
        pointer.moved ||= Math.hypot(
          currentEvent.clientX - event.clientX,
          currentEvent.clientY - event.clientY
        ) > 4;
        if ( !pointer.moved ) {
          return startSelection;
        }
      }
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

function inlineMarkupWordAtPointer(
  view: EditorView,
  event: MouseEvent
): SelectionRange | undefined {
  const native = view.posAndSideAtCoords({
    x: event.clientX,
    y: event.clientY
  }, false );
  const line = view.state.doc.lineAt( native.pos );
  // Capture the word before the first click reveals delimiters and moves text
  // beneath the pointer. The second click must reuse these source positions.
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
  initialRange: SelectionRange,
  tableEvent?: MouseEvent
): MouseSelectionStyle {
  let startSelection = view.state.selection;
  let startRange = initialRange;

  return {
    get( currentEvent, extend, multiple ) {
      let range = startRange;
      if (
        tableEvent &&
        ( currentEvent.clientX !== tableEvent.clientX ||
          currentEvent.clientY !== tableEvent.clientY )
      ) {
        const cell = tableCellAtPointer( view, currentEvent );
        const position = inlineMarkupPointerPosition( view, currentEvent );
        const current = tableEvent.detail === 3 && cell
          ? EditorSelection.range( cell.from, cell.to )
          : tableWordAtPointer( view, currentEvent, cell ) ??
            EditorSelection.cursor( position.pos, position.assoc );
        range = extend ? current : current.from < startRange.from
          ? EditorSelection.range( startRange.to, current.from )
          : EditorSelection.range( startRange.from, Math.max( startRange.to, current.to ) );
      }
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

      startRange = startRange.map( update.changes );
      startSelection = startSelection.map( update.changes );

      return false;
    }
  };
}

function tableWordAtPointer(
  view: EditorView,
  event: MouseEvent,
  cell?: LiveMarkdownTableCell
): SelectionRange | undefined {
  const position = inlineMarkupPointerPosition( view, event ).pos;
  const word = view.state.wordAt( position );
  if ( !word ) {
    return undefined;
  }
  const from = Math.max( word.from, cell?.from ?? 0 );
  const to = Math.min( word.to, cell?.to ?? view.state.doc.length );

  return from < to ? EditorSelection.range( from, to ) : undefined;
}

function inlineMarkupPointerPosition(
  view: EditorView,
  event: MouseEvent
): { assoc: -1 | 1; pos: number } {
  const wikiPosition = wikiLinkPointerPosition( view, event );
  if ( wikiPosition !== undefined ) {
    return { pos: wikiPosition, assoc: 1 };
  }
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
  const cell = tableCellAtPointer( view, event );
  if ( !cell ) {
    return undefined;
  }
  const native = view.posAndSideAtCoords({
    x: event.clientX,
    y: event.clientY
  }, false );
  if (
    native.pos < cell.to ||
    view.state.sliceDoc( cell.to, cell.editableTo ).trim()
  ) {
    return undefined;
  }

  return { pos: cell.to, assoc: -1 };
}

function tableCellAtPointer(
  view: EditorView,
  event: MouseEvent
): LiveMarkdownTableCell | undefined {
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

  const position = view.posAtDOM( cellElement );
  return liveMarkdownDocumentModel( view.state ).tables.flatMap( ( table ) =>
    [ table.header, ...table.rows ].map( ( row ) =>
      position >= row.from && position <= row.to
        ? row.cells[ columnIndex ]
        : undefined
    )
  ).find( ( candidate ) => candidate !== undefined );
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
  const presentation = presentationSelection( view );
  if ( !presentation.focused ) {
    return false;
  }

  return presentation.selection.ranges.some( ( selection ) =>
    ( selection.from >= span.from && selection.to <= span.to ) ||
    span.syntax.some( ( syntax ) => rangesOverlap( selection, syntax ) )
  );
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
  const presentation = presentationSelection( view );
  if ( !presentation.focused ) {
    return false;
  }

  return presentation.selection.ranges.some( ( selection ) =>
    selectionRevealsConstruct( selection, construct )
  );
}

function selectionRevealsConstruct(
  selection: SelectionRange,
  construct: LiveConstruct
): boolean {
  // A caret inside formatted text needs the same source access as a selection.
  if (
    construct.revealWhenSelectedWithin &&
    selection.from >= construct.from &&
    selection.to <= construct.to
  ) {
    return true;
  }

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

  return construct.syntax.some( ( syntax ) =>
    selection.from < syntax.to && selection.to > syntax.from
  );
}
