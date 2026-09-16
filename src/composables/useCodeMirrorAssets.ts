import { computed, ref, type EmitFn, type Ref } from 'vue';
import { isolateHistory } from '@codemirror/commands';
import { EditorSelection } from '@codemirror/state';
import { NOTE_DRAG_MIME, notify } from '../stores/vault';
import { liveMarkdownDocumentModel } from '../lib/liveMarkdownDocumentModel';
import {
  decodeMarkdownImageDestination,
  imageMediaTypeForPath,
  NOTE_IMAGE_DRAG_MIME,
  resolveMarkdownImagePath,
  VAULT_IMAGE_DRAG_MIME
} from '../lib/imageEmbeds';
import { sanitizeImageUrl } from '../lib/markdown';
import { VAULT_ATTACHMENT_DRAG_MIME } from '../lib/markdownAttachments';
import { parseMarkdownImageAt } from '../lib/markdownImages';
import { isTauri, readWorkspaceImage } from '../services/native';
import type { EditorView, ViewUpdate } from '@codemirror/view';
import type { MarkdownAttachmentMetadata, ParsedMarkdownAttachment } from '../lib/markdownAttachments';
import type { ParsedMarkdownImage } from '../lib/markdownImages';
import type {
  AttachmentInsertionCapture,
  EmbeddedAttachment,
  EmbeddedImage,
  ImageInsertionCapture,
  VaultAttachmentFile
} from '../types';

const MAX_EXTERNAL_DROP_FILES = 100;

interface CodeMirrorAssetProps {
  attachmentFiles: VaultAttachmentFile[];
  embeddedAttachments: EmbeddedAttachment[];
  embeddedImages: EmbeddedImage[];
  noteId: string;
  noteRelativePath: string;
  noteLinkForId: ( noteId: string ) => string | undefined;
  readOnly: boolean;
  vaultPath: string | null;
}

export interface CodeMirrorAssetEvents {
  activateAttachment: [
    assetId: string | undefined,
    relativePath: string,
    mediaType: string | undefined,
    openingDisabled: boolean | undefined
  ];
  externalFileDrop: [capture: AttachmentInsertionCapture, files: File[], rejectedCount: number];
  pasteImage: [capture: ImageInsertionCapture, file?: File];
  revealAttachmentInTree: [assetId: string | undefined, relativePath: string];
  requestEmbedAttachment: [capture: AttachmentInsertionCapture];
  requestEmbedImage: [capture: ImageInsertionCapture];
  showAttachmentInFolder: [assetId: string | undefined, relativePath: string];
  vaultImageDrop: [capture: ImageInsertionCapture, relativePath: string];
  vaultAttachmentDrop: [capture: AttachmentInsertionCapture, relativePath: string];
}

