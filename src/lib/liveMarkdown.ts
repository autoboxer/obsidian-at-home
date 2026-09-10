export type LiveMarkdownBlockType =
  | 'blank'
  | 'blockquote'
  | 'code'
  | 'frontmatter'
  | 'heading'
  | 'horizontal-rule'
  | 'list'
  | 'task'
  | 'text';

export interface LiveMarkdownRange {
  from: number;
  to: number;
}

export interface LiveMarkdownTextEdit extends LiveMarkdownRange {
  insert: string;
}

export interface LiveMarkdownTask {
  checked: boolean;
  marker: LiveMarkdownRange;
  check: LiveMarkdownRange;
}

export interface LiveMarkdownList {
  ordered: boolean;
  depth: number;
  indentation: number;
  marker: LiveMarkdownRange;
  number?: number;
}

export interface LiveMarkdownListGroup {
  depth: number;
  indentation: number;
  ordered: boolean;
  delimiter: string;
  items: LiveMarkdownBlock[];
  parent?: LiveMarkdownListGroup;
}

export interface LiveMarkdownQuote {
  depth: number;
}

export interface LiveMarkdownBlock {
  type: LiveMarkdownBlockType;
  lineNumber: number;
  from: number;
  to: number;
  end: number;
  source: string;
  content: LiveMarkdownRange;
  syntax: LiveMarkdownRange[];
  headingLevel?: number;
  list?: LiveMarkdownList;
  quote?: LiveMarkdownQuote;
  task?: LiveMarkdownTask;
}

interface SourceLine {
  lineNumber: number;
  from: number;
  to: number;
  end: number;
  source: string;
}

interface OpenFence {
  marker: '`' | '~';
  length: number;
}

export function parseLiveMarkdownBlocks( value: string ): LiveMarkdownBlock[] {
  const lines = readSourceLines( value );
  const frontmatterEnd = findFrontmatterEnd( lines );
  const blocks: LiveMarkdownBlock[] = [];
  let fence: OpenFence | undefined;

  for ( const [ index, line ] of lines.entries() ) {
    if ( frontmatterEnd !== undefined && index <= frontmatterEnd ) {
      blocks.push( createPlainBlock( line, 'frontmatter' ) );

      continue;
    }

    if ( fence ) {
      blocks.push( createPlainBlock( line, 'code' ) );
      if ( closesFence( line.source, fence ) ) {
        fence = undefined;
      }

      continue;
    }

    const openingFence = line.source.match( /^ {0,3}(`{3,}|~{3,})/ );
    if ( openingFence ) {
      const marker = openingFence[ 1 ]!;
      fence = {
        marker: marker[ 0 ]! as '`' | '~',
        length: marker.length
      };
      blocks.push( createPlainBlock( line, 'code' ) );

      continue;
    }

    if ( !line.source.trim() ) {
      blocks.push( createPlainBlock( line, 'blank' ) );

      continue;
    }

    const horizontalRule = parseHorizontalRule( line );
    if ( horizontalRule ) {
      blocks.push( horizontalRule );

      continue;
    }

    const heading = parseHeading( line );
    if ( heading ) {
      blocks.push( heading );

      continue;
    }

    const task = parseTask( line );
    if ( task ) {
      blocks.push( task );

      continue;
    }

    const list = parseList( line );
    if ( list ) {
      blocks.push( list );

      continue;
    }

    const quote = parseBlockquote( line );
    blocks.push( quote ?? createPlainBlock( line, 'text' ) );
  }

  arrangeListItems( blocks );

  return blocks;
}

export function findLiveMarkdownBlock(
  blocks: readonly LiveMarkdownBlock[],
  offset: number
): LiveMarkdownBlock | undefined {
  if ( !blocks.length ) {
    return undefined;
  }

  const position = Math.max( 0, offset );
  for ( const [ index, block ] of blocks.entries() ) {
    if ( position < block.end || index === blocks.length - 1 ) {
      return block;
    }
  }

  return blocks.at( -1 );
}

export function activeLiveMarkdownBlocks(
  blocks: readonly LiveMarkdownBlock[],
  anchor: number,
  head: number
): LiveMarkdownBlock[] {
  const selectionFrom = Math.min( anchor, head );
  const selectionTo = Math.max( anchor, head );
  const first = findLiveMarkdownBlock( blocks, selectionFrom );
  const last = findLiveMarkdownBlock(
    blocks,
    selectionTo === selectionFrom ? selectionTo : selectionTo - 1
  );
  if ( !first || !last ) {
    return [];
  }

  const firstIndex = blocks.indexOf( first );
  const lastIndex = blocks.indexOf( last );

  return blocks.slice( firstIndex, lastIndex + 1 );
}

