import { syntaxTree } from '@codemirror/language';
import { Decoration } from '@codemirror/view';
import { highlightCodeRanges } from './highlight';
import { liveMarkdownListGroups } from './liveMarkdown';
import {
  HorizontalRuleWidget,
  ListMarkerWidget,
  QuoteMarkerWidget,
  renderedListMarker,
  TaskWidget
} from './liveMarkdownCodeMirrorWidgets';
import {
  addConstruct,
  addLineDecoration,
  addMarkDecoration,
  lineDecoration
} from './liveMarkdownDecorationModel';
import {
  CodeFenceFooterWidget,
  CodeFenceHeaderWidget,
  EmptyTableCellWidget,
  TableCellBreakWidget,
  TableDelimiterWidget
} from './liveMarkdownRegionWidgets';
import { parseMarkdownAttachmentAt } from './markdownAttachments';
import type { EditorState } from '@codemirror/state';
import type { SyntaxNode, Tree } from '@lezer/common';
import type { LiveMarkdownBlock, LiveMarkdownListGroup, LiveMarkdownRange } from './liveMarkdown';
import type { LiveMarkdownCodeFence } from './liveMarkdownCode';
import type {
  HiddenSyntax,
  LiveMarkdownModel,
  LiveMarkdownOptions,
  StoredDecoration
} from './liveMarkdownDecorationModel';
import type {
  LiveMarkdownTable,
  LiveMarkdownTableAlignment,
  LiveMarkdownTableRow
} from './liveMarkdownTable';

const LIST_INDENT_STEP_EM = 1.65;
const LIST_MARKER_CHARACTER_WIDTH_EM = 0.9;

export function addBlockDecorations(
  model: LiveMarkdownModel,
  block: LiveMarkdownBlock,
  value: string,
  options: LiveMarkdownOptions,
  quoteDepth = block.quote?.depth ?? 0,
  quotePrefix?: string,
  listIndentation?: number
): void {
  const classes = [
    'live-markdown-block',
    `is-${ quoteDepth ? 'blockquote' : block.type }`
  ];
  if ( block.headingLevel ) {
    classes.push( `heading-level-${ block.headingLevel }` );
  }
  if ( block.list ) {
    classes.push( `list-depth-${ block.list.depth % 3 }` );
    if ( listContentIsAttachmentCard( block, value, options ) ) {
      classes.push( 'is-attachment-card-only' );
    }
  }
  if ( quoteDepth ) {
    classes.push( `quote-depth-${ Math.min( quoteDepth, 3 ) }` );
    if ( !block.quote ) {
      classes.push( 'is-blockquote-continuation' );
    }
  }
  if ( block.task?.checked ) {
    classes.push( 'is-checked' );
  }
  addLineDecoration( model, block.from, classes.join( ' ' ) );

  if ( block.type === 'heading' ) {
    addConstruct( model, block.from, block.to, block.syntax );

    return;
  }
  if ( block.type === 'horizontal-rule' ) {
    addConstruct( model, block.from, block.to, [{
      from: block.from,
      to: block.to,
      widget: new HorizontalRuleWidget( block.from, block.to )
    }]);

    return;
  }
  if ( block.type === 'task' && block.task && block.list ) {
    const markerSource = value.slice( block.from, block.content.from );
    addConstruct( model, block.from, block.content.from, [{
      from: block.from,
      to: block.content.from,
      widget: new TaskWidget(
        markerSource,
        block.task.checked,
        block.task.check.from,
        block.from,
        block.content.from
      )
    }], [ renderedListLineDecoration( block, listIndentation ) ], { boundaryReveal: 'construct' });
    if ( block.task.checked && block.content.from < block.content.to ) {
      addMarkDecoration( model, block.content, 'live-task-content' );
    }

    return;
  }
  if ( block.type === 'list' && block.list ) {
    const markerSource = value.slice( block.from, block.content.from );
    addConstruct( model, block.from, block.content.from, [{
      from: block.from,
      to: block.content.from,
      widget: new ListMarkerWidget(
        markerSource,
        renderedListMarker( block ),
        block.from,
        block.content.from
      )
    }], [ renderedListLineDecoration( block, listIndentation ) ], { boundaryReveal: 'construct' });

    return;
  }
  if ( block.type === 'blockquote' && block.quote ) {
    const prefix = quotePrefix ?? value.slice( block.from, block.content.from );
    addConstruct( model, block.from, block.content.from, [{
      from: block.from,
      to: block.content.from,
      widget: new QuoteMarkerWidget(
        prefix,
        quoteDepth,
        block.from,
        block.content.from
      )
    }]);

    return;
  }
  if ( quoteDepth ) {
    const prefix = quotePrefix ?? '> '.repeat( quoteDepth );
    model.decorations.push({
      from: block.from,
      to: block.from,
      decoration: Decoration.widget({
        side: -1,
        widget: new QuoteMarkerWidget(
          prefix,
          quoteDepth,
          block.from,
          block.from
        )
      })
    });
  }
}

