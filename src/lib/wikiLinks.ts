import { markdownLanguage } from '@codemirror/lang-markdown';
import type { Backlink, Note, NoteLink, WikiLink } from '../types';
import { leadingFrontmatterEnd } from './frontmatter';
import { parsePairedInlineMarkup } from './inlineMarkup';
import { parseLiveMarkdownBlocks, type LiveMarkdownTextEdit } from './liveMarkdown';
import { parseInlineMarkdownLinkAt, parseMarkdownNoteLinks, type MarkdownNoteTarget } from './markdownLinks';
import { relativeImageDestination } from './markdownImages';

export type WikiLinkTarget = Pick<WikiLink, 'target' | 'heading'>;

export interface WikiLinkCompletion {
  from: number;
  to: number;
  query: string;
  target?: string;
  suffix: string;
}

const NOTE_EXTENSION = /\.(?:md|markdown)$/i;

interface TextRange {
  start: number;
  end: number;
}

/** complete the active destination on one line, preserving its heading/alias suffix */
export function wikiLinkCompletionAt( line: string, cursor: number ): WikiLinkCompletion | undefined {
  if ( cursor < 2 || cursor > line.length ) {
    return undefined;
  }

  let opening = line.lastIndexOf( '[[', cursor - 2 );

  while ( opening >= 0 && isEscaped( line, opening ) ) {
    opening = line.lastIndexOf( '[[', opening - 1 );
  }

  if ( opening < 0 ) {
    return undefined;
  }

  const start = opening + 2;
  const close = findWikiClose( line, start );
  const before = line.slice( start, cursor );

  if ( ( close >= 0 && cursor > close ) || splitUnescaped( before, '|', 2 ).length > 1 ) {
    return undefined;
  }

  const after = line.slice( start, close >= 0 ? close : line.length );
  const destination = splitUnescaped( after, '|', 2 )[ 0 ]!;
  const rawTarget = splitUnescaped( destination, '#', 2 )[ 0 ]!;
  const heading = cursor > start + rawTarget.length;
  const from = heading ? start + rawTarget.length + 1 : start;
  // an explicit heading or alias still belongs to an unclosed link. Otherwise
  // complete only through the caret, leaving unfinished trailing text outside
  const hasSuffix = destination.length < after.length || ( !heading && rawTarget.length < destination.length );
  const end = close >= 0 ? close : hasSuffix ? line.length : cursor;
  const suffixFrom = heading ? start + destination.length : start + rawTarget.length;

  return {
    from,
    to: close >= 0 ? close + 2 : end,
    query: unescapeWikiPart( line.slice( from, cursor ) ),
    ...( heading ? { target: normalizeWikiTarget( rawTarget ) } : {}),
    suffix: line.slice( Math.min( suffixFrom, end ), end ) + ']]'
  };
}

/** preserve path prefixes and extensions while removing a wiki alias/heading */
export function normalizeWikiTarget( value: string ): string {
  const destination = splitUnescaped( value.trim(), '|', 2 )[ 0 ] ?? '';
  const withoutHeading = splitUnescaped( destination, '#' )[ 0 ] ?? '';

  return unescapeWikiPart( withoutHeading ).replace( /\\/g, '/' ).trim();
}

/** return the note-name portion of a path-like wiki target */
export function wikiTargetTitle( value: string ): string {
  const normalized = normalizeWikiTarget( value );
  const slash = normalized.lastIndexOf( '/' );

  return normalized.slice( slash + 1 ).replace( NOTE_EXTENSION, '' );
}

/**
 * parse Obsidian-style wiki links while ignoring frontmatter and code
 * `target` retains its path prefix and extension so explicit destinations
 * remain distinguishable. The heading and alias are parsed separately
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

/** parse a link beginning exactly at `index`. Useful to Markdown renderers */
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

  // a heading-only link is valid, but a link with neither a target nor heading is not
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

