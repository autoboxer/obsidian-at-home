import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  readImage as readNativeClipboardImage,
  writeText as writeNativeClipboardText,
} from "@tauri-apps/plugin-clipboard-manager";
import type {
  AttachmentEmbedSettings,
  ExportResult,
  ImageEmbedSettings,
  ImportResult,
  Note,
  NoteEditorPosition,
  VaultData,
  VaultDescriptor,
  WorkspaceArchiveResult,
  WorkspaceAttachmentCopyResult,
  WorkspaceAttachmentNoteUpdate,
  WorkspaceBootstrap,
  WorkspaceEmbedImageResult,
  WorkspaceEmbedAttachmentResult,
  WorkspaceImageNoteUpdate,
  WorkspaceImportImagesResult,
  WorkspaceImportSaveResult,
  WorkspaceLoad,
  WorkspaceRelocateImageResult,
  WorkspaceRelocateAttachmentResult,
  WorkspaceRecoveryMutationResult,
  WorkspaceRestoreResult,
  WorkspaceSaveResult,
} from "../types";

export const isTauri = (): boolean => Boolean(window.__TAURI__?.core?.invoke);

export async function writeClipboardText(value: string): Promise<void> {
  if (isTauri()) {
    await writeNativeClipboardText(value);

    return;
  }

  if (!navigator.clipboard?.writeText) {
    throw new Error("Clipboard access is unavailable in this browser.");
  }
  await navigator.clipboard.writeText(value);
}

export async function readClipboardImagePng(): Promise<Uint8Array> {
  if (!isTauri()) {
    throw new Error("Clipboard image access is available in the desktop app.");
  }

  const image = await readNativeClipboardImage();
  try {
    const [{ width, height }, rgba] = await Promise.all([image.size(), image.rgba()]);
    if (!width || !height || rgba.length !== width * height * 4) {
      throw new Error("The clipboard image could not be decoded.");
    }

    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    if (!context) {
      throw new Error("The clipboard image could not be prepared.");
    }
    context.putImageData(
      new ImageData(new Uint8ClampedArray(rgba), width, height),
      0,
      0,
    );
    const blob = await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob(
        (value) => value ? resolve(value) : reject(new Error("The clipboard image could not be encoded.")),
        "image/png",
      );
    });

    return new Uint8Array(await blob.arrayBuffer());
  } finally {
    await image.close().catch(() => undefined);
  }
}

export async function applyAppZoom(scaleFactor: number): Promise<void> {
  if (isTauri()) {
    try {
      await getCurrentWebview().setZoom(scaleFactor);
      document.documentElement.style.zoom = "";

      return;
    } catch {
      // CSS zoom keeps browser previews and older webviews functional.
    }
  }

  document.documentElement.style.zoom = String(scaleFactor);
}

export interface SystemFont {
  family: string;
  monospaced: boolean;
}

type NativeInvokeArgs = Record<string, unknown> | number[] | ArrayBuffer | Uint8Array;

async function invoke<T>(
  command: string,
  args?: NativeInvokeArgs,
  options?: { headers: Record<string, string> },
): Promise<T> {
  const tauriInvoke = window.__TAURI__?.core?.invoke;
  if (!tauriInvoke) {
    throw new Error("Native vault access is available in the Obsidian At Home desktop app.");
  }

  return tauriInvoke<T>(command, args, options);
}

export async function listSystemFonts(): Promise<SystemFont[]> {
  return invoke<SystemFont[]>("list_system_fonts");
}

export async function pickFolder(): Promise<string | null> {
  return invoke<string | null>("pick_folder");
}

export async function pickImageFile(): Promise<string | null> {
  return invoke<string | null>("pick_image_file");
}

export async function pickAttachmentFile(): Promise<string | null> {
  return invoke<string | null>("pick_attachment_file");
}

export async function bootstrapWorkspace(defaults: VaultData): Promise<WorkspaceBootstrap> {
  return invoke<WorkspaceBootstrap>("workspace_bootstrap", { defaults });
}

export async function openWorkspace(path: string, defaults: VaultData): Promise<WorkspaceLoad> {
  return invoke<WorkspaceLoad>("workspace_open", { path, defaults });
}

