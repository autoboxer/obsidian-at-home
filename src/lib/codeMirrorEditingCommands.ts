import { isolateHistory } from '@codemirror/commands';
import {
  getIndentation,
  getIndentUnit,
  IndentContext,
  indentString,
  syntaxTree
} from '@codemirror/language';
import { NodeProp } from '@lezer/common';
import { countColumn, EditorSelection, findClusterBreak } from '@codemirror/state';
import { Direction, EditorView } from '@codemirror/view';
import { notify } from '../stores/vault';
import { multiCursorChanges, type MultiCursorEdit } from './codeMirrorMultiCursor';
import { minimalDocumentChange } from './codeMirrorDocument';
import { liveMarkdownDocumentModel } from './liveMarkdownDocumentModel';
import {
  deleteEmptyLiveMarkdownTableRow,
  insertLiveMarkdownTableLineBreak,
  insertLiveMarkdownTableRow,
  isLiveMarkdownTableCellBoundary,
  liveMarkdownTableCellTextBounds,
  moveAcrossLiveMarkdownTableCellBoundary,
  navigateLiveMarkdownTable,
  type LiveMarkdownTableNavigation
} from './liveMarkdownTableNavigation';
import { toggleInlineFormatting, wrapInlineCode, wrapMarkdownLink } from './markdownFormatting';
import type { SelectionRange } from '@codemirror/state';
import type { Command } from '@codemirror/view';
import type { LiveMarkdownTextEdit } from './liveMarkdown';
import type { MarkdownSelectionEdit } from './markdownFormatting';

const INDENT = '  ';

interface ListLine {
  indent: string;
  ordered: boolean;
  number: number;
  delimiter: '.' | ')';
  bullet: '-' | '+' | '*';
  spacing: string;
  contentOffset: number;
  task: boolean;
}

export const handleEnter: Command = ( view ) => enterAtSelections( view, false );
export const handleShiftEnter: Command = ( view ) => enterAtSelections( view, true );

function enterAtSelections( view: EditorView, soft: boolean ): boolean {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }
  const value = view.state.doc.toString();
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  const edits = view.state.selection.ranges.map( ( range ) => {
    const tableEdit = soft
      ? insertLiveMarkdownTableLineBreak( value, tables, range.anchor, range.head )
      : insertLiveMarkdownTableRow( value, tables, range.head );
    return tableEdit
      ? markdownRangeEdit( value, tableEdit )
      : soft ? softLineBreakEdit( view, range )
        : smartEnterEdit( view, range ) ?? replacementRangeEdit( range, '\n' );
  });

  return applyRangeEdits( view, edits, 'input' );
}

function applyTableNavigation(
  view: EditorView,
  navigation: LiveMarkdownTableNavigation
): boolean {
  const value = view.state.doc.toString();
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  const edits = view.state.selection.ranges.map( ( range ) =>
    navigateLiveMarkdownTable( value, tables, range.head, navigation )
  );
  if ( !edits.some( ( edit ) => edit ) ) {
    return false;
  }

  return applyRangeEdits( view, edits.map( ( edit, index ) => {
    const range = view.state.selection.ranges[ index ]!;
    if ( edit && ( !view.state.readOnly || edit.value === value ) ) {
      return markdownRangeEdit( value, edit );
    }
    const down = navigation === 'down-row';
    return { changes: [], range: view.moveVertically( range, down ) };
  }), 'select.table' );
}

export const handleTab: Command = ( view ) => indentSelections( view, false );
export const handleShiftTab: Command = ( view ) => indentSelections( view, true );

function indentSelections( view: EditorView, outdent: boolean ): boolean {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }
  const value = view.state.doc.toString();
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  return applyRangeEdits( view, view.state.selection.ranges.map( ( range ) => {
    const tableEdit = navigateLiveMarkdownTable(
      value, tables, range.head, outdent ? 'previous-cell' : 'next-cell'
    );
    if ( tableEdit ) {
      return markdownRangeEdit( value, tableEdit );
    }
    return selectedLineEdits( view, range, outdent ) ??
      ( outdent ? { changes: [], range } : replacementRangeEdit( range, INDENT ) );
  }), 'input.indent' );
}

export const handleTableArrowDown: Command = ( view ) => {
  if ( view.composing ) {
    return false;
  }

  return applyTableNavigation( view, 'down-row' );
};

export const handleTableArrowUp: Command = ( view ) => {
  if ( view.composing ) {
    return false;
  }

  return applyTableNavigation( view, 'up-row' );
};

