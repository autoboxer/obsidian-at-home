import { getCurrentWebview } from "@tauri-apps/api/webview";
import { writeText as writeNativeClipboardText } from "@tauri-apps/plugin-clipboard-manager";
import type {
  ExportResult,
  ImportResult,
  Note,
  NoteEditorPosition,
  VaultData,
  VaultDescriptor,
  WorkspaceArchiveResult,
  WorkspaceBootstrap,
  WorkspaceLoad,
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

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const tauriInvoke = window.__TAURI__?.core?.invoke;
  if (!tauriInvoke) {
    throw new Error("Native vault access is available in the Obsidian At Home desktop app.");
  }

  return tauriInvoke<T>(command, args);
}

export async function listSystemFonts(): Promise<SystemFont[]> {
  return invoke<SystemFont[]>("list_system_fonts");
}

export async function pickFolder(): Promise<string | null> {
  return invoke<string | null>("pick_folder");
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

export async function importObsidianVault(path: string): Promise<ImportResult> {
  return invoke<ImportResult>("import_obsidian_vault", { path });
}

export async function exportObsidianVault(
  parentPath: string,
  vaultName: string,
  payload: {
    notes: unknown[];
    templates: unknown[];
    snippets: unknown[];
  },
): Promise<ExportResult> {
  return invoke<ExportResult>("export_obsidian_vault", {
    parentPath,
    vaultName,
    ...payload,
  });
}
