import { createSeedVault } from '../data/seed';
import type { Note, VaultData, VaultDescriptor } from '../types';
import type { SmartFolderSelection } from './vaultState';

const APP_ZOOM_KEY = 'obsidian-at-home.zoom.v1';
const RECENT_NOTE_LIMIT = 10;
const ZOOM_STEP = 0.1;

export const MIN_ZOOM = 0.7;
export const MAX_ZOOM = 1.5;

export function normalizeVault( input: Partial<VaultData> ): VaultData {
  const fallback = createSeedVault();
  const rawNotes = Array.isArray( input.notes ) ? input.notes : fallback.notes;
  const notes: Note[] = rawNotes.map( ( note ) => ({
    ...note,
    relativePath: typeof note.relativePath === 'string' ? note.relativePath : '',
    tags: Array.isArray( note.tags ) ? note.tags : []
  }) );
  const folders = Array.isArray( input.folders ) ? input.folders : fallback.folders;

  const currentBuiltInSnippets = new Map(
    fallback.snippets
      .filter( ( snippet ) => snippet.builtIn )
      .map( ( snippet ) => [ snippet.id, snippet ])
  );
  const snippets = ( Array.isArray( input.snippets )
    ? input.snippets
    : fallback.snippets
  ).map( ( snippet ) => {
    const current = currentBuiltInSnippets.get( snippet.id );
    if ( snippet.builtIn && current ) {
      return {
        ...snippet,
        name: current.name,
        description: current.description,
        css: current.css
      };
    }

    return snippet;
  });

  const activeNoteId = typeof input.activeNoteId === 'string'
    && notes.some( ( note ) => note.id === input.activeNoteId )
    ? input.activeNoteId
    : notes[ 0 ]?.id ?? null;
  const recentNoteIds = normalizeRecentNoteIds(
    input.recentNoteIds,
    notes,
    activeNoteId
  );
  const selectedFolderId = normalizeFolderSelection(
    input.selectedFolderId,
    notes,
    recentNoteIds
  );
  const embeddedImages = Array.isArray( input.embeddedImages )
    ? input.embeddedImages.filter( ( image ) =>
      image
      && typeof image.id === 'string'
      && typeof image.relativePath === 'string'
      && typeof image.mediaType === 'string'
    )
    : [];
  const imageFiles = Array.isArray( input.imageFiles )
    ? input.imageFiles.filter( ( image ) =>
      image
      && ( image.assetId === undefined || typeof image.assetId === 'string' )
      && typeof image.relativePath === 'string'
      && typeof image.mediaType === 'string'
    )
    : [];
  const imageEmbedSettings = normalizeAssetEmbedSettings( input.imageEmbedSettings );
  const embeddedAttachments = Array.isArray( input.embeddedAttachments )
    ? input.embeddedAttachments.filter( ( attachment ) =>
      attachment
      && typeof attachment.id === 'string'
      && typeof attachment.relativePath === 'string'
      && typeof attachment.mediaType === 'string'
      && Number.isSafeInteger( attachment.byteLength )
      && attachment.byteLength >= 0
      && typeof attachment.openingDisabled === 'boolean'
    )
    : [];
  const attachmentFiles = Array.isArray( input.attachmentFiles )
    ? input.attachmentFiles.filter( ( attachment ) =>
      attachment
      && ( attachment.assetId === undefined || typeof attachment.assetId === 'string' )
      && typeof attachment.relativePath === 'string'
      && typeof attachment.mediaType === 'string'
      && Number.isSafeInteger( attachment.byteLength )
      && attachment.byteLength >= 0
      && typeof attachment.openingDisabled === 'boolean'
    )
    : [];
  const attachmentEmbedSettings = normalizeAssetEmbedSettings(
    input.attachmentEmbedSettings
  );

  return {
    name: typeof input.name === 'string' && input.name.trim()
      ? input.name
      : fallback.name,
    notes,
    folders,
    templates: Array.isArray( input.templates ) && input.templates.length
      ? input.templates
      : fallback.templates,
    snippets,
    activeNoteId,
    recentNoteIds,
    selectedFolderId,
    embeddedImages,
    imageFiles,
    imageEmbedSettings,
    embeddedAttachments,
    attachmentFiles,
    attachmentEmbedSettings
  };
}

