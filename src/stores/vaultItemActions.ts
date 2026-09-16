import { revealItemInDir } from '@tauri-apps/plugin-opener';
import {
  markdownAttachmentIsArchive,
  markdownAttachmentIsExecutable
} from '../lib/markdownAttachments';
import {
  locateWorkspaceVaultItem,
  openWorkspaceAttachment,
  saveWorkspaceAttachmentCopy,
  showWorkspaceVaultItemInFolder,
  type WorkspaceVaultItemKind
} from '../services/native';
import type { VaultAttachmentFile } from '../types';
import { errorMessage } from './vaultPersistence';
import {
  uiState,
  vaultSession,
  vaultState,
  vaultTreeRevealTarget,
  type ToastAction,
  type ToastTone
} from './vaultState';

const ARCHIVE_COPY_DIRECTORY_KEY = 'obsidian-at-home.archive-copy-directory.v1';

export interface VaultItemLocator {
  assetId?: string;
  itemId?: string;
  kind: WorkspaceVaultItemKind;
  relativePath: string;
}

interface VaultItemActionsDependencies {
  flushVault: () => Promise<boolean>;
  folderPath: ( id: string | null ) => string;
  notify: ( message: string, tone: ToastTone, action?: ToastAction ) => void;
}

export function createVaultItemActions(
  dependencies: VaultItemActionsDependencies
) {
  const { flushVault, folderPath, notify } = dependencies;
  let vaultTreeRevealOperation = 0;

  async function activateVaultAttachment(
    attachment: Pick<
      VaultAttachmentFile,
      'assetId' | 'mediaType' | 'openingDisabled' | 'relativePath'
    >
  ): Promise<void> {
    if ( vaultSession.backend !== 'native' || !vaultSession.path ) {
      notify( 'Attachment files can be opened in the desktop app', 'warning' );

      return;
    }
    if ( markdownAttachmentIsExecutable(
      attachment.relativePath,
      attachment.openingDisabled
    ) ) {
      notify( 'Opening executable or installer attachments is not supported', 'warning' );

      return;
    }
    try {
      if ( markdownAttachmentIsArchive( attachment.relativePath, attachment.mediaType ) ) {
        let preferredDirectory: string | undefined;
        try {
          preferredDirectory = window.localStorage.getItem( ARCHIVE_COPY_DIRECTORY_KEY )
            || undefined;
        } catch {
          // A Downloads default remains available when browser storage is unavailable.
        }
        const result = await saveWorkspaceAttachmentCopy(
          vaultSession.path,
          attachment.relativePath,
          attachment.assetId,
          preferredDirectory
        );
        if ( !result ) {
          return;
        }
        const directory = parentSystemPath( result.path );
        if ( directory ) {
          try {
            window.localStorage.setItem( ARCHIVE_COPY_DIRECTORY_KEY, directory );
          } catch {
            // Remembering the folder is helpful but not required for a successful copy.
          }
        }
        notify( 'Saved the archive outside the vault', 'success', {
          label: 'Reveal archive',
          run: () => {
            void revealItemInDir( result.path ).catch( ( error ) =>
              notify( errorMessage( error, 'The saved archive could not be revealed.' ), 'warning' )
            );
          }
        });

        return;
      }
      await openWorkspaceAttachment(
        vaultSession.path,
        attachment.relativePath,
        attachment.assetId
      );
    } catch ( error ) {
      notify( errorMessage( error, 'The attachment could not be opened.' ), 'warning' );
    }
  }

  async function locateVaultItem( locator: VaultItemLocator ): Promise<string | undefined> {
    if ( vaultSession.backend !== 'native' || !vaultSession.path ) {
      return locator.relativePath;
    }
    const sourcePath = vaultSession.path;
    let relativePath = locator.relativePath;
    if ( locator.kind === 'note' || locator.kind === 'folder' ) {
      if ( !( await flushVault() ) ) {
        return undefined;
      }
      if ( vaultSession.backend !== 'native' || vaultSession.path !== sourcePath ) {
        return undefined;
      }
      if ( locator.kind === 'note' ) {
        const note = locator.itemId
          ? vaultState.notes.find( ( candidate ) => candidate.id === locator.itemId )
          : vaultState.notes.find( ( candidate ) => candidate.relativePath === locator.relativePath );
        relativePath = note?.relativePath ?? '';
      } else {
        const folder = locator.itemId
          ? vaultState.folders.find( ( candidate ) => candidate.id === locator.itemId )
          : vaultState.folders.find( ( candidate ) => folderPath( candidate.id ) === locator.relativePath );
        relativePath = folder ? folderPath( folder.id ) : '';
      }
      if ( !relativePath ) {
        notify( 'The vault item is no longer available.', 'warning' );

        return undefined;
      }
    }
    try {
      return await locateWorkspaceVaultItem(
        sourcePath,
        locator.kind,
        relativePath,
        locator.assetId
      );
    } catch ( error ) {
      notify( errorMessage( error, 'The vault item could not be located.' ), 'warning' );

      return undefined;
    }
  }

  async function revealVaultItemInTree( locator: VaultItemLocator ): Promise<boolean> {
    const operation = ++vaultTreeRevealOperation;
    const sourceVaultKey = currentVaultTreeKey();
    const locatedPath = await locateVaultItem( locator );
    if (
      !locatedPath
      || operation !== vaultTreeRevealOperation
      || sourceVaultKey !== currentVaultTreeKey()
    ) {
      return false;
    }

    const target = currentVaultTreeItem( locator, locatedPath );
    if ( !target ) {
      notify( "The vault item could not be found in the app's file tree.", 'warning' );

      return false;
    }

    uiState.commandOpen = false;
    uiState.tool = 'notes';
    uiState.notesView = 'editor';
    uiState.explorerOpen = true;
    uiState.noteFilter = '';
    vaultState.selectedFolderId = 'all';
    vaultTreeRevealTarget.assetId = target.assetId ?? null;
    vaultTreeRevealTarget.kind = locator.kind;
    vaultTreeRevealTarget.relativePath = target.relativePath;
    vaultTreeRevealTarget.vaultKey = sourceVaultKey;
    vaultTreeRevealTarget.requestId += 1;

    return true;
  }

  function vaultTreeItemIsRevealed( locator: VaultItemLocator ): boolean {
    if (
      !vaultTreeRevealTarget.requestId
      || vaultTreeRevealTarget.vaultKey !== currentVaultTreeKey()
      || vaultTreeRevealTarget.kind !== locator.kind
    ) {
      return false;
    }
    if ( vaultTreeRevealTarget.assetId ) {
      return locator.assetId === vaultTreeRevealTarget.assetId;
    }

    return locator.relativePath === vaultTreeRevealTarget.relativePath;
  }

  function vaultTreeRevealIncludesFolder( relativePath: string ): boolean {
    if (
      !vaultTreeRevealTarget.requestId
      || vaultTreeRevealTarget.vaultKey !== currentVaultTreeKey()
    ) {
      return false;
    }

    return vaultTreeRevealTarget.relativePath === relativePath
      || vaultTreeRevealTarget.relativePath.startsWith( `${ relativePath }/` );
  }

  async function showVaultItemInFolder( locator: VaultItemLocator ): Promise<void> {
    if ( vaultSession.backend !== 'native' || !vaultSession.path ) {
      notify( 'Showing vault files in a system folder is available in the desktop app', 'warning' );

      return;
    }
    const sourcePath = vaultSession.path;
    const relativePath = await locateVaultItem( locator );
    if (
      !relativePath
      || vaultSession.backend !== 'native'
      || vaultSession.path !== sourcePath
    ) {
      return;
    }
    try {
      await showWorkspaceVaultItemInFolder(
        sourcePath,
        locator.kind,
        relativePath,
        locator.assetId
      );
    } catch ( error ) {
      notify( errorMessage( error, 'The vault item could not be shown in its folder.' ), 'warning' );
    }
  }

  function currentVaultTreeKey(): string {
    return `${ vaultSession.backend }\u0000${ vaultSession.path ?? vaultState.name }`;
  }

  function currentVaultTreeItem(
    locator: VaultItemLocator,
    locatedPath: string
  ): { assetId?: string; relativePath: string } | undefined {
    if ( locator.kind === 'attachment' ) {
      const attachment = locator.assetId
        ? vaultState.attachmentFiles.find( ( candidate ) => candidate.assetId === locator.assetId )
        : vaultState.attachmentFiles.find( ( candidate ) => candidate.relativePath === locatedPath );

      return attachment
        ? {
          ...( attachment.assetId ? { assetId: attachment.assetId } : {}),
          relativePath: attachment.relativePath
        }
        : undefined;
    }
    if ( locator.kind === 'image' ) {
      const image = locator.assetId
        ? vaultState.imageFiles.find( ( candidate ) => candidate.assetId === locator.assetId )
        : vaultState.imageFiles.find( ( candidate ) => candidate.relativePath === locatedPath );

      return image
        ? {
          ...( image.assetId ? { assetId: image.assetId } : {}),
          relativePath: image.relativePath
        }
        : undefined;
    }
    if ( locator.kind === 'note' ) {
      const note = vaultState.notes.find( ( candidate ) => candidate.relativePath === locatedPath );

      return note ? { relativePath: note.relativePath } : undefined;
    }
    const folder = vaultState.folders.find( ( candidate ) => folderPath( candidate.id ) === locatedPath );

    return folder ? { relativePath: locatedPath } : undefined;
  }

  function parentSystemPath( path: string ): string | undefined {
    const index = Math.max( path.lastIndexOf( '/' ), path.lastIndexOf( '\\' ) );

    return index > 0 ? path.slice( 0, index ) : undefined;
  }

  return {
    activateVaultAttachment,
    locateVaultItem,
    revealVaultItemInTree,
    showVaultItemInFolder,
    vaultTreeItemIsRevealed,
    vaultTreeRevealIncludesFolder
  };
}