export function blockquoteDepthAt( tree: Tree, position: number ): number {
  let depth = 0;
  let node: SyntaxNode | null = tree.resolve( position, 1 );

  while ( node ) {
    if ( node.name === 'Blockquote' ) {
      depth += 1;
    }
    node = node.parent;
  }

  return depth;
}

export function addCodeFenceDecorations(
  model: LiveMarkdownModel,
  state: EditorState,
  fence: LiveMarkdownCodeFence
): void {
  const opening = state.doc.line( fence.openingLine );
  const openingClasses = [
    'live-markdown-block',
    'is-code-opening',
    ...( fence.lineNumbers.length === 1 ? [ 'is-code-last' ] : [])
  ];
  addConstruct(
    model,
    opening.from,
    opening.to,
    [{
      from: opening.from,
      to: opening.to,
      widget: new CodeFenceHeaderWidget( fence, opening.from, opening.to )
    }],
    [ lineDecoration( opening.from, openingClasses.join( ' ' ) ) ]
  );

  const finalContentLine = fence.closingLine === undefined
    ? fence.lineNumbers.at( -1 )
    : fence.closingLine - 1;
  for (
    let lineNumber = fence.openingLine + 1;
    lineNumber <= ( finalContentLine ?? fence.openingLine );
    lineNumber += 1
  ) {
    const line = state.doc.line( lineNumber );
    const classes = [ 'live-markdown-block', 'is-code-content' ];
    if ( fence.closingLine === undefined && lineNumber === finalContentLine ) {
      classes.push( 'is-code-last' );
    }
    addLineDecoration( model, line.from, classes.join( ' ' ) );
  }

  addCodeHighlightDecorations( model, state, fence );

  if ( fence.closingLine !== undefined ) {
    const closing = state.doc.line( fence.closingLine );
    addConstruct(
      model,
      closing.from,
      closing.to,
      [{
        from: closing.from,
        to: closing.to,
        widget: new CodeFenceFooterWidget( closing.from, closing.to )
      }],
      [ lineDecoration(
        closing.from,
        'live-markdown-block is-code-closing is-code-last'
      ) ]
    );
  }
}

function addCodeHighlightDecorations(
  model: LiveMarkdownModel,
  state: EditorState,
  fence: LiveMarkdownCodeFence
): void {
  if ( !fence.code || !fence.language || fence.openingLine >= state.doc.lines ) {
    return;
  }

  const codeFrom = state.doc.line( fence.openingLine + 1 ).from;
  for ( const range of highlightCodeRanges( fence.code, fence.language ) ) {
    addMultilineMarkDecoration(
      model,
      state,
      codeFrom + range.from,
      codeFrom + range.to,
      range.className
    );
  }
}

function addMultilineMarkDecoration(
  model: LiveMarkdownModel,
  state: EditorState,
  from: number,
  to: number,
  className: string
): void {
  let cursor = from;
  while ( cursor < to ) {
    const line = state.doc.lineAt( cursor );
    const segmentTo = Math.min( to, line.to );
    if ( cursor < segmentTo ) {
      addMarkDecoration( model, { from: cursor, to: segmentTo }, className );
    }
    cursor = segmentTo < to ? line.to + 1 : to;
  }
}