export function cloneValue<T>( value: T ): T {
  return JSON.parse( JSON.stringify( value ) ) as T;
}

export function mergeRecentVaults(
  current: VaultDescriptor,
  recentVaults: VaultDescriptor[]
): VaultDescriptor[] {
  const merged = [
    current,
    ...recentVaults.filter( ( vault ) => vault.path !== current.path )
  ];

  return merged.slice( 0, 12 );
}

export function errorMessage( error: unknown, fallback: string ): string {
  if ( typeof error === 'string' && error.trim() ) {
    return error;
  }
  if ( error instanceof Error && error.message.trim() ) {
    return error.message;
  }

  return fallback;
}

export function isRevisionConflict( message: string ): boolean {
  const normalized = message.toLocaleLowerCase();

  return normalized.includes( 'changed' )
    && (
      normalized.includes( 'vault' )
      || normalized.includes( 'file' )
      || normalized.includes( 'disk' )
    );
}

export function safeStorageGet( key: string ): string | null {
  if ( typeof localStorage === 'undefined' ) {
    return null;
  }
  try {
    return localStorage.getItem( key );
  } catch {
    return null;
  }
}

export function safeStorageSet( key: string, value: string ): void {
  if ( typeof localStorage === 'undefined' ) {
    return;
  }
  try {
    localStorage.setItem( key, value );
  } catch {
    // Local preferences are non-critical when browser storage is unavailable.
  }
}

export function readStoredZoom(): number {
  const storedZoom = Number.parseFloat( safeStorageGet( APP_ZOOM_KEY ) ?? '' );

  return Number.isFinite( storedZoom ) ? clampZoom( storedZoom ) : 1;
}

export function persistStoredZoom( zoom: number ): void {
  safeStorageSet( APP_ZOOM_KEY, String( zoom ) );
}

export function clampZoom( zoom: number ): number {
  const roundedZoom = Number(
    ( Math.round( zoom / ZOOM_STEP ) * ZOOM_STEP ).toFixed( 2 )
  );

  return Math.min( MAX_ZOOM, Math.max( MIN_ZOOM, roundedZoom ) );
}

export function zoomStep(): number {
  return ZOOM_STEP;
}

function normalizeAssetEmbedSettings(
  value: VaultData[ 'imageEmbedSettings' ] | undefined
): VaultData[ 'imageEmbedSettings' ] {
  const legacySettings = value as {
    folderPath?: unknown;
    location?: string;
  } | undefined;
  if ( legacySettings?.location === 'specified-folder-mirrored' ) {
    return {
      location: 'specified-folder',
      folderPath: typeof legacySettings.folderPath === 'string'
        ? legacySettings.folderPath
        : ''
    };
  }
  if (
    value?.location === 'note-folder'
    || value?.location === 'specified-folder'
  ) {
    return {
      location: value.location,
      folderPath: typeof value.folderPath === 'string' ? value.folderPath : ''
    };
  }

  return { location: 'vault-root', folderPath: '' };
}

function normalizeFolderSelection(
  selection: unknown,
  notes: Note[],
  recentNoteIds: string[]
): SmartFolderSelection {
  if ( selection === 'recent' && recentNoteIds.length ) {
    return 'recent';
  }
  if ( selection === 'favorites' && notes.some( ( note ) => note.pinned ) ) {
    return 'favorites';
  }

  return 'all';
}

function normalizeRecentNoteIds(
  value: unknown,
  notes: Note[],
  activeNoteId: string | null
): string[] {
  const noteIds = new Set( notes.map( ( note ) => note.id ) );
  const recentNoteIds: string[] = [];
  const addNote = ( id: unknown ): void => {
    if (
      typeof id === 'string'
      && noteIds.has( id )
      && !recentNoteIds.includes( id )
      && recentNoteIds.length < RECENT_NOTE_LIMIT
    ) {
      recentNoteIds.push( id );
    }
  };

  addNote( activeNoteId );
  if ( Array.isArray( value ) ) {
    value.forEach( addNote );
  }

  return recentNoteIds;
}
