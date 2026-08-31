import { resolveWikiLink } from '../lib';
import type { CssSnippet, Folder, Note, NoteTemplate } from '../types';
import {
  createId,
  descendantFolderIds,
  folderConflictsWithNote,
  folderNameKey,
  noteFileNameKeys,
  noteStemKey,
  replaceTemplateTokens,
  uniqueNoteTitle
} from './vaultModel';
import { isSmartFolderSelection } from './vaultNavigation';
import {
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
  function createNote(
    folderId?: string | null,
    title = 'Untitled note',
    content?: string
  ): Note {
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
    vaultState.notes.unshift( note );
    dependencies.selectNote( note.id );
    vaultState.selectedFolderId = 'all';
    uiState.tool = 'notes';
    uiState.notesView = 'editor';
    uiState.noteFilter = '';
    dependencies.notify( 'New note created', 'success' );

    return note;
  }

  function createLinkedNote( target: string ): Note {
    const cleanTarget = target.replace( /\.md$/i, '' ).split( '/' ).pop()?.trim()
      || 'Untitled note';
    const existing = resolveWikiLink(
      cleanTarget,
      vaultState.notes,
      dependencies.activeNote()
    );
    if ( existing ) {
      dependencies.selectNote( existing.id );

      return existing;
    }

    return createNote(
      dependencies.activeNote()?.folderId ?? dependencies.currentFolderId(),
      cleanTarget
    );
  }

  function updateNote( id: string, patch: NotePatch ): void {
    const note = vaultState.notes.find( ( candidate ) => candidate.id === id );
    if ( !note ) {
      return;
    }
    const locationChanged = (
      patch.title !== undefined && patch.title !== note.title
    ) || (
      patch.folderId !== undefined && patch.folderId !== note.folderId
    );
    if ( locationChanged ) {
      dependencies.rememberNoteOriginalPath( note );
    }
    if ( patch.title !== undefined ) {
      note.title = patch.title;
    }
    if ( patch.content !== undefined ) {
      note.content = patch.content;
    }
    if ( patch.folderId !== undefined ) {
      note.folderId = patch.folderId;
    }
    if ( patch.tags !== undefined ) {
      note.tags = patch.tags;
    }
    if ( patch.pinned !== undefined ) {
      note.pinned = patch.pinned;
    }
    note.updatedAt = Date.now();
  }

  async function moveNoteToFolder(
    noteId: string,
    folderId: string | null
  ): Promise<boolean> {
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

    updateNote( noteId, { folderId });
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
    for ( const note of vaultState.notes ) {
      if ( note.folderId && affectedFolders.has( note.folderId ) ) {
        dependencies.rememberNoteOriginalPath( note );
      }
    }
    folder.name = cleanName;
  }

  function moveFolder( folderId: string, parentId: string | null ): boolean {
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

    for ( const note of vaultState.notes ) {
      if ( note.folderId && affectedFolders.has( note.folderId ) ) {
        dependencies.rememberNoteOriginalPath( note );
      }
    }
    folder.parentId = parentId;
    dependencies.notify(
      `Moved ${ folder.name } to ${ parent?.name ?? 'Vault root' }`,
      'success'
    );

    return true;
  }

  function deleteFolder( id: string ): void {
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
  ): NoteTemplate {
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
  ): CssSnippet {
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
