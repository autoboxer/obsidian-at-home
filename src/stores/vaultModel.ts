import type { Folder, ImportedNote, Note, VaultData } from '../types';

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

/** Reserve the whole imported batch before rewriting any of its references. */
export function planImportedNotes(
  vault: VaultData,
  importedNotes: readonly ImportedNote[],
  replace: boolean
): {
  folders: Folder[];
  notes: Array<{ source: ImportedNote; title: string; folderId: string | null; relativePath: string }>;
} {
  const sourcePaths = new Set<string>();
  const sourceFolders = new Set<string>();
  for ( const note of importedNotes ) {
    const parts = note.relativePath.split( '/' );
    if (
      parts.some( ( part ) => !part || part === '.' || part === '..' )
      || !/\.(?:md|markdown)$/i.test( parts.at( -1 )! )
      || sourcePaths.has( note.relativePath )
    ) {
      throw new Error( 'Imported notes must have unique relative Markdown file paths.' );
    }
    sourcePaths.add( note.relativePath );
    for ( let length = 1; length < parts.length; length += 1 ) {
      sourceFolders.add( parts.slice( 0, length ).join( '/' ) );
    }
  }

  const key = ( path: string ): string => path.normalize( 'NFC' ).toLowerCase();
  const assetPaths = [ ...vault.imageFiles, ...vault.attachmentFiles ].map( ( file ) => file.relativePath );
  const notePaths = vault.notes.map( ( note ) => projectedNoteRelativePath( note, vault.folders, note.relativePath ) );
  // Even replacement cannot create a directory on top of an existing file.
  const folderFileKeys = new Set([ ...notePaths, ...assetPaths ].map( key ) );
  const fileKeys = new Set([ ...( replace ? [] : notePaths ), ...assetPaths ].map( key ) );
  const directoryKeys = new Set( vault.folders.map( ( folder ) => key( folderPathFromFolders( folder.id, vault.folders ) ) ) );
  const plannedVault = { ...vault, folders: replace ? [] : vault.folders.map( ( folder ) => ({ ...folder }) ) };
  if ( replace ) {
    for ( const path of assetPaths ) {
      ensureFolderPath( plannedVault, path.split( '/' ).slice( 0, -1 ).join( '/' ), () => createId( 'folder' ) );
    }
  }
  const foldersByPath = new Map( plannedVault.folders.map( ( folder ) => [
    key( folderPathFromFolders( folder.id, plannedVault.folders ) ), folder
  ]) );
  const folderOwners = new Map<string, string>();
  const mappedFolders = new Map<string, string | null>([[ '', null ]]);
  const preferredFolderNames = new Map<string, Set<string>>();
  for ( const path of sourceFolders ) {
    const parts = path.split( '/' );
    const name = safeNoteFileStem( parts.pop()! );
    const parent = parts.join( '/' );
    const names = preferredFolderNames.get( parent ) ?? new Set<string>();
    names.add( key( name ) );
    preferredFolderNames.set( parent, names );
  }
  // Parents precede children; exact source paths distinguish case-only siblings.
  for ( const source of [ ...sourceFolders ].sort( ( a, b ) => {
    const aParts = a.split( '/' );
    const bParts = b.split( '/' );
    const aName = aParts.at( -1 )!;
    const bName = bParts.at( -1 )!;

    return aParts.length - bParts.length
      || Number( bName === safeNoteFileStem( bName ) ) - Number( aName === safeNoteFileStem( aName ) )
      || ( a < b ? -1 : 1 );
  }) ) {
    const parts = source.split( '/' );
    const base = safeNoteFileStem( parts.pop()! );
    const sourceParent = parts.join( '/' );
    const parentId = mappedFolders.get( sourceParent ) ?? null;
    const parentPath = folderPathFromFolders( parentId, plannedVault.folders );
    for ( let suffix = 1; ; suffix += 1 ) {
      const name = suffixedImportStem( base, suffix );
      const path = parentPath ? `${ parentPath }/${ name }` : name;
      const pathKey = key( path );
      const existing = foldersByPath.get( pathKey );
      if (
        folderFileKeys.has( pathKey )
        || ( existing && folderOwners.has( existing.id ) )
        || ( suffix > 1 && preferredFolderNames.get( sourceParent )?.has( key( name ) ) )
      ) {
        continue;
      }
      const folder = existing ?? { id: createId( 'folder' ), name, parentId, createdAt: Date.now() };
      if ( !existing ) {
        plannedVault.folders.push( folder );
        foldersByPath.set( pathKey, folder );
      }
      mappedFolders.set( source, folder.id );
      folderOwners.set( folder.id, source );
      directoryKeys.add( pathKey );
      break;
    }
  }

  const plans = importedNotes.map( ( source ) => {
    const parts = source.relativePath.split( '/' );
    const filename = parts.pop()!;
    const extension = filename.match( /\.(md|markdown)$/i )![ 1 ]!;
    const title = safeNoteFileStem( filename.slice( 0, -extension.length - 1 ) );
    const folderId = mappedFolders.get( parts.join( '/' ) ) ?? null;
    const folder = folderPathFromFolders( folderId, plannedVault.folders );
    const destination = ( name: string ): string => `${ folder ? `${ folder }/` : '' }${ name }.${ extension }`;

    return { source, title, folderId, relativePath: destination( title ), destination };
  });
  // Do not let a suffixed collision steal another imported file's original name.
  const preferredFiles = new Set( plans.map( ( plan ) => key( plan.relativePath ) ) );
  for ( const plan of [ ...plans ].sort( ( a, b ) => {
    const originalNameKept = ( plan: typeof a ): boolean =>
      plan.relativePath.split( '/' ).at( -1 ) === plan.source.relativePath.split( '/' ).at( -1 );

    return Number( originalNameKept( b ) ) - Number( originalNameKept( a ) )
      || ( a.source.relativePath < b.source.relativePath ? -1 : 1 );
  }) ) {
    for ( let suffix = 1; ; suffix += 1 ) {
      const title = suffixedImportStem( plan.title, suffix );
      const path = plan.destination( title );
      const pathKey = key( path );
      if ( fileKeys.has( pathKey ) || directoryKeys.has( pathKey ) || ( suffix > 1 && preferredFiles.has( pathKey ) ) ) {
        continue;
      }
      plan.title = title;
      plan.relativePath = path;
      fileKeys.add( pathKey );
      break;
    }
  }

  return { folders: plannedVault.folders, notes: plans.map( ({ destination: _destination, ...plan }) => plan ) };
}

function suffixedImportStem( base: string, suffix: number ): string {
  if ( suffix === 1 ) {
    return base;
  }
  const ending = ` ${ suffix }`;
  const encoder = new TextEncoder();
  let stem = '';
  for ( const character of base ) {
    if ( encoder.encode( stem + character + ending ).length > 120 ) {
      break;
    }
    stem += character;
  }

  return stem.trimEnd() + ending;
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
