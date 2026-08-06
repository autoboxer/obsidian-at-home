<script setup lang="ts">
import { computed, ref } from "vue";
import type { ExportResult, ImportResult } from "../types";
import {
  buildExportPayload,
  clearVault,
  deleteVault,
  mergeImportedVault,
  vaultState,
} from "../stores/vault";
import {
  exportObsidianVault,
  importObsidianVault,
  isTauri,
  pickFolder,
} from "../services/native";
import AppIcon from "./AppIcon.vue";

type ActiveTask = "import" | "export" | null;
type FeedbackTone = "info" | "success" | "warning" | "error";

interface Feedback {
  tone: FeedbackTone;
  title: string;
  message: string;
  warnings?: string[];
}

const nativeAvailable = isTauri();
const activeTask = ref<ActiveTask>(null);
const importReview = ref<ImportResult | null>(null);
const importSourcePath = ref("");
const exportResult = ref<ExportResult | null>(null);
const feedback = ref<Feedback | null>(null);
const replaceConfirming = ref(false);
const clearConfirming = ref(false);
const deleteConfirming = ref(false);

const wordCount = computed(() =>
  vaultState.notes.reduce((total, note) => {
    const words = note.content.trim().match(/\S+/g);
    return total + (words?.length ?? 0);
  }, 0),
);

const enabledSnippetCount = computed(
  () => vaultState.snippets.filter((snippet) => snippet.enabled).length,
);

const vaultStats = computed(() => [
  { label: "Notes", value: vaultState.notes.length, icon: "notes" },
  { label: "Folders", value: vaultState.folders.length, icon: "folder" },
  { label: "Templates", value: vaultState.templates.length, icon: "templates" },
  { label: "Words", value: wordCount.value, icon: "edit" },
]);

const importFolderCount = computed(() => {
  if (!importReview.value) return 0;
  return new Set(
    importReview.value.notes
      .map((note) => note.folderPath)
      .filter((folderPath) => folderPath.trim().length > 0),
  ).size;
});

const importPreviewNotes = computed(() => importReview.value?.notes.slice(0, 5) ?? []);
const importPreviewOverflow = computed(() =>
  Math.max(0, (importReview.value?.notes.length ?? 0) - importPreviewNotes.value.length),
);

const exportPayload = computed(() => ({
  notes: vaultState.notes.length,
  templates: vaultState.templates.length,
  snippets: vaultState.snippets.length,
}));

function formatCount(value: number): string {
  return new Intl.NumberFormat().format(value);
}

function feedbackIcon(tone: FeedbackTone): string {
  if (tone === "success") return "check";
  if (tone === "error") return "x";
  return "info";
}

function errorMessage(error: unknown, fallback: string): string {
  if (typeof error === "string" && error.trim()) return error;
  if (error instanceof Error && error.message.trim()) return error.message;
  return fallback;
}

async function chooseVaultToImport(): Promise<void> {
  if (!nativeAvailable || activeTask.value) return;

  activeTask.value = "import";
  feedback.value = null;
  replaceConfirming.value = false;
  clearConfirming.value = false;
  deleteConfirming.value = false;

  try {
    const selectedPath = await pickFolder();
    if (!selectedPath) {
      feedback.value = {
        tone: "info",
        title: "Import cancelled",
        message: "Nothing changed in your vault.",
      };
      return;
    }

    importReview.value = null;
    importSourcePath.value = selectedPath;
    const result = await importObsidianVault(selectedPath);
    importReview.value = result;
    feedback.value = {
      tone: result.warnings.length ? "warning" : "info",
      title: "Import ready to review",
      message: `Obsidian At Home found ${formatCount(result.notes.length)} Markdown ${result.notes.length === 1 ? "note" : "notes"}. Choose how to bring them in.`,
    };
  } catch (error) {
    feedback.value = {
      tone: "error",
      title: "Could not read that vault",
      message: errorMessage(error, "The selected folder could not be imported."),
    };
  } finally {
    activeTask.value = null;
  }
}

function cancelImportReview(): void {
  importReview.value = null;
  importSourcePath.value = "";
  replaceConfirming.value = false;
  feedback.value = {
    tone: "info",
    title: "Import review closed",
    message: "Your current vault was not changed.",
  };
}