export async function createWorkspace(
  parentPath: string,
  name: string,
  initial: VaultData,
): Promise<WorkspaceLoad> {
  return invoke<WorkspaceLoad>("workspace_create", { parentPath, name, initial });
}

export async function saveWorkspace(
  path: string,
  vault: VaultData,
  expectedRevision: number,
): Promise<WorkspaceSaveResult> {
  return invoke<WorkspaceSaveResult>("workspace_save", { path, vault, expectedRevision });
}

export async function saveWorkspaceWithImageImport(
  path: string,
  vault: VaultData,
  expectedRevision: number,
  transactionId: string,
): Promise<WorkspaceImportSaveResult> {
  return invoke<WorkspaceImportSaveResult>("workspace_save_with_image_import", {
    path,
    vault,
    expectedRevision,
    transactionId,
  });
}

export async function archiveWorkspaceNote(
  path: string,
  vault: VaultData,
  note: Note,
  originalFolderPath: string,
  editorPosition: NoteEditorPosition | undefined,
  expectedRevision: number,
): Promise<WorkspaceArchiveResult> {
  return invoke<WorkspaceArchiveResult>("workspace_archive_note", {
    path,
    vault,
    note,
    originalFolderPath,
    editorPosition,
    expectedRevision,
  });
}

export async function restoreRecentlyDeletedNote(
  path: string,
  deletedNoteId: string,
  vault: VaultData,
  expectedRevision: number,
): Promise<WorkspaceRestoreResult> {
  return invoke<WorkspaceRestoreResult>("workspace_restore_recently_deleted_note", {
    path,
    deletedNoteId,
    vault,
    expectedRevision,
  });
}

export async function deleteRecentlyDeletedNotes(
  path: string,
  deletedNoteIds: string[],
  expectedRevision: number,
): Promise<WorkspaceRecoveryMutationResult> {
  return invoke<WorkspaceRecoveryMutationResult>("workspace_delete_recently_deleted_notes", {
    path,
    deletedNoteIds,
    expectedRevision,
  });
}

export async function pruneRecentlyDeletedNotes(
  path: string,
  expectedRevision: number,
): Promise<WorkspaceRecoveryMutationResult> {
  return invoke<WorkspaceRecoveryMutationResult>("workspace_prune_recently_deleted_notes", {
    path,
    expectedRevision,
  });
}

export async function saveWorkspaceEditorPositions(
  path: string,
  positions: Record<string, NoteEditorPosition>,
  expectedRevision: string | null,
): Promise<string> {
  return invoke<string>("workspace_save_editor_positions", {
    path,
    positions,
    expectedRevision,
  });
}

export async function forgetWorkspace(path: string): Promise<VaultDescriptor[]> {
  return invoke<VaultDescriptor[]>("workspace_forget", { path });
}

export async function getWorkspaceRevision(path: string): Promise<number> {
  return invoke<number>("workspace_revision", { path });
}

export async function embedWorkspaceImageFile(
  path: string,
  sourcePath: string,
  noteRelativePath: string,
  settings: ImageEmbedSettings,
  expectedRevision: number,
): Promise<WorkspaceEmbedImageResult> {
  return invoke<WorkspaceEmbedImageResult>("workspace_embed_image_file", {
    path,
    sourcePath,
    noteRelativePath,
    settings,
    expectedRevision,
  });
}

export async function embedWorkspaceVaultImage(
  path: string,
  imageRelativePath: string,
  noteRelativePath: string,
  settings: ImageEmbedSettings,
  expectedRevision: number,
): Promise<WorkspaceEmbedImageResult> {
  return invoke<WorkspaceEmbedImageResult>("workspace_embed_vault_image", {
    path,
    imageRelativePath,
    noteRelativePath,
    settings,
    expectedRevision,
  });
}

export async function embedWorkspaceAttachmentFile(
  path: string,
  sourcePath: string,
  noteRelativePath: string,
  settings: AttachmentEmbedSettings,
  expectedRevision: number,
): Promise<WorkspaceEmbedAttachmentResult> {
  return invoke<WorkspaceEmbedAttachmentResult>("workspace_embed_attachment_file", {
    path,
    sourcePath,
    noteRelativePath,
    settings,
    expectedRevision,
  });
}