function deleteAtTableSelections(
  view: EditorView,
  forward: boolean,
  unit: 'character' | 'word' | 'line'
): boolean {
  if ( view.composing || view.state.readOnly ) {
    return false;
  }
  const value = view.state.doc.toString();
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  if ( !view.state.selection.ranges.some( ( range ) =>
    liveMarkdownTableCellTextBounds( tables, range.head )
  ) ) {
    return false;
  }
  const deletedRows = !forward && unit === 'character'
    ? view.state.selection.ranges.flatMap( ( range ) => {
      const edit = range.empty ? deleteEmptyLiveMarkdownTableRow( value, tables, range.head ) : undefined;
      return edit?.change ? [ edit ] : [];
    }) : [];
  const survivingRowTarget = ( position: number ): number => {
    for ( let index = deletedRows.length - 1; index >= 0; index -= 1 ) {
      const edit = deletedRows[ index ]!;
      if ( edit.change!.from <= position && position < edit.change!.to ) {
        position = edit.selectionStart;
      }
    }
    return position;
  };
  const skipAtomic = ( position: number, after: boolean ): number => {
    for ( const ranges of view.state.facet( EditorView.atomicRanges ) ) {
      ranges( view ).between( position, position, ( from, to ) => {
        if ( from < position && position < to ) {
          position = after ? to : from;
        }
      });
    }
    return position;
  };
  return applyRangeEdits( view, view.state.selection.ranges.map( ( range ) => {
    if ( range.empty && !forward && unit === 'character' ) {
      const rowEdit = deleteEmptyLiveMarkdownTableRow( value, tables, range.head );
      if ( rowEdit ) {
        const edit = markdownRangeEdit( value, rowEdit );
        edit.range = EditorSelection.cursor( survivingRowTarget( rowEdit.selectionStart ), -1 );
        return edit;
      }
      const target = survivingRowTarget( range.head );
      if ( target !== range.head ) {
        return { changes: [], range: EditorSelection.cursor( target, -1 ) };
      }
    }
    let from = range.from;
    let to = range.to;
    if ( range.empty ) {
      const line = view.state.doc.lineAt( range.head );
      let target = range.head;
      if ( unit === 'line' ) {
        target = view.moveToLineBoundary( range, forward ).head;
      } else if ( unit === 'word' ) {
        const categorize = view.state.charCategorizer( range.head );
        let category: ReturnType<typeof categorize> | undefined;
        while ( target !== ( forward ? line.to : line.from ) ) {
          const next = line.from + findClusterBreak( line.text, target - line.from, forward );
          const text = value.slice( Math.min( target, next ), Math.max( target, next ) );
          const nextCategory = categorize( text );
          if ( category !== undefined && category !== nextCategory ) {
            break;
          }
          if ( text !== ' ' || target !== range.head ) {
            category = nextCategory;
          }
          target = next;
        }
      } else {
        target = line.from + findClusterBreak( line.text, range.head - line.from, forward, forward );
        const before = line.text.slice( 0, range.head - line.from );
        if ( !forward && before.length && before.length < 200 && !/[^ \t]/.test( before ) ) {
          const unit = getIndentUnit( view.state );
          const drop = countColumn( before, view.state.tabSize ) % unit || unit;
          target = range.head;
          for ( let index = 0; index < drop && before[ before.length - 1 - index ] === ' '; index += 1 ) {
            target -= 1;
          }
          if ( before.endsWith( '\t' ) ) {
            target -= 1;
          }
        } else if ( !forward && /[\ufe00-\ufe0f]/.test( value.slice( target, range.head ) ) ) {
          target = line.from + findClusterBreak( line.text, target - line.from, false, false );
        }
      }
      if ( target === range.head ) {
        target = Math.max( 0, Math.min( value.length, target + ( forward ? 1 : -1 ) ) );
      }
      const bounds = liveMarkdownTableCellTextBounds( tables, range.head );
      target = bounds
        ? forward
          ? Math.max( range.head, Math.min( target, bounds.to ) )
          : Math.min( range.head, Math.max( target, bounds.from ) )
        : skipAtomic( target, forward );
      from = Math.min( range.head, target );
      to = Math.max( range.head, target );
    } else {
      from = skipAtomic( from, false );
      to = skipAtomic( to, true );
    }
    return {
      changes: from === to ? [] : [{ from, to, insert: '' }],
      range: from === to ? range : EditorSelection.cursor( from, forward ? 1 : -1 )
    };
  }), forward ? 'delete.forward' : 'delete.backward' );
}

