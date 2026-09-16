import { markdownLanguage } from '@codemirror/lang-markdown';
import { EditorState, Transaction } from '@codemirror/state';
import { liveMarkdownListGroups, nextOrderedListNumber } from './liveMarkdown';
import { liveMarkdownDocumentModelForText } from './liveMarkdownDocumentModel';
import type { Extension, Text } from '@codemirror/state';
import type {
  LiveMarkdownBlock,
  LiveMarkdownListGroup,
  LiveMarkdownRange,
  LiveMarkdownTextEdit
} from './liveMarkdown';

interface PreviousOrderedItem {
  block: LiveMarkdownBlock;
  group: LiveMarkdownListGroup;
  index: number;
}

export function orderedListRenumberingExtension(
  skip: ( transaction: Transaction ) => boolean
): Extension {
  return EditorState.transactionFilter.of( ( transaction ) => {
    if (
      !transaction.docChanged || transaction.startState.readOnly ||
      transaction.isUserEvent( 'undo' ) || transaction.isUserEvent( 'redo' ) ||
      transaction.annotation( Transaction.remote ) || skip( transaction )
    ) {
      return transaction;
    }

    const edits = orderedListRenumberingEdits( transaction );

    // Compose the repair with the original edit, mapping every selection and
    // effect through it and retaining one history entry.
    return edits.length
      ? [ transaction, { changes: edits, sequential: true }]
      : transaction;
  });
}

function orderedListRenumberingEdits( transaction: Transaction ): LiveMarkdownTextEdit[] {
  const ranges: { fromA: number; toA: number; fromB: number; toB: number }[] = [];
  transaction.changes.iterChangedRanges( ( fromA, toA, fromB, toB ) => {
    ranges.push({ fromA, toA, fromB, toB });
  });
  const before = transaction.startState.doc;
  const after = transaction.newDoc;
  if ( !ranges.some( ( range ) =>
    before.lineAt( range.fromA ).number !== before.lineAt( range.toA ).number ||
    after.lineAt( range.fromB ).number !== after.lineAt( range.toB ).number ||
    touchesOrderedListPrefix( before, range.fromA ) ||
    touchesOrderedListPrefix( after, range.fromB )
  ) ) {
    return [];
  }

  const previousGroups = orderedListGroupsForText( before );
  const nextGroups = orderedListGroupsForText( after );
  const previousByLine = new Map<number, PreviousOrderedItem>();
  for ( const group of previousGroups ) {
    for ( const [ index, block ] of group.items.entries() ) {
      // Include exact line boundaries: a removed item must not become the
      // identity of the following line when its former position maps there.
      if ( ranges.some( ( range ) => range.fromA <= block.from && range.toA >= block.to ) ) {
        continue;
      }
      const position = transaction.changes.mapPos( block.content.from, 1 );
      const line = after.lineAt( position );
      if ( !previousByLine.has( line.from ) ) {
        previousByLine.set( line.from, { block, group, index });
      }
    }
  }
  const survivors = new Set( nextGroups.flatMap( ( group ) =>
    group.items.flatMap( ( block ) => {
      const previous = previousByLine.get( block.from );

      return previous ? [ previous.block ] : [];
    })
  ) );
  const edits: LiveMarkdownTextEdit[] = [];
  for ( const group of nextGroups ) {
    const previousItems = group.items.map( ( block ) => previousByLine.get( block.from ) );
    const previousGroup = previousItems.find( ( item ) => item )?.group;
    // A newly pasted, independent list has no existing siblings to repair.
    if ( !previousGroup ) {
      continue;
    }
    const first = group.items[ 0 ]!;
    const previousFirst = previousItems[ 0 ];
    const parent = listGroupParentItem( group );
    const previousParent = listGroupParentItem( previousGroup );
    // A sibling can become the new parent without changing the remaining
    // children's indentation. Moving their existing parent with them keeps
    // the same list, even when the whole subtree changes indentation.
    const newlyNested = Boolean( parent && previousFirst && (
      !previousParent ||
      ranges.some( ( range ) =>
        range.fromA <= previousParent.from && range.toA >= previousParent.end
      ) ||
      after.lineAt( transaction.changes.mapPos( previousParent.content.from, 1 ) ).from !== parent.from
    ) );
    const affected = previousGroup.items.length !== group.items.length ||
      newlyNested ||
      previousItems.some( ( previous, index ) => !previous ||
        previous.group !== previousGroup || previous.index !== index ||
        sourceListNumber( previous.block ) !== sourceListNumber( group.items[ index ]! ) ||
        previous.block.list!.indentation !== group.indentation
      );
    if ( !affected ) {
      continue;
    }

    const oldFirst = previousGroup.items[ 0 ]!;
    const oldStart = transaction.changes.mapPos( oldFirst.from, -1 );
    const deletedStart = previousFirst && previousFirst.index > 0 &&
      !survivors.has( oldFirst ) && oldStart <= first.from &&
      liveMarkdownDocumentModelForText( after ).blocks
        .filter( ( block ) => block.from >= oldStart && block.from < first.from )
        .every( ( block ) => block.type === 'text' ||
          ( block.list && block.list.indentation > group.indentation )
        );
    const repeatedOnes = previousGroup.items.every( ( block ) => sourceListNumber( block ) === 1 );
    let number = newlyNested ? 1 : deletedStart ? sourceListNumber( oldFirst ) : sourceListNumber( first );
    let repairing = Boolean( deletedStart || newlyNested );
    let previousIndex = deletedStart ? 0 : previousFirst?.index ?? 0;

    for ( const [ index, block ] of group.items.entries() ) {
      const previous = previousItems[ index ];
      const marker = block.list!.marker;
      const sourceNumber = sourceListNumber( block );
      const numberChanged = previous && sourceListNumber( previous.block ) !== sourceNumber;
      const numberTyped = !previous && ranges.some( ( range ) =>
        range.fromB <= marker.to && range.toB >= marker.from &&
        before.lineAt( range.fromA ).number === before.lineAt( range.toA ).number &&
        after.lineAt( range.fromB ).number === after.lineAt( range.toB ).number
      );
      if ( numberChanged || numberTyped ) {
        number = sourceNumber;
        repairing = true;
      } else if ( !previous || previous.group !== previousGroup ||
        previous.index !== previousIndex || previous.block.list!.indentation !== group.indentation ) {
        repairing = true;
      }
      if ( !repairing ) {
        number = repeatedOnes ? index + 1 : sourceNumber;
      }
      if ( sourceNumber !== number ) {
        edits.push({ from: marker.from, to: marker.to - 1, insert: String( number ) });
      }
      number = nextOrderedListNumber( number );
      if ( previous?.group === previousGroup ) {
        previousIndex = previous.index + 1;
      }
    }
  }

  return edits.sort( ( left, right ) => left.from - right.from );
}

