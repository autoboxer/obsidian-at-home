import {
  relocateWorkspaceAttachment,
  relocateWorkspaceImage
} from '../services/native';
import type {
  VaultAttachmentFile,
  VaultImageFile,
  WorkspaceAttachmentNoteUpdate,
  WorkspaceEmbedAttachmentResult,
  WorkspaceEmbedImageResult,
  WorkspaceExternalAssetDiscardResult,
  WorkspaceImageNoteUpdate,
  WorkspaceRelocateAttachmentResult,
  WorkspaceRelocateImageResult,
  WorkspaceSaveResult
} from '../types';
import {
  isSafeVaultAttachmentFileName,
  isSafeVaultImageFileName,
  rewriteVaultAttachmentReferences,
  rewriteVaultImageReferences,
  upsertVaultAttachmentFile,
  upsertVaultImageFile
} from './vaultAssets';
import { createId } from './vaultModel';
import { errorMessage, isRevisionConflict } from './vaultPersistence';
import { canEditVault, uiState, vaultSession, vaultState, type ToastTone } from './vaultState';

interface VaultAssetOperationsDependencies {
  applyVaultMutation: ( mutation: () => void ) => void;
  applyWorkspaceSaveResult: ( result: WorkspaceSaveResult ) => void;
  flushVault: () => Promise<boolean>;
  folderPath: ( id: string | null ) => string;
  notify: ( message: string, tone: ToastTone ) => void;
  runExclusiveVaultDataOperation: <T>( fallback: T, operation: () => Promise<T> ) => Promise<T>;
}