export const deleteTableBackward: Command = ( view ) => deleteAtTableSelections( view, false, 'character' );
export const deleteTableForward: Command = ( view ) => deleteAtTableSelections( view, true, 'character' );
export const deleteTableWordBackward: Command = ( view ) => deleteAtTableSelections( view, false, 'word' );
export const deleteTableWordForward: Command = ( view ) => deleteAtTableSelections( view, true, 'word' );
export const deleteToTableCellTextStart: Command = ( view ) => deleteAtTableSelections( view, false, 'line' );
export const deleteToTableCellTextEnd: Command = ( view ) => deleteAtTableSelections( view, true, 'line' );

function moveHorizontal(
  view: EditorView,
  direction: 'left' | 'right',
  extend = false
): boolean {
  if ( view.composing ) {
    return false;
  }
  const value = view.state.doc.toString();
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  let handled = false;
  const ranges = view.state.selection.ranges.map( ( range ) => {
    const forward = ( direction === 'right' ) === ( view.textDirectionAt( range.head ) === Direction.LTR );
    const boundary = forward ? 'end' : 'start';
    if ( extend && isLiveMarkdownTableCellBoundary( tables, range.head, boundary ) ) {
      handled = true;
      return range;
    }
    const tableTarget = range.empty && !extend
      ? moveAcrossLiveMarkdownTableCellBoundary( value, tables, range.head, direction )
      : undefined;
    const line = view.state.doc.lineAt( range.head );
    const offset = renderedListTextOffset( line.text );
    const revealList = !extend && range.empty && !forward && offset !== undefined &&
      range.head === line.from + offset && offset > 0;
    let target: SelectionRange;
    if ( tableTarget ) {
      handled = true;
      target = EditorSelection.cursor( tableTarget.position, tableTarget.assoc );
    } else if ( revealList ) {
      handled = true;
      target = EditorSelection.cursor( range.head - 1 );
    } else {
      target = range.empty || extend
        ? view.moveByChar( range, forward )
        : EditorSelection.cursor( forward ? range.to : range.from );
    }
    return extend
      ? EditorSelection.range( range.anchor, target.head, target.goalColumn, target.bidiLevel ?? undefined, target.assoc )
      : target;
  });
  return handled && applyRangeEdits( view, ranges.map( ( range ) => ({ changes: [], range }) ), 'select' );
}

export const moveAcrossTableCellLeft: Command = ( view ) => moveHorizontal( view, 'left' );
export const moveAcrossTableCellRight: Command = ( view ) => moveHorizontal( view, 'right' );
export const protectTableCellSelectionLeft: Command = ( view ) => moveHorizontal( view, 'left', true );
export const protectTableCellSelectionRight: Command = ( view ) => moveHorizontal( view, 'right', true );

function setTextBoundary(
  view: EditorView,
  boundary: 'end' | 'start' | 'left' | 'right',
  extend: boolean
): boolean {
  if ( view.composing ) {
    return false;
  }
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  let handled = false;
  const ranges = view.state.selection.ranges.map( ( range ) => {
    const forward = boundary === 'end' || ( boundary === 'left' || boundary === 'right' ) &&
      ( ( boundary === 'right' ) === ( view.textDirectionAt( range.head ) === Direction.LTR ) );
    const block = view.lineBlockAt( range.head );
    let target = view.moveToLineBoundary( range, forward );
    if ( target.head === range.head && target.head !== ( forward ? block.to : block.from ) ) {
      target = view.moveToLineBoundary( range, forward, false );
    }
    let position = target.head;
    let assoc = target.assoc;
    const bounds = liveMarkdownTableCellTextBounds( tables, range.head );
    const anchorBounds = extend ? liveMarkdownTableCellTextBounds( tables, range.anchor ) : bounds;
    const line = view.state.doc.lineAt( range.head );
    const offset = renderedListTextOffset( line.text );
    if ( bounds && anchorBounds && bounds.from === anchorBounds.from && bounds.to === anchorBounds.to ) {
      handled = true;
      position = Math.max( bounds.from, Math.min( position, bounds.to ) );
      if ( bounds.from === bounds.to ) {
        assoc = forward ? -1 : 1;
      } else if ( position === bounds.from ) {
        assoc = 1;
      } else if ( position === bounds.to ) {
        assoc = -1;
      }
    } else if ( !forward && offset !== undefined && position <= line.from + offset ) {
      handled = true;
      const textStart = line.from + offset;
      position = range.head < textStart || range.head === textStart && ( extend || range.empty )
        ? line.from : textStart;
    } else if ( !forward && position === block.from && block.length ) {
      const whitespace = view.state.sliceDoc( block.from, block.to ).match( /^\s*/ )![ 0 ].length;
      if ( whitespace && range.head !== block.from + whitespace ) {
        position += whitespace;
      }
    }
    return extend
      ? EditorSelection.range( range.anchor, position, target.goalColumn, target.bidiLevel ?? undefined, assoc )
      : EditorSelection.cursor( position, assoc );
  });
  return handled && applyRangeEdits( view, ranges.map( ( range ) => ({ changes: [], range }) ), 'select' );
}

