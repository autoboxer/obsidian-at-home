import { addCursorAbove, addCursorBelow } from '@codemirror/commands';
import { EditorSelection, EditorState } from '@codemirror/state';
import {
  EditorView,
  keymap,
  rectangularSelection,
  ViewPlugin
} from '@codemirror/view';
import type { ChangeSet, Extension, SelectionRange } from '@codemirror/state';
import type { Command } from '@codemirror/view';
import type { LiveMarkdownTextEdit } from './liveMarkdown';

export interface MultiCursorEdit {
  changes: readonly LiveMarkdownTextEdit[];
  // Like changeByRange, this range is relative to this edit's own result.
  range: SelectionRange;
}

export function multiCursorChanges(
  state: EditorState,
  edits: readonly MultiCursorEdit[]
): { changes: ChangeSet; selection: EditorSelection } | undefined {
  const key = ( change: LiveMarkdownTextEdit ): string => JSON.stringify([ change.from, change.to, change.insert ]);
  const unique = new Map( edits.flatMap( ( edit ) => edit.changes )
    .filter( ( change ) => change.from !== change.to || change.insert )
    .map( ( change ) => [ key( change ), change ]) );
  const ordered = [ ...unique.values() ].sort( ( a, b ) => a.from - b.from || a.to - b.to );
  const merged: { change: LiveMarkdownTextEdit; keys: string[] }[] = [];
  for ( const change of ordered ) {
    const previous = merged.at( -1 );
    if ( previous && ( previous.change.to > change.from || previous.change.from === change.from ) ) {
      if ( previous.change.insert || change.insert ) {
        // Conflicting structural edits cannot be applied independently.
        return undefined;
      }
      previous.change.to = Math.max( previous.change.to, change.to );
      previous.keys.push( key( change ) );
    } else {
      merged.push({ change: { ...change }, keys: [ key( change ) ] });
    }
  }

  const changes = state.changes( merged.map( ( item ) => item.change ) );
  const starts = new Map<string, number>();
  let offset = 0;
  for ( const { change, keys } of merged ) {
    for ( const id of keys ) {
      starts.set( id, change.from + offset );
    }
    offset += change.insert.length - ( change.to - change.from );
  }
  const ranges = edits.map( ( edit ) => {
    const ownChanges = [ ...edit.changes ].sort( ( a, b ) => a.from - b.from || a.to - b.to );
    const mapPosition = ( position: number, assoc: number ): number => {
      let ownOffset = 0;
      for ( const change of ownChanges ) {
        const start = change.from + ownOffset;
        if ( position < start ) {
          break;
        }
        const mappedStart = starts.get( key( change ) );
        if ( mappedStart !== undefined && position <= start + change.insert.length ) {
          return mappedStart + position - start;
        }
        ownOffset += change.insert.length - ( change.to - change.from );
      }

      return changes.mapPos( position - ownOffset, assoc );
    };
    const { range } = edit;
    const head = mapPosition( range.head, range.assoc || 1 );

    return range.empty
      ? EditorSelection.cursor( head, range.assoc, range.bidiLevel ?? undefined, range.goalColumn )
      : EditorSelection.range(
        mapPosition( range.anchor, range.anchor < range.head ? 1 : -1 ),
        head,
        range.goalColumn,
        range.bidiLevel ?? undefined,
        range.assoc
      );
  });

  return { changes, selection: EditorSelection.create( ranges, state.selection.mainIndex ) };
}

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
    // Linux desktops commonly reserve Ctrl+Alt+arrows for workspace switching.
    { linux: 'Ctrl-Shift-ArrowUp', run: addCursorAbove, preventDefault: true },
    { linux: 'Ctrl-Shift-ArrowDown', run: addCursorBelow, preventDefault: true },
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
