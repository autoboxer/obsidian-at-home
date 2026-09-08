import { markdownLanguage } from '@codemirror/lang-markdown';
import type { MarkdownNoteLink } from '../types';
import { leadingFrontmatterEnd } from './frontmatter';

export type MarkdownNoteTarget = Pick<MarkdownNoteLink, 'destination' | 'target' | 'heading'>;

const MARKDOWN_PUNCTUATION_ESCAPE = /\\([!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~])/g;

/** Keep escaped local URL delimiters literal through rendering and URI decoding. */
export function normalizeMarkdownLinkDestination( value: string ): string {
  const raw = value.trim().replace( /^<|>$/g, '' );
  const unescaped = unescapeMarkdownPunctuation( raw );
  if ( unescaped.startsWith( '//' ) || /^[a-z][a-z0-9+.-]*:/i.test( unescaped ) ) {
    return unescaped;
  }

  return raw.replace( MARKDOWN_PUNCTUATION_ESCAPE, ( _match, character: string ) =>
    '#?%'.includes( character ) ? encodeURIComponent( character ) : character
  );
}

/** Parse a local Markdown note URL, splitting the fragment before decoding once. */
export function parseMarkdownNoteTarget( href: string ): MarkdownNoteTarget | undefined {
  const destination = normalizeMarkdownLinkDestination( href );
  if ( !destination || destination.startsWith( '//' ) || /^[a-z][a-z0-9+.-]*:/i.test( destination ) ) {
    return undefined;
  }

  const fragmentStart = destination.indexOf( '#' );
  const rawTarget = fragmentStart < 0 ? destination : destination.slice( 0, fragmentStart );
  if ( rawTarget.includes( '?' ) ) {
    return undefined;
  }

  let target: string;
  let heading: string;
  try {
    target = decodeURIComponent( rawTarget );
    heading = fragmentStart < 0 ? '' : decodeURIComponent( destination.slice( fragmentStart + 1 ) ).trim();
  } catch {
    return undefined;
  }
  if (
    target.startsWith( '//' )
    || target.includes( '\\' )
    || /[\u0000-\u001f\u007f]/u.test( target + heading )
    || /^[a-z][a-z0-9+.-]*:/i.test( target )
  ) {
    return undefined;
  }

  // File links name Markdown files. Retain extensionless heading links, but
  // leave other file types and extensionless attachments to asset handling.
  const filename = target.split( '/' ).at( -1 ) ?? '';
  if ( target
    ? !/\.(?:md|markdown)$/i.test( filename ) && !( heading && filename && !filename.includes( '.' ) )
    : !heading
  ) {
    return undefined;
  }

  return { destination, target, ...( heading ? { heading } : {}) };
}

/** Index the same inline links as the editor, excluding code, images and metadata. */
export function parseMarkdownNoteLinks( source: string ): MarkdownNoteLink[] {
  const bodyStart = leadingFrontmatterEnd( source ) ?? 0;
  const links: MarkdownNoteLink[] = [];
  markdownLanguage.parser.parse( source ).iterate({
    enter( node ) {
      if ( node.name === 'Image' ) {
        return false;
      }
      if ( node.name !== 'Link' || node.from < bodyStart ) {
        return;
      }
      const parsed = parseInlineMarkdownLinkAt( source, node.from );
      const target = parsed && parsed.end + 1 === node.to
        ? parseMarkdownNoteTarget( parsed.destination )
        : undefined;
      if ( parsed && target ) {
        links.push({ ...target, raw: parsed.raw, display: parsed.label, index: node.from });
      }

      return false;
    }
  });

  return links;
}

export interface ParsedInlineMarkdownLink {
  destination: string;
  destinationFrom: number;
  destinationTo: number;
  end: number;
  label: string;
  raw: string;
  start: number;
  title?: string;
}

