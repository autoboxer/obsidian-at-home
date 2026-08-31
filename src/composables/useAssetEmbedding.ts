import {
  nextTick,
  onBeforeUnmount,
  ref,
  watch,
  type Ref
} from 'vue';
import {
  imageAltFromPath,
  isSupportedImageFileName,
  pastedImageFileName
} from '../lib/imageEmbeds';
import {
  formatMarkdownImage,
  relativeImageDestination
} from '../lib/markdownImages';
import {
  attachmentLabelFromPath,
  formatMarkdownAttachment,
  relativeAttachmentDestination
} from '../lib/markdownAttachments';
import {
  discardWorkspaceExternalAsset,
  embedWorkspaceAttachmentFile,
  embedWorkspaceExternalAttachment,
  embedWorkspaceExternalImage,
  embedWorkspaceImageBytes,
  embedWorkspaceImageFile,
  embedWorkspaceVaultAttachment,
  embedWorkspaceVaultImage,
  isTauri,
  pickAttachmentFile,
  pickImageFile,
  readClipboardImagePng
} from '../services/native';
import {
  activeNote,
  activateVaultAttachment,
  applyEmbeddedAttachmentResult,
  applyEmbeddedImageResult,
  applyExternalAssetDiscardResult,
  flushVault,
  notify,
  revealVaultItemInTree,
  showVaultItemInFolder,
  vaultAttachmentInsertRequest,
  vaultImageInsertRequest,
  vaultSession,
  vaultState
} from '../stores/vault';
import type {
  AssetInsertionCapture,
  AttachmentInsertionCapture,
  ImageInsertionCapture,
  Note,
  WorkspaceEmbedAttachmentResult,
  WorkspaceEmbedImageResult,
  WorkspaceExternalAssetDiscardResult
} from '../types';

export interface AssetEmbeddingEditor {
  cancelAttachmentInsertion: ( capture: AttachmentInsertionCapture ) => void;
  cancelImageInsertion: ( capture: ImageInsertionCapture ) => void;
  captureAttachmentInsertion: () => AttachmentInsertionCapture | undefined;
  captureImageInsertion: () => ImageInsertionCapture | undefined;
  insertEmbeddedAttachment: (
    capture: AttachmentInsertionCapture,
    markdownAttachment: string
  ) => boolean;
  insertEmbeddedImage: (
    capture: ImageInsertionCapture,
    markdownImage: string
  ) => boolean;
}

interface AssetEmbedContext {
  note: Note;
  noteRelativePath: string;
  vaultPath: string;
}

interface StoreAndInsertOptions {
  announce?: boolean;
  cleanupFailedInsertion?: (
    context: AssetEmbedContext,
    assetId: string,
    relativePath: string,
    expectedRevision: number
  ) => Promise<WorkspaceExternalAssetDiscardResult>;
  markdownPrefix?: string;
}