export function setLiveMarkdownTaskChecked(
  value: string,
  block: LiveMarkdownBlock,
  checked: boolean
): string {
  if ( block.type !== 'task' || !block.task ) {
    return value;
  }

  const { from, to } = block.task.check;
  if ( to - from !== 1 || !/[ xX]/.test( value.slice( from, to ) ) ) {
    return value;
  }

  return `${ value.slice( 0, from ) }${ checked ? 'x' : ' ' }${ value.slice( to ) }`;
}

function readSourceLines( value: string ): SourceLine[] {
  const lines: SourceLine[] = [];
  const lineEnding = /\r\n|\n|\r/g;
  let from = 0;
  let lineNumber = 1;
  let match: RegExpExecArray | null;

  while ( ( match = lineEnding.exec( value ) ) ) {
    lines.push({
      lineNumber,
      from,
      to: match.index,
      end: lineEnding.lastIndex,
      source: value.slice( from, match.index )
    });
    from = lineEnding.lastIndex;
    lineNumber += 1;
  }

  lines.push({
    lineNumber,
    from,
    to: value.length,
    end: value.length,
    source: value.slice( from )
  });

  return lines;
}

function findFrontmatterEnd( lines: readonly SourceLine[]): number | undefined {
  const opening = lines[ 0 ]?.source.replace( /^\u{feff}/u, '' ).trimEnd();
  if ( opening !== '---' ) {
    return undefined;
  }

  for ( let index = 1; index < lines.length; index += 1 ) {
    const line = lines[ index ]!.source.trimEnd();
    if ( line === '---' || line === '...' ) {
      return index;
    }
  }

  return undefined;
}

