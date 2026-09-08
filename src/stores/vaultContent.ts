import {
  normalizeWikiTarget,
  rewriteMarkdownNoteLinksForNotePaths,
  wikiLinkCandidates,
  type WikiLinkTarget
} from '../lib/wikiLinks';
import { parseFrontmatterTags, updateFrontmatterTags } from '../lib/frontmatterTags';
import type { CssSnippet, Folder, Note, NoteTemplate } from '../types';
import {
  createId,
  descendantFolderIds,
  folderConflictsWithNote,
  folderNameKey,
  noteFileNameKeys,
  noteStemKey,
  replaceTemplateTokens,
  safeNoteStem,
  uniqueNoteTitle
} from './vaultModel';
import { isSmartFolderSelection } from './vaultNavigation';
import {
  canEditVault,
  uiState,
  vaultSession,
  vaultState,
  type ToastTone
} from './vaultState';

interface VaultContentDependencies {
  activeNote: () => Note | undefined;
  currentFolderId: () => string | null;
  flushVault: () => Promise<boolean>;
  folderContainsAssets: ( id: string ) => boolean;
  notify: ( message: string, tone: ToastTone ) => void;
  noteLinkPaths: () => ReadonlyMap<string, string>;
  rememberNoteOriginalPath: ( note: Note ) => void;
  selectNote: ( id: string ) => void;
}

type NotePatch = Partial<Pick<
  Note,
  'title' | 'content' | 'folderId' | 'tags' | 'pinned'
>>;

