import { StateField } from '@codemirror/state';
import { parseLiveMarkdownBlocks } from './liveMarkdown';
import { parseLiveMarkdownTables } from './liveMarkdownTable';
import type { EditorState, Text } from '@codemirror/state';
import type { LiveMarkdownBlock } from './liveMarkdown';
import type { LiveMarkdownTable } from './liveMarkdownTable';

export interface LiveMarkdownDocumentModel {
  blocks: readonly LiveMarkdownBlock[];
  tableCellCaretEnds: ReadonlySet<number>;
  tables: readonly LiveMarkdownTable[];
}

const documentModelCache = new WeakMap<Text, LiveMarkdownDocumentModel>();

export const liveMarkdownDocumentModelField =
  StateField.define<LiveMarkdownDocumentModel>({
    create( state ) {
      return liveMarkdownDocumentModelForText( state.doc );
    },
    update( model, transaction ) {
      return transaction.docChanged
        ? liveMarkdownDocumentModelForText( transaction.newDoc )
        : model;
    }
  });

export function liveMarkdownDocumentModel(
  state: EditorState
): LiveMarkdownDocumentModel {
  return state.field( liveMarkdownDocumentModelField );
}

export function liveMarkdownDocumentModelForText(
  document: Text
): LiveMarkdownDocumentModel {
  const cached = documentModelCache.get( document );
  if ( cached ) {
    return cached;
  }

  const value = document.toString();
  const blocks = parseLiveMarkdownBlocks( value );
  const tables = parseLiveMarkdownTables( value, blocks );
  const model: LiveMarkdownDocumentModel = {
    blocks,
    tableCellCaretEnds: new Set( tables.flatMap( ( table ) =>
      [ table.header, ...table.rows ].flatMap( ( row ) =>
        row.cells.slice( 0, table.columnCount )
          .filter( ( cell ) => cell.editableFrom < cell.editableTo )
          .map( ( cell ) => cell.editableTo )
      )
    ) ),
    tables
  };
  documentModelCache.set( document, model );

  return model;
}
