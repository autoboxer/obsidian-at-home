import { resolveMarkdownImagePath, utf8ByteLength } from '../lib/imageEmbeds';
import {
  formatMarkdownImage,
  parseMarkdownImages,
  relativeImageDestination
} from '../lib/markdownImages';
import {
  formatMarkdownAttachment,
  parseMarkdownAttachments,
  relativeAttachmentDestination,
  type ParsedMarkdownAttachment
} from '../lib/markdownAttachments';
import type {
  Folder,
  VaultAttachmentFile,
  VaultData,
  VaultImageFile
} from '../types';

type MarkdownReplacement = { from: number; to: number; value: string };

export function folderContainsVaultAssets(
  vault: VaultData,
  folderPath: string
): boolean {
  if ( !folderPath ) {
    return false;
  }
  const folderKey = folderPath.toLocaleLowerCase();
  const containsImage = vault.imageFiles.some( ( image ) =>
    image.relativePath.toLocaleLowerCase().startsWith( `${ folderKey }/` )
  );

  return containsImage || vault.attachmentFiles.some( ( attachment ) =>
    attachment.relativePath.toLocaleLowerCase().startsWith( `${ folderKey }/` )
  );
}

export function rewriteVaultImageReferences(
  vault: VaultData,
  content: string,
  noteRelativePath: string,
  sourceRelativePath: string,
  targetRelativePath: string,
  previousAssetId: string | undefined,
  assetId: string
): string {
  const replacements: MarkdownReplacement[] = [];
  for ( const image of parseMarkdownImages( content ) ) {
    const trackedPath = trackedImagePath( vault, image.assetId );
    const resolvedPath = resolveMarkdownImagePath( noteRelativePath, image.destination );
    const matchesAsset = Boolean( previousAssetId && image.assetId === previousAssetId );
    const matchesPath = !trackedPath
      && resolvedPath?.toLocaleLowerCase() === sourceRelativePath.toLocaleLowerCase();
    if ( matchesAsset || matchesPath ) {
      replacements.push({
        from: image.start,
        to: image.end + 1,
        value: formatMarkdownImage({
          alt: image.alt,
          assetId,
          destination: relativeImageDestination( noteRelativePath, targetRelativePath ),
          ...( image.width ? { width: image.width } : {}),
          ...( image.height ? { height: image.height } : {}),
          ...( image.title !== undefined ? { title: image.title } : {}),
          inTable: image.raw.includes( '\\|' )
        })
      });
    }
  }

  return applyMarkdownReplacements( content, replacements );
}

export function rewriteVaultAttachmentReferences(
  vault: VaultData,
  content: string,
  noteRelativePath: string,
  sourceRelativePath: string,
  targetRelativePath: string,
  previousAssetId: string | undefined,
  assetId: string
): string {
  const replacements: MarkdownReplacement[] = [];
  const sourceName = sourceRelativePath.split( '/' ).at( -1 ) || 'Attachment';
  const targetName = targetRelativePath.split( '/' ).at( -1 ) || 'Attachment';
  for ( const attachment of parseVaultAttachmentReferences(
    vault,
    content,
    noteRelativePath
  ) ) {
    const resolvedPath = resolveMarkdownImagePath(
      noteRelativePath,
      attachment.destination
    );
    const matchesAsset = Boolean(
      previousAssetId && attachment.assetId === previousAssetId
    );
    const matchesPath = !attachment.assetId
      && resolvedPath?.toLocaleLowerCase() === sourceRelativePath.toLocaleLowerCase();
    if ( matchesAsset || matchesPath ) {
      replacements.push({
        from: attachment.start,
        to: attachment.end + 1,
        value: formatMarkdownAttachment({
          label: attachment.label === sourceName ? targetName : attachment.label,
          assetId,
          destination: relativeAttachmentDestination(
            noteRelativePath,
            targetRelativePath
          ),
          ...( attachment.title !== undefined ? { title: attachment.title } : {}),
          inTable: attachment.raw.includes( '\\|' )
        })
      });
    }
  }

  return applyMarkdownReplacements( content, replacements );
}

export function rewriteVaultAssetDestinationsForNotePath(
  vault: VaultData,
  content: string,
  sourceNotePath: string,
  targetNotePath: string
): string {
  if ( !sourceNotePath || !targetNotePath || sourceNotePath === targetNotePath ) {
    return content;
  }
  const replacements: MarkdownReplacement[] = [];
  for ( const image of parseMarkdownImages( content ) ) {
    const trackedPath = trackedImagePath( vault, image.assetId );
    const imagePath = trackedPath
      ?? resolveMarkdownImagePath( sourceNotePath, image.destination );
    if ( !imagePath ) {
      continue;
    }
    replacements.push({
      from: image.start,
      to: image.end + 1,
      value: formatMarkdownImage({
        alt: image.alt,
        ...( image.assetId ? { assetId: image.assetId } : {}),
        destination: relativeImageDestination( targetNotePath, imagePath ),
        ...( image.width ? { width: image.width } : {}),
        ...( image.height ? { height: image.height } : {}),
        ...( image.title !== undefined ? { title: image.title } : {}),
        inTable: image.raw.includes( '\\|' )
      })
    });
  }
  for ( const attachment of parseVaultAttachmentReferences(
    vault,
    content,
    sourceNotePath
  ) ) {
    const trackedPath = attachment.assetId
      ? vault.embeddedAttachments.find( ( asset ) => asset.id === attachment.assetId )
        ?.relativePath
        ?? vault.attachmentFiles.find( ( file ) => file.assetId === attachment.assetId )
          ?.relativePath
      : undefined;
    const attachmentPath = trackedPath
      ?? resolveMarkdownImagePath( sourceNotePath, attachment.destination );
    if ( !attachmentPath ) {
      continue;
    }
    replacements.push({
      from: attachment.start,
      to: attachment.end + 1,
      value: formatMarkdownAttachment({
        label: attachment.label,
        ...( attachment.assetId ? { assetId: attachment.assetId } : {}),
        destination: relativeAttachmentDestination( targetNotePath, attachmentPath ),
        ...( attachment.title !== undefined ? { title: attachment.title } : {}),
        inTable: attachment.raw.includes( '\\|' )
      })
    });
  }

  return applyMarkdownReplacements( content, replacements );
}

