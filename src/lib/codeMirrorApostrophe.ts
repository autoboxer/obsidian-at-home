import { Transaction } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import type { ChangeSpec, Extension } from '@codemirror/state';
import type { Command } from '@codemirror/view';

const SMART_QUOTES: Readonly<Record<string, string>> = {
  '‘': "'",
  '’': "'",
  '“': '"',
  '”': '"'
};

export const insertLiteralApostrophe: Command = ( view ) => insertQuote( view, "'" );
export const insertLiteralDoubleQuote: Command = ( view ) => insertQuote( view, '"' );

export const literalApostropheExtension: Extension = [
  EditorView.domEventHandlers({
    beforeinput( event, view ) {
      if (
        view.composing || event.isComposing || view.state.readOnly
        || !event.cancelable || event.inputType !== 'insertText' || !event.data
      ) {
        return false;
      }

      const touchesQuote = /['"]/.test( event.data )
        || view.state.selection.ranges.some( ( range ) =>
          /['"]/.test( view.state.doc.lineAt( range.head ).text )
        );
      if ( !touchesQuote ) {
        return false;
      }

      // Native smart punctuation can rewrite earlier text and its DOM caret.
      // Keep typing on quoted lines in the same editing path as the quote key.
      event.preventDefault();
      view.dispatch( view.state.replaceSelection( event.data ), {
        scrollIntoView: true,
        userEvent: 'input.type'
      });

      return true;
    }
  }),
  EditorView.inputHandler.of( preserveLiteralQuotes )
];

function preserveLiteralQuotes(
  view: EditorView,
  from: number,
  to: number,
  text: string,
  insert: () => Transaction
): boolean {
  if ( view.composing || view.state.readOnly ) {
    return false;
  }

  const correctedInput = correctQuoteSubstitution( view, from, to, text );
  if ( correctedInput === text ) {
    return false;
  }

  const transaction = insert();
  if ( !transaction.isUserEvent( 'input.type' ) || transaction.isUserEvent( 'input.type.compose' ) ) {
    return false;
  }

  const edits: ChangeSpec[] = [];
  transaction.changes.iterChanges( ( fromA, toA, _fromB, _toB, inserted ) => {
    const corrected = correctQuoteSubstitution( view, fromA, toA, inserted.toString() );
    if ( corrected !== view.state.sliceDoc( fromA, toA ) ) {
      edits.push({ from: fromA, to: toA, insert: corrected });
    }
  });
  const changes = view.state.changes( edits );
  const addToHistory = transaction.annotation( Transaction.addToHistory );
  const remote = transaction.annotation( Transaction.remote );

  // A delayed substitution alone must not move the caret or create an undo
  // step. Retain other simultaneous edits, selections, and transaction effects.
  view.dispatch({
    changes,
    selection: changes.empty ? view.state.selection : transaction.newSelection,
    effects: transaction.effects,
    annotations: [
      Transaction.userEvent.of( transaction.annotation( Transaction.userEvent )! ),
      Transaction.time.of( transaction.annotation( Transaction.time )! ),
      ...( addToHistory === undefined ? [] : [ Transaction.addToHistory.of( addToHistory ) ]),
      ...( remote === undefined ? [] : [ Transaction.remote.of( remote ) ])
    ],
    scrollIntoView: !changes.empty && transaction.scrollIntoView
  });

  return true;
}

function correctQuoteSubstitution(
  view: EditorView,
  from: number,
  to: number,
  text: string
): string {
  if ( view.state.selection.ranges.some( ( range ) =>
    !range.empty && range.from === from && range.to === to
  ) ) {
    return text;
  }

  const previous = view.state.sliceDoc( from, to );

  return text.split( '' ).map( ( character, offset ) =>
    SMART_QUOTES[ character ] !== undefined && SMART_QUOTES[ character ] === previous[ offset ]
      ? previous[ offset ]
      : character
  ).join( '' );
}

function insertQuote( view: EditorView, quote: string ): boolean {
  if ( view.composing || view.state.readOnly ) {
    return false;
  }

  view.dispatch( view.state.replaceSelection( quote ), {
    scrollIntoView: true,
    userEvent: 'input.type'
  });

  return true;
}
