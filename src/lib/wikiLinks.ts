import { markdownLanguage } from '@codemirror/lang-markdown';
import type { Backlink, Note, NoteLink, WikiLink } from '../types';
import { leadingFrontmatterEnd } from './frontmatter';
import {
  parseInlineMarkdownLinkAt,
  parseMarkdownNoteLinks,
  type MarkdownNoteTarget
} from './markdownLinks';
import { relativeImageDestination } from './markdownImages';

export type WikiLinkTarget = Pick<WikiLink, 'target' | 'heading'>;

const NOTE_EXTENSION = /\.(?:md|markdown)$/i;

interface TextRange {
  start: number;
  end: number;
}

/** Preserve path prefixes and extensions while removing a wiki alias/heading. */
export function normalizeWikiTarget( value: string ): string {
  const destination = splitUnescaped( value.trim(), '|', 2 )[ 0 ] ?? '';
  const withoutHeading = splitUnescaped( destination, '#' )[ 0 ] ?? '';

  return unescapeWikiPart( withoutHeading ).replace( /\\/g, '/' ).trim();
}

/** Return the note-name portion of a path-like wiki target. */
export function wikiTargetTitle( value: string ): string {
  const normalized = normalizeWikiTarget( value );
  const slash = normalized.lastIndexOf( '/' );

  return normalized.slice( slash + 1 ).replace( NOTE_EXTENSION, '' );
}

/**
 * Parse Obsidian-style wiki links while ignoring frontmatter and code.
 * `target` retains its path prefix and extension so explicit destinations
 * remain distinguishable. The heading and alias are parsed separately.
 */
