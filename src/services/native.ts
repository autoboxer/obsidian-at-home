import type { ExportResult, ImportResult } from "../types";

export const isTauri = (): boolean => Boolean(window.__TAURI__?.core?.invoke);

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const tauriInvoke = window.__TAURI__?.core?.invoke;
  if (!tauriInvoke) {
    throw new Error("Native vault access is available in the Obsidian At Home desktop app.");
  }
  return tauriInvoke<T>(command, args);
}

export async function pickFolder(): Promise<string | null> {
  return invoke<string | null>("pick_folder");
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