export function addTableDecorations(
  model: LiveMarkdownModel,
  state: EditorState,
  table: LiveMarkdownTable
): void {
  addTableRowDecorations( model, state, table, table.header, 'header', false );

  const delimiterLast = table.rows.length === 0;
  addConstruct(
    model,
    table.delimiter.from,
    table.delimiter.to,
    [{
      from: table.delimiter.from,
      to: table.delimiter.to,
      widget: new TableDelimiterWidget(
        table.delimiter.from,
        table.delimiter.to
      )
    }],
    [ lineDecoration(
      table.delimiter.from,
      [
        'live-markdown-block',
        'is-table-delimiter',
        ...( delimiterLast ? [ 'is-table-last' ] : [])
      ].join( ' ' )
    ) ]
  );

  table.rows.forEach( ( row, index ) => {
    addTableRowDecorations(
      model,
      state,
      table,
      row,
      'body',
      index === table.rows.length - 1
    );
  });
}

function addTableRowDecorations(
  model: LiveMarkdownModel,
  state: EditorState,
  table: LiveMarkdownTable,
  row: LiveMarkdownTableRow,
  role: 'body' | 'header',
  last: boolean
): void {
  const cells = row.cells.slice( 0, table.columnCount );
  const syntax = [
    ...tableRowSyntax( row, cells ),
    ...tableCellBreakSyntax( state, cells )
  ].sort( ( left, right ) => left.from - right.from || left.to - right.to );
  const classes = [
    'live-markdown-block',
    'is-table-row',
    `is-table-${ role }`,
    ...( last ? [ 'is-table-last' ] : [])
  ];
  const renderedDecorations: StoredDecoration[] = [ lineDecoration(
    row.from,
    classes.join( ' ' )
  ) ];

  for ( let index = 0; index < table.columnCount; index += 1 ) {
    const cell = cells[ index ];
    const cellLast = index === table.columnCount - 1;
    const className = tableCellClass(
      table.alignments[ index ],
      cellLast
    );
    if ( cell && cell.editableFrom < cell.editableTo ) {
      renderedDecorations.push({
        from: cell.editableFrom,
        to: cell.editableTo,
        decoration: Decoration.mark({
          attributes: { 'data-column-index': String( index ) },
          class: [ className, ...( !cell.source ? [ 'is-empty' ] : []) ]
            .join( ' ' ),
          inclusive: true
        })
      });
    } else {
      const position = cell?.editableFrom ?? row.to;
      renderedDecorations.push({
        from: position,
        to: position,
        decoration: Decoration.widget({
          side: index + 1,
          widget: new EmptyTableCellWidget( position, index, className )
        })
      });
    }
  }

  addConstruct(
    model,
    row.from,
    row.to,
    syntax,
    renderedDecorations,
    {
      atomicRanges: tableCellTrailingPadding( state, cells ),
      boundaryReveal: 'construct',
      revealWithinSyntax: false
    }
  );
}

function tableCellTrailingPadding(
  state: EditorState,
  cells: readonly LiveMarkdownTableRow[ 'cells' ][ number ][]
): LiveMarkdownRange[] {
  return cells.flatMap( ( cell ) => {
    if (
      cell.to >= cell.editableTo ||
      !/\s/.test( state.sliceDoc( cell.editableTo - 1, cell.editableTo ) )
    ) {
      return [];
    }

    return [{ from: cell.editableTo - 1, to: cell.editableTo }];
  });
}

function tableRowSyntax(
  row: LiveMarkdownTableRow,
  cells: readonly LiveMarkdownTableRow[ 'cells' ][ number ][]
): HiddenSyntax[] {
  const syntax: HiddenSyntax[] = [];
  let cursor = row.from;

  for ( const cell of cells ) {
    if ( cursor < cell.editableFrom ) {
      syntax.push({ from: cursor, to: cell.editableFrom });
    }
    cursor = Math.max( cursor, cell.editableTo );
  }
  if ( cursor < row.to ) {
    syntax.push({ from: cursor, to: row.to });
  }

  return syntax;
}