export function useCodeMirrorAssets(
  props: CodeMirrorAssetProps,
  emit: EmitFn<CodeMirrorAssetEvents>,
  editorView: Ref<EditorView | undefined>,
  editorRenderReady: Readonly<Ref<boolean>>,
  blockPendingEditorInteraction: ( event: Event ) => void
) {
  const externalFileDragActive = ref( false );
  let externalFileDragDepth = 0;
  let imageInsertionSequence = 0;
  let imageResolverDisposed = false;
  let imageResolverGeneration = 0;

  interface PendingImageInsertion extends ImageInsertionCapture {
    from: number;
    to: number;
  }

  const pendingImageInsertions = new Map<string, PendingImageInsertion>();
  const imageSourcePromises = new Map<string, Promise<string>>();
  const imageObjectUrls = new Set<string>();

  async function resolveLiveMarkdownImageSource(
    image: ParsedMarkdownImage
  ): Promise<string | undefined> {
    const source = sanitizeImageUrl( image.destination );
    if ( !source ) {
      return undefined;
    }
    if (
      !isTauri()
      || !props.vaultPath
      || /^[a-z][a-z0-9+.-]*:/i.test( source )
      || source.startsWith( '//' )
    ) {
      return source;
    }

    const generation = imageResolverGeneration;
    const cacheKey = `${ generation }\u0000${ props.vaultPath }\u0000${ props.noteRelativePath }\u0000${
      image.assetId ?? ''
    }\u0000${ image.destination }`;
    const cached = imageSourcePromises.get( cacheKey );
    if ( cached ) {
      return cached;
    }

    const pending = ( async () => {
      const bytes = await readWorkspaceImage(
        props.vaultPath!,
        props.noteRelativePath,
        decodeMarkdownImageDestination( image.destination ),
        image.assetId
      );
      if ( imageResolverDisposed || generation !== imageResolverGeneration ) {
        throw new Error( 'The image changed before it finished loading.' );
      }
      const mediaType = props.embeddedImages.find( ( asset ) => asset.id === image.assetId )?.mediaType
        ?? imageMediaTypeForPath( image.destination );
      const url = URL.createObjectURL( new Blob([ bytes.slice().buffer ], { type: mediaType }) );
      imageObjectUrls.add( url );

      return url;
    })();
    imageSourcePromises.set( cacheKey, pending );
    pending.catch( () => {
      if ( imageSourcePromises.get( cacheKey ) === pending ) {
        imageSourcePromises.delete( cacheKey );
      }
    });

    return pending;
  }

  function clearLiveMarkdownImageSources(): void {
    imageResolverGeneration += 1;
    imageSourcePromises.clear();
    for ( const url of imageObjectUrls ) {
      URL.revokeObjectURL( url );
    }
    imageObjectUrls.clear();
  }

  const attachmentPathKeys = computed( () => new Set(
    props.attachmentFiles.map( ( attachment ) => attachment.relativePath.toLocaleLowerCase() )
  ) );

  function isKnownExtensionlessAttachment( destination: string ): boolean {
    const relativePath = resolveMarkdownImagePath( props.noteRelativePath, destination );

    return Boolean(
      relativePath
      && attachmentPathKeys.value.has( relativePath.toLocaleLowerCase() )
    );
  }

  function resolveLiveMarkdownAttachmentMetadata(
    attachment: ParsedMarkdownAttachment
  ): MarkdownAttachmentMetadata | undefined {
    const tracked = attachment.assetId
      ? props.embeddedAttachments.find( ( asset ) => asset.id === attachment.assetId )
      : undefined;
    const relativePath = tracked?.relativePath
      ?? resolveMarkdownImagePath( props.noteRelativePath, attachment.destination );
    if ( !relativePath ) {
      return undefined;
    }
    const portablePath = relativePath.toLocaleLowerCase();
    const matches = attachment.assetId
      ? props.attachmentFiles.filter( ( candidate ) =>
        candidate.assetId === attachment.assetId
      )
      : props.attachmentFiles.filter( ( candidate ) =>
        candidate.relativePath.toLocaleLowerCase() === portablePath
      );
    const file = matches.length === 1 ? matches[ 0 ] : undefined;
    const renameTarget = file && props.vaultPath && isTauri()
      ? {
        ...( file.assetId ? { assetId: file.assetId } : {}),
        relativePath: file.relativePath
      }
      : undefined;

    return {
      byteLength: tracked?.byteLength ?? file?.byteLength,
      mediaType: tracked?.mediaType ?? file?.mediaType,
      openingDisabled: tracked?.openingDisabled ?? file?.openingDisabled,
      ...( renameTarget ? { renameTarget } : {}),
      relativePath: tracked?.relativePath ?? file?.relativePath ?? relativePath
    };
  }

  function activateLiveMarkdownAttachment(
    attachment: ParsedMarkdownAttachment,
    metadata: MarkdownAttachmentMetadata | null | undefined
  ): void {
    const relativePath = metadata?.relativePath
      ?? resolveMarkdownImagePath( props.noteRelativePath, attachment.destination );
    if ( relativePath ) {
      emit(
        'activateAttachment',
        attachment.assetId,
        relativePath,
        metadata?.mediaType,
        metadata?.openingDisabled
      );
    }
  }

  function revealLiveMarkdownAttachmentInTree(
    attachment: ParsedMarkdownAttachment,
    metadata: MarkdownAttachmentMetadata | null | undefined
  ): void {
    emitLiveMarkdownAttachmentLocation( 'revealAttachmentInTree', attachment, metadata );
  }

  function showLiveMarkdownAttachmentInFolder(
    attachment: ParsedMarkdownAttachment,
    metadata: MarkdownAttachmentMetadata | null | undefined
  ): void {
    emitLiveMarkdownAttachmentLocation( 'showAttachmentInFolder', attachment, metadata );
  }

  function emitLiveMarkdownAttachmentLocation(
    event: 'revealAttachmentInTree' | 'showAttachmentInFolder',
    attachment: ParsedMarkdownAttachment,
    metadata: MarkdownAttachmentMetadata | null | undefined
  ): void {
    const relativePath = metadata?.relativePath
      ?? resolveMarkdownImagePath( props.noteRelativePath, attachment.destination );
    if ( !relativePath ) {
      return;
    }
    const assetId = metadata?.renameTarget?.assetId;
    if ( event === 'revealAttachmentInTree' ) {
      emit( 'revealAttachmentInTree', assetId, relativePath );
    } else {
      emit( 'showAttachmentInFolder', assetId, relativePath );
    }
  }

  function captureImageInsertion(
    view = editorView.value
  ): ImageInsertionCapture | undefined {
    if ( props.readOnly || !view ) {
      return undefined;
    }
    if ( view.state.selection.ranges.length !== 1 ) {
      notify( 'Use a single selection to insert an image or file.', 'neutral' );
      return undefined;
    }

    const selection = view.state.selection.main;
    const inTable = liveMarkdownDocumentModel( view.state ).tables.some( ( table ) =>
      selection.from >= table.from && selection.to <= table.to
    );
    imageInsertionSequence += 1;
    const token = `${ Date.now().toString( 36 ) }-${ imageInsertionSequence.toString( 36 ) }`;
    const insertion: PendingImageInsertion = {
      from: selection.from,
      inTable,
      noteId: props.noteId,
      selectedText: view.state.sliceDoc( selection.from, selection.to ),
      to: selection.to,
      token
    };
    pendingImageInsertions.set( token, insertion );

    return {
      inTable: insertion.inTable,
      noteId: insertion.noteId,
      selectedText: insertion.selectedText,
      token: insertion.token
    };
  }

  function cancelImageInsertion( capture: ImageInsertionCapture ): void {
    pendingImageInsertions.delete( capture.token );
  }

  function insertEmbeddedImage(
    capture: ImageInsertionCapture,
    markdownImage: string
  ): boolean {
    const view = editorView.value;
    const insertion = pendingImageInsertions.get( capture.token );
    pendingImageInsertions.delete( capture.token );
    if (
      props.readOnly
      || !view
      || !insertion
      || insertion.noteId !== props.noteId
      || insertion.from < 0
      || insertion.to < insertion.from
      || insertion.to > view.state.doc.length
      || view.state.sliceDoc( insertion.from, insertion.to ) !== insertion.selectedText
    ) {
      return false;
    }

    if ( view.state.selection.ranges.length !== 1 ) {
      notify( 'Use a single selection to insert an image or file.', 'neutral' );
      return false;
    }

    const cursor = insertion.from + markdownImage.length;
    view.dispatch({
      changes: {
        from: insertion.from,
        to: insertion.to,
        insert: markdownImage
      },
      selection: EditorSelection.cursor( cursor ),
      scrollIntoView: true,
      userEvent: 'input'
    });
    view.focus();

    return true;
  }

  function requestImageEmbed( view: EditorView ): boolean {
    const capture = captureImageInsertion( view );
    if ( !capture ) {
      return view.state.selection.ranges.length > 1;
    }
    emit( 'requestEmbedImage', capture );

    return true;
  }

  function captureAttachmentInsertion(
    view = editorView.value
  ): AttachmentInsertionCapture | undefined {
    return captureImageInsertion( view );
  }

  function cancelAttachmentInsertion( capture: AttachmentInsertionCapture ): void {
    cancelImageInsertion( capture );
  }

  function insertEmbeddedAttachment(
    capture: AttachmentInsertionCapture,
    markdownAttachment: string
  ): boolean {
    return insertEmbeddedImage( capture, markdownAttachment );
  }

  function requestAttachmentEmbed( view: EditorView ): boolean {
    const capture = captureAttachmentInsertion( view );
    if ( !capture ) {
      return view.state.selection.ranges.length > 1;
    }
    emit( 'requestEmbedAttachment', capture );

    return true;
  }

  function handleSourceEditorPaste( event: ClipboardEvent ): void {
    if ( props.readOnly || !editorRenderReady.value ) {
      blockPendingEditorInteraction( event );

      return;
    }

    const clipboard = event.clipboardData;
    if ( !clipboard ) {
      return;
    }
    const imageItem = Array.from( clipboard.items ).find( ( item ) =>
      item.kind === 'file' && item.type.toLocaleLowerCase().startsWith( 'image/' )
    );
    const imageSignaled = Boolean( imageItem )
      || Array.from( clipboard.types ).some( ( type ) =>
        type === 'Files' || type.toLocaleLowerCase().startsWith( 'image/' )
      );
    if ( !imageSignaled ) {
      return;
    }

    event.preventDefault();
    event.stopImmediatePropagation();
    const capture = captureImageInsertion();
    if ( !capture ) {
      return;
    }
    emit( 'pasteImage', capture, imageItem?.getAsFile() ?? undefined );
  }

  function isExternalFileDrag( event: DragEvent ): boolean {
    const types = Array.from( event.dataTransfer?.types ?? []);
    return types.includes( 'Files' )
      && !types.includes( NOTE_DRAG_MIME )
      && !types.includes( NOTE_IMAGE_DRAG_MIME )
      && !types.includes( VAULT_IMAGE_DRAG_MIME )
      && !types.includes( VAULT_ATTACHMENT_DRAG_MIME );
  }

  function clearExternalFileDrag(): void {
    externalFileDragDepth = 0;
    externalFileDragActive.value = false;
  }

  function handleSourceEditorDragEnter( event: DragEvent ): void {
    if ( props.readOnly || !editorRenderReady.value || !isExternalFileDrag( event ) ) {
      return;
    }
    externalFileDragDepth += 1;
    externalFileDragActive.value = true;
  }

  function handleSourceEditorDragLeave(): void {
    if ( !externalFileDragActive.value ) {
      return;
    }
    externalFileDragDepth = Math.max( 0, externalFileDragDepth - 1 );
    if ( !externalFileDragDepth ) {
      externalFileDragActive.value = false;
    }
  }

  function handleSourceEditorDragOver( event: DragEvent ): void {
    const types = Array.from( event.dataTransfer?.types ?? []);
    const movingWithinNote = types.includes( NOTE_IMAGE_DRAG_MIME );
    const vaultNote = types.includes( NOTE_DRAG_MIME );
    const vaultImage = types.includes( VAULT_IMAGE_DRAG_MIME );
    const vaultAttachment = types.includes( VAULT_ATTACHMENT_DRAG_MIME );
    const externalFiles = isExternalFileDrag( event );
    if (
      !movingWithinNote
      && !vaultNote
      && !vaultImage
      && !vaultAttachment
      && !externalFiles
    ) {
      return;
    }
    event.preventDefault();
    event.stopImmediatePropagation();
    if ( externalFiles && editorRenderReady.value && !props.readOnly ) {
      externalFileDragActive.value = true;
    }
    if ( event.dataTransfer ) {
      event.dataTransfer.dropEffect = props.readOnly ? 'none' : movingWithinNote ? 'move' : 'copy';
    }
  }

  function blockUnavailableEditorDrag( event: DragEvent ): void {
    if ( props.readOnly || !editorRenderReady.value ) {
      clearExternalFileDrag();
      if ( event.dataTransfer ) {
        event.dataTransfer.dropEffect = 'none';
      }
      blockPendingEditorInteraction( event );
    }
  }

  function handleSourceEditorDrop( event: DragEvent ): void {
    clearExternalFileDrag();
    if ( props.readOnly ) {
      event.preventDefault();
      event.stopImmediatePropagation();

      return;
    }
    const transfer = event.dataTransfer;
    const internalImage = parseInternalImageDrag( transfer?.getData( NOTE_IMAGE_DRAG_MIME ) );
    const relativePath = transfer?.getData( VAULT_IMAGE_DRAG_MIME ).trim() ?? '';
    const attachmentRelativePath = transfer
      ?.getData( VAULT_ATTACHMENT_DRAG_MIME )
      .trim() ?? '';
    const types = Array.from( transfer?.types ?? []);
    const vaultNote = types.includes( NOTE_DRAG_MIME );
    const externalFiles = !internalImage
      && !vaultNote
      && !relativePath
      && !attachmentRelativePath
      && types.includes( 'Files' );
    const view = editorView.value;
    if (
      !internalImage
      && !vaultNote
      && !relativePath
      && !attachmentRelativePath
      && !externalFiles
    ) {
      return;
    }
    event.preventDefault();
    event.stopImmediatePropagation();
    if ( !view || !editorRenderReady.value ) {
      return;
    }
    const position = view.posAtCoords({ x: event.clientX, y: event.clientY }, false )
      ?? view.state.selection.main.head;
    if ( vaultNote ) {
      const link = props.noteLinkForId( transfer?.getData( NOTE_DRAG_MIME ).trim() ?? '' );
      if ( !link ) {
        notify( 'That note is no longer available in this vault', 'warning' );

        return;
      }
      view.dispatch({
        changes: { from: position, insert: link },
        selection: EditorSelection.cursor( position + link.length ),
        scrollIntoView: true,
        annotations: isolateHistory.of( 'full' ),
        userEvent: 'input.drop'
      });
      view.focus();

      return;
    }
    if ( internalImage ) {
      moveImageReferenceWithinNote( view, internalImage.from, internalImage.to, position );

      return;
    }
    view.dispatch({
      selection: EditorSelection.cursor( position ),
      scrollIntoView: true,
      userEvent: 'select.pointer'
    });
    if ( externalFiles && transfer ) {
      const dropped = collectExternalDroppedFiles( transfer );
      const capture = captureAttachmentInsertion( view );
      if ( capture ) {
        emit( 'externalFileDrop', capture, dropped.files, dropped.rejectedCount );
      }

      return;
    }
    if ( attachmentRelativePath ) {
      const capture = captureAttachmentInsertion( view );
      if ( capture ) {
        emit( 'vaultAttachmentDrop', capture, attachmentRelativePath );
      }
    } else {
      const capture = captureImageInsertion( view );
      if ( !capture ) {
        return;
      }
      emit( 'vaultImageDrop', capture, relativePath );
    }
  }

  interface ExternalFileSystemEntry {
    isDirectory: boolean;
    isFile: boolean;
  }

  function isAmbiguousExternalDroppedFile( file: File ): boolean {
    // Without entry metadata, WebKit can represent an unavailable folder this way.
    return file.size === 0 && !file.type.trim();
  }

  function collectExternalDroppedFiles(
    transfer: DataTransfer
  ): { files: File[]; rejectedCount: number } {
    const items = Array.from( transfer.items ).filter( ( item ) => item.kind === 'file' );
    const files: File[] = [];
    let rejectedCount = 0;
    if ( items.length ) {
      for ( const item of items ) {
        const entry = ( item as DataTransferItem & {
          webkitGetAsEntry?: () => ExternalFileSystemEntry | null;
        }).webkitGetAsEntry?.();
        const file = entry?.isDirectory || ( entry && !entry.isFile )
          ? null
          : item.getAsFile();
        if (
          !file
          || ( !entry && isAmbiguousExternalDroppedFile( file ) )
          || files.length >= MAX_EXTERNAL_DROP_FILES
        ) {
          rejectedCount += 1;
        } else {
          files.push( file );
        }
      }

      return { files, rejectedCount };
    }

    for ( const file of Array.from( transfer.files ) ) {
      if (
        isAmbiguousExternalDroppedFile( file )
        || files.length >= MAX_EXTERNAL_DROP_FILES
      ) {
        rejectedCount += 1;
      } else {
        files.push( file );
      }
    }

    return { files, rejectedCount };
  }

  function parseInternalImageDrag(
    value: string | undefined
  ): { from: number; to: number } | undefined {
    if ( !value ) {
      return undefined;
    }
    try {
      const parsed = JSON.parse( value ) as { from?: unknown; to?: unknown };
      if (
        Number.isSafeInteger( parsed.from )
        && Number.isSafeInteger( parsed.to )
        && ( parsed.from as number ) >= 0
        && ( parsed.to as number ) > ( parsed.from as number )
      ) {
        return { from: parsed.from as number, to: parsed.to as number };
      }
    } catch {
      // Ignore malformed drag data from outside this editor.
    }

    return undefined;
  }

  function moveImageReferenceWithinNote(
    view: EditorView,
    from: number,
    to: number,
    position: number
  ): void {
    if (
      from < 0
      || to > view.state.doc.length
      || from >= to
      || ( position >= from && position <= to )
    ) {
      view.focus();

      return;
    }
    const source = view.state.sliceDoc( from, to );
    const image = parseMarkdownImageAt( source, 0 );
    if ( !image || image.end + 1 !== source.length ) {
      view.focus();

      return;
    }
    const insertionStart = position < from ? position : position - ( to - from );
    const changes = position < from
      ? [{ from: position, insert: source }, { from, to, insert: '' }]
      : [{ from, to, insert: '' }, { from: position, insert: source }];
    view.dispatch({
      changes,
      selection: EditorSelection.cursor( insertionStart + source.length ),
      scrollIntoView: true,
      userEvent: 'input.move'
    });
    view.focus();
  }

  function mapPendingImageInsertions( update: ViewUpdate ): void {
    for ( const insertion of pendingImageInsertions.values() ) {
      const empty = insertion.from === insertion.to;
      insertion.from = update.changes.mapPos( insertion.from, -1 );
      insertion.to = update.changes.mapPos( insertion.to, empty ? -1 : 1 );
    }
  }

  function disposeAssets(): void {
    imageResolverDisposed = true;
    pendingImageInsertions.clear();
    clearLiveMarkdownImageSources();
  }

  return {
    activateLiveMarkdownAttachment,
    blockUnavailableEditorDrag,
    cancelAttachmentInsertion,
    cancelImageInsertion,
    captureAttachmentInsertion,
    captureImageInsertion,
    clearExternalFileDrag,
    clearLiveMarkdownImageSources,
    disposeAssets,
    externalFileDragActive,
    handleSourceEditorDragEnter,
    handleSourceEditorDragLeave,
    handleSourceEditorDragOver,
    handleSourceEditorDrop,
    handleSourceEditorPaste,
    insertEmbeddedAttachment,
    insertEmbeddedImage,
    isKnownExtensionlessAttachment,
    mapPendingImageInsertions,
    requestAttachmentEmbed,
    requestImageEmbed,
    resolveLiveMarkdownAttachmentMetadata,
    resolveLiveMarkdownImageSource,
    revealLiveMarkdownAttachmentInTree,
    showLiveMarkdownAttachmentInFolder
  };
}