function applyImport(replace: boolean): void {
  if (activeTask.value) return;
  const result = importReview.value;
  if (!result) return;

  const { noteCount, saved } = mergeImportedVault(result, replace);
  const warnings = [...result.warnings];
  importReview.value = null;
  importSourcePath.value = "";
  replaceConfirming.value = false;
  clearConfirming.value = false;
  deleteConfirming.value = false;
  feedback.value = {
    tone: !saved || warnings.length ? "warning" : "success",
    title: saved
      ? replace ? "Vault replaced" : "Vault merged"
      : replace ? "Vault replaced, but not saved" : "Vault merged, but not saved",
    message: saved
      ? `${formatCount(noteCount)} Markdown ${noteCount === 1 ? "note was" : "notes were"} imported from ${result.vaultName || "the selected vault"}.`
      : `${formatCount(noteCount)} Markdown ${noteCount === 1 ? "note is" : "notes are"} open, but the change could not be saved. Keep the app open and try again.`,
    warnings,
  };
}

async function exportVault(): Promise<void> {
  if (!nativeAvailable || activeTask.value) return;

  activeTask.value = "export";
  feedback.value = null;
  exportResult.value = null;
  clearConfirming.value = false;
  deleteConfirming.value = false;

  try {
    const parentPath = await pickFolder();
    if (!parentPath) {
      feedback.value = {
        tone: "info",
        title: "Export cancelled",
        message: "No files were written.",
      };
      return;
    }

    const result = await exportObsidianVault(
      parentPath,
      vaultState.name,
      buildExportPayload(),
    );
    exportResult.value = result;
    feedback.value = {
      tone: result.warnings.length ? "warning" : "success",
      title: "Portable vault created",
      message: `Exported ${formatCount(result.noteCount)} Markdown ${result.noteCount === 1 ? "note" : "notes"} to a new folder.`,
      warnings: result.warnings,
    };
  } catch (error) {
    feedback.value = {
      tone: "error",
      title: "Could not export the vault",
      message: errorMessage(error, "The destination folder could not be written."),
    };
  } finally {
    activeTask.value = null;
  }
}

function resetTransferState(): void {
  importReview.value = null;
  importSourcePath.value = "";
  exportResult.value = null;
  replaceConfirming.value = false;
}

function clearCurrentVault(): void {
  if (activeTask.value) return;
  const saved = clearVault();
  resetTransferState();
  clearConfirming.value = false;
  deleteConfirming.value = false;
  feedback.value = {
    tone: saved ? "success" : "warning",
    title: saved ? "Vault cleared" : "Vault cleared, but not saved",
    message: saved
      ? "All notes and folders were removed. Templates and CSS snippets were kept."
      : "The current view is empty, but the change could not be saved. Keep the app open and try again.",
  };
}

function deleteCurrentVault(): void {
  if (activeTask.value) return;
  const deletedName = vaultState.name;
  const saved = deleteVault();
  resetTransferState();
  clearConfirming.value = false;
  deleteConfirming.value = false;
  feedback.value = {
    tone: saved ? "success" : "warning",
    title: saved ? "Vault deleted" : "Vault deleted, but not saved",
    message: saved
      ? `${deletedName} was erased. A new empty vault with the built-in templates and CSS snippets is ready.`
      : "A new empty vault is open, but the change could not be saved. Keep the app open and try again.",
  };
}
</script>