function listGroupParentItem( group: LiveMarkdownListGroup ): LiveMarkdownBlock | undefined {
  const items = group.parent?.items ?? [];
  for ( let index = items.length - 1; index >= 0; index -= 1 ) {
    if ( items[ index ]!.from < group.items[ 0 ]!.from ) {
      return items[ index ];
    }
  }

  return undefined;
}

function orderedListGroupsForText( document: Text ): LiveMarkdownListGroup[] {
  const { blocks, tables } = liveMarkdownDocumentModelForText( document );
  const tableLines = new Set( tables.flatMap( ( table ) => table.lineNumbers ) );
  // The line model intentionally accepts loose list indentation. The Markdown
  // grammar distinguishes literal examples in nested fences and indented code.
  const protectedRanges: ( LiveMarkdownRange & { insideList: boolean })[] = [];
  markdownLanguage.parser.parse( document.toString() ).iterate({
    enter( node ) {
      if ( node.name === 'FencedCode' || node.name === 'CodeBlock' ||
        node.name === 'HTMLBlock' || node.name === 'CommentBlock' ||
        node.name === 'ProcessingInstructionBlock' ) {
        let parent = node.node.parent;
        while ( parent && parent.name !== 'ListItem' ) {
          parent = parent.parent;
        }
        protectedRanges.push({ from: node.from, to: node.to, insideList: Boolean( parent ) });

        return false;
      }
    }
  });
  let protectedIndex = 0;
  const editableBlocks = blocks.map( ( block ) => {
    if ( tableLines.has( block.lineNumber ) ) {
      return { ...block, list: undefined, type: 'blank' as const };
    }
    while ( protectedIndex < protectedRanges.length && protectedRanges[ protectedIndex ]!.to <= block.from ) {
      protectedIndex += 1;
    }
    const range = protectedRanges[ protectedIndex ];

    // Standalone blocks separate lists. Blocks belonging to a list item keep
    // its sequence intact, including blank lines inside the literal content.
    return range && range.from < block.to && block.from < range.to
      ? { ...block, list: undefined, type: range.insideList ? 'text' as const : 'blank' as const }
      : block;
  });

  return liveMarkdownListGroups( editableBlocks ).filter( ( group ) => group.ordered );
}

function sourceListNumber( block: LiveMarkdownBlock ): number {
  return Number.parseInt( block.source.slice( block.list!.marker.from - block.from ), 10 );
}

function touchesOrderedListPrefix( document: Text, position: number ): boolean {
  const line = document.lineAt( position );
  const prefix = line.text.match( /^[ \t]*\d{1,9}[.)](?:[ \t]|$)/ )?.[ 0 ];

  return Boolean( prefix && position <= line.from + prefix.length );
}