export const moveToTableCellTextStart: Command = ( view ) => setTextBoundary( view, 'start', false );
export const selectToTableCellTextStart: Command = ( view ) => setTextBoundary( view, 'start', true );
export const moveToTableCellTextEnd: Command = ( view ) => setTextBoundary( view, 'end', false );
export const selectToTableCellTextEnd: Command = ( view ) => setTextBoundary( view, 'end', true );
export const moveToTableCellTextLeft: Command = ( view ) => setTextBoundary( view, 'left', false );
export const selectToTableCellTextLeft: Command = ( view ) => setTextBoundary( view, 'left', true );
export const moveToTableCellTextRight: Command = ( view ) => setTextBoundary( view, 'right', false );
export const selectToTableCellTextRight: Command = ( view ) => setTextBoundary( view, 'right', true );

export const toggleBold: Command = ( view ) => toggleSelectionFormatting( view, '**', [ '__' ]);
export const toggleItalic: Command = ( view ) => toggleSelectionFormatting( view, '*', [ '_' ]);
export const toggleStrikethrough: Command = ( view ) => toggleSelectionFormatting( view, '~~' );
export const insertLiteralHyphen: Command = ( view ) => {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }

  view.dispatch(
    view.state.replaceSelection( '-' ),
    {
      scrollIntoView: true,
      userEvent: 'input.type'
    }
  );

  return true;
};
export const wrapSelectionAsInlineCode: Command = ( view ) => {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }
  const value = view.state.doc.toString();
  return applyRangeEdits( view, view.state.selection.ranges.map( ( range ) =>
    range.empty ? replacementRangeEdit( range, '`' ) :
      markdownRangeEdit( value, wrapInlineCode( value, range.from, range.to ), range )
  ), 'input.format' );
};
export const wrapSelectionAsMarkdownLink: Command = ( view ) => {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }
  const value = view.state.doc.toString();
  return applyRangeEdits( view, view.state.selection.ranges.map( ( range ) =>
    markdownRangeEdit( value, wrapMarkdownLink( value, range.from, range.to ), range )
  ), 'input.format' );
};

function toggleSelectionFormatting(
  view: EditorView,
  marker: string,
  alternatives: string[] = []
): boolean {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }

  const value = view.state.doc.toString();
  return applyRangeEdits( view, view.state.selection.ranges.map( ( range ) =>
    markdownRangeEdit( value, toggleInlineFormatting(
      value, range.from, range.to, marker, alternatives
    ), range )
  ), 'input.format' );
}

function replacementRangeEdit( range: SelectionRange, insert: string ): MultiCursorEdit {
  return {
    changes: [{ from: range.from, to: range.to, insert }],
    range: EditorSelection.cursor( range.from + insert.length )
  };
}

function softLineBreakEdit( view: EditorView, range: SelectionRange ): MultiCursorEdit {
  const { state } = view;
  let { from, to } = range;
  const line = state.doc.lineAt( from );
  let brackets: { from: number; to: number } | undefined;
  if ( range.empty ) {
    if ( /\(\)|\[\]|\{\}/.test( state.sliceDoc( Math.max( 0, from - 1 ), from + 1 ) ) ) {
      brackets = { from, to };
    } else {
      const context = syntaxTree( state ).resolveInner( from );
      const before = context.childBefore( from );
      const after = context.childAfter( from );
      if ( before && after && before.to <= from && after.from >= from &&
        before.type.prop( NodeProp.closedBy )?.includes( after.name ) &&
        state.doc.lineAt( before.to ).from === state.doc.lineAt( after.from ).from &&
        !/\S/.test( state.sliceDoc( before.to, after.from ) ) ) {
        brackets = { from: before.to, to: after.from };
      }
    }
  }
  const context = new IndentContext( state, { simulateBreak: from, simulateDoubleBreak: Boolean( brackets ) });
  const indentation = getIndentation( context, from ) ??
    countColumn( line.text.match( /^\s*/ )![ 0 ], state.tabSize );
  while ( to < line.to && /\s/.test( line.text[ to - line.from ]! ) ) {
    to += 1;
  }
  if ( brackets ) {
    ({ from, to } = brackets );
  } else if ( from > line.from && !/\S/.test( state.sliceDoc( line.from, from ) ) ) {
    from = line.from;
  }
  const indent = indentString( state, indentation );
  const closing = brackets ? `\n${ indentString( state, context.lineIndent( line.from, -1 ) ) }` : '';
  return {
    changes: [{ from, to, insert: `\n${ indent }${ closing }` }],
    range: EditorSelection.cursor( from + 1 + indent.length )
  };
}