function tableCellBreakSyntax(
  state: EditorState,
  cells: readonly LiveMarkdownTableRow[ 'cells' ][ number ][]
): HiddenSyntax[] {
  const syntax: HiddenSyntax[] = [];
  const firstCell = cells[ 0 ];
  const lastCell = cells.at( -1 );
  if ( !firstCell || !lastCell ) {
    return syntax;
  }

  syntaxTree( state ).iterate({
    from: firstCell.editableFrom,
    to: lastCell.editableTo,
    enter( reference ) {
      if ( reference.name !== 'HTMLTag' ) {
        return undefined;
      }

      const cell = cells.find( ( candidate ) =>
        reference.from >= candidate.editableFrom &&
        reference.to <= candidate.editableTo
      );
      const source = state.sliceDoc( reference.from, reference.to );
      if (
        !cell ||
        !/^<br[ \t]*\/?>$/i.test( source ) ||
        characterIsEscaped(
          state.sliceDoc( cell.editableFrom, cell.editableTo ),
          reference.from - cell.editableFrom
        )
      ) {
        return false;
      }

      syntax.push({
        from: reference.from,
        to: reference.to,
        widget: new TableCellBreakWidget()
      });

      return false;
    }
  });

  return syntax;
}

function characterIsEscaped( value: string, index: number ): boolean {
  let backslashes = 0;
  for ( let cursor = index - 1; cursor >= 0 && value[ cursor ] === '\\'; cursor -= 1 ) {
    backslashes += 1;
  }

  return backslashes % 2 === 1;
}

function tableCellClass(
  alignment: LiveMarkdownTableAlignment | undefined,
  last: boolean
): string {
  return [
    'live-table-cell',
    ...( alignment ? [ `align-${ alignment }` ] : []),
    ...( last ? [ 'is-last' ] : [])
  ].join( ' ' );
}

function listContentIsAttachmentCard(
  block: LiveMarkdownBlock,
  value: string,
  options: LiveMarkdownOptions
): boolean {
  const content = value.slice( block.content.from, block.content.to );
  const leadingWhitespace = content.match( /^[\t ]*/ )?.[ 0 ].length ?? 0;
  const trailingWhitespace = content.match( /[\t ]*$/ )?.[ 0 ].length ?? 0;
  const from = block.content.from + leadingWhitespace;
  const to = block.content.to - trailingWhitespace;
  if ( from >= to ) {
    return false;
  }

  const attachment = parseMarkdownAttachmentAt( value, from, {
    acceptExtensionless: options.acceptExtensionlessAttachment
  });

  return attachment?.end === to - 1;
}

export function renderedListIndentations( blocks: readonly LiveMarkdownBlock[]): Map<number, number> {
  const groupIndentations = new Map<LiveMarkdownListGroup, number>();
  const indentations = new Map<number, number>();

  // Include offscreen siblings and resolve parents first so their extra width
  // also shifts nested lists. Single-digit markers keep the usual spacing.
  for ( const group of liveMarkdownListGroups( blocks ) ) {
    const markerLength = group.items.reduce( ( width, block ) =>
      Math.max( width, block.task ? 0 : renderedListMarker( block ).length ), 0
    );
    const extraWidth = Math.max( 0, markerLength - 2 ) * LIST_MARKER_CHARACTER_WIDTH_EM;
    const parentIndentation = group.parent ? groupIndentations.get( group.parent ) ?? 0 : 0;
    const indentation = parentIndentation + LIST_INDENT_STEP_EM + extraWidth;
    groupIndentations.set( group, indentation );
    for ( const block of group.items ) {
      indentations.set( block.from, indentation );
    }
  }

  return indentations;
}

function renderedListLineDecoration(
  block: LiveMarkdownBlock,
  indentation = ( ( block.list?.depth ?? 0 ) + 1 ) * LIST_INDENT_STEP_EM
): StoredDecoration {
  return {
    from: block.from,
    to: block.from,
    decoration: Decoration.line({
      attributes: {
        style: `--live-list-content-indent: ${ indentation }em`
      },
      class: 'is-rendered-list'
    })
  };
}
