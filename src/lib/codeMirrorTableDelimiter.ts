import { EditorSelection, Transaction } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { liveMarkdownDocumentModel } from './liveMarkdownDocumentModel';
import { isLiveMarkdownTableDelimiterCandidate } from './liveMarkdownTable';
import type { Extension } from '@codemirror/state';

const SMART_DASHES = new Set([ '–', '—' ]);

export const tableDelimiterHyphenExtension: Extension =
  EditorView.inputHandler.of( preserveTableDelimiterHyphens );

function preserveTableDelimiterHyphens(
  view: EditorView,
  from: number,
  to: number,
  text: string
): boolean {
  if ( view.composing || !SMART_DASHES.has( text ) ) {
    return false;
  }

  const value = view.state.doc.toString();

  if ( !isLiveMarkdownTableDelimiterCandidate(
    value,
    from,
    to,
    liveMarkdownDocumentModel( view.state ).blocks
  ) ) {
    return false;
  }

  // let CodeMirror restore the state's ASCII hyphens through its DOM observer
  // a same-text document transaction would consume or invalidate the previous undo step
  const current = view.state.selection.main;

  if ( current.from === from && current.to === to ) {
    view.dispatch({
      selection: view.state.selection.replaceRange(
        EditorSelection.cursor( to ),
        view.state.selection.mainIndex
      ),
      annotations: Transaction.addToHistory.of( false ),
      scrollIntoView: true,
      userEvent: 'input.type'
    });
  }

  return true;
}
