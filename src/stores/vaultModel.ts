import type { Folder, Note, VaultData } from '../types';

export function folderPathFromFolders(
  id: string | null,
  folders: readonly Folder[]
): string {
  if ( !id ) {
    return '';
  }
  const parts: string[] = [];
  const seen = new Set<string>();
  const foldersById = new Map( folders.map( ( folder ) => [ folder.id, folder ]) );
  let cursor = foldersById.get( id );
  while ( cursor && !seen.has( cursor.id ) ) {
    parts.unshift( cursor.name );
    seen.add( cursor.id );
    cursor = cursor.parentId ? foldersById.get( cursor.parentId ) : undefined;
  }

  return parts.join( '/' );
}

export function projectedNoteRelativePath(
  note: Note,
  folders: readonly Folder[],
  originalPath: string
): string {
  const targetFolder = folderPathFromFolders( note.folderId, folders );
  const originalName = originalPath.split( '/' ).at( -1 ) || 'Untitled note.md';
  const originalFolder = originalPath.split( '/' ).slice( 0, -1 ).join( '/' );
  const extensionMatch = originalName.match( /\.(markdown|md)$/iu );
  const extension = extensionMatch?.[ 1 ] ?? 'md';
  const originalStem = originalName.slice(
    0,
    extensionMatch?.index ?? originalName.length
  );
  const fileName = originalFolder === targetFolder && originalStem === note.title
    ? originalName
    : `${ safeNoteFileStem( note.title ) }.${ extension }`;

  return targetFolder ? `${ targetFolder }/${ fileName }` : fileName;
}

export function folderNameKey( name: string ): string {
  return name.trim().toLowerCase();
}

export function noteStemKey( note: Note ): string {
  return safeNoteStem( note.title ).toLowerCase();
}

export function noteFileNameKeys( note: Note ): Set<string> {
  const stem = noteStemKey( note );

  return new Set([ `${ stem }.md`, `${ stem }.markdown` ]);
}

export function folderConflictsWithNote(
  folderName: string,
  note: Note
): boolean {
  return noteFileNameKeys( note ).has( folderNameKey( folderName ) );
}

export function safeNoteStem( title: string ): string {
  const encoder = new TextEncoder();
  let result = '';
  let byteLength = 0;
  let previousWasReplacement = false;
  for ( const character of title.trim() ) {
    const forbidden = /[\u0000-\u001f\u007f-\u009f/\\:*?"<>|]/u.test( character );
    const addition = forbidden
      ? ( previousWasReplacement ? '' : '-' )
      : character;
    if ( addition ) {
      result += addition;
      byteLength += encoder.encode( addition ).length;
    }
    previousWasReplacement = forbidden;
    if ( byteLength >= 120 ) {
      break;
    }
  }

  result = result.replace( /^[ .]+|[ .]+$/g, '' ) || 'Untitled note';
  const windowsBase = result.split( '.' )[ 0 ]?.toUpperCase();
  if ([
    'CON',
    'PRN',
    'AUX',
    'NUL',
    'COM1',
    'COM2',
    'COM3',
    'COM4',
    'COM5',
    'COM6',
    'COM7',
    'COM8',
    'COM9',
    'LPT1',
    'LPT2',
    'LPT3',
    'LPT4',
    'LPT5',
    'LPT6',
    'LPT7',
    'LPT8',
    'LPT9'
  ].includes( windowsBase ) ) {
    return `_${ result }`;
  }

  return result;
}

export function uniqueNoteTitle( vault: VaultData, base: string ): string {
  const normalized = new Set(
    vault.notes.map( ( note ) => note.title.toLocaleLowerCase() )
  );
  if ( !normalized.has( base.toLocaleLowerCase() ) ) {
    return base;
  }
  let suffix = 2;
  while ( normalized.has( `${ base } ${ suffix }`.toLocaleLowerCase() ) ) {
    suffix += 1;
  }

  return `${ base } ${ suffix }`;
}

export function ensureFolderPath(
  vault: VaultData,
  path: string,
  createFolderId: () => string
): string | null {
  const parts = path
    .split( /[\\/]/ )
    .map( ( part ) => part.trim() )
    .filter( Boolean );
  let parentId: string | null = null;
  for ( const part of parts ) {
    let folder = vault.folders.find( ( candidate ) =>
      candidate.parentId === parentId
      && candidate.name.toLocaleLowerCase() === part.toLocaleLowerCase()
    );
    if ( !folder ) {
      folder = {
        id: createFolderId(),
        name: part,
        parentId,
        createdAt: Date.now()
      };
      vault.folders.push( folder );
    }
    parentId = folder.id;
  }

  return parentId;
}

export function descendantFolderIds( vault: VaultData, id: string ): string[] {
  const result: string[] = [];
  const queue = [ id ];
  while ( queue.length ) {
    const parent = queue.shift();
    for ( const folder of vault.folders ) {
      if ( folder.parentId === parent && !result.includes( folder.id ) ) {
        result.push( folder.id );
        queue.push( folder.id );
      }
    }
  }

  return result;
}

export function replaceTemplateTokens(
  value: string,
  tokens: Record<string, string>
): string {
  return value.replace(
    /{{\s*(date|time|title)\s*}}/gi,
    ( _, key: string ) => tokens[ key.toLocaleLowerCase() ] ?? ''
  );
}

export function createId( prefix: string ): string {
  const random = typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? crypto.randomUUID()
    : `${ Date.now().toString( 36 ) }-${ Math.random().toString( 36 ).slice( 2 ) }`;

  return `${ prefix }-${ random }`;
}

function safeNoteFileStem( value: string ): string {
  const encoder = new TextEncoder();
  let result = '';
  let previousWasReplacement = false;
  for ( const character of value.trim() ) {
    if ( /[\p{Cc}/\\:*?"<>|]/u.test( character ) ) {
      if ( !previousWasReplacement ) {
        result += '-';
        previousWasReplacement = true;
      }
    } else {
      result += character;
      previousWasReplacement = false;
    }
    if ( encoder.encode( result ).length >= 120 ) {
      break;
    }
  }
  result = result.replace( /^[ .]+|[ .]+$/g, '' ) || 'Untitled note';

  return /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/iu.test( result )
    ? `_${ result }`
    : result;
}