function closesFence( source: string, fence: OpenFence ): boolean {
  const closing = source.match( /^ {0,3}(`+|~+)\s*$/ )?.[ 1 ];

  return Boolean(
    closing
    && closing[ 0 ] === fence.marker
    && closing.length >= fence.length
  );
}

function parseHeading( line: SourceLine ): LiveMarkdownBlock | undefined {
  const heading = line.source.match( /^( {0,3})(#{1,6})([\t ]+)(.*)$/ );
  if ( !heading ) {
    return undefined;
  }

  const prefixLength = heading[ 1 ]!.length + heading[ 2 ]!.length + heading[ 3 ]!.length;
  const openingMarker = { from: line.from, to: line.from + prefixLength };
  const syntax = [ openingMarker ];
  const closingMarker = heading[ 4 ]!.match( /[\t ]+#+[\t ]*$/ )?.[ 0 ];
  const contentTo = closingMarker ? line.to - closingMarker.length : line.to;
  if ( closingMarker ) {
    syntax.push({ from: contentTo, to: line.to });
  }

  return {
    ...line,
    type: 'heading',
    content: { from: openingMarker.to, to: contentTo },
    syntax,
    headingLevel: heading[ 2 ]!.length
  };
}

function parseHorizontalRule( line: SourceLine ): LiveMarkdownBlock | undefined {
  if ( !/^ {0,3}(?:-{3,}|\*{3,}|_{3,})\s*$/.test( line.source ) ) {
    return undefined;
  }

  const syntax = { from: line.from, to: line.to };

  return {
    ...line,
    type: 'horizontal-rule',
    content: { from: line.to, to: line.to },
    syntax: [ syntax ]
  };
}

function parseTask( line: SourceLine ): LiveMarkdownBlock | undefined {
  const task = line.source.match(
    /^([ \t]*)([-+*]|\d{1,9}[.)])([ \t]+)\[([ xX])\]([ \t]+)(.*)$/
  );
  if ( !task ) {
    return undefined;
  }

  const indentLength = task[ 1 ]!.length;
  const markerStart = line.from + indentLength;
  const checkFrom = markerStart + task[ 2 ]!.length + task[ 3 ]!.length + 1;
  const contentFrom = checkFrom + 2 + task[ 5 ]!.length;
  const marker = { from: markerStart, to: contentFrom };

  return {
    ...line,
    type: 'task',
    content: { from: contentFrom, to: line.to },
    syntax: [ marker ],
    list: createListMetadata(
      task[ 1 ]!,
      task[ 2 ]!,
      markerStart
    ),
    task: {
      checked: task[ 4 ]!.toLocaleLowerCase() === 'x',
      marker,
      check: { from: checkFrom, to: checkFrom + 1 }
    }
  };
}

function parseList( line: SourceLine ): LiveMarkdownBlock | undefined {
  const list = line.source.match(
    /^([ \t]*)([-+*]|\d{1,9}[.)])([ \t]+)(.*)$/
  );
  if ( !list ) {
    return undefined;
  }

  const markerStart = line.from + list[ 1 ]!.length;
  const markerEnd = markerStart + list[ 2 ]!.length;
  const contentFrom = markerEnd + list[ 3 ]!.length;

  return {
    ...line,
    type: 'list',
    content: { from: contentFrom, to: line.to },
    syntax: [{ from: markerStart, to: contentFrom }],
    list: createListMetadata(
      list[ 1 ]!,
      list[ 2 ]!,
      markerStart
    )
  };
}

function parseBlockquote( line: SourceLine ): LiveMarkdownBlock | undefined {
  let prefixLength = 0;
  let depth = 0;

  while ( prefixLength < line.source.length ) {
    const marker = line.source.slice( prefixLength ).match( /^ {0,3}> ?/ )?.[ 0 ];
    if ( !marker ) {
      break;
    }

    prefixLength += marker.length;
    depth += 1;
  }

  if ( !depth ) {
    return undefined;
  }

  const contentFrom = line.from + prefixLength;
  const marker = { from: line.from, to: contentFrom };

  return {
    ...line,
    type: 'blockquote',
    content: { from: contentFrom, to: line.to },
    syntax: [ marker ],
    quote: { depth }
  };
}

function createListMetadata(
  indentation: string,
  sourceMarker: string,
  markerFrom: number
): LiveMarkdownList {
  const orderedMarker = sourceMarker.match( /^(\d{1,9})[.)]$/ );

  return {
    ordered: Boolean( orderedMarker ),
    depth: 0,
    indentation: indentationWidth( indentation ),
    marker: {
      from: markerFrom,
      to: markerFrom + sourceMarker.length
    },
    ...( orderedMarker
      ? { number: Number.parseInt( orderedMarker[ 1 ]!, 10 ) }
      : {})
  };
}

const MAX_ORDERED_LIST_NUMBER = 999_999_999;

function arrangeListItems( blocks: readonly LiveMarkdownBlock[]): void {
  for ( const group of liveMarkdownListGroups( blocks ) ) {
    // Repeated 1. is conventional Markdown. Otherwise show explicit numbers,
    // including nested starting numbers and deliberate resets in the source.
    const repeatedOnes = group.ordered && group.items.every( ( block ) => block.list!.number === 1 );
    for ( const [ index, block ] of group.items.entries() ) {
      block.list!.depth = group.depth;
      if ( repeatedOnes ) {
        block.list!.number = index + 1;
      }
    }
  }
}

export function nextOrderedListNumber( number: number ): number {
  return number >= MAX_ORDERED_LIST_NUMBER ? 1 : number + 1;
}

export function liveMarkdownListGroups(
  blocks: readonly LiveMarkdownBlock[]
): LiveMarkdownListGroup[] {
  const levels: LiveMarkdownListGroup[] = [];
  const groups: LiveMarkdownListGroup[] = [];

  for ( const block of blocks ) {
    if ( !block.list ) {
      // Without a blank separator, plain text can continue an item. This also
      // keeps following siblings together when a user removes a marker.
      if ( block.type === 'text' ) {
        continue;
      }
      levels.length = 0;

      continue;
    }

    const list = block.list;
    const matchingLevel = levels.findIndex( ( level ) => level.indentation === list.indentation );
    let depth = matchingLevel;

    if ( matchingLevel >= 0 ) {
      levels.length = matchingLevel + 1;
    } else {
      let parentDepth = levels.length - 1;
      while ( parentDepth >= 0 && levels[ parentDepth ]!.indentation >= list.indentation ) {
        parentDepth -= 1;
      }

      levels.length = parentDepth + 1;
      depth = levels.length;
    }

    const delimiter = block.source[ list.marker.to - block.from - 1 ]!;
    let level = levels[ depth ];
    if ( !level || level.ordered !== list.ordered || level.delimiter !== delimiter ) {
      level = {
        depth,
        indentation: list.indentation,
        ordered: list.ordered,
        delimiter,
        items: [],
        parent: levels[ depth - 1 ]
      };
      levels[ depth ] = level;
      groups.push( level );
    }
    level.items.push( block );
  }

  return groups;
}

function indentationWidth( indentation: string ): number {
  let width = 0;

  for ( const character of indentation ) {
    width += character === '\t' ? 4 : 1;
  }

  return width;
}

function createPlainBlock(
  line: SourceLine,
  type: 'blank' | 'code' | 'frontmatter' | 'text'
): LiveMarkdownBlock {
  return {
    ...line,
    type,
    content: { from: line.from, to: line.to },
    syntax: []
  };
}
