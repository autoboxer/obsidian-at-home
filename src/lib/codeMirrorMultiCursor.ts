import { EditorSelection, EditorState } from '@codemirror/state';
import {
  EditorView,
  keymap,
  rectangularSelection,
  ViewPlugin
} from '@codemirror/view';
import type { Extension, SelectionRange } from '@codemirror/state';
import type { Command } from '@codemirror/view';

export const selectNextOccurrence: Command = ( view ) => {
  if ( view.composing ) {
    return false;
  }
  const { state } = view;
  const selection = state.selection;
  if ( selection.ranges.some( ( range ) => range.empty ) ) {
    const ranges = selection.ranges.map( ( range ) =>
      range.empty ? state.wordAt( range.head ) ?? range : range
    );
    if ( ranges.some( ( range, index ) => range !== selection.ranges[ index ]) ) {
      return selectRanges( view, ranges, selection.mainIndex );
    }
    if ( selection.main.empty ) {
      return true;
    }
  }

  const text = state.sliceDoc( selection.main.from, selection.main.to );
  const document = state.doc.toString();
  // Search past the main range first, then wrap. Skip overlaps so adding a
  // match never merges or changes an existing selection.
  for ( const [ start, end ] of [[ selection.main.to, document.length ], [ 0, selection.main.from ]]) {
    let index = 0;
    for ( let from = document.indexOf( text, start ); from >= 0 && from < end; from = document.indexOf( text, from + 1 ) ) {
      while ( index < selection.ranges.length && selection.ranges[ index ]!.to <= from ) {
        index += 1;
      }
      const next = selection.ranges[ index ];
      if ( next && next.from < from + text.length ) {
        continue;
      }
      view.dispatch({
        selection: selection.addRange( EditorSelection.range( from, from + text.length ) ),
        scrollIntoView: true,
        userEvent: 'select.multi'
      });

      return true;
    }
  }

  return true;
};

export const selectAllOccurrences: Command = ( view ) => {
  if ( view.composing ) {
    return false;
  }
  const { state } = view;
  const main = state.selection.main;
  const target = main.empty ? state.wordAt( main.head ) : main;
  if ( !target || target.empty ) {
    return true;
  }

  const text = state.sliceDoc( target.from, target.to );
  const document = state.doc.toString();
  const ranges: SelectionRange[] = [];
  // Keep the original occurrence and direction, even when possible matches
  // overlap it (for example, selecting the middle "aa" in "aaaa").
  for ( let from = document.indexOf( text ); from >= 0 && from + text.length <= target.from; from = document.indexOf( text, from + text.length ) ) {
    ranges.push( EditorSelection.range( from, from + text.length ) );
  }
  const mainIndex = ranges.length;
  ranges.push( target );
  for ( let from = document.indexOf( text, target.to ); from >= 0; from = document.indexOf( text, from + text.length ) ) {
    ranges.push( EditorSelection.range( from, from + text.length ) );
  }

  return selectRanges( view, ranges, mainIndex );
};

export function isMultiCursorGesture( event: MouseEvent ): boolean {
  return event.button === 0 && event.altKey && !event.ctrlKey && !event.metaKey &&
    !( event.target instanceof Element && event.target.closest( 'input, textarea, select' ) );
}

const rectangularSelectionCursor = ViewPlugin.fromClass( class {
  active = false;

  constructor( readonly view: EditorView ) {}

  set( event?: KeyboardEvent | MouseEvent ): void {
    const active = Boolean( event?.altKey && event.shiftKey && !event.ctrlKey && !event.metaKey );
    if ( active !== this.active ) {
      this.active = active;
      this.view.update([]);
    }
  }
}, {
  eventObservers: {
    keydown( event ) {
      this.set( event );
    },
    keyup( event ) {
      this.set( event );
    },
    mousemove( event ) {
      this.set( event );
    },
    mousedown( event ) {
      this.set( event );
    },
    blur() {
      this.set();
    }
  }
});

export const codeMirrorMultiCursorExtension: Extension = [
  EditorState.allowMultipleSelections.of( true ),
  EditorView.clickAddsSelectionRange.of( ( event ) => isMultiCursorGesture( event ) && !event.shiftKey ),
  rectangularSelection({ eventFilter: ( event ) => isMultiCursorGesture( event ) && event.shiftKey }),
  rectangularSelectionCursor,
  EditorView.contentAttributes.of( ( view ) => view.plugin( rectangularSelectionCursor )?.active
    ? { style: 'cursor: crosshair' } : null ),
  keymap.of([
    { key: 'Mod-d', run: selectNextOccurrence, preventDefault: true },
    { key: 'Mod-Shift-l', run: selectAllOccurrences, preventDefault: true }
  ])
];

function selectRanges(
  view: EditorView,
  ranges: readonly SelectionRange[],
  mainIndex: number
): boolean {
  view.dispatch({
    selection: EditorSelection.create( ranges, mainIndex ),
    scrollIntoView: true,
    userEvent: 'select.multi'
  });

  return true;
}