/** Parse the shared inline-link grammar used by Markdown links and images. */
export function parseInlineMarkdownLinkAt(
  source: string,
  start: number,
  image = false
): ParsedInlineMarkdownLink | undefined {
  const opening = image ? '![' : '[';
  if ( !source.startsWith( opening, start ) ) {
    return undefined;
  }

  const labelStart = start + opening.length;
  const labelEnd = findClosingLabel( source, labelStart );
  if ( labelEnd < 0 || source[ labelEnd + 1 ] !== '(' ) {
    return undefined;
  }

  const destinationEnd = findClosingDestination( source, labelEnd + 2 );
  if ( destinationEnd < 0 ) {
    return undefined;
  }

  const destinationSource = source.slice( labelEnd + 2, destinationEnd );
  const destinationParts = parseDestination( destinationSource.trim() );
  if ( !destinationParts ) {
    return undefined;
  }

  const destinationFrom = labelEnd + 2
    + destinationSource.length - destinationSource.trimStart().length
    + ( destinationSource.trimStart().startsWith( '<' ) ? 1 : 0 );

  return {
    destination: destinationParts.destination,
    destinationFrom,
    destinationTo: destinationFrom + destinationParts.destination.length,
    end: destinationEnd,
    label: unescapeMarkdownPunctuation( source.slice( labelStart, labelEnd ) ),
    raw: source.slice( start, destinationEnd + 1 ),
    start,
    ...( destinationParts.title ? { title: destinationParts.title } : {})
  };
}

function findClosingLabel( source: string, start: number ): number {
  let depth = 1;
  for ( let index = start; index < source.length; index += 1 ) {
    const character = source[ index ]!;
    if ( character === '\n' || character === '\r' ) {
      return -1;
    }
    if ( character === '\\' ) {
      index += 1;
    } else if ( character === '[' ) {
      depth += 1;
    } else if ( character === ']' ) {
      depth -= 1;
      if ( !depth ) {
        return index;
      }
    }
  }

  return -1;
}

function findClosingDestination( source: string, start: number ): number {
  let depth = 1;
  let angleDestination = source[ start ] === '<';
  let quote: '"' | "'" | undefined;

  for ( let index = start; index < source.length; index += 1 ) {
    const character = source[ index ]!;
    if ( character === '\n' || character === '\r' ) {
      return -1;
    }
    if ( character === '\\' ) {
      index += 1;
      continue;
    }
    if ( angleDestination ) {
      if ( character === '>' ) {
        angleDestination = false;
      }
      continue;
    }
    if ( quote ) {
      if ( character === quote ) {
        quote = undefined;
      }
      continue;
    }
    const previousCharacter = source[ index - 1 ];
    const followsTitleSeparator = index > start
      && ( previousCharacter === ' ' || previousCharacter === '\t' );
    if (
      ( character === '"' || character === "'" )
      && depth === 1
      && followsTitleSeparator
    ) {
      quote = character;
    } else if ( character === '(' ) {
      depth += 1;
    } else if ( character === ')' ) {
      depth -= 1;
      if ( !depth ) {
        return index;
      }
    }
  }

  return -1;
}

function parseDestination(
  raw: string
): { destination: string; title?: string } | undefined {
  const match = raw.match(
    /^(<(?:\\.|[^>\\])+>|\S+?)(?:\s+(?:"((?:\\.|[^"\\])*)"|'((?:\\.|[^'\\])*)'|\(((?:\\.|[^)\\])*)\)))?$/
  );
  if ( !match ) {
    return undefined;
  }

  const destination = match[ 1 ]!.replace( /^<|>$/g, '' );
  const title = match[ 2 ] ?? match[ 3 ] ?? match[ 4 ];

  return {
    destination,
    ...( title === undefined ? {} : { title: unescapeMarkdownPunctuation( title ) })
  };
}

export function unescapeMarkdownPunctuation( value: string ): string {
  return value.replace( MARKDOWN_PUNCTUATION_ESCAPE, '$1' );
}
