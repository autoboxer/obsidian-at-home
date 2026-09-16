import { EditorSelection } from '@codemirror/state';
import { lineNumbers } from '@codemirror/view';
import {
  joinLeadingFrontmatter,
  leadingFrontmatterEnd,
  markdownBodyStart,
  splitLeadingFrontmatter
} from './frontmatter';
import type { Extension } from '@codemirror/state';
import type { ViewUpdate } from '@codemirror/view';
import type { NoteEditorHistorySnapshot } from '../stores/editorHistories';
import type { NoteEditorPosition } from '../types';

export function minimalDocumentChange(
  current: string,
  next: string
): { from: number; to: number; insert: string } {
  let from = 0;
  const sharedLength = Math.min( current.length, next.length );
  while ( from < sharedLength && current[ from ] === next[ from ]) {
    from += 1;
  }

  let currentTo = current.length;
  let nextTo = next.length;
  while (
    currentTo > from &&
    nextTo > from &&
    current[ currentTo - 1 ] === next[ nextTo - 1 ]
  ) {
    currentTo -= 1;
    nextTo -= 1;
  }

  return {
    from,
    to: currentTo,
    insert: next.slice( from, nextTo )
  };
}

export function preferredLineEnding( value: string ): '\n' | '\r' | '\r\n' {
  const lineEnding = value.match( /\r\n|\r|\n/ )?.[ 0 ];

  return lineEnding === '\r\n' || lineEnding === '\r'
    ? lineEnding
    : '\n';
}

export function normalizeDocumentText( value: string ): string {
  return value.replace( /\r\n|\r/g, '\n' );
}

export function projectEditableDocument(
  normalizedMarkdown: string,
  showFrontmatter: boolean
) {
  if ( !showFrontmatter ) {
    return splitLeadingFrontmatter( normalizedMarkdown );
  }

  return {
    body: normalizedMarkdown,
    bodyStart: 0,
    lineNumberOffset: 0,
    prefix: ''
  };
}

export function restorableEditorHistory(
  snapshot: NoteEditorHistorySnapshot | undefined,
  normalizedMarkdown: string,
  editableDocument: ReturnType<typeof projectEditableDocument>
): NoteEditorHistorySnapshot | undefined {
  if ( !snapshot ) {
    return undefined;
  }
  if ( snapshot.doc === editableDocument.body ) {
    return snapshot;
  }
  const savedMarkdown = joinLeadingFrontmatter( snapshot.prefix, snapshot.doc );
  if ( savedMarkdown !== normalizedMarkdown ) {
    return undefined;
  }
  const savedBodyStart = markdownBodyStart( snapshot.prefix, snapshot.doc );
  const hidingEditedFrontmatter = savedBodyStart === 0
    && editableDocument.bodyStart > 0
    && snapshot.frontmatterHistoryChanged;

  return hidingEditedFrontmatter ? undefined : snapshot;
}

export function frontmatterVisibilitySelection(
  selection: EditorSelection,
  currentBodyStart: number,
  editableDocument: ReturnType<typeof projectEditableDocument>
): EditorSelection {
  const clamp = ( position: number ): number => Math.min(
    editableDocument.body.length,
    Math.max( 0, position + currentBodyStart - editableDocument.bodyStart )
  );

  return EditorSelection.create( selection.ranges.map( ( range ) =>
    EditorSelection.range( clamp( range.anchor ), clamp( range.head ) )
  ), selection.mainIndex );
}

export function changeTouchesLeadingFrontmatter( update: ViewUpdate ): boolean {
  const previousEnd = leadingFrontmatterEnd( update.startState.doc.toString() ) ?? 0;
  const nextEnd = leadingFrontmatterEnd( update.state.doc.toString() ) ?? 0;
  let touchesFrontmatter = previousEnd !== nextEnd;
  update.changes.iterChangedRanges( ( fromA, _toA, fromB ) => {
    if ( fromA < previousEnd || fromB < nextEnd ) {
      touchesFrontmatter = true;
    }
  });

  return touchesFrontmatter;
}

export function normalizeInitialPosition(
  position: NoteEditorPosition | undefined,
  documentLength: number,
  bodyStart: number
): NoteEditorPosition | undefined {
  if ( !position ) {
    return undefined;
  }

  const clamp = ( value: number ): number => Math.min(
    documentLength,
    Math.max( 0, Math.trunc( value ) - bodyStart )
  );

  return {
    selection: {
      anchor: clamp( position.selection.anchor ),
      head: clamp( position.selection.head )
    },
    viewport: {
      anchor: clamp( position.viewport.anchor ),
      offset: position.viewport.offset,
      left: Math.max( 0, position.viewport.left )
    }
  };
}

export function restoreLineEndings(
  value: string,
  lineEnding: '\n' | '\r' | '\r\n'
): string {
  return lineEnding === '\n' ? value : value.replace( /\n/g, lineEnding );
}

export function editorLineNumbers( offset: number ): Extension {
  return lineNumbers({
    formatNumber: ( lineNumber ) => String( lineNumber + offset )
  });
}