export function parseWikiLinks( markdown: string ): WikiLink[] {
  const protectedRanges = protectedWikiRanges( markdown );
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

  const noteName = target.split( '/' ).at( -1 )!.replace( NOTE_EXTENSION, '' );
  const fallbackDisplay = target
    ? noteName + ( heading ? ` (${ heading })` : '' )
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

/** Resolve only a unique match; an explicit path never falls back to a basename. */
export function resolveWikiLink(
  link: WikiLinkTarget | string,
  notes: readonly Note[],
  sourceNote?: Note,
  notePaths?: ReadonlyMap<string, string>
): Note | undefined {
  const candidates = wikiLinkCandidates( link, notes, sourceNote, notePaths );

  return candidates.length === 1 ? candidates[ 0 ] : undefined;
}

/**
 * Qualified wiki paths start at the vault root; ./ and ../ start at the source
 * note. Bare names prefer the source folder, then a unique match in the vault.
 * Exact spelling wins within each tier; case-insensitive matches must be unique.
 */
export function wikiLinkCandidates(
  link: WikiLinkTarget | string,
  notes: readonly Note[],
  sourceNote?: Note,
  notePaths?: ReadonlyMap<string, string>
): Note[] {
  // Parsed targets are already unescaped: a literal # or | is part of the path.
  const target = typeof link === 'string' ? normalizeWikiTarget( link ) : link.target;
  if ( !target ) {
    return sourceNote ? [ sourceNote ] : [];
  }
  if ( target.endsWith( '/' ) || target.startsWith( '//' ) ) {
    return [];
  }
  const sourcePath = sourceNote ? noteLinkPath( sourceNote, notePaths ) : undefined;
  const sourceFolder = sourcePath?.split( '/' ).slice( 0, -1 ).join( '/' );
  const entries = notes.flatMap( ( note ) => {
    const path = noteLinkPath( note, notePaths );

    return path ? [{ note, path }] : [];
  });
  const match = ( path: string, candidates = entries, basename = false ): Note[] => {
    const compareExtension = NOTE_EXTENSION.test( path );
    const name = ( candidate: typeof entries[ number ]): string => {
      const candidatePath = basename ? candidate.path.split( '/' ).at( -1 )! : candidate.path;

      return compareExtension ? candidatePath : candidatePath.replace( NOTE_EXTENSION, '' );
    };
    const exact = candidates.filter( ( candidate ) => name( candidate ) === path );
    const matches = exact.length ? exact : candidates.filter( ( candidate ) =>
      name( candidate ).toLowerCase() === path.toLowerCase()
    );

    return matches.map( ( candidate ) => candidate.note );
  };

  if ( target.includes( '/' ) ) {
    const relative = target.startsWith( './' ) || target.startsWith( '../' );
    if ( relative && sourceFolder === undefined ) {
      return [];
    }
    const path = canonicalNotePath( relative ? `${ sourceFolder }/${ target }` : target );

    return path ? match( path ) : [];
  }

  const name = canonicalNotePath( target );
  if ( !name ) {
    return [];
  }
  if ( sourceFolder !== undefined ) {
    const nearby = match( name, entries.filter( ( candidate ) =>
      candidate.path.split( '/' ).slice( 0, -1 ).join( '/' ) === sourceFolder
    ), true );
    if ( nearby.length ) {
      return nearby;
    }
  }

  return match( name, entries, true );
}

/** Suggest unambiguous vault paths without scanning the vault for every option. */
export function wikiLinkSuggestions(
  notes: readonly Note[],
  notePaths?: ReadonlyMap<string, string>
): string[] {
  const entries = notes.flatMap( ( note ) => {
    const path = noteLinkPath( note, notePaths );

    return path ? [{ path, stem: path.replace( NOTE_EXTENSION, '' ) }] : [];
  });
  const paths = new Map<string, number>();
  const stems = new Map<string, number>();
  const names = new Map<string, number>();
  for ( const { path, stem } of entries ) {
    const name = stem.split( '/' ).at( -1 )!.toLowerCase();
    paths.set( path, ( paths.get( path ) ?? 0 ) + 1 );
    stems.set( stem, ( stems.get( stem ) ?? 0 ) + 1 );
    names.set( name, ( names.get( name ) ?? 0 ) + 1 );
  }

  return entries.flatMap( ({ path, stem }) => {
    if ( paths.get( path ) !== 1 ) {
      return [];
    }
    // An extension distinguishes Topic.md from Topic.markdown in the same folder.
    let target = stem && !NOTE_EXTENSION.test( stem ) && stems.get( stem ) === 1 ? stem : path;
    if ( !target.includes( '/' ) && names.get( stem.toLowerCase() ) !== 1 ) {
      target = `/${ target }`;
    }

    return [ target.replace( /[\\|#[\]]/g, '\\$&' ) ];
  });
}

function noteLinkPath( note: Note, paths?: ReadonlyMap<string, string> ): string | undefined {
  return canonicalNotePath( paths?.get( note.id ) || note.relativePath || `${ note.title }.md` );
}

function canonicalNotePath( value: string ): string | undefined {
  if ( /^[a-z][a-z0-9+.-]*:/i.test( value ) || /[\u0000-\u001f\u007f]/u.test( value ) ) {
    return undefined;
  }
  const parts: string[] = [];
  for ( const part of value.normalize( 'NFC' ).split( '/' ) ) {
    if ( !part || part === '.' ) {
      continue;
    }
    if ( part === '..' ) {
      if ( !parts.pop() ) {
        return undefined;
      }
    } else {
      parts.push( part );
    }
  }

  return parts.length ? parts.join( '/' ) : undefined;
}

/** Keep source order when displaying wiki and ordinary Markdown connections. */
export function parseNoteLinks( markdown: string ): NoteLink[] {
  return [ ...parseWikiLinks( markdown ), ...parseMarkdownNoteLinks( markdown ) ]
    .sort( ( a, b ) => a.index - b.index );
}

/** Markdown file URLs are source-relative, including paths without a ./ prefix. */
export function resolveNoteLink(
  link: WikiLinkTarget | MarkdownNoteTarget,
  notes: readonly Note[],
  sourceNote?: Note,
  notePaths?: ReadonlyMap<string, string>
): Note | undefined {
  const target = 'destination' in link && link.target && !link.target.startsWith( '/' )
    ? `./${ link.target }`
    : link.target;

  return resolveWikiLink({ ...link, target }, notes, sourceNote, notePaths );
}

type NoteLinkResolver = ( link: WikiLinkTarget | MarkdownNoteTarget, source: Note ) => Note | undefined;
type UniqueNoteMatches = Map<string, Note | null>;

/** Snapshot unique matches once; null retains ambiguity without storing candidates. */
function indexedNoteLinkResolver(
  notes: readonly Note[],
  paths: ReadonlyMap<string, string>
): NoteLinkResolver {
  const exactPaths: UniqueNoteMatches = new Map();
  const foldedPaths: UniqueNoteMatches = new Map();
  const exactNames: UniqueNoteMatches = new Map();
  const foldedNames: UniqueNoteMatches = new Map();
  const nearbyNames: UniqueNoteMatches = new Map();
  const add = ( index: UniqueNoteMatches, key: string, note: Note ): void => {
    index.set( key, index.has( key ) ? null : note );
  };
  for ( const note of notes ) {
    const path = noteLinkPath( note, paths );
    if ( !path ) {
      continue;
    }
    const folder = path.split( '/' ).slice( 0, -1 ).join( '/' );
    for ( const [ prefix, value ] of [[ 'file:', path ], [ 'stem:', path.replace( NOTE_EXTENSION, '' ) ]] as const ) {
      const name = value.split( '/' ).at( -1 )!;
      add( exactPaths, prefix + value, note );
      add( foldedPaths, prefix + value.toLowerCase(), note );
      add( exactNames, prefix + name, note );
      add( foldedNames, prefix + name.toLowerCase(), note );
      // Nearby matching folds only the filename, not the source folder's case.
      add( nearbyNames, prefix + folder + '/' + name.toLowerCase(), note );
    }
  }
  const match = ( exact: UniqueNoteMatches, folded: UniqueNoteMatches, key: string ): Note | null | undefined =>
    exact.has( key ) ? exact.get( key ) : folded.get( key.toLowerCase() );

  return ( link, source ) => {
    const target = 'destination' in link && link.target && !link.target.startsWith( '/' )
      ? `./${ link.target }`
      : link.target;
    if ( !target ) {
      return source;
    }
    if ( target.endsWith( '/' ) || target.startsWith( '//' ) ) {
      return undefined;
    }
    const sourceFolder = noteLinkPath( source, paths )?.split( '/' ).slice( 0, -1 ).join( '/' );
    if ( target.includes( '/' ) ) {
      const relative = target.startsWith( './' ) || target.startsWith( '../' );
      const path = relative && sourceFolder === undefined
        ? undefined
        : canonicalNotePath( relative ? `${ sourceFolder }/${ target }` : target );

      return path ? match( exactPaths, foldedPaths, ( NOTE_EXTENSION.test( path ) ? 'file:' : 'stem:' ) + path ) ?? undefined : undefined;
    }
    const name = canonicalNotePath( target );
    if ( !name ) {
      return undefined;
    }
    const prefix = NOTE_EXTENSION.test( name ) ? 'file:' : 'stem:';
    if ( sourceFolder !== undefined ) {
      const key = prefix + ( sourceFolder ? sourceFolder + '/' : '' ) + name;
      const nearby = exactPaths.has( key )
        ? exactPaths.get( key )
        : nearbyNames.get( prefix + sourceFolder + '/' + name.toLowerCase() );
      if ( nearby !== undefined ) {
        return nearby ?? undefined;
      }
    }

    return match( exactNames, foldedNames, prefix + name ) ?? undefined;
  };
}

/** Reuse old/new lookup indexes throughout one note or folder mutation. */
export function createNoteLinkRewriter(
  notes: readonly Note[],
  previousPaths: ReadonlyMap<string, string>,
  nextPaths: ReadonlyMap<string, string>
): ( note: Note ) => string {
  // Notes without link syntax need neither Markdown parsing nor lookup indexes.
  let previousResolver: NoteLinkResolver | undefined;
  let nextResolver: NoteLinkResolver | undefined;

  return ( note ) => {
    if ( !note.content.includes( '[' ) ) {
      return note.content;
    }
    previousResolver ??= indexedNoteLinkResolver( notes, previousPaths );
    nextResolver ??= indexedNoteLinkResolver( notes, nextPaths );

    return rewriteNoteLinks( note, previousPaths, nextPaths, previousResolver, nextResolver );
  };
}

/** Preserve resolved note identities across source and destination path changes. */
export function rewriteNoteLinksForNotePaths(
  note: Note,
  notes: readonly Note[],
  previousPaths: ReadonlyMap<string, string>,
  nextPaths: ReadonlyMap<string, string>
): string {
  return rewriteNoteLinks(
    note, previousPaths, nextPaths,
    ( link, source ) => resolveNoteLink( link, notes, source, previousPaths ),
    ( link, source ) => resolveNoteLink( link, notes, source, nextPaths )
  );
}

function rewriteNoteLinks(
  note: Note,
  previousPaths: ReadonlyMap<string, string>,
  nextPaths: ReadonlyMap<string, string>,
  previousResolver: NoteLinkResolver,
  nextResolver: NoteLinkResolver
): string {
  const previousPath = noteLinkPath( note, previousPaths );
  const nextPath = noteLinkPath( note, nextPaths );
  if ( !previousPath || !nextPath ) {
    return note.content;
  }
  const absoluteTarget = ( source: string, target: string ): string | undefined =>
    canonicalNotePath( target.startsWith( '/' )
      ? target
      : `${ source.split( '/' ).slice( 0, -1 ).join( '/' ) }/${ target }` );

  let content = note.content;
  for ( const link of parseNoteLinks( note.content ).reverse() ) {
    // Heading-only links already follow their source note.
    if ( !link.target ) {
      continue;
    }
    const target = previousResolver( link, note );
    const targetPath = target
      ? noteLinkPath( target, nextPaths )
      : 'destination' in link ? absoluteTarget( previousPath, link.target ) : undefined;
    if (
      !targetPath
      || ( target
        ? nextResolver( link, note )?.id === target.id
        : absoluteTarget( nextPath, link.target ) === targetPath )
    ) {
      continue;
    }

    if ( !( 'destination' in link ) ) {
      if ( !target ) {
        continue;
      }
      const destination = rewrittenWikiTarget( link, target, targetPath, note, nextPaths, nextResolver );
      if ( destination === undefined ) {
        continue;
      }
      const openingLength = link.embedded ? 3 : 2;
      const rawDestination = splitUnescaped( link.raw.slice( openingLength, -2 ), '|', 2 )[ 0 ]!;
      const rawTarget = splitUnescaped( rawDestination, '#', 2 )[ 0 ]!;
      const from = link.index + openingLength + rawTarget.length - rawTarget.trimStart().length;
      const to = link.index + openingLength + rawTarget.trimEnd().length;
      content = content.slice( 0, from ) + destination + content.slice( to );
      continue;
    }

    const parsed = parseInlineMarkdownLinkAt( note.content, link.index );
    if ( !parsed ) {
      continue;
    }
    // Missing and ambiguous links keep their original absolute path; do not
    // guess a note in the new folder. Resolved siblings follow the same move.
    const path = link.target.startsWith( '/' )
      ? `/${ targetPath }`
      : relativeImageDestination( nextPath, targetPath );
    const destination = path.split( '/' ).map( ( part ) =>
      encodeURIComponent( part ).replace( /[!'()*]/g, ( character ) =>
        `%${ character.charCodeAt( 0 ).toString( 16 ).toUpperCase() }`
      )
    ).join( '/' );
    let pathLength = 0;
    while ( pathLength < parsed.destination.length && parsed.destination[ pathLength ] !== '#' ) {
      pathLength += parsed.destination[ pathLength ] === '\\' ? 2 : 1;
    }
    // Replace only the path: retain fragment spelling, wrappers, title, label,
    // and surrounding whitespace exactly as the user wrote them.
    content = content.slice( 0, parsed.destinationFrom ) + destination
      + content.slice( Math.min( parsed.destinationFrom + pathLength, parsed.destinationTo ) );
  }

  return content;
}

function rewrittenWikiTarget(
  link: WikiLink,
  target: Note,
  targetPath: string,
  source: Note,
  paths: ReadonlyMap<string, string>,
  resolve: NoteLinkResolver
): string | undefined {
  const path = NOTE_EXTENSION.test( link.target ) ? targetPath : targetPath.replace( NOTE_EXTENSION, '' );
  let candidates: string[];
  if ( link.target.startsWith( './' ) || link.target.startsWith( '../' ) ) {
    const relative = relativeImageDestination( noteLinkPath( source, paths )!, path );
    candidates = [ relative.startsWith( '../' ) ? relative : `./${ relative }` ];
  } else if ( link.target.startsWith( '/' ) ) {
    candidates = [ `/${ path }` ];
  } else {
    candidates = link.target.includes( '/' ) ? [ path ] : [ path.split( '/' ).at( -1 )!, path ];
  }
  // Qualify the path and extension if preserving the old style would select a
  // duplicate name or confuse a filename ending in .md with its extension.
  candidates.push( `/${ targetPath }` );
  const destination = candidates.find( ( candidate ) =>
    resolve({ target: candidate }, source )?.id === target.id
  );

  return destination?.replace( /[\\|#[\]]/g, '\\$&' );
}

/** Find every incoming wiki or Markdown note-link occurrence for a note. */
export function findBacklinks(
  target: Note | string,
  notes: readonly Note[],
  notePaths?: ReadonlyMap<string, string>
): Backlink[] {
  const targetNote = typeof target === 'string'
    ? resolveWikiLink( target, notes, undefined, notePaths )
    : target;

  if ( !targetNote ) {
    return [];
  }

  const backlinks: Backlink[] = [];
  for ( const note of notes ) {
    if ( note.id === targetNote.id ) {
      continue;
    }

    for ( const link of parseNoteLinks( note.content ) ) {
      const resolved = resolveNoteLink( link, notes, note, notePaths );
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

function protectedWikiRanges( markdown: string ): TextRange[] {
  const bodyStart = leadingFrontmatterEnd( markdown ) ?? 0;
  const ranges: TextRange[] = bodyStart ? [{ start: 0, end: bodyStart }] : [];
  markdownLanguage.parser.parse( markdown ).iterate({
    enter( node ) {
      if ([ 'FencedCode', 'CodeBlock', 'InlineCode' ].includes( node.name ) ) {
        ranges.push({ start: node.from, end: node.to });

        return false;
      }
    }
  });

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
