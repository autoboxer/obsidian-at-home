import { watch, type WatchStopHandle } from 'vue';
import { createSeedVault, legacyBuiltInSnippets } from '../data/seed';
import { hasFrontmatterTags, normalizeTags, parseFrontmatterTags, updateFrontmatterTags } from '../lib/frontmatterTags';
import { isFinderMetadataPath } from '../lib/markdownAttachments';
import type { BuiltInSnippetDefaults, Note, VaultData, VaultDescriptor } from '../types';
import type { SmartFolderSelection } from './vaultState';

const APP_ZOOM_KEY = 'obsidian-at-home.zoom.v1';
const RECENT_NOTE_LIMIT = 10;
const ZOOM_STEP = 0.1;

export const MIN_ZOOM = 0.7;
export const MAX_ZOOM = 1.5;

export function createVaultChangeTracker(
  vault: VaultData,
  canTrack: () => boolean,
  onChange: () => void
) {
  let version = 0;
  let savedVersion = 0;
  let batchDepth = 0;
  const notes = new Map<string, number>();
  const fields = new Map<keyof VaultData, number>();
  const pendingNotes = new Set<string>();
  const pendingFields = new Set<keyof VaultData>();
  const noteWatchers = new Map<Note, { id: string; stop: WatchStopHandle }>();

  function publish(): void {
    if ( !pendingNotes.size && !pendingFields.size ) {
      return;
    }
    version += 1;
    for ( const id of pendingNotes ) {
      notes.set( id, version );
    }
    for ( const field of pendingFields ) {
      fields.set( field, version );
    }
    pendingNotes.clear();
    pendingFields.clear();
    onChange();
  }

  function batch<T>( mutation: () => T ): T {
    batchDepth += 1;
    try {
      return mutation();
    } finally {
      batchDepth -= 1;
      if ( !batchDepth ) {
        publish();
      }
    }
  }

  function reconcileNotes(): void {
    const currentNotes = new Set( vault.notes );
    for ( const [ note, watcher ] of noteWatchers ) {
      if ( !currentNotes.has( note ) ) {
        watcher.stop();
        noteWatchers.delete( note );
        if ( canTrack() ) {
          pendingNotes.add( watcher.id );
        }
      }
    }
    for ( const note of currentNotes ) {
      if ( noteWatchers.has( note ) ) {
        continue;
      }
      const watcher = {
        id: note.id,
        stop: watch( note, () => {
          if ( canTrack() ) {
            // Retain the old identity too if a caller replaces an ID in place.
            pendingNotes.add( watcher.id );
            pendingNotes.add( note.id );
            if ( !batchDepth ) {
              publish();
            }
          }
          watcher.id = note.id;
        }, { deep: true, flush: 'sync' })
      };
      noteWatchers.set( note, watcher );
      if ( canTrack() ) {
        pendingNotes.add( note.id );
      }
    }
  }

  reconcileNotes();
  pendingNotes.clear();
  const stopNotes = watch( () => vault.notes, () => batch( () => {
    if ( canTrack() ) {
      pendingFields.add( 'notes' );
    }
    // Observe membership only here; a body edit traverses just its own note.
    // Reconcile even during hydration so old objects cannot dirty the new vault.
    reconcileNotes();
  }), { deep: 1, flush: 'sync' });
  const stopFields = ( Object.keys( vault ) as ( keyof VaultData )[])
    .filter( ( field ) => field !== 'notes' )
    .map( ( field ) => watch( () => vault[ field ], () => {
      if ( !canTrack() ) {
        return;
      }
      pendingFields.add( field );
      // Folder edits can change descendant paths without changing note objects.
      if ( field === 'folders' ) {
        for ( const note of vault.notes ) {
          pendingNotes.add( note.id );
        }
      }
      if ( !batchDepth ) {
        publish();
      }
    }, { deep: true, flush: 'sync' }) );

  return {
    batch,
    get version() {
      return version;
    },
    get savedVersion() {
      return savedVersion;
    },
    get hasChanges() {
      return Boolean( notes.size || fields.size || pendingNotes.size || pendingFields.size );
    },
    capture() {
      // An explicit save inside a batch must include changes already made.
      publish();

      return { version, noteIds: [ ...notes.keys() ], fields: [ ...fields.keys() ] };
    },
    acknowledge( targetVersion: number ): void {
      savedVersion = Math.max( savedVersion, targetVersion );
      for ( const [ id, changedAt ] of notes ) {
        if ( changedAt <= targetVersion ) {
          notes.delete( id );
        }
      }
      for ( const [ field, changedAt ] of fields ) {
        if ( changedAt <= targetVersion ) {
          fields.delete( field );
        }
      }
    },
    reset( baseline = 0 ): void {
      version = baseline;
      savedVersion = baseline;
      notes.clear();
      fields.clear();
      pendingNotes.clear();
      pendingFields.clear();
    },
    stop(): void {
      stopNotes();
      stopFields.forEach( ( stop ) => stop() );
      noteWatchers.forEach( ( watcher ) => watcher.stop() );
      noteWatchers.clear();
    }
  };
}

export function normalizeNote( note: Note ): Note {
  let content = note.content;
  const storedTags = normalizeTags( Array.isArray( note.tags ) ? note.tags : []);
  // Older browser notes and seed data stored control edits only in the tag array.
  // An explicit source field, including an empty one, takes precedence.
  if ( storedTags.length && !hasFrontmatterTags( content ) ) {
    try {
      content = updateFrontmatterTags( content, storedTags );
    } catch {
      // Preserve legacy metadata when unfinished frontmatter cannot be migrated.
      return { ...note, relativePath: note.relativePath ?? '', tags: storedTags };
    }
  }

  return {
    ...note,
    content,
    relativePath: typeof note.relativePath === 'string' ? note.relativePath : '',
    tags: parseFrontmatterTags( content )
  };
}

export function normalizeVault( input: Partial<VaultData> ): VaultData {
  const fallback = createSeedVault();
  const rawNotes = Array.isArray( input.notes ) ? input.notes : fallback.notes;
  const notes = rawNotes.map( normalizeNote );
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
      // Only fields still matching their bundled baseline receive app updates.
      // For older vaults, recognize exact shipped definitions; preserve anything
      // else rather than guessing which parts the user changed.
      const baseline = validSnippetDefaults( snippet.builtInDefaults )
        ? snippet.builtInDefaults
        : [ current, ...legacyBuiltInSnippets ].find( ( candidate ) =>
          candidate.id === snippet.id
          && candidate.name === snippet.name
          && candidate.description === snippet.description
          && candidate.css === snippet.css
        );
      return {
        ...snippet,
        name: baseline && snippet.name === baseline.name ? current.name : snippet.name,
        description: baseline && snippet.description === baseline.description ? current.description : snippet.description,
        css: baseline && snippet.css === baseline.css ? current.css : snippet.css,
        builtInDefaults: {
          name: current.name,
          description: current.description,
          css: current.css
        }
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
      && !isFinderMetadataPath( attachment.relativePath )
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
      && !isFinderMetadataPath( attachment.relativePath )
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

function validSnippetDefaults( value: unknown ): value is BuiltInSnippetDefaults {
  if ( !value || typeof value !== 'object' ) {
    return false;
  }
  const defaults = value as Partial<BuiltInSnippetDefaults>;

  return typeof defaults.name === 'string'
    && typeof defaults.description === 'string'
    && typeof defaults.css === 'string';
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