<template>
  <main class="settings-view">
    <header class="settings-hero">
      <div class="settings-hero__copy">
        <span class="settings-eyebrow">Preferences &amp; portability</span>
        <h1 class="settings-hero__title">Settings</h1>
        <p class="settings-hero__description">
          Review vault details, import or export Markdown, and manage your data.
        </p>
      </div>
    </header>

    <section class="settings-section settings-section--overview" aria-labelledby="settings-overview-title">
      <div class="settings-section__heading">
        <div>
          <span class="settings-eyebrow">Current vault</span>
          <h2 id="settings-overview-title" class="settings-section__title">{{ vaultState.name }}</h2>
        </div>
        <span class="settings-section__meta">
          {{ enabledSnippetCount }} of {{ vaultState.snippets.length }} CSS snippets enabled
        </span>
      </div>

      <div class="settings-stat-grid">
        <article v-for="stat in vaultStats" :key="stat.label" class="settings-stat-card">
          <span class="settings-stat-card__icon">
            <AppIcon :name="stat.icon" :size="18" />
          </span>
          <strong class="settings-stat-card__value">{{ formatCount(stat.value) }}</strong>
          <span class="settings-stat-card__label">{{ stat.label }}</span>
        </article>
      </div>
    </section>

    <Transition name="collapse-fade">
      <div
        v-if="feedback"
        class="settings-feedback"
        :class="`settings-feedback--${feedback.tone}`"
        :role="feedback.tone === 'error' ? 'alert' : 'status'"
        aria-live="polite"
      >
        <span class="settings-feedback__icon">
          <AppIcon :name="feedbackIcon(feedback.tone)" :size="18" />
        </span>
        <div class="settings-feedback__body">
          <strong class="settings-feedback__title">{{ feedback.title }}</strong>
          <p class="settings-feedback__message">{{ feedback.message }}</p>
          <details v-if="feedback.warnings?.length" class="settings-warning-details">
            <summary>
              {{ feedback.warnings.length }}
              {{ feedback.warnings.length === 1 ? "warning" : "warnings" }}
            </summary>
            <ul class="settings-warning-list">
              <li v-for="(warning, index) in feedback.warnings" :key="`${index}-${warning}`">
                {{ warning }}
              </li>
            </ul>
          </details>
        </div>
        <button
          type="button"
          class="settings-icon-button"
          aria-label="Dismiss message"
          @click="feedback = null"
        >
          <AppIcon name="x" :size="16" />
        </button>
      </div>
    </Transition>

    <section class="settings-section settings-section--transfer" aria-labelledby="settings-transfer-title">
      <div class="settings-section__heading">
        <div>
          <span class="settings-eyebrow">Obsidian import and export</span>
          <h2 id="settings-transfer-title" class="settings-section__title">Portable Markdown</h2>
          <p class="settings-section__description">
            Import an Obsidian vault or export one that Obsidian can open directly.
            Wiki links and Markdown remain plain text.
          </p>
        </div>
      </div>

      <div v-if="!nativeAvailable" class="settings-desktop-notice" role="note">
        <AppIcon name="info" :size="16" />
        <p>Import and export are available in the desktop app.</p>
      </div>

      <div class="settings-transfer-grid">
        <article class="settings-transfer-card settings-transfer-card--import">
          <div class="settings-transfer-card__header">
            <span class="settings-transfer-card__icon">
              <AppIcon name="import" :size="21" />
            </span>
            <div>
              <h3 class="settings-transfer-card__title">Import from Obsidian</h3>
              <p class="settings-transfer-card__description">
                Import Markdown notes, folders, frontmatter tags, and CSS snippets.
              </p>
            </div>
          </div>

          <ul class="settings-feature-list" aria-label="Imported content">
            <li><AppIcon name="check" :size="14" /> Original Markdown content</li>
            <li><AppIcon name="check" :size="14" /> Nested note folders</li>
            <li><AppIcon name="check" :size="14" /> Enabled CSS snippet state</li>
          </ul>

          <button
            type="button"
            class="settings-button settings-button--primary settings-button--full"
            :disabled="!nativeAvailable || activeTask !== null"
            @click="chooseVaultToImport"
          >
            <AppIcon :name="activeTask === 'import' ? 'refresh' : 'folder-open'" :size="17" />
            {{ activeTask === "import" ? "Reading vault…" : "Choose Obsidian vault" }}
          </button>
        </article>

        <article class="settings-transfer-card settings-transfer-card--export">
          <div class="settings-transfer-card__header">
            <span class="settings-transfer-card__icon">
              <AppIcon name="export" :size="21" />
            </span>
            <div>
              <h3 class="settings-transfer-card__title">Export portable vault</h3>
              <p class="settings-transfer-card__description">
                Choose a parent folder. Obsidian At Home creates a new vault inside it
                without overwriting existing files.
              </p>
            </div>
          </div>

          <div class="settings-export-summary" aria-label="Items ready to export">
            <span><strong>{{ formatCount(exportPayload.notes) }}</strong> notes</span>
            <span><strong>{{ formatCount(exportPayload.templates) }}</strong> templates</span>
            <span><strong>{{ formatCount(exportPayload.snippets) }}</strong> snippets</span>
          </div>

          <button
            type="button"
            class="settings-button settings-button--secondary settings-button--full"
            :disabled="!nativeAvailable || activeTask !== null"
            @click="exportVault"
          >
            <AppIcon :name="activeTask === 'export' ? 'refresh' : 'export'" :size="17" />
            {{ activeTask === "export" ? "Creating vault…" : "Choose export destination" }}
          </button>

          <Transition name="collapse-fade">
            <div v-if="exportResult" class="settings-export-receipt">
              <span class="settings-export-receipt__icon">
                <AppIcon name="check" :size="16" />
              </span>
              <div>
                <strong>Saved as a new folder</strong>
                <code class="settings-export-receipt__path">{{ exportResult.path }}</code>
                <span class="settings-export-receipt__counts">
                  {{ exportResult.noteCount }} notes · {{ exportResult.templateCount }} templates ·
                  {{ exportResult.snippetCount }} snippets
                </span>
              </div>
            </div>
          </Transition>
        </article>
      </div>

      <Transition name="workspace-fade">
        <article
          v-if="importReview"
          class="settings-import-review"
          aria-labelledby="settings-import-review-title"
        >
        <div class="settings-import-review__header">
          <div class="settings-import-review__identity">
            <span class="settings-import-review__icon">
              <AppIcon name="archive" :size="21" />
            </span>
            <div>
              <span class="settings-eyebrow">Ready to import</span>
              <h3 id="settings-import-review-title" class="settings-import-review__title">
                {{ importReview.vaultName || "Obsidian vault" }}
              </h3>
              <code class="settings-import-review__path">{{ importSourcePath }}</code>
            </div>
          </div>
          <button
            type="button"
            class="settings-icon-button"
            aria-label="Close import review"
            @click="cancelImportReview"
          >
            <AppIcon name="x" :size="17" />
          </button>
        </div>

        <div class="settings-import-review__stats">
          <div class="settings-import-review__stat">
            <strong>{{ formatCount(importReview.notes.length) }}</strong>
            <span>Markdown notes</span>
          </div>
          <div class="settings-import-review__stat">
            <strong>{{ formatCount(importFolderCount) }}</strong>
            <span>Nested folders</span>
          </div>
          <div class="settings-import-review__stat">
            <strong>{{ formatCount(importReview.snippets.length) }}</strong>
            <span>CSS snippets</span>
          </div>
        </div>

        <div v-if="importPreviewNotes.length" class="settings-import-preview">
          <span class="settings-import-preview__label">Files found</span>
          <ul class="settings-import-preview__list">
            <li v-for="note in importPreviewNotes" :key="note.relativePath">
              <AppIcon name="file-text" :size="14" />
              <span>{{ note.relativePath }}</span>
            </li>
          </ul>
          <span v-if="importPreviewOverflow" class="settings-import-preview__overflow">
            +{{ formatCount(importPreviewOverflow) }} more
          </span>
        </div>

        <div v-else class="settings-import-empty" role="note">
          <AppIcon name="info" :size="17" />
          No Markdown notes were found. You can still import any discovered CSS snippets.
        </div>

        <details v-if="importReview.warnings.length" class="settings-warning-details settings-warning-details--review">
          <summary>
            Review {{ importReview.warnings.length }}
            {{ importReview.warnings.length === 1 ? "warning" : "warnings" }}
          </summary>
          <ul class="settings-warning-list">
            <li v-for="(warning, index) in importReview.warnings" :key="`${index}-${warning}`">
              {{ warning }}
            </li>
          </ul>
        </details>

        <div class="settings-import-choice">
          <div class="settings-import-choice__copy">
            <strong>Choose an import method</strong>
            <p>
              Merge keeps everything already here. Replace clears the current notes and
              folder tree first; your templates and existing CSS snippets stay available.
            </p>
          </div>
          <div class="settings-import-choice__actions">
            <button
              type="button"
              class="settings-button settings-button--primary"
              :disabled="activeTask !== null"
              @click="applyImport(false)"
            >
              <AppIcon name="plus" :size="16" />
              Merge with vault
            </button>
            <button
              type="button"
              class="settings-button settings-button--danger-ghost"
              :disabled="activeTask !== null"
              @click="replaceConfirming = true"
            >
              <AppIcon name="refresh" :size="16" />
              Replace notes &amp; folders
            </button>
          </div>
        </div>

        <Transition name="collapse-fade">
          <div v-if="replaceConfirming" class="settings-confirmation settings-confirmation--danger" role="alert">
            <span class="settings-confirmation__icon">
              <AppIcon name="info" :size="18" />
            </span>
            <div class="settings-confirmation__copy">
              <strong>Replace the current note collection?</strong>
              <p>This removes {{ formatCount(vaultState.notes.length) }} current notes before importing.</p>
            </div>
            <div class="settings-confirmation__actions">
              <button
                type="button"
                class="settings-button settings-button--danger"
                :disabled="activeTask !== null"
                @click="applyImport(true)"
              >
                Yes, replace
              </button>
              <button
                type="button"
                class="settings-button settings-button--quiet"
                @click="replaceConfirming = false"
              >
                Cancel
              </button>
            </div>
          </div>
        </Transition>
        </article>
      </Transition>
    </section>

    <section class="settings-section settings-section--danger" aria-labelledby="settings-clear-title">
      <div class="settings-danger-row">
        <div class="settings-danger-row__icon">
          <AppIcon name="archive" :size="19" />
        </div>
        <div class="settings-danger-row__copy">
          <span class="settings-eyebrow">Vault content</span>
          <h2 id="settings-clear-title" class="settings-section__title">Clear vault</h2>
          <p class="settings-section__description">
            Remove every note, folder, and tag. The vault name, templates, and CSS snippets stay.
          </p>
        </div>
        <button
          type="button"
          class="settings-button settings-button--danger-ghost"
          :disabled="activeTask !== null"
          @click="clearConfirming = true; deleteConfirming = false"
        >
          Clear vault
        </button>
      </div>

      <Transition name="collapse-fade">
        <div v-if="clearConfirming" class="settings-confirmation settings-confirmation--danger" role="alert">
          <span class="settings-confirmation__icon">
            <AppIcon name="info" :size="18" />
          </span>
          <div class="settings-confirmation__copy">
            <strong>Clear {{ formatCount(vaultState.notes.length) }} notes and {{ formatCount(vaultState.folders.length) }} folders?</strong>
            <p>Templates and CSS snippets will stay. Export first if you need a backup.</p>
          </div>
          <div class="settings-confirmation__actions">
            <button
              type="button"
              class="settings-button settings-button--danger"
              :disabled="activeTask !== null"
              @click="clearCurrentVault"
            >
              Yes, clear vault
            </button>
            <button type="button" class="settings-button settings-button--quiet" @click="clearConfirming = false">
              Cancel
            </button>
          </div>
        </div>
      </Transition>

      <div class="settings-danger-divider" />

      <div class="settings-danger-row">
        <div class="settings-danger-row__icon">
          <AppIcon name="trash" :size="19" />
        </div>
        <div class="settings-danger-row__copy">
          <span class="settings-eyebrow">Entire vault</span>
          <h2 class="settings-section__title">Delete vault</h2>
          <p class="settings-section__description">
            Erase this vault and its custom templates and snippets. A new empty vault will keep only the built-in examples.
          </p>
        </div>
        <button
          type="button"
          class="settings-button settings-button--danger-ghost"
          :disabled="activeTask !== null"
          @click="deleteConfirming = true; clearConfirming = false"
        >
          Delete vault
        </button>
      </div>

      <Transition name="collapse-fade">
        <div v-if="deleteConfirming" class="settings-confirmation settings-confirmation--danger" role="alert">
          <span class="settings-confirmation__icon">
            <AppIcon name="info" :size="18" />
          </span>
          <div class="settings-confirmation__copy">
            <strong>Delete “{{ vaultState.name }}”?</strong>
            <p>Notes, folders, and custom templates and snippets will be erased. Imported source folders and exported folders are not affected.</p>
          </div>
          <div class="settings-confirmation__actions">
            <button
              type="button"
              class="settings-button settings-button--danger"
              :disabled="activeTask !== null"
              @click="deleteCurrentVault"
            >
              Yes, delete vault
            </button>
            <button type="button" class="settings-button settings-button--quiet" @click="deleteConfirming = false">
              Cancel
            </button>
          </div>
        </div>
      </Transition>
    </section>
  </main>
</template>