function markdownRangeEdit(
  value: string,
  edit: MarkdownSelectionEdit & { change?: LiveMarkdownTextEdit },
  original?: SelectionRange
): MultiCursorEdit {
  return {
    changes: value === edit.value ? [] : [ edit.change ?? minimalDocumentChange( value, edit.value ) ],
    range: original && original.anchor > original.head
      ? EditorSelection.range( edit.selectionEnd, edit.selectionStart )
      : EditorSelection.range( edit.selectionStart, edit.selectionEnd )
  };
}

function applyRangeEdits(
  view: EditorView,
  edits: readonly MultiCursorEdit[],
  userEvent: string
): boolean {
  const result = multiCursorChanges( view.state, edits );
  if ( !result ) {
    notify( 'These selections affect the same text. Adjust the selections and try again.', 'neutral' );
    return true;
  }
  if ( view.state.readOnly && !result.changes.empty ) {
    return false;
  }
  const docChanged = !result.changes.empty;
  view.dispatch( result, {
    annotations: docChanged ? isolateHistory.of( 'full' ) : undefined,
    scrollIntoView: true,
    userEvent: docChanged && userEvent.startsWith( 'select.' ) ? 'input.table' : userEvent
  });
  if ( docChanged ) {
    // Record the final ranges after transaction filters (including renumbering)
    // so redo restores the intentional caret positions in every edited range.
    view.dispatch({ selection: view.state.selection, userEvent: 'select' });
  }
  return true;
}

function smartEnterEdit( view: EditorView, selection: SelectionRange ): MultiCursorEdit | undefined {
  if ( !selection.empty ) {
    return undefined;
  }

  const value = view.state.doc.toString();
  const position = selection.head;
  const line = view.state.doc.lineAt( position );
  const source = line.text;
  const openingFence = source.match( /^([ \t]*)(`{3,}|~{3,})([^\n]*)$/ );
  if (
    openingFence &&
    position === line.to &&
    !activeFenceBefore( value, line.from )
  ) {
    const indent = openingFence[ 1 ]!;
    const marker = openingFence[ 2 ]!;
    const followingLineStart = line.to < value.length ? line.to + 1 : value.length;
    const followingLine = view.state.doc.lineAt( followingLineStart ).text;
    const existingClosing = followingLine.match( /^([ \t]*)(`+|~+)\s*$/ );
    const closesFence = Boolean( existingClosing
      && existingClosing[ 2 ]![ 0 ] === marker[ 0 ]
      && existingClosing[ 2 ]!.length >= marker.length );
    const insertion = closesFence
      ? `\n${ indent }`
      : `\n${ indent }\n${ indent }${ marker }`;
    return {
      changes: [{ from: position, to: position, insert: insertion }],
      range: EditorSelection.cursor( position + 1 + indent.length )
    };
  }

  if ( activeFenceBefore( value, line.from ) ) {
    return undefined;
  }

  const item = matchEditableListLine( source );
  if ( !item ) {
    return undefined;
  }

  const contentStart = line.from + item.contentOffset;
  const insertionPosition = Math.max( position, contentStart );
  const bodyBefore = value.slice( contentStart, insertionPosition );
  const bodyAfter = value.slice( insertionPosition, line.to );
  const fullBody = `${ bodyBefore }${ bodyAfter }`;

  if ( !fullBody.trim() && insertionPosition === line.to ) {
    const replacement = item.indent;
    return {
      changes: [{ from: line.from, to: line.to, insert: replacement }],
      range: EditorSelection.cursor( line.from + replacement.length )
    };
  }

  const marker = item.ordered
    ? `${ item.number >= 999_999_999 ? 1 : item.number + 1 }${ item.delimiter }`
    : item.bullet;
  const taskPrefix = item.task ? '[ ] ' : '';
  const continuation = `${ item.indent }${ marker }${ item.spacing }${ taskPrefix }`;
  const separatorLength = bodyAfter.match( /^[ \t]+/ )?.[ 0 ].length ?? 0;
  const cursor = insertionPosition + 1 + continuation.length;
  return {
    changes: [{ from: insertionPosition, to: insertionPosition + separatorLength, insert: `\n${ continuation }` }],
    range: EditorSelection.cursor( cursor )
  };
}

