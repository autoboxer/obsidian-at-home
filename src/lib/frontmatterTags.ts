interface FrontmatterBounds {
  start: number;
  end: number;
  lineEnding: string;
}

// Keep reading semantics aligned with workspace/persistence/frontmatter.rs.
// Source is preserved verbatim; tag controls only rewrite simple tag fields.
export function parseFrontmatterTags( content: string ): string[] {
  const bounds = frontmatterBounds( content );
  if ( !bounds ) {
    return [];
  }
  const tags: string[] = [];
  let readingList = false;
  for ( const line of content.slice( bounds.start, bounds.end ).split( '\n' ) ) {
    const trimmed = line.trim();
    if ( !trimmed || trimmed.startsWith( '#' ) ) {
      continue;
    }
    const indented = /^[ \t]/.test( line );
    if ( readingList && ( indented || /^-(?:[ \t]|$)/.test( trimmed ) ) ) {
      if ( trimmed.startsWith( '-' ) ) {
        tags.push( parseScalar( trimmed.slice( 1 ) ) );
      }
      continue;
    }
    readingList = false;
    const field = indented ? null : /^tags\s*:(.*)$/i.exec( line.trimEnd() );
    if ( !field ) {
      continue;
    }
    const value = field[ 1 ]!.trim();
    readingList = !value;
    if ( value.startsWith( '[' ) && value.includes( ']' ) ) {
      tags.push( ...inlineListItems( value ).map( parseScalar ) );
    } else if ( value ) {
      tags.push( parseScalar( value ) );
    }
  }

  return normalizeTags( tags );
}