export function isSafeVaultImageFileName( value: string ): boolean {
  if ( !isSafeVaultFileName( value ) ) {
    return false;
  }
  const extension = value.split( '.' ).at( -1 )?.toLocaleLowerCase();

  return Boolean(
    extension
    && [ 'png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'avif' ].includes( extension )
  );
}

export function isSafeVaultAttachmentFileName( value: string ): boolean {
  if ( !isSafeVaultFileName( value ) ) {
    return false;
  }
  const extension = value.split( '.' ).at( -1 )?.toLocaleLowerCase();

  return !extension
    || ![ 'md', 'markdown', 'png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'avif' ]
      .includes( extension );
}

export function upsertVaultImageFile(
  vault: VaultData,
  image: VaultImageFile,
  createFolderId: () => string
): void {
  const portablePath = image.relativePath.toLocaleLowerCase();
  const index = vault.imageFiles.findIndex( ( candidate ) =>
    ( image.assetId && candidate.assetId === image.assetId )
    || candidate.relativePath.toLocaleLowerCase() === portablePath
  );
  if ( index >= 0 ) {
    vault.imageFiles.splice( index, 1, { ...image });
  } else {
    vault.imageFiles.push({ ...image });
  }
  ensureVaultAssetFolders( vault, image.relativePath, createFolderId );
}

export function upsertVaultAttachmentFile(
  vault: VaultData,
  attachment: VaultAttachmentFile,
  createFolderId: () => string
): void {
  const portablePath = attachment.relativePath.toLocaleLowerCase();
  const index = vault.attachmentFiles.findIndex( ( candidate ) =>
    ( attachment.assetId && candidate.assetId === attachment.assetId )
    || candidate.relativePath.toLocaleLowerCase() === portablePath
  );
  if ( index >= 0 ) {
    vault.attachmentFiles.splice( index, 1, { ...attachment });
  } else {
    vault.attachmentFiles.push({ ...attachment });
  }
  ensureVaultAssetFolders( vault, attachment.relativePath, createFolderId );
}

export function rebuildVaultAssetFolders(
  vault: VaultData,
  createFolderId: () => string
): void {
  for ( const image of vault.imageFiles ) {
    ensureVaultAssetFolders( vault, image.relativePath, createFolderId );
  }
  for ( const attachment of vault.attachmentFiles ) {
    ensureVaultAssetFolders( vault, attachment.relativePath, createFolderId );
  }
}

function trackedImagePath(
  vault: VaultData,
  assetId: string | undefined
): string | undefined {
  if ( !assetId ) {
    return undefined;
  }

  return vault.embeddedImages.find( ( image ) => image.id === assetId )?.relativePath
    ?? vault.imageFiles.find( ( image ) => image.assetId === assetId )?.relativePath;
}

function parseVaultAttachmentReferences(
  vault: VaultData,
  content: string,
  noteRelativePath: string
): ParsedMarkdownAttachment[] {
  const attachmentPaths = new Set(
    vault.attachmentFiles.map( ( attachment ) =>
      attachment.relativePath.toLocaleLowerCase()
    )
  );

  return parseMarkdownAttachments( content, {
    acceptExtensionless( destination ) {
      const relativePath = resolveMarkdownImagePath( noteRelativePath, destination );

      return Boolean(
        relativePath
        && attachmentPaths.has( relativePath.toLocaleLowerCase() )
      );
    }
  });
}

export function applyMarkdownReplacements(
  content: string,
  replacements: MarkdownReplacement[]
): string {
  let result = content;
  for ( const replacement of [ ...replacements ].sort(
    ( left, right ) => right.from - left.from
  ) ) {
    result = `${ result.slice( 0, replacement.from ) }${ replacement.value }${ result.slice( replacement.to ) }`;
  }

  return result;
}

function isSafeVaultFileName( value: string ): boolean {
  return Boolean(
    value
    && value === value.trim()
    && value !== '.'
    && value !== '..'
    && utf8ByteLength( value ) <= 180
    && !value.endsWith( '.' )
    && !/[\u0000-\u001f\u007f/\\:*?"<>|]/u.test( value )
    && !/^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/iu.test( value )
  );
}

function ensureVaultAssetFolders(
  vault: VaultData,
  relativePath: string,
  createFolderId: () => string
): void {
  const components = relativePath.split( '/' ).slice( 0, -1 ).filter( Boolean );
  let parentId: string | null = null;
  for ( const name of components ) {
    let folder: Folder | undefined = vault.folders.find( ( candidate ) =>
      candidate.parentId === parentId
      && candidate.name.localeCompare( name, undefined, { sensitivity: 'base' }) === 0
    );
    if ( !folder ) {
      folder = {
        id: createFolderId(),
        name,
        parentId,
        createdAt: Date.now()
      };
      vault.folders.push( folder );
    }
    parentId = folder.id;
  }
}