export async function embedWorkspaceVaultAttachment(
  path: string,
  attachmentRelativePath: string,
  noteRelativePath: string,
  settings: AttachmentEmbedSettings,
  expectedRevision: number,
): Promise<WorkspaceEmbedAttachmentResult> {
  return invoke<WorkspaceEmbedAttachmentResult>("workspace_embed_vault_attachment", {
    path,
    attachmentRelativePath,
    noteRelativePath,
    settings,
    expectedRevision,
  });
}

export async function relocateWorkspaceImage(
  path: string,
  imageRelativePath: string,
  targetRelativePath: string,
  assetId: string,
  noteUpdates: WorkspaceImageNoteUpdate[],
  expectedRevision: number,
  managedByNoteMove = false,
): Promise<WorkspaceRelocateImageResult> {
  return invoke<WorkspaceRelocateImageResult>("workspace_relocate_image", {
    path,
    imageRelativePath,
    targetRelativePath,
    assetId,
    noteUpdates,
    expectedRevision,
    managedByNoteMove,
  });
}

export async function relocateWorkspaceAttachment(
  path: string,
  attachmentRelativePath: string,
  targetRelativePath: string,
  assetId: string,
  noteUpdates: WorkspaceAttachmentNoteUpdate[],
  expectedRevision: number,
  managedByNoteMove = false,
): Promise<WorkspaceRelocateAttachmentResult> {
  return invoke<WorkspaceRelocateAttachmentResult>("workspace_relocate_attachment", {
    path,
    attachmentRelativePath,
    targetRelativePath,
    assetId,
    noteUpdates,
    expectedRevision,
    managedByNoteMove,
  });
}

export async function openWorkspaceAttachment(
  path: string,
  attachmentRelativePath: string,
  assetId?: string,
): Promise<void> {
  await invoke<void>("workspace_open_attachment", {
    path,
    attachmentRelativePath,
    assetId,
  });
}

export async function saveWorkspaceAttachmentCopy(
  path: string,
  attachmentRelativePath: string,
  assetId?: string,
  preferredDirectory?: string,
): Promise<WorkspaceAttachmentCopyResult | null> {
  return invoke<WorkspaceAttachmentCopyResult | null>("workspace_save_attachment_copy", {
    path,
    attachmentRelativePath,
    assetId,
    preferredDirectory,
  });
}

export async function embedWorkspaceImageBytes(
  path: string,
  fileName: string,
  bytes: Uint8Array,
  noteRelativePath: string,
  settings: ImageEmbedSettings,
  expectedRevision: number,
): Promise<WorkspaceEmbedImageResult> {
  const metadata = encodeURIComponent(JSON.stringify({
    path,
    fileName,
    noteRelativePath,
    settings,
    expectedRevision,
  }));

  return invoke<WorkspaceEmbedImageResult>(
    "workspace_embed_image_bytes",
    bytes,
    { headers: { "x-oah-image-metadata": metadata } },
  );
}

export async function readWorkspaceImage(
  path: string,
  noteRelativePath: string,
  destination: string,
  assetId?: string,
): Promise<Uint8Array> {
  const bytes = await invoke<ArrayBuffer | Uint8Array>("workspace_read_image", {
    path,
    assetId,
    noteRelativePath,
    destination,
  });

  return bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
}

export async function importObsidianVault(path: string): Promise<ImportResult> {
  return invoke<ImportResult>("import_obsidian_vault", { path });
}

export async function importWorkspaceImages(
  path: string,
  sourcePath: string,
  imagePaths: string[],
  expectedRevision: number,
): Promise<WorkspaceImportImagesResult> {
  return invoke<WorkspaceImportImagesResult>("workspace_import_images", {
    path,
    sourcePath,
    imagePaths,
    expectedRevision,
  });
}

export async function exportObsidianVault(
  parentPath: string,
  sourcePath: string,
  vaultName: string,
  payload: {
    notes: unknown[];
    templates: unknown[];
    snippets: unknown[];
  },
): Promise<ExportResult> {
  return invoke<ExportResult>("export_obsidian_vault", {
    parentPath,
    sourcePath,
    vaultName,
    ...payload,
  });
}