export function createVaultContent(
  dependencies: VaultContentDependencies
) {
  function preserveRelocatedNoteLinks( previousPaths: ReadonlyMap<string, string> ): void {
    const nextPaths = dependencies.noteLinkPaths();
    for ( const note of vaultState.notes ) {
      if ( previousPaths.get( note.id ) === nextPaths.get( note.id ) ) {
        continue;
      }
      const content = rewriteMarkdownNoteLinksForNotePaths(
        note, vaultState.notes, previousPaths, nextPaths
      );
      if ( content !== note.content ) {
        note.content = content;
        note.updatedAt = Date.now();
      }
    }
  }

  function createNote(
    folderId?: string | null,
    title = 'Untitled note',
    content?: string
  ): Note | undefined {
    if ( !canEditVault.value ) {
      return undefined;
    }
    const now = Date.now();
    const note: Note = {
      id: createId( 'note' ),
      title: uniqueNoteTitle( vaultState, title.trim() || 'Untitled note' ),
      content: content ?? '# Untitled note\n\n',
      relativePath: '',
      folderId: folderId === undefined ? dependencies.currentFolderId() : folderId,
      tags: [],
      pinned: false,
      createdAt: now,
      updatedAt: now
    };
    if ( content === undefined ) {
      note.content = `# ${ note.title }\n\n`;
    }
    note.tags = parseFrontmatterTags( note.content );
    vaultState.notes.unshift( note );
    dependencies.selectNote( note.id );
    vaultState.selectedFolderId = 'all';
    uiState.tool = 'notes';
    uiState.notesView = 'editor';
    uiState.noteFilter = '';
    dependencies.notify( 'New note created', 'success' );

    return note;
  }

  function createLinkedNote( link: WikiLinkTarget | string ): Note | undefined {
    const target = typeof link === 'string' ? normalizeWikiTarget( link ) : link.target;
    const candidates = wikiLinkCandidates(
      { target },
      vaultState.notes,
      dependencies.activeNote(),
      dependencies.noteLinkPaths()
    );
    if ( candidates.length === 1 ) {
      const existing = candidates[ 0 ]!;
      dependencies.selectNote( existing.id );

      return existing;
    }
    if ( candidates.length > 1 ) {
      dependencies.notify( `More than one note matches “${ target }”. Use the full file path in the link.`, 'warning' );

      return undefined;
    }
    if ( !canEditVault.value ) {
      dependencies.notify( `Could not find note “${ target }” in this read-only vault`, 'warning' );

      return undefined;
    }

    const title = target.replace( /\.md$/i, '' );
    // Creating a basename-only note would silently change an explicit destination.
    if (
      !title || target.includes( '/' ) || /\.markdown$/i.test( target )
      || safeNoteStem( title ) !== title
      || uniqueNoteTitle( vaultState, title ) !== title
    ) {
      dependencies.notify( `Could not find note “${ target }”`, 'warning' );

      return undefined;
    }

    return createNote(
      dependencies.activeNote()?.folderId ?? dependencies.currentFolderId(),
      title
    );
  }

  function updateNote( id: string, patch: NotePatch ): boolean {
    if ( !canEditVault.value ) {
      return false;
    }
    const note = vaultState.notes.find( ( candidate ) => candidate.id === id );
    if ( !note ) {
      return false;
    }
    let content = patch.content ?? note.content;
    if ( patch.tags !== undefined ) {
      try {
        content = updateFrontmatterTags( content, patch.tags );
      } catch ( error ) {
        const reason = error instanceof Error ? error.message : 'The tags could not be updated.';
        dependencies.notify( `${ reason } Edit the tags in Markdown source instead.`, 'warning' );

        return false;
      }
    }
    const locationChanged = (
      patch.title !== undefined && patch.title !== note.title
    ) || (
      patch.folderId !== undefined && patch.folderId !== note.folderId
    );
    const previousPaths = locationChanged ? dependencies.noteLinkPaths() : undefined;
    if ( locationChanged ) {
      dependencies.rememberNoteOriginalPath( note );
    }
    if ( patch.title !== undefined ) {
      note.title = patch.title;
    }
    if ( patch.content !== undefined || patch.tags !== undefined ) {
      note.content = content;
      const tags = parseFrontmatterTags( content );
      if ( tags.length !== note.tags.length || tags.some( ( tag, index ) => tag !== note.tags[ index ]) ) {
        note.tags = tags;
      }
    }
    if ( patch.folderId !== undefined ) {
      note.folderId = patch.folderId;
    }
    if ( patch.pinned !== undefined ) {
      note.pinned = patch.pinned;
    }
    if ( previousPaths ) {
      preserveRelocatedNoteLinks( previousPaths );
    }
    note.updatedAt = Date.now();

    return true;
  }

  async function moveNoteToFolder(
    noteId: string,
    folderId: string | null
  ): Promise<boolean> {
    if ( !canEditVault.value ) {
      return false;
    }
    let note = vaultState.notes.find( ( candidate ) => candidate.id === noteId );
    if ( !note ) {
      dependencies.notify( 'Could not move that note', 'warning' );

      return false;
    }

    const folder = folderId === null
      ? null
      : vaultState.folders.find( ( candidate ) => candidate.id === folderId );
    if ( folderId !== null && !folder ) {
      dependencies.notify( 'That folder is no longer available', 'warning' );

      return false;
    }
    if ( note.folderId === folderId ) {
      return false;
    }
    const movingNoteStem = noteStemKey( note );
    const duplicateNote = vaultState.notes.some(
      ( candidate ) => candidate.id !== noteId
        && candidate.folderId === folderId
        && noteStemKey( candidate ) === movingNoteStem
    );
    const noteFileNames = noteFileNameKeys( note );
    const duplicateFolder = vaultState.folders.some(
      ( candidate ) => candidate.parentId === folderId
        && noteFileNames.has( folderNameKey( candidate.name ) )
    );
    if ( duplicateNote || duplicateFolder ) {
      dependencies.notify( 'A file with that name already exists there', 'warning' );

      return false;
    }

    if ( vaultSession.backend === 'native' ) {
      if ( !( await dependencies.flushVault() ) ) {
        return false;
      }
      note = vaultState.notes.find( ( candidate ) => candidate.id === noteId );
      if ( !note?.relativePath ) {
        dependencies.notify( 'Save the note before moving it', 'warning' );

        return false;
      }
    }

    if ( !updateNote( noteId, { folderId }) ) {
      return false;
    }
    if ( vaultSession.backend === 'native' && !( await dependencies.flushVault() ) ) {
      return false;
    }

    dependencies.notify( `Moved to ${ folder?.name ?? 'Vault root' }`, 'success' );

    return true;
  }

  function togglePinned( id: string ): void {
    const note = vaultState.notes.find( ( candidate ) => candidate.id === id );
    if ( note ) {
      updateNote( id, { pinned: !note.pinned });
    }
  }

  function createFolder(
    name: string,
    parentId: string | null = null
  ): Folder | undefined {
    if ( !canEditVault.value ) {
      return undefined;
    }
    const cleanName = name.trim().replace( /[\\/]/g, ' ' );
    if ( !cleanName ) {
      return undefined;
    }
    const duplicate = vaultState.folders.some(
      ( folder ) => folder.parentId === parentId
        && folderNameKey( folder.name ) === folderNameKey( cleanName )
    ) || vaultState.notes.some(
      ( note ) => note.folderId === parentId
        && folderConflictsWithNote( cleanName, note )
    );
    if ( duplicate ) {
      dependencies.notify(
        'A file or folder with that name already exists here',
        'warning'
      );

      return undefined;
    }
    const folder: Folder = {
      id: createId( 'folder' ),
      name: cleanName,
      parentId,
      createdAt: Date.now()
    };
    vaultState.folders.push( folder );
    if ( !isSmartFolderSelection( vaultState.selectedFolderId ) ) {
      vaultState.selectedFolderId = 'all';
    }
    dependencies.notify( `Created ${ cleanName }`, 'success' );

    return folder;
  }

  function renameFolder( id: string, name: string ): void {
    if ( !canEditVault.value ) {
      return;
    }
    const folder = vaultState.folders.find( ( candidate ) => candidate.id === id );
    const cleanName = name.trim().replace( /[\\/]/g, ' ' );
    if ( !folder || !cleanName || folder.name === cleanName ) {
      return;
    }

    const duplicate = vaultState.folders.some(
      ( candidate ) => candidate.id !== id
        && candidate.parentId === folder.parentId
        && folderNameKey( candidate.name ) === folderNameKey( cleanName )
    ) || vaultState.notes.some(
      ( note ) => note.folderId === folder.parentId
        && folderConflictsWithNote( cleanName, note )
    );
    if ( duplicate ) {
      dependencies.notify(
        'A file or folder with that name already exists here',
        'warning'
      );

      return;
    }

    if ( dependencies.folderContainsAssets( id ) ) {
      dependencies.notify(
        'Move contained assets before renaming this folder',
        'warning'
      );

      return;
    }

    const affectedFolders = new Set([ id, ...descendantFolderIds( vaultState, id ) ]);
    const previousPaths = dependencies.noteLinkPaths();
    for ( const note of vaultState.notes ) {
      if ( note.folderId && affectedFolders.has( note.folderId ) ) {
        dependencies.rememberNoteOriginalPath( note );
      }
    }
    folder.name = cleanName;
    preserveRelocatedNoteLinks( previousPaths );
  }

  function moveFolder( folderId: string, parentId: string | null ): boolean {
    if ( !canEditVault.value ) {
      return false;
    }
    const folder = vaultState.folders.find( ( candidate ) => candidate.id === folderId );
    if ( !folder ) {
      dependencies.notify( 'Could not move that folder', 'warning' );

      return false;
    }

    const parent = parentId === null
      ? null
      : vaultState.folders.find( ( candidate ) => candidate.id === parentId );
    if ( parentId !== null && !parent ) {
      dependencies.notify( 'That folder is no longer available', 'warning' );

      return false;
    }
    if ( folder.parentId === parentId ) {
      return false;
    }

    const affectedFolders = new Set([
      folderId,
      ...descendantFolderIds( vaultState, folderId )
    ]);
    if ( parentId !== null && affectedFolders.has( parentId ) ) {
      dependencies.notify( 'A folder cannot be moved inside itself', 'warning' );

      return false;
    }

    const duplicate = vaultState.folders.some(
      ( candidate ) => candidate.id !== folderId
        && candidate.parentId === parentId
        && folderNameKey( candidate.name ) === folderNameKey( folder.name )
    ) || vaultState.notes.some(
      ( note ) => note.folderId === parentId
        && folderConflictsWithNote( folder.name, note )
    );
    if ( duplicate ) {
      dependencies.notify(
        'A file or folder with that name already exists there',
        'warning'
      );

      return false;
    }

    if ( dependencies.folderContainsAssets( folderId ) ) {
      dependencies.notify(
        'Move contained assets before moving this folder',
        'warning'
      );

      return false;
    }

    const previousPaths = dependencies.noteLinkPaths();
    for ( const note of vaultState.notes ) {
      if ( note.folderId && affectedFolders.has( note.folderId ) ) {
        dependencies.rememberNoteOriginalPath( note );
      }
    }
    folder.parentId = parentId;
    preserveRelocatedNoteLinks( previousPaths );
    dependencies.notify(
      `Moved ${ folder.name } to ${ parent?.name ?? 'Vault root' }`,
      'success'
    );

    return true;
  }

  function deleteFolder( id: string ): void {
    if ( !canEditVault.value ) {
      return;
    }
    const folder = vaultState.folders.find( ( candidate ) => candidate.id === id );
    if ( !folder ) {
      return;
    }
    if ( dependencies.folderContainsAssets( id ) ) {
      dependencies.notify(
        'Move contained assets before removing this folder',
        'warning'
      );

      return;
    }
    const affectedFolders = new Set([ id, ...descendantFolderIds( vaultState, id ) ]);
    const children = vaultState.folders.filter(
      ( candidate ) => candidate.parentId === id
    );
    const destinationFolders = vaultState.folders.filter(
      ( candidate ) => candidate.id !== id
        && candidate.parentId === folder.parentId
    );
    const destinationNotes = vaultState.notes.filter(
      ( note ) => note.folderId === folder.parentId
    );
    const folderCollision = children.some( ( child ) => (
      destinationFolders.some(
        ( candidate ) => folderNameKey( candidate.name ) === folderNameKey( child.name )
      ) || destinationNotes.some(
        ( note ) => folderConflictsWithNote( child.name, note )
      )
    ) );
    const noteCollision = vaultState.notes
      .filter( ( note ) => note.folderId === id )
      .some( ( note ) => (
        destinationNotes.some(
          ( candidate ) => noteStemKey( candidate ) === noteStemKey( note )
        ) || destinationFolders.some(
          ( candidate ) => folderConflictsWithNote( candidate.name, note )
        )
      ) );
    if ( folderCollision || noteCollision ) {
      dependencies.notify(
        'Move or rename conflicting items before removing this folder',
        'warning'
      );

      return;
    }
    const previousPaths = dependencies.noteLinkPaths();
    for ( const child of children ) {
      child.parentId = folder.parentId;
    }
    for ( const note of vaultState.notes ) {
      if ( !note.folderId || !affectedFolders.has( note.folderId ) ) {
        continue;
      }
      dependencies.rememberNoteOriginalPath( note );
      if ( note.folderId === id ) {
        note.folderId = folder.parentId;
      }
    }
    vaultState.folders.splice( vaultState.folders.indexOf( folder ), 1 );
    preserveRelocatedNoteLinks( previousPaths );
    if ( vaultState.selectedFolderId === id ) {
      vaultState.selectedFolderId = 'all';
    }
    dependencies.notify(
      'Folder removed; its contents moved up one level',
      'neutral'
    );
  }

  function createFromTemplate(
    templateId: string,
    requestedTitle?: string
  ): Note | undefined {
    const template = vaultState.templates.find(
      ( candidate ) => candidate.id === templateId
    );
    if ( !template ) {
      return undefined;
    }
    const now = new Date();
    const date = new Intl.DateTimeFormat( 'en', {
      month: 'long',
      day: 'numeric',
      year: 'numeric'
    }).format( now );
    const time = new Intl.DateTimeFormat( 'en', {
      hour: 'numeric',
      minute: '2-digit'
    }).format( now );
    const title = requestedTitle?.trim() || replaceTemplateTokens(
      template.titlePattern,
      { date, time, title: template.name }
    );
    const uniqueTitle = uniqueNoteTitle( vaultState, title || template.name );
    const content = replaceTemplateTokens(
      template.content,
      { date, time, title: uniqueTitle }
    );

    return createNote( dependencies.currentFolderId(), uniqueTitle, content );
  }

  function saveTemplate(
    template: Partial<NoteTemplate> & Pick<NoteTemplate, 'name' | 'content'>
  ): NoteTemplate | undefined {
    if ( !canEditVault.value ) {
      return undefined;
    }
    const existing = template.id
      ? vaultState.templates.find( ( candidate ) => candidate.id === template.id )
      : undefined;
    if ( existing ) {
      Object.assign( existing, template );

      return existing;
    }
    const created: NoteTemplate = {
      id: createId( 'template' ),
      name: template.name.trim() || 'Untitled template',
      description: template.description?.trim() || 'A custom note structure.',
      titlePattern: template.titlePattern?.trim() || 'Untitled note',
      content: template.content,
      glyph: template.glyph || 'file-text',
      createdAt: Date.now()
    };
    vaultState.templates.push( created );

    return created;
  }

  function saveSnippet(
    snippet: Partial<CssSnippet> & Pick<CssSnippet, 'name' | 'css'>
  ): CssSnippet | undefined {
    if ( !canEditVault.value ) {
      return undefined;
    }
    const existing = snippet.id
      ? vaultState.snippets.find( ( candidate ) => candidate.id === snippet.id )
      : undefined;
    if ( existing ) {
      Object.assign( existing, snippet );

      return existing;
    }
    const created: CssSnippet = {
      id: createId( 'snippet' ),
      name: snippet.name.trim() || 'Untitled snippet',
      description: snippet.description?.trim() || 'A custom interface style.',
      css: snippet.css,
      enabled: snippet.enabled ?? true,
      createdAt: Date.now()
    };
    vaultState.snippets.push( created );

    return created;
  }

  function deleteSnippet( id: string ): void {
    if ( !canEditVault.value ) {
      return;
    }
    const index = vaultState.snippets.findIndex(
      ( snippet ) => snippet.id === id
    );
    if ( index >= 0 ) {
      vaultState.snippets.splice( index, 1 );
    }
  }

  return {
    createFolder,
    createFromTemplate,
    createLinkedNote,
    createNote,
    deleteFolder,
    deleteSnippet,
    moveFolder,
    moveNoteToFolder,
    renameFolder,
    saveSnippet,
    saveTemplate,
    togglePinned,
    updateNote
  };
}