/** source offsets for the caret boundaries in a rendered wiki-link label */
export function wikiLinkDisplayOffsets( raw: string ): number[] {
  const opening = raw.startsWith( '!' ) ? 3 : 2;
  const inner = raw.slice( opening, -2 );
  const [ destination = '', alias ] = splitUnescaped( inner, '|', 2 );

  const segment = ( value: string, start: number ) => {
    let text = '';
    const offsets: number[] = [];

    for ( let index = 0; index < value.length; index += 1 ) {
      offsets.push( start + index );

      if ( value[ index ] === '\\' && /[\\|#[\]]/.test( value[ index + 1 ] ?? '' ) ) {
        index += 1;
      }

      text += value[ index ];
    }

    offsets.push( start + value.length );
    const from = text.length - text.trimStart().length;
    const to = text.trimEnd().length;

    return { text: text.slice( from, to ), offsets: offsets.slice( from, to + 1 ) };
  };

  const aliased = alias === undefined ? undefined : segment( alias, opening + destination.length + 1 );

  if ( aliased?.text ) {
    return aliased.offsets;
  }

  const [ rawTarget = '', ...headings ] = splitUnescaped( destination, '#' );
  const target = segment( rawTarget, opening );
  const path = target.text.replace( /\\/g, '/' );
  const nameFrom = path.lastIndexOf( '/' ) + 1;
  const nameTo = path.replace( NOTE_EXTENSION, '' ).length;
  const name = target.offsets.slice( nameFrom, nameTo + 1 );
  const heading = segment( headings.join( '#' ), opening + rawTarget.length + 1 );

  // the displayed space and parentheses are synthetic. Keep the filename's
  // end before its hidden extension, then map the heading to its own source
  return heading.text
    ? target.text
      ? [ ...name, heading.offsets[ 0 ]!, ...heading.offsets, heading.offsets.at( -1 )! ]
      : heading.offsets
    : name;
}

/** resolve only a unique match; an explicit path never falls back to a basename */
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
 * qualified wiki paths start at the vault root; ./ and ../ start at the source
 * note. Bare names prefer the source folder, then a unique match in the vault
 * exact spelling wins within each tier; case-insensitive matches must be unique
 */
export function wikiLinkCandidates(
  link: WikiLinkTarget | string,
  notes: readonly Note[],
  sourceNote?: Note,
  notePaths?: ReadonlyMap<string, string>
): Note[] {
  // parsed targets are already unescaped: a literal # or | is part of the path
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

/** suggest unambiguous vault paths without scanning the vault for every option */
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

    // an extension distinguishes Topic.md from Topic.markdown in the same folder
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

/** keep source order when displaying wiki and ordinary Markdown connections */
export function parseNoteLinks( markdown: string ): NoteLink[] {
  return [ ...parseWikiLinks( markdown ), ...parseMarkdownNoteLinks( markdown ) ]
    .sort( ( a, b ) => a.index - b.index );
}

/** markdown file URLs are source-relative, including paths without a ./ prefix */
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

/** snapshot unique matches once; null retains ambiguity without storing candidates */
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
      // nearby matching folds only the filename, not the source folder's case
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

      return path
        ? match( exactPaths, foldedPaths, ( NOTE_EXTENSION.test( path ) ? 'file:' : 'stem:' ) + path ) ?? undefined
        : undefined;
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

/** reuse old/new lookup indexes throughout one note, folder, or import mutation */
export function createNoteLinkRewriter(
  notes: readonly Note[],
  previousPaths: ReadonlyMap<string, string>,
  nextPaths: ReadonlyMap<string, string>,
  options: { destinationNotes?: readonly Note[]; preserveUnresolvedPaths?: boolean } = {}
): ( note: Note ) => string {
  // notes without link syntax need neither Markdown parsing nor lookup indexes
  let previousResolver: NoteLinkResolver | undefined;
  let nextResolver: NoteLinkResolver | undefined;

  return ( note ) => {
    if ( !note.content.includes( '[' ) ) {
      return note.content;
    }

    previousResolver ??= indexedNoteLinkResolver( notes, previousPaths );
    nextResolver ??= indexedNoteLinkResolver( options.destinationNotes ?? notes, nextPaths );

    return rewriteNoteLinks(
      note, previousPaths, nextPaths, previousResolver, nextResolver, options.preserveUnresolvedPaths
    );
  };
}

/** preserve resolved note identities across source and destination path changes */
export function rewriteNoteLinksForNotePaths(
  note: Note,
  notes: readonly Note[],
  previousPaths: ReadonlyMap<string, string>,
  nextPaths: ReadonlyMap<string, string>,
  options: { destinationNotes?: readonly Note[]; preserveUnresolvedPaths?: boolean } = {}
): string {
  return rewriteNoteLinks(
    note, previousPaths, nextPaths,
    ( link, source ) => resolveNoteLink( link, notes, source, previousPaths ),
    ( link, source ) => resolveNoteLink( link, options.destinationNotes ?? notes, source, nextPaths ),
    options.preserveUnresolvedPaths
  );
}

function rewriteNoteLinks(
  note: Note,
  previousPaths: ReadonlyMap<string, string>,
  nextPaths: ReadonlyMap<string, string>,
  previousResolver: NoteLinkResolver,
  nextResolver: NoteLinkResolver,
  preserveUnresolvedPaths = true
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

  const replacements: Array<{ from: number; to: number; value: string }> = [];

  for ( const link of parseNoteLinks( note.content ) ) {
    // heading-only links already follow their source note
    if ( !link.target ) {
      continue;
    }

    const target = previousResolver( link, note );

    if ( !target && !preserveUnresolvedPaths ) {
      continue;
    }

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

      replacements.push({ from, to, value: destination });
      continue;
    }

    const parsed = parseInlineMarkdownLinkAt( note.content, link.index );

    if ( !parsed ) {
      continue;
    }

    // missing and ambiguous links keep their original absolute path; do not
    // guess a note in the new folder. Resolved siblings follow the same move
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

    // replace only the path: retain fragment spelling, wrappers, title, label,
    // and surrounding whitespace exactly as the user wrote them
    replacements.push({
      from: parsed.destinationFrom,
      to: Math.min( parsed.destinationFrom + pathLength, parsed.destinationTo ),
      value: destination
    });
  }

  let content = note.content;

  // actual destination offsets also handle wiki links inside Markdown labels
  for ( const { from, to, value } of replacements.sort( ( a, b ) => b.from - a.from ) ) {
    content = content.slice( 0, from ) + value + content.slice( to );
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

  // qualify the path and extension if preserving the old style would select a
  // duplicate name or confuse a filename ending in .md with its extension
  candidates.push( `/${ targetPath }` );

  const destination = candidates.find( ( candidate ) =>
    resolve({ target: candidate }, source )?.id === target.id
  );

  return destination?.replace( /[\\|#[\]]/g, '\\$&' );
}

/** find every incoming wiki or Markdown note-link occurrence for a note */
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

    const excerptEdits = new Map<string, LiveMarkdownTextEdit[]>();

    for ( const link of parseNoteLinks( note.content ) ) {
      const resolved = resolveNoteLink( link, notes, note, notePaths );

      if ( resolved?.id !== targetNote.id ) {
        continue;
      }

      backlinks.push({
        note,
        link,
        excerpt: excerptAround( note.content, link.index, link.raw.length, excerptEdits )
      });
    }
  }

  return backlinks;
}

/** alias that reads naturally at call sites displaying a backlink panel */
export const getBacklinks = findBacklinks;

function excerptAround(
  content: string,
  index: number,
  length: number,
  excerptEdits: Map<string, LiveMarkdownTextEdit[]>
): string {
  const lineStart = content.lastIndexOf( '\n', index - 1 ) + 1;
  const nextNewline = content.indexOf( '\n', index + length );
  const lineEnd = nextNewline < 0 ? content.length : nextNewline;
  let line = content.slice( lineStart, lineEnd ).trimEnd();
  let relativeIndex = index - lineStart;

  // unwrap nested quote/list/task prefixes using the editor's block grammar
  while ( line ) {
    const block = parseLiveMarkdownBlocks( line )[ 0 ]!;

    if ( !block.content.from ) {
      break;
    }

    relativeIndex -= block.content.from;
    line = line.slice( block.content.from, block.content.to );

    // only containers can contain another block. Heading content is inline,
    // so prefixes such as "2026." or ">" remain meaningful text
    if ( ![ 'blockquote', 'list', 'task' ].includes( block.type ) ) {
      break;
    }
  }

  relativeIndex -= line.length - line.trimStart().length;
  line = line.trimStart();

  // several backlinks can share a long line; parse its presentation only once
  let edits = excerptEdits.get( line );

  if ( !edits ) {
    edits = backlinkExcerptEdits( line ).sort( ( a, b ) => a.from - b.from || b.to - a.to );
    excerptEdits.set( line, edits );
  }

  let excerpt = '';
  let cursor = 0;
  let readableIndex = relativeIndex;

  for ( const edit of edits ) {
    if ( edit.from < cursor ) {
      continue;
    }

    excerpt += line.slice( cursor, edit.from ) + edit.insert;
    cursor = edit.to;

    if ( edit.to <= relativeIndex ) {
      readableIndex += edit.insert.length - ( edit.to - edit.from );
    } else if ( edit.from < relativeIndex ) {
      readableIndex += edit.from - relativeIndex;
    }
  }

  excerpt += line.slice( cursor );
  // map this occurrence through both syntax removal and whitespace collapse
  // before cropping; raw source offsets drift after long URLs or aliases
  readableIndex = excerpt.slice( 0, readableIndex ).replace( /\s+/g, ' ' ).trimStart().length;
  excerpt = excerpt.replace( /\s+/g, ' ' ).trim();

  if ( excerpt.length > 180 ) {
    const start = Math.max( 0, Math.min( readableIndex - 65, excerpt.length - 180 ) );

    excerpt = `${ start > 0 ? '…' : '' }${ excerpt.slice( start, start + 180 ).trim() }${
      start + 180 < excerpt.length ? '…' : ''
    }`;
  }

  return excerpt;
}

/** plain text only: the backlink card owns navigation, not links in its excerpt */
function backlinkExcerptEdits( line: string ): LiveMarkdownTextEdit[] {
  const edits: LiveMarkdownTextEdit[] = parseWikiLinks( line ).map( ( link ) => ({
    from: link.index,
    to: link.index + link.raw.length,
    insert: link.display
  }) );

  markdownLanguage.parser.parse( line ).iterate({
    enter( node ) {
      if ( edits.some( ( edit ) => node.from >= edit.from && node.to <= edit.to ) ) {
        return false;
      }

      if ( node.name === 'InlineCode' ) {
        const marks = node.node.getChildren( 'CodeMark' );
        let code = line.slice( marks[ 0 ]!.to, marks.at( -1 )!.from );

        if ( code.startsWith( ' ' ) && code.endsWith( ' ' ) && code.trim() ) {
          code = code.slice( 1, -1 );
        }

        edits.push({ from: node.from, to: node.to, insert: code });

        return false;
      }

      if ( node.name === 'Link' || node.name === 'Image' ) {
        const link = parseInlineMarkdownLinkAt( line, node.from, node.name === 'Image' );

        if ( link && link.end + 1 === node.to ) {
          const marks = node.node.getChildren( 'LinkMark' );

          edits.push(
            { from: node.from, to: marks[ 0 ]!.to, insert: '' },
            { from: marks[ 1 ]!.from, to: node.to, insert: '' }
          );
        }
      } else if ( node.name === 'EmphasisMark' || node.name === 'Escape' ) {
        edits.push({
          from: node.from,
          to: node.name === 'Escape' ? node.from + 1 : node.to,
          insert: ''
        });
      }
    }
  });

  // include the editor's additional paired markup (such as strikethrough),
  // leaving code, literal wiki labels and already-handled syntax alone
  for ( const span of parsePairedInlineMarkup( line, edits ) ) {
    edits.push(
      { from: span.from, to: span.contentFrom, insert: '' },
      { from: span.contentTo, to: span.to, insert: '' }
    );
  }

  return edits;
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