export function useAssetEmbedding<T extends AssetEmbeddingEditor>(
  sourceEditor: Ref<T | undefined>
) {
  const nativeAvailable = isTauri();
  const attachmentEmbedBusy = ref( false );
  const imageEmbedBusy = ref( false );
  let externalFileDropAbort: AbortController | undefined;

  async function cleanupFailedExternalInsertion(
    context: AssetEmbedContext,
    assetId: string,
    relativePath: string,
    expectedRevision: number
  ): Promise<WorkspaceExternalAssetDiscardResult> {
    return discardWorkspaceExternalAsset(
      context.vaultPath,
      assetId,
      relativePath,
      expectedRevision
    );
  }

  async function prepareExternalFileFinish(
    context: AssetEmbedContext,
    signal: AbortSignal
  ): Promise<number> {
    if ( signal.aborted ) {
      throw new DOMException( 'The dropped-file transfer was cancelled.', 'AbortError' );
    }
    if ( !( await flushVault() ) ) {
      throw new Error(
        vaultSession.error || 'Save the note before finishing the file drop.'
      );
    }
    if (
      signal.aborted
      || vaultSession.path !== context.vaultPath
      || activeNote.value?.id !== context.note.id
      || context.note.relativePath !== context.noteRelativePath
    ) {
      throw new Error( 'The note or vault changed before the file drop could finish.' );
    }

    return vaultSession.revision;
  }

  function embedError( error: unknown, fallback: string ): string {
    if ( typeof error === 'string' && error.trim() ) {
      return error;
    }
    if ( error instanceof Error && error.message.trim() ) {
      return error.message;
    }

    return fallback;
  }

  function assetEmbedContext(
    capture: AssetInsertionCapture
  ): AssetEmbedContext | undefined {
    const note = vaultState.notes.find( ( candidate ) => candidate.id === capture.noteId );
    const vaultPath = vaultSession.backend === 'native' ? vaultSession.path : null;
    if ( !nativeAvailable || !note || !vaultPath || activeNote.value?.id !== note.id ) {
      return undefined;
    }

    return {
      note,
      noteRelativePath: note.relativePath,
      vaultPath
    };
  }

  async function storeAndInsertImage(
    capture: ImageInsertionCapture,
    embed: (
      context: AssetEmbedContext,
      expectedRevision: number
    ) => Promise<WorkspaceEmbedImageResult>,
    options: StoreAndInsertOptions = {}
  ): Promise<boolean> {
    const context = assetEmbedContext( capture );
    if ( !context ) {
      throw new Error( 'Images can be embedded into an open note in a desktop vault.' );
    }
    if ( !( await flushVault() ) ) {
      throw new Error(
        vaultSession.error || 'Save the current note before embedding an image.'
      );
    }
    if (
      vaultSession.path !== context.vaultPath
      || activeNote.value?.id !== context.note.id
    ) {
      throw new Error( 'The note or vault changed before the image could be embedded.' );
    }
    context.noteRelativePath = context.note.relativePath;
    if ( !context.noteRelativePath ) {
      throw new Error(
        'The note does not have a saved file path for the embedded image.'
      );
    }

    const result = await embed( context, vaultSession.revision );
    if (
      vaultSession.path !== context.vaultPath
      || activeNote.value?.id !== context.note.id
    ) {
      await retainOrDiscardFailedImage( context, result, options );
      throw new Error( 'The note or vault changed before the image could be inserted.' );
    }
    const selectedAlt = capture.selectedText.trim();
    const alt = selectedAlt
      && !/[\r\n]/.test( selectedAlt )
      && selectedAlt.length <= 240
      ? selectedAlt
      : imageAltFromPath( result.image.relativePath );
    const markdownImage = `${ options.markdownPrefix ?? '' }${ formatMarkdownImage({
      alt,
      assetId: result.image.id,
      destination: relativeImageDestination(
        context.noteRelativePath,
        result.image.relativePath
      ),
      inTable: capture.inTable
    }) }`;
    const inserted = sourceEditor.value?.insertEmbeddedImage(
      capture,
      markdownImage
    ) ?? false;
    if ( !inserted ) {
      const discarded = await retainOrDiscardFailedImage( context, result, options );
      notify(
        discarded
          ? 'The image reference could not be inserted, so its unused stored copy was removed.'
          : 'The image was saved, but its Markdown reference could not be inserted.',
        'warning'
      );

      return false;
    }
    applyEmbeddedImageResult( result );

    if ( result.warnings.length ) {
      notify( result.warnings[ 0 ]!, 'warning' );
    } else if ( options.announce !== false ) {
      notify( `Embedded ${ imageAltFromPath( result.image.relativePath ) }`, 'success' );
    }

    return true;
  }

  async function retainOrDiscardFailedImage(
    context: AssetEmbedContext,
    result: WorkspaceEmbedImageResult,
    options: StoreAndInsertOptions
  ): Promise<boolean> {
    if ( !options.cleanupFailedInsertion ) {
      if ( vaultSession.path === context.vaultPath ) {
        applyEmbeddedImageResult( result );
      }

      return false;
    }
    try {
      const cleanup = await options.cleanupFailedInsertion(
        context,
        result.image.id,
        result.image.relativePath,
        result.revision
      );
      if ( vaultSession.path === context.vaultPath ) {
        if ( cleanup.discarded ) {
          applyExternalAssetDiscardResult( cleanup );
        } else {
          applyEmbeddedImageResult({
            ...result,
            revision: cleanup.revision,
            savedAt: cleanup.savedAt,
            warnings: cleanup.warnings
          });
        }
      }

      return cleanup.discarded;
    } catch {
      if ( vaultSession.path === context.vaultPath ) {
        applyEmbeddedImageResult( result );
      }

      return false;
    }
  }

  async function embedImageFromFile( capture: ImageInsertionCapture ): Promise<void> {
    if ( imageEmbedBusy.value || attachmentEmbedBusy.value ) {
      sourceEditor.value?.cancelImageInsertion( capture );
      notify( 'Wait for the current image to finish embedding.', 'warning' );

      return;
    }

    imageEmbedBusy.value = true;
    try {
      const context = assetEmbedContext( capture );
      if ( !context ) {
        throw new Error( 'Images can be embedded into an open note in a desktop vault.' );
      }
      if ( !( await flushVault() ) ) {
        throw new Error(
          vaultSession.error || 'Save the current note before embedding an image.'
        );
      }
      const sourcePath = await pickImageFile();
      if ( !sourcePath ) {
        return;
      }
      if (
        vaultSession.path !== context.vaultPath
        || activeNote.value?.id !== context.note.id
      ) {
        throw new Error( 'The note or vault changed before the image could be embedded.' );
      }
      await storeAndInsertImage( capture, ( current, expectedRevision ) =>
        embedWorkspaceImageFile(
          current.vaultPath,
          sourcePath,
          current.noteRelativePath,
          { ...vaultState.imageEmbedSettings },
          expectedRevision
        )
      );
    } catch ( error ) {
      notify( embedError( error, 'The image could not be embedded.' ), 'warning' );
    } finally {
      sourceEditor.value?.cancelImageInsertion( capture );
      imageEmbedBusy.value = false;
    }
  }

  async function embedImageFromClipboard(
    capture: ImageInsertionCapture,
    file?: File
  ): Promise<void> {
    if ( imageEmbedBusy.value || attachmentEmbedBusy.value ) {
      sourceEditor.value?.cancelImageInsertion( capture );
      notify( 'Wait for the current image to finish embedding.', 'warning' );

      return;
    }

    imageEmbedBusy.value = true;
    try {
      const bytes = file
        ? new Uint8Array( await file.arrayBuffer() )
        : await readClipboardImagePng();
      const fileName = file?.name?.trim() || pastedImageFileName();
      await storeAndInsertImage( capture, ( context, expectedRevision ) =>
        embedWorkspaceImageBytes(
          context.vaultPath,
          fileName,
          bytes,
          context.noteRelativePath,
          { ...vaultState.imageEmbedSettings },
          expectedRevision
        )
      );
    } catch ( error ) {
      notify( embedError( error, 'The image could not be embedded.' ), 'warning' );
    } finally {
      sourceEditor.value?.cancelImageInsertion( capture );
      imageEmbedBusy.value = false;
    }
  }

  async function embedImageFromVault(
    capture: ImageInsertionCapture,
    relativePath: string
  ): Promise<void> {
    if ( imageEmbedBusy.value || attachmentEmbedBusy.value ) {
      sourceEditor.value?.cancelImageInsertion( capture );
      notify( 'Wait for the current image to finish embedding.', 'warning' );

      return;
    }

    imageEmbedBusy.value = true;
    try {
      await storeAndInsertImage( capture, ( context, expectedRevision ) =>
        embedWorkspaceVaultImage(
          context.vaultPath,
          relativePath,
          context.noteRelativePath,
          { ...vaultState.imageEmbedSettings },
          expectedRevision
        )
      );
    } catch ( error ) {
      notify( embedError( error, 'The image could not be embedded.' ), 'warning' );
    } finally {
      sourceEditor.value?.cancelImageInsertion( capture );
      imageEmbedBusy.value = false;
    }
  }

  function requestImageFromToolbar(): void {
    const capture = sourceEditor.value?.captureImageInsertion();
    if ( capture ) {
      void embedImageFromFile( capture );
    }
  }

  async function storeAndInsertAttachment(
    capture: AttachmentInsertionCapture,
    embed: (
      context: AssetEmbedContext,
      expectedRevision: number
    ) => Promise<WorkspaceEmbedAttachmentResult>,
    options: StoreAndInsertOptions = {}
  ): Promise<boolean> {
    const context = assetEmbedContext( capture );
    if ( !context ) {
      throw new Error( 'Files can be embedded into an open note in a desktop vault.' );
    }
    if ( !( await flushVault() ) ) {
      throw new Error(
        vaultSession.error || 'Save the current note before embedding a file.'
      );
    }
    if (
      vaultSession.path !== context.vaultPath
      || activeNote.value?.id !== context.note.id
    ) {
      throw new Error( 'The note or vault changed before the file could be embedded.' );
    }
    context.noteRelativePath = context.note.relativePath;
    if ( !context.noteRelativePath ) {
      throw new Error( 'The note does not have a saved file path for the embedded file.' );
    }

    const result = await embed( context, vaultSession.revision );
    if (
      vaultSession.path !== context.vaultPath
      || activeNote.value?.id !== context.note.id
    ) {
      await retainOrDiscardFailedAttachment( context, result, options );
      throw new Error( 'The note or vault changed before the file could be inserted.' );
    }
    const selectedLabel = capture.selectedText.trim();
    const label = selectedLabel
      && !/[\r\n]/.test( selectedLabel )
      && selectedLabel.length <= 240
      ? selectedLabel
      : attachmentLabelFromPath( result.attachment.relativePath );
    const markdownAttachment = `${ options.markdownPrefix ?? '' }${ formatMarkdownAttachment({
      label,
      assetId: result.attachment.id,
      destination: relativeAttachmentDestination(
        context.noteRelativePath,
        result.attachment.relativePath
      ),
      inTable: capture.inTable
    }) }`;
    const inserted = sourceEditor.value?.insertEmbeddedAttachment(
      capture,
      markdownAttachment
    ) ?? false;
    if ( !inserted ) {
      const discarded = await retainOrDiscardFailedAttachment(
        context,
        result,
        options
      );
      notify(
        discarded
          ? 'The file reference could not be inserted, so its unused stored copy was removed.'
          : 'The file was saved, but its Markdown reference could not be inserted.',
        'warning'
      );

      return false;
    }
    applyEmbeddedAttachmentResult( result );

    if ( result.warnings.length ) {
      notify( result.warnings[ 0 ]!, 'warning' );
    } else if ( options.announce !== false ) {
      notify(
        `Embedded ${ attachmentLabelFromPath( result.attachment.relativePath ) }`,
        'success'
      );
    }

    return true;
  }

  async function retainOrDiscardFailedAttachment(
    context: AssetEmbedContext,
    result: WorkspaceEmbedAttachmentResult,
    options: StoreAndInsertOptions
  ): Promise<boolean> {
    if ( !options.cleanupFailedInsertion ) {
      if ( vaultSession.path === context.vaultPath ) {
        applyEmbeddedAttachmentResult( result );
      }

      return false;
    }
    try {
      const cleanup = await options.cleanupFailedInsertion(
        context,
        result.attachment.id,
        result.attachment.relativePath,
        result.revision
      );
      if ( vaultSession.path === context.vaultPath ) {
        if ( cleanup.discarded ) {
          applyExternalAssetDiscardResult( cleanup );
        } else {
          applyEmbeddedAttachmentResult({
            ...result,
            revision: cleanup.revision,
            savedAt: cleanup.savedAt,
            warnings: cleanup.warnings
          });
        }
      }

      return cleanup.discarded;
    } catch {
      if ( vaultSession.path === context.vaultPath ) {
        applyEmbeddedAttachmentResult( result );
      }

      return false;
    }
  }

  async function embedAttachmentFromFile(
    capture: AttachmentInsertionCapture
  ): Promise<void> {
    if ( attachmentEmbedBusy.value || imageEmbedBusy.value ) {
      sourceEditor.value?.cancelAttachmentInsertion( capture );
      notify( 'Wait for the current file to finish embedding.', 'warning' );

      return;
    }

    attachmentEmbedBusy.value = true;
    try {
      const context = assetEmbedContext( capture );
      if ( !context ) {
        throw new Error( 'Files can be embedded into an open note in a desktop vault.' );
      }
      const sourcePath = await pickAttachmentFile();
      if ( !sourcePath ) {
        return;
      }
      if (
        vaultSession.path !== context.vaultPath
        || activeNote.value?.id !== context.note.id
      ) {
        throw new Error( 'The note or vault changed before the file could be embedded.' );
      }
      await storeAndInsertAttachment( capture, ( current, expectedRevision ) =>
        embedWorkspaceAttachmentFile(
          current.vaultPath,
          sourcePath,
          current.noteRelativePath,
          { ...vaultState.attachmentEmbedSettings },
          expectedRevision
        )
      );
    } catch ( error ) {
      notify( embedError( error, 'The file could not be embedded.' ), 'warning' );
    } finally {
      sourceEditor.value?.cancelAttachmentInsertion( capture );
      attachmentEmbedBusy.value = false;
    }
  }

  async function embedAttachmentFromVault(
    capture: AttachmentInsertionCapture,
    relativePath: string
  ): Promise<void> {
    if ( attachmentEmbedBusy.value || imageEmbedBusy.value ) {
      sourceEditor.value?.cancelAttachmentInsertion( capture );
      notify( 'Wait for the current file to finish embedding.', 'warning' );

      return;
    }

    attachmentEmbedBusy.value = true;
    try {
      await storeAndInsertAttachment( capture, ( context, expectedRevision ) =>
        embedWorkspaceVaultAttachment(
          context.vaultPath,
          relativePath,
          context.noteRelativePath,
          { ...vaultState.attachmentEmbedSettings },
          expectedRevision
        )
      );
    } catch ( error ) {
      notify( embedError( error, 'The file could not be embedded.' ), 'warning' );
    } finally {
      sourceEditor.value?.cancelAttachmentInsertion( capture );
      attachmentEmbedBusy.value = false;
    }
  }

  async function embedExternalFiles(
    capture: AttachmentInsertionCapture,
    files: File[],
    rejectedCount: number
  ): Promise<void> {
    if ( attachmentEmbedBusy.value || imageEmbedBusy.value ) {
      sourceEditor.value?.cancelAttachmentInsertion( capture );
      notify( 'Wait for the current file to finish embedding.', 'warning' );

      return;
    }
    if ( !files.length ) {
      sourceEditor.value?.cancelAttachmentInsertion( capture );
      notify(
        rejectedCount
          ? 'Folders and unavailable items cannot be embedded.'
          : 'No regular files were available in that drop.',
        'warning'
      );

      return;
    }
    const initialContext = assetEmbedContext( capture );
    if ( !initialContext ) {
      sourceEditor.value?.cancelAttachmentInsertion( capture );
      notify( 'Drop files into an open note in a desktop vault.', 'warning' );

      return;
    }

    const controller = new AbortController();
    externalFileDropAbort?.abort();
    externalFileDropAbort = controller;
    attachmentEmbedBusy.value = true;
    imageEmbedBusy.value = true;
    const imageSettings = { ...vaultState.imageEmbedSettings };
    const attachmentSettings = { ...vaultState.attachmentEmbedSettings };
    let currentCapture: AssetInsertionCapture | undefined = capture;
    let embeddedCount = 0;
    let failedCount = rejectedCount;
    const errors: string[] = rejectedCount
      ? [ `${ rejectedCount } folder${ rejectedCount === 1 ? ' or item was' : 's or items were' } skipped.` ]
      : [];

    try {
      for ( let index = 0; index < files.length && currentCapture; index += 1 ) {
        const file = files[ index ]!;
        const options: StoreAndInsertOptions = {
          announce: false,
          cleanupFailedInsertion: cleanupFailedExternalInsertion,
          ...( embeddedCount ? { markdownPrefix: ' ' } : {})
        };
        try {
          const inserted = isSupportedImageFileName( file.name )
            ? await storeAndInsertImage(
              currentCapture,
              ( context, expectedRevision ) => embedWorkspaceExternalImage(
                context.vaultPath,
                file,
                context.noteRelativePath,
                imageSettings,
                expectedRevision,
                controller.signal,
                () => prepareExternalFileFinish( context, controller.signal )
              ),
              options
            )
            : await storeAndInsertAttachment(
              currentCapture,
              ( context, expectedRevision ) => embedWorkspaceExternalAttachment(
                context.vaultPath,
                file,
                context.noteRelativePath,
                attachmentSettings,
                expectedRevision,
                controller.signal,
                () => prepareExternalFileFinish( context, controller.signal )
              ),
              options
            );
          if ( !inserted ) {
            failedCount += 1;
            errors.push( 'A stored file could not be inserted into the note.' );
            currentCapture = undefined;
            break;
          }
          embeddedCount += 1;
          currentCapture = index + 1 < files.length
            ? sourceEditor.value?.captureAttachmentInsertion()
            : undefined;
          if ( index + 1 < files.length && !currentCapture ) {
            failedCount += files.length - index - 1;
            errors.push( 'The remaining files could not retain their drop position.' );
          }
        } catch ( error ) {
          failedCount += 1;
          errors.push( `${ file.name.trim() || 'Dropped file' }: ${ embedError(
            error,
            'The file could not be embedded.'
          ) }` );
          if (
            controller.signal.aborted
            || !currentCapture
            || !assetEmbedContext( currentCapture )
          ) {
            break;
          }
        }
      }
    } finally {
      if ( currentCapture ) {
        sourceEditor.value?.cancelAttachmentInsertion( currentCapture );
      }
      if ( externalFileDropAbort === controller ) {
        externalFileDropAbort = undefined;
      }
      attachmentEmbedBusy.value = false;
      imageEmbedBusy.value = false;
    }

    if ( controller.signal.aborted ) {
      notify(
        'The file drop was cancelled because the note or vault changed.',
        'warning'
      );
    } else if ( errors.length ) {
      notify(
        embeddedCount
          ? `Embedded ${ embeddedCount } file${ embeddedCount === 1 ? '' : 's' }; ${ failedCount } could not be added.`
          : errors[ 0 ]!,
        'warning'
      );
    } else {
      notify(
        files.length === 1
          ? `Embedded ${ files[ 0 ]!.name }`
          : `Embedded ${ files.length } files`,
        'success'
      );
    }
  }

  function requestAttachmentFromToolbar(): void {
    const capture = sourceEditor.value?.captureAttachmentInsertion();
    if ( capture ) {
      void embedAttachmentFromFile( capture );
    }
  }

  async function insertRequestedVaultImage(): Promise<void> {
    const relativePath = vaultImageInsertRequest.relativePath;
    await nextTick();
    const capture = sourceEditor.value?.captureImageInsertion();
    if ( !capture || !relativePath ) {
      notify( 'Place the cursor in an open note before inserting an image', 'warning' );

      return;
    }
    await embedImageFromVault( capture, relativePath );
  }

  async function insertRequestedVaultAttachment(): Promise<void> {
    const relativePath = vaultAttachmentInsertRequest.relativePath;
    await nextTick();
    const capture = sourceEditor.value?.captureAttachmentInsertion();
    if ( !capture || !relativePath ) {
      notify( 'Place the cursor in an open note before inserting a file', 'warning' );

      return;
    }
    await embedAttachmentFromVault( capture, relativePath );
  }

  function activateEmbeddedAttachment(
    assetId: string | undefined,
    relativePath: string,
    mediaType: string | undefined,
    openingDisabled: boolean | undefined
  ): void {
    void activateVaultAttachment({
      ...( assetId ? { assetId } : {}),
      relativePath,
      mediaType: mediaType ?? 'application/octet-stream',
      openingDisabled: openingDisabled ?? false
    });
  }

  function revealEmbeddedAttachmentInTree(
    assetId: string | undefined,
    relativePath: string
  ): void {
    void revealVaultItemInTree({
      ...( assetId ? { assetId } : {}),
      kind: 'attachment',
      relativePath
    });
  }

  function showEmbeddedAttachmentInFolder(
    assetId: string | undefined,
    relativePath: string
  ): void {
    void showVaultItemInFolder({
      ...( assetId ? { assetId } : {}),
      kind: 'attachment',
      relativePath
    });
  }

  watch(
    [ () => vaultSession.path, () => activeNote.value?.id ],
    () => externalFileDropAbort?.abort()
  );

  watch(
    () => vaultImageInsertRequest.id,
    () => void insertRequestedVaultImage()
  );

  watch(
    () => vaultAttachmentInsertRequest.id,
    () => void insertRequestedVaultAttachment()
  );

  onBeforeUnmount( () => externalFileDropAbort?.abort() );

  return {
    activateEmbeddedAttachment,
    attachmentEmbedBusy,
    embedAttachmentFromFile,
    embedAttachmentFromVault,
    embedExternalFiles,
    embedImageFromClipboard,
    embedImageFromFile,
    embedImageFromVault,
    imageEmbedBusy,
    nativeAvailable,
    requestAttachmentFromToolbar,
    requestImageFromToolbar,
    revealEmbeddedAttachmentInTree,
    storeAndInsertAttachment,
    showEmbeddedAttachmentInFolder
  };
}