export function createVaultAssetOperations(
  dependencies: VaultAssetOperationsDependencies
) {
  const {
    applyVaultMutation,
    applyWorkspaceSaveResult,
    flushVault,
    folderPath,
    notify,
    runExclusiveVaultDataOperation
  } = dependencies;

  async function renameVaultImage(
    image: VaultImageFile,
    fileName: string
  ): Promise<boolean> {
    const parent = image.relativePath.split( '/' ).slice( 0, -1 ).join( '/' );

    return relocateVaultImage( image, parent, fileName );
  }

  async function moveVaultImageToFolder(
    image: VaultImageFile,
    folderId: string | null
  ): Promise<boolean> {
    const fileName = image.relativePath.split( '/' ).at( -1 ) || 'Image.png';

    return relocateVaultImage( image, folderPath( folderId ), fileName );
  }

  async function relocateVaultImage(
    image: VaultImageFile,
    targetFolderPath: string,
    requestedFileName: string
  ): Promise<boolean> {
    const fileName = requestedFileName.trim();
    if ( !isSafeVaultImageFileName( fileName ) ) {
      notify( 'Enter a safe image file name with a supported extension', 'warning' );

      return false;
    }
    const targetRelativePath = targetFolderPath
      ? `${ targetFolderPath }/${ fileName }`
      : fileName;
    if ( targetRelativePath === image.relativePath ) {
      return false;
    }
    const targetKey = targetRelativePath.toLocaleLowerCase();
    if ( vaultState.imageFiles.some( ( candidate ) =>
      candidate.relativePath.toLocaleLowerCase() === targetKey
      && candidate.relativePath.toLocaleLowerCase() !== image.relativePath.toLocaleLowerCase()
    ) ) {
      notify( 'An image with that name already exists there', 'warning' );

      return false;
    }
    if ( vaultSession.backend !== 'native' || !vaultSession.path ) {
      notify( 'Image files can be reorganized in the desktop app', 'warning' );

      return false;
    }

    return runExclusiveVaultDataOperation( false, async () => {
      if ( !( await flushVault() ) ) {
        return false;
      }
      const currentImage = vaultState.imageFiles.find( ( candidate ) =>
        ( image.assetId && candidate.assetId === image.assetId )
        || candidate.relativePath.toLocaleLowerCase() === image.relativePath.toLocaleLowerCase()
      );
      if ( !currentImage ) {
        notify( 'That image can no longer be moved', 'warning' );

        return false;
      }

      const assetId = currentImage.assetId || createId( 'image' );
      const noteUpdates = vaultState.notes.flatMap( ( note ): WorkspaceImageNoteUpdate[] => {
        const content = rewriteVaultImageReferences(
          vaultState,
          note.content,
          note.relativePath,
          currentImage.relativePath,
          targetRelativePath,
          currentImage.assetId,
          assetId
        );

        return content === note.content ? [] : [{
          noteId: note.id,
          relativePath: note.relativePath,
          expectedContent: note.content,
          content
        }];
      });

      try {
        const result = await relocateWorkspaceImage(
          vaultSession.path!,
          currentImage.relativePath,
          targetRelativePath,
          assetId,
          noteUpdates,
          vaultSession.revision
        );
        applyRelocatedImageResult( result, noteUpdates );
        uiState.imageRefreshToken += 1;
        notify(
          targetFolderPath
            ? `Moved image to ${ targetFolderPath }`
            : 'Moved image to Vault root',
          'success'
        );

        return true;
      } catch ( error ) {
        const message = errorMessage( error, 'The image could not be moved.' );
        vaultSession.error = message;
        vaultSession.conflict = isRevisionConflict( message );
        notify( message, 'warning' );

        return false;
      }
    });
  }

  function applyRelocatedImageResult(
    result: WorkspaceRelocateImageResult,
    noteUpdates: WorkspaceImageNoteUpdate[]
  ): void {
    const updatesById = new Map( noteUpdates.map( ( update ) => [ update.noteId, update.content ]) );
    applyVaultMutation( () => {
      for ( const note of vaultState.notes ) {
        const content = updatesById.get( note.id );
        if ( content !== undefined ) {
          note.content = content;
          note.updatedAt = Date.now();
        }
      }
      const oldPathKey = result.previousRelativePath.toLocaleLowerCase();
      const oldFileIndex = vaultState.imageFiles.findIndex( ( candidate ) =>
        candidate.assetId === result.image.id
        || candidate.relativePath.toLocaleLowerCase() === oldPathKey
      );
      if ( oldFileIndex >= 0 ) {
        vaultState.imageFiles.splice( oldFileIndex, 1 );
      }
      const oldEmbeddedIndex = vaultState.embeddedImages.findIndex( ( candidate ) =>
        candidate.id === result.image.id
        || candidate.relativePath.toLocaleLowerCase() === oldPathKey
      );
      if ( oldEmbeddedIndex >= 0 ) {
        vaultState.embeddedImages.splice( oldEmbeddedIndex, 1 );
      }
      vaultState.embeddedImages.push( result.image );
      upsertWorkspaceImageFile({
        assetId: result.image.id,
        relativePath: result.image.relativePath,
        mediaType: result.image.mediaType
      });
    });
    applyWorkspaceSaveResult( result );
  }

  async function renameVaultAttachment(
    attachment: Pick<VaultAttachmentFile, 'assetId' | 'relativePath'>,
    fileName: string
  ): Promise<boolean> {
    return relocateVaultAttachment( attachment, {
      fileName: fileName.trim(),
      kind: 'rename'
    });
  }

  async function moveVaultAttachmentToFolder(
    attachment: Pick<VaultAttachmentFile, 'assetId' | 'relativePath'>,
    folderId: string | null
  ): Promise<boolean> {
    return relocateVaultAttachment( attachment, {
      kind: 'move',
      targetFolderId: folderId
    });
  }

  type VaultAttachmentRelocation = {
    fileName: string;
    kind: 'rename';
  } | {
    kind: 'move';
    targetFolderId: string | null;
  };

  async function relocateVaultAttachment(
    attachment: Pick<VaultAttachmentFile, 'assetId' | 'relativePath'>,
    relocation: VaultAttachmentRelocation
  ): Promise<boolean> {
    if (
      relocation.kind === 'rename'
      && !isSafeVaultAttachmentFileName( relocation.fileName )
    ) {
      notify( 'Enter a safe non-Markdown, non-image file name', 'warning' );

      return false;
    }
    if ( vaultSession.backend !== 'native' || !vaultSession.path ) {
      notify( 'Attachment files can be reorganized in the desktop app', 'warning' );

      return false;
    }

    return runExclusiveVaultDataOperation( false, async () => {
      if ( !( await flushVault() ) ) {
        return false;
      }
      const currentAttachment = resolveCurrentVaultAttachment( attachment );
      if ( !currentAttachment ) {
        notify( 'That attachment could not be uniquely found in the vault', 'warning' );

        return false;
      }
      const currentFileName = currentAttachment.relativePath.split( '/' ).at( -1 )
        || 'Attachment';
      const fileName = relocation.kind === 'rename'
        ? relocation.fileName
        : currentFileName;
      const targetFolder = relocation.kind === 'move' && relocation.targetFolderId
        ? vaultState.folders.find( ( folder ) => folder.id === relocation.targetFolderId )
        : null;
      if ( relocation.kind === 'move' && relocation.targetFolderId && !targetFolder ) {
        notify( 'That destination folder could not be found', 'warning' );

        return false;
      }
      const targetFolderPath = relocation.kind === 'rename'
        ? currentAttachment.relativePath.split( '/' ).slice( 0, -1 ).join( '/' )
        : folderPath( targetFolder?.id ?? null );
      const targetRelativePath = targetFolderPath
        ? `${ targetFolderPath }/${ fileName }`
        : fileName;
      if ( targetRelativePath === currentAttachment.relativePath ) {
        return false;
      }
      const targetKey = targetRelativePath.toLocaleLowerCase();
      if ( vaultState.attachmentFiles.some( ( candidate ) =>
        candidate.relativePath.toLocaleLowerCase() === targetKey
        && candidate.relativePath.toLocaleLowerCase()
          !== currentAttachment.relativePath.toLocaleLowerCase()
      ) ) {
        notify( 'A file with that name already exists there', 'warning' );

        return false;
      }

      const assetId = currentAttachment.assetId || createId( 'attachment' );
      const noteUpdates = vaultState.notes.flatMap( ( note ): WorkspaceAttachmentNoteUpdate[] => {
        const content = rewriteVaultAttachmentReferences(
          vaultState,
          note.content,
          note.relativePath,
          currentAttachment.relativePath,
          targetRelativePath,
          currentAttachment.assetId,
          assetId
        );

        return content === note.content ? [] : [{
          noteId: note.id,
          relativePath: note.relativePath,
          expectedContent: note.content,
          content
        }];
      });

      try {
        const result = await relocateWorkspaceAttachment(
          vaultSession.path!,
          currentAttachment.relativePath,
          targetRelativePath,
          assetId,
          noteUpdates,
          vaultSession.revision
        );
        applyRelocatedAttachmentResult( result, noteUpdates );
        uiState.attachmentRefreshToken += 1;
        notify(
          relocation.kind === 'rename'
            ? `Renamed attachment to ${ fileName }`
            : targetFolderPath
              ? `Moved attachment to ${ targetFolderPath }`
              : 'Moved attachment to Vault root',
          'success'
        );

        return true;
      } catch ( error ) {
        const message = errorMessage( error, 'The attachment could not be renamed or moved.' );
        vaultSession.error = message;
        vaultSession.conflict = isRevisionConflict( message );
        notify( message, 'warning' );

        return false;
      }
    });
  }

  function resolveCurrentVaultAttachment(
    attachment: Pick<VaultAttachmentFile, 'assetId' | 'relativePath'>
  ): VaultAttachmentFile | undefined {
    const matches = attachment.assetId
      ? vaultState.attachmentFiles.filter( ( candidate ) =>
        candidate.assetId === attachment.assetId
      )
      : vaultState.attachmentFiles.filter( ( candidate ) =>
        candidate.relativePath.toLocaleLowerCase()
          === attachment.relativePath.toLocaleLowerCase()
      );

    return matches.length === 1 ? matches[ 0 ] : undefined;
  }

  function applyRelocatedAttachmentResult(
    result: WorkspaceRelocateAttachmentResult,
    noteUpdates: WorkspaceAttachmentNoteUpdate[]
  ): void {
    const updatesById = new Map( noteUpdates.map( ( update ) => [ update.noteId, update.content ]) );
    applyVaultMutation( () => {
      for ( const note of vaultState.notes ) {
        const content = updatesById.get( note.id );
        if ( content !== undefined ) {
          note.content = content;
          note.updatedAt = Date.now();
        }
      }
      const oldPathKey = result.previousRelativePath.toLocaleLowerCase();
      const oldFileIndex = vaultState.attachmentFiles.findIndex( ( candidate ) =>
        candidate.assetId === result.attachment.id
        || candidate.relativePath.toLocaleLowerCase() === oldPathKey
      );
      if ( oldFileIndex >= 0 ) {
        vaultState.attachmentFiles.splice( oldFileIndex, 1 );
      }
      const oldEmbeddedIndex = vaultState.embeddedAttachments.findIndex( ( candidate ) =>
        candidate.id === result.attachment.id
        || candidate.relativePath.toLocaleLowerCase() === oldPathKey
      );
      if ( oldEmbeddedIndex >= 0 ) {
        vaultState.embeddedAttachments.splice( oldEmbeddedIndex, 1 );
      }
      vaultState.embeddedAttachments.push( result.attachment );
      upsertWorkspaceAttachmentFile({
        assetId: result.attachment.id,
        relativePath: result.attachment.relativePath,
        mediaType: result.attachment.mediaType,
        byteLength: result.attachment.byteLength,
        openingDisabled: result.attachment.openingDisabled
      });
    });
    applyWorkspaceSaveResult( result );
  }

  function applyEmbeddedImageResult( result: WorkspaceEmbedImageResult ): void {
    if ( !canEditVault.value ) {
      return;
    }
    applyVaultMutation( () => {
      const index = vaultState.embeddedImages.findIndex( ( image ) => image.id === result.image.id );
      if ( index >= 0 ) {
        vaultState.embeddedImages.splice( index, 1, result.image );
      } else {
        vaultState.embeddedImages.push( result.image );
      }
      upsertWorkspaceImageFile({
        assetId: result.image.id,
        relativePath: result.image.relativePath,
        mediaType: result.image.mediaType
      });
    });
    applyWorkspaceSaveResult( result );
    uiState.imageRefreshToken += 1;
  }

  function applyEmbeddedAttachmentResult(
    result: WorkspaceEmbedAttachmentResult
  ): void {
    if ( !canEditVault.value ) {
      return;
    }
    applyVaultMutation( () => {
      const index = vaultState.embeddedAttachments.findIndex(
        ( attachment ) => attachment.id === result.attachment.id
      );
      if ( index >= 0 ) {
        vaultState.embeddedAttachments.splice( index, 1, result.attachment );
      } else {
        vaultState.embeddedAttachments.push( result.attachment );
      }
      upsertWorkspaceAttachmentFile({
        assetId: result.attachment.id,
        relativePath: result.attachment.relativePath,
        mediaType: result.attachment.mediaType,
        byteLength: result.attachment.byteLength,
        openingDisabled: result.attachment.openingDisabled
      });
    });
    applyWorkspaceSaveResult( result );
    uiState.attachmentRefreshToken += 1;
  }

  function applyExternalAssetDiscardResult(
    result: WorkspaceExternalAssetDiscardResult
  ): void {
    applyWorkspaceSaveResult( result );
  }

  function applyWorkspaceImageFiles( images: VaultImageFile[]): void {
    applyVaultMutation( () => {
      for ( const image of images ) {
        upsertWorkspaceImageFile( image );
      }
    });
  }

  function applyWorkspaceAttachmentFiles( attachments: VaultAttachmentFile[]): void {
    applyVaultMutation( () => {
      for ( const attachment of attachments ) {
        upsertWorkspaceAttachmentFile( attachment );
      }
    });
  }

  function upsertWorkspaceImageFile( image: VaultImageFile ): void {
    upsertVaultImageFile( vaultState, image, () => createId( 'folder' ) );
  }

  function upsertWorkspaceAttachmentFile( attachment: VaultAttachmentFile ): void {
    upsertVaultAttachmentFile( vaultState, attachment, () => createId( 'folder' ) );
  }

  return {
    applyEmbeddedAttachmentResult,
    applyEmbeddedImageResult,
    applyExternalAssetDiscardResult,
    applyWorkspaceAttachmentFiles,
    applyWorkspaceImageFiles,
    moveVaultAttachmentToFolder,
    moveVaultImageToFolder,
    renameVaultAttachment,
    renameVaultImage
  };
}
