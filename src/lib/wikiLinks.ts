import type { Backlink, Note, WikiLink } from '../types';

const FENCE_START = /^ {0,3}(`{3,}|~{3,})/;

interface TextRange {
  start: number;
  end: number;
}

/** Remove the parts of an Obsidian link that do not identify a note. */
export function normalizeWikiTarget( value: string ): string {
  const withoutHeading = splitUnescaped( value.trim(), '#' )[ 0 ] ?? '';

  return unescapeWikiPart( withoutHeading )
    .replace( /\\/g, '/' )
    .replace( /^\.\//, '' )
    .replace( /^\/+|\/+$/g, '' )
    .replace( /\.md$/i, '' )
    .replace( /\/{2,}/g, '/' )
    .trim();
}

/** Return the note-name portion of a path-like wiki target. */
export function wikiTargetTitle( value: string ): string {
  const normalized = normalizeWikiTarget( value );
  const slash = normalized.lastIndexOf( '/' );

  return normalized.slice( slash + 1 );
}

/**
 * Parse Obsidian-style wiki links while deliberately ignoring fenced and inline
 * code. `target` has its heading and `.md` suffix removed; folder information is
 * retained so callers that know paths can still use it.
 */
export function parseWikiLinks( markdown: string ): WikiLink[] {
  const protectedRanges = codeRanges( markdown );
  const links: WikiLink[] = [];
  let rangeIndex = 0;

  for ( let index = 0; index < markdown.length; index += 1 ) {
    while (
      rangeIndex < protectedRanges.length &&
      index >= protectedRanges[ rangeIndex ]!.end
    ) {
      rangeIndex += 1;
    }

    const protectedRange = protectedRanges[ rangeIndex ];
    if (
      protectedRange &&
      index >= protectedRange.start &&
      index < protectedRange.end
    ) {
      index = protectedRange.end - 1;
      continue;
    }

    const parsed = parseWikiLinkAt( markdown, index );
    if ( !parsed ) {
      continue;
    }

    links.push( parsed );
    index += parsed.raw.length - 1;
  }

  return links;
}

/** Parse a link beginning exactly at `index`. Useful to Markdown renderers. */
export function parseWikiLinkAt(
  source: string,
  index: number
): WikiLink | undefined {
  const embedded = source[ index ] === '!';
  const openIndex = embedded ? index + 1 : index;

  if ( embedded && isEscaped( source, index ) ) {
    return undefined;
  }

  if ( source[ openIndex ] !== '[' || source[ openIndex + 1 ] !== '[' ) {
    return undefined;
  }

  if ( isEscaped( source, openIndex ) ) {
    return undefined;
  }

  const closeIndex = findWikiClose( source, openIndex + 2 );
  if ( closeIndex < 0 ) {
    return undefined;
  }

  const inner = source.slice( openIndex + 2, closeIndex );
  if ( !inner.trim() || inner.includes( '\n' ) || inner.includes( '\r' ) ) {
    return undefined;
  }

  const [ destination = '', alias ] = splitUnescaped( inner, '|', 2 );
  const [ rawTarget = '', ...headingParts ] = splitUnescaped( destination, '#' );
  const heading = headingParts.length
    ? unescapeWikiPart( headingParts.join( '#' ) ).trim()
    : undefined;
  const target = normalizeWikiTarget( rawTarget );

  // A heading-only link is valid, but a link with neither a target nor heading is not.
  if ( !target && !heading ) {
    return undefined;
  }

  const fallbackDisplay = target
    ? wikiTargetTitle( target )
    : heading ?? '';
  const display = alias === undefined
    ? fallbackDisplay
    : unescapeWikiPart( alias ).trim() || fallbackDisplay;

  return {
    raw: source.slice( index, closeIndex + 2 ),
    target,
    display,
    ...( heading ? { heading } : {}),
    embedded,
    index
  };
}

/**
 * Resolve a wiki link against note titles. Exact titles win over basename
 * matches, making the result stable even when path-like links are imported from
 * Obsidian. Heading-only links resolve to `sourceNote` when one is supplied.
 */
export function resolveWikiLink(
  link: WikiLink | string,
  notes: readonly Note[],
  sourceNote?: Note
): Note | undefined {
  const rawTarget = typeof link === 'string' ? link : link.target;
  const normalizedTarget = normalizeForComparison( rawTarget );

  if ( !normalizedTarget ) {
    return sourceNote;
  }

  const targetTitle = normalizeForComparison( wikiTargetTitle( rawTarget ) );
  let basenameMatch: Note | undefined;

  for ( const note of notes ) {
    const normalizedTitle = normalizeForComparison( note.title );
    if ( normalizedTitle === normalizedTarget ) {
      return note;
    }

    if (
      !basenameMatch &&
      normalizeForComparison( wikiTargetTitle( note.title ) ) === targetTitle
    ) {
      basenameMatch = note;
    }
  }

  return basenameMatch;
}

/** Find every incoming wiki-link occurrence for a note. */
export function findBacklinks(
  target: Note | string,
  notes: readonly Note[]
): Backlink[] {
  const targetNote = typeof target === 'string'
    ? resolveWikiLink( target, notes )
    : target;

  if ( !targetNote ) {
    return [];
  }

  const backlinks: Backlink[] = [];
  for ( const note of notes ) {
    if ( note.id === targetNote.id ) {
      continue;
    }

    for ( const link of parseWikiLinks( note.content ) ) {
      const resolved = resolveWikiLink( link, notes, note );
      if ( resolved?.id !== targetNote.id ) {
        continue;
      }

      backlinks.push({
        note,
        link,
        excerpt: excerptAround( note.content, link.index, link.raw.length )
      });
    }
  }

  return backlinks;
}

/** Alias that reads naturally at call sites displaying a backlink panel. */
export const getBacklinks = findBacklinks;

function normalizeForComparison( value: string ): string {
  return normalizeWikiTarget( value )
    .normalize( 'NFKD' )
    .replace( /[\u0300-\u036f]/g, '' )
    .toLocaleLowerCase()
    .replace( /\s+/g, ' ' )
    .trim();
}

function excerptAround( content: string, index: number, length: number ): string {
  const lineStart = content.lastIndexOf( '\n', index - 1 ) + 1;
  const nextNewline = content.indexOf( '\n', index + length );
  const lineEnd = nextNewline < 0 ? content.length : nextNewline;
  let excerpt = content
    .slice( lineStart, lineEnd )
    .replace( /^\s{0,3}(?:#{1,6}|>|[-+*]|\d+[.)])\s+/, '' )
    .replace( /\s+/g, ' ' )
    .trim();

  if ( excerpt.length > 180 ) {
    const relativeIndex = index - lineStart;
    const start = Math.max( 0, Math.min( relativeIndex - 65, excerpt.length - 180 ) );
    excerpt = `${ start > 0 ? '…' : '' }${ excerpt.slice( start, start + 180 ).trim() }${
      start + 180 < excerpt.length ? '…' : ''
    }`;
  }

  return excerpt;
}

function codeRanges( markdown: string ): TextRange[] {
  const ranges: TextRange[] = [];
  const linePattern = /.*(?:\n|$)/g;
  let fence:
    | { marker: '`' | '~'; size: number; start: number }
    | undefined;
  let match: RegExpExecArray | null;

  while ( ( match = linePattern.exec( markdown ) ) !== null ) {
    if ( !match[ 0 ]) {
      break;
    }
    const lineStart = match.index;
    const line = match[ 0 ].replace( /\r?\n$/, '' );

    if ( !fence ) {
      const opening = line.match( FENCE_START );
      if ( opening ) {
        const run = opening[ 1 ]!;
        fence = {
          marker: run[ 0 ] as '`' | '~',
          size: run.length,
          start: lineStart
        };
      }
    } else {
      const closing = line.match( /^ {0,3}(`+|~+)\s*$/ );
      if (
        closing &&
        closing[ 1 ]![ 0 ] === fence.marker &&
        closing[ 1 ]!.length >= fence.size
      ) {
        ranges.push({ start: fence.start, end: linePattern.lastIndex });
        fence = undefined;
      }
    }

    if ( linePattern.lastIndex >= markdown.length ) {
      break;
    }
  }

  if ( fence ) {
    ranges.push({ start: fence.start, end: markdown.length });
  }

  // Inline code ranges only need to be found outside fences.
  let fenceIndex = 0;
  for ( let index = 0; index < markdown.length; index += 1 ) {
    while ( fenceIndex < ranges.length && index >= ranges[ fenceIndex ]!.end ) {
      fenceIndex += 1;
    }
    const fenced = ranges[ fenceIndex ];
    if ( fenced && index >= fenced.start && index < fenced.end ) {
      index = fenced.end - 1;
      continue;
    }
    if ( markdown[ index ] !== '`' || isEscaped( markdown, index ) ) {
      continue;
    }

    const start = index;
    while ( markdown[ index + 1 ] === '`' ) {
      index += 1;
    }
    const size = index - start + 1;
    const delimiter = '`'.repeat( size );
    const close = markdown.indexOf( delimiter, index + 1 );
    if ( close >= 0 && !markdown.slice( index + 1, close ).includes( '\n\n' ) ) {
      ranges.push({ start, end: close + size });
      index = close + size - 1;
    }
  }

  return ranges.sort( ( a, b ) => a.start - b.start );
}

function findWikiClose( source: string, start: number ): number {
  for ( let index = start; index < source.length - 1; index += 1 ) {
    if (
      source[ index ] === ']' &&
      source[ index + 1 ] === ']' &&
      !isEscaped( source, index )
    ) {
      return index;
    }
  }

  return -1;
}

function splitUnescaped( value: string, separator: string, limit = Infinity ): string[] {
  const pieces: string[] = [];
  let start = 0;

  for ( let index = 0; index < value.length && pieces.length < limit - 1; index += 1 ) {
    if ( value[ index ] === separator && !isEscaped( value, index ) ) {
      pieces.push( value.slice( start, index ) );
      start = index + 1;
    }
  }

  pieces.push( value.slice( start ) );

  return pieces;
}

function unescapeWikiPart( value: string ): string {
  return value.replace( /\\([\\|#[\]])/g, '$1' );
}

function isEscaped( value: string, index: number ): boolean {
  let slashCount = 0;
  for ( let cursor = index - 1; cursor >= 0 && value[ cursor ] === '\\'; cursor -= 1 ) {
    slashCount += 1;
  }

  return slashCount % 2 === 1;
}