function matchEditableListLine( line: string ): ListLine | undefined {
  const match = line.match( /^([ \t]*)(?:(\d{1,9})([.)])|([-+*]))([ \t]+)(.*)$/ );
  if ( !match ) {
    return undefined;
  }
  const ordered = Boolean( match[ 2 ]);
  const markerLength = ordered ? match[ 2 ]!.length + 1 : 1;
  const spacing = match[ 5 ]!;
  const listPrefixLength = match[ 1 ]!.length + markerLength + spacing.length;
  const taskPrefix = match[ 6 ]!.match( /^\[[ xX]\][ \t]+/ )?.[ 0 ];

  return {
    indent: match[ 1 ]!,
    ordered,
    number: ordered ? Number.parseInt( match[ 2 ]!, 10 ) : 1,
    delimiter: ordered ? match[ 3 ]! as '.' | ')' : '.',
    bullet: ordered ? '-' : match[ 4 ]! as '-' | '+' | '*',
    spacing,
    contentOffset: listPrefixLength + ( taskPrefix?.length ?? 0 ),
    task: Boolean( taskPrefix )
  };
}

function renderedListTextOffset( line: string ): number | undefined {
  const item = matchEditableListLine( line );
  if ( !item ) {
    return undefined;
  }

  return item.contentOffset;
}

function activeFenceBefore(
  value: string,
  position: number
): { marker: string; length: number } | undefined {
  const lines = value.slice( 0, position ).split( '\n' );
  lines.pop();
  let active: { marker: string; length: number } | undefined;

  for ( const line of lines ) {
    if ( !active ) {
      const opening = line.match( /^[ \t]*(`{3,}|~{3,})[^\n]*$/ );
      if ( opening ) {
        active = { marker: opening[ 1 ]![ 0 ]!, length: opening[ 1 ]!.length };
      }

      continue;
    }

    const closing = line.match( /^[ \t]*(`+|~+)\s*$/ );
    if (
      closing &&
      closing[ 1 ]![ 0 ] === active.marker &&
      closing[ 1 ]!.length >= active.length
    ) {
      active = undefined;
    }
  }

  return active;
}

function selectedLineEdits(
  view: EditorView,
  selection: SelectionRange,
  outdent: boolean
): MultiCursorEdit | undefined {
  const value = view.state.doc.toString();
  const selectionStart = selection.from;
  const selectionEnd = selection.to;
  const hasSelection = !selection.empty;
  const firstLine = view.state.doc.lineAt( selectionStart );

  if ( !hasSelection && !matchEditableListLine( firstLine.text ) ) {
    if ( !outdent || !/^[ \t]/.test( firstLine.text ) ) {
      return undefined;
    }
  }

  const selectionEndsAtLineStart = hasSelection
    && selectionEnd > 0
    && value[ selectionEnd - 1 ] === '\n';
  const effectiveEnd = selectionEndsAtLineStart ? selectionEnd - 1 : selectionEnd;
  const lastLine = view.state.doc.lineAt( effectiveEnd );
  const block = value.slice( firstLine.from, lastLine.to );
  const lines = block.split( '\n' );
  const edits: { from: number; to: number; insert: string }[] = [];
  let sourceOffset = firstLine.from;

  for ( const line of lines ) {
    if ( !outdent ) {
      edits.push({ from: sourceOffset, to: sourceOffset, insert: INDENT });
      sourceOffset += line.length + 1;

      continue;
    }

    const removable = line.startsWith( '\t' ) ? 1 : line.match( /^ {1,2}/ )?.[ 0 ].length ?? 0;
    if ( removable ) {
      edits.push({ from: sourceOffset, to: sourceOffset + removable, insert: '' });
    }
    sourceOffset += line.length + 1;
  }

  return { changes: edits, range: selection.map( view.state.changes( edits ), 1 ) };
}