export function normalizeTags( tags: readonly string[]): string[] {
  return [ ...new Set( tags.map( ( tag ) => tag.trim().replace( /^#+/, '' ).trim() ).filter( Boolean ) ) ];
}

export function hasFrontmatterTags( content: string ): boolean {
  const bounds = frontmatterBounds( content );

  return bounds !== undefined && /^tags\s*:/im.test( content.slice( bounds.start, bounds.end ) );
}

export function updateFrontmatterTags( content: string, tags: readonly string[]): string {
  const normalized = normalizeTags( tags );
  if ( JSON.stringify( parseFrontmatterTags( content ) ) === JSON.stringify( normalized ) ) {
    return content;
  }
  const bounds = frontmatterBounds( content );
  if ( !bounds ) {
    const bom = content.startsWith( '\uFEFF' ) ? '\uFEFF' : '';
    const body = content.slice( bom.length );
    if ( body.split( '\n', 1 )[ 0 ]!.trim() === '---' ) {
      throw new Error( 'The existing frontmatter is not closed.' );
    }
    const ending = body.includes( '\r\n' ) ? '\r\n' : '\n';

    return `${ bom }---${ ending }${ tagBlock( normalized, ending ) }---${ ending }${ ending }${ body }`;
  }
  const body = content.slice( bounds.start, bounds.end );
  const span = editableTagSpan( body );
  const block = normalized.length ? tagBlock( normalized, bounds.lineEnding ) : '';
  const updated = span
    ? body.slice( 0, span.start ) + block + body.slice( span.end )
    : body + block;

  return content.slice( 0, bounds.start ) + updated + content.slice( bounds.end );
}

function frontmatterBounds( content: string ): FrontmatterBounds | undefined {
  const bomLength = content.startsWith( '\uFEFF' ) ? 1 : 0;
  const firstNewline = content.indexOf( '\n', bomLength );
  const start = firstNewline < 0 ? content.length : firstNewline + 1;
  const first = content.slice( bomLength, start );
  if ( first.trim() !== '---' ) {
    return undefined;
  }
  let end = start;
  while ( end < content.length ) {
    const newline = content.indexOf( '\n', end );
    const next = newline < 0 ? content.length : newline + 1;
    const line = content.slice( end, next );
    if ( line.trim() === '---' || line.trim() === '...' ) {
      return { start, end, lineEnding: first.endsWith( '\r\n' ) ? '\r\n' : '\n' };
    }
    end = next;
  }

  return undefined;
}

function sourceLines( value: string ): string[] {
  return value.match( /[^\n]*\n|[^\n]+$/g ) ?? [];
}

function inlineListItems( value: string ): string[] {
  const inner = value.slice( 1, value.lastIndexOf( ']' ) );
  const items: string[] = [];
  let start = 0;
  let quote = '';
  let escaped = false;
  for ( let index = 0; index < inner.length; index += 1 ) {
    const character = inner[ index ]!;
    if ( escaped ) {
      escaped = false;
    } else if ( character === '\\' && quote === '"' ) {
      escaped = true;
    } else if ( character === "'" || character === '"' ) {
      if ( quote === character ) {
        quote = '';
      } else if ( !quote ) {
        quote = character;
      }
    } else if ( character === ',' && !quote ) {
      items.push( inner.slice( start, index ).trim() );
      start = index + 1;
    }
  }
  items.push( inner.slice( start ).trim() );

  return items;
}

function parseScalar( input: string ): string {
  const value = input.trim();
  if ( value.length >= 2 && value.startsWith( "'" ) && value.endsWith( "'" ) ) {
    return value.slice( 1, -1 ).replace( /''/g, "'" );
  }
  if ( value.length >= 2 && value.startsWith( '"' ) && value.endsWith( '"' ) ) {
    return value.slice( 1, -1 ).replace( /\\(.)|\\$/gs, ( _, escaped: string | undefined ) => {
      return escaped === 'n' ? '\n' : escaped === 'r' ? '\r' : escaped === 't' ? '\t' : escaped ?? '';
    });
  }

  return value.split( ' #', 1 )[ 0 ]!.trim();
}

function editableTagSpan( body: string ): { start: number; end: number } | undefined {
  const lines = sourceLines( body );
  let span: { start: number; end: number } | undefined;
  let offset = 0;
  for ( let index = 0; index < lines.length; index += 1 ) {
    const line = lines[ index ]!;
    const field = /^tags\s*:(.*)$/i.exec( line.trimEnd() );
    const start = offset;
    offset += line.length;
    if ( !field ) {
      continue;
    }
    if ( span ) {
      throw new Error( 'Frontmatter contains more than one tags field.' );
    }
    const value = field[ 1 ]!.trim();
    if ( value && !( value.startsWith( '[' ) && value.endsWith( ']' )
      ? inlineListItems( value ).every( isSimpleScalar ) : isSimpleScalar( value ) ) ) {
      throw new Error( 'The tags field uses comments or complex YAML.' );
    }
    if ( value ) {
      for ( const continuation of lines.slice( index + 1 ) ) {
        const trimmed = continuation.trim();
        if ( !trimmed || trimmed.startsWith( '#' ) ) {
          continue;
        }
        if ( /^[ \t]/.test( continuation ) || /^-(?:[ \t]|$)/.test( trimmed ) ) {
          throw new Error( 'The tags field uses complex YAML.' );
        }
        break;
      }
    }
    span = { start, end: offset };
    let listIndent: string | undefined;
    while ( !value && index + 1 < lines.length ) {
      const continuation = lines[ index + 1 ]!;
      const trimmed = continuation.trim();
      if ( trimmed.startsWith( '#' ) ) {
        throw new Error( 'The tags field contains comments.' );
      }
      if ( trimmed && !/^-(?:[ \t]|$)/.test( trimmed ) ) {
        if ( /^[ \t]/.test( continuation ) ) {
          throw new Error( 'The tags field uses complex YAML.' );
        }
        break;
      }
      if ( trimmed && !isSimpleScalar( trimmed.slice( 1 ).trim() ) ) {
        throw new Error( 'The tags field uses comments or complex YAML.' );
      }
      if ( trimmed ) {
        const indent = /^[ \t]*/.exec( continuation )![ 0 ];
        if ( listIndent !== undefined && indent !== listIndent ) {
          throw new Error( 'The tags field uses a nested or inconsistently indented list.' );
        }
        listIndent = indent;
      }
      offset += continuation.length;
      index += 1;
      if ( trimmed ) {
        span.end = offset;
      }
    }
  }

  return span;
}

function isSimpleScalar( value: string ): boolean {
  if ( value.startsWith( "'" ) ) {
    return /^'(?:[^']|'')*'$/.test( value );
  }
  if ( value.startsWith( '"' ) ) {
    return /^"(?:[^"\\]|\\.)*"$/.test( value );
  }

  return !/^[&*!|>]/.test( value ) && !/[#[\]{},]|:\s/.test( value );
}

function tagBlock( tags: readonly string[], ending: string ): string {
  const escaped = tags.map( ( tag ) => tag.replace( /[\\"\x00-\x1f\x7f-\x9f]/g, ( character ) => {
    switch ( character ) {
      case '\\': return '\\\\';
      case '"': return '\\"';
      case '\n': return '\\n';
      case '\r': return '\\r';
      case '\t': return '\\t';
      default: return ' ';
    }
  }) );

  return `tags:${ ending }${ escaped.map( ( tag ) => `  - "${ tag }"${ ending }` ).join( '' ) }`;
}
