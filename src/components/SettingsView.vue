<script setup lang="ts">
import { computed, ref } from "vue";
import type { ExportResult, ImportResult } from "../types";
import {
  buildExportPayload,
  mergeImportedVault,
  resetDemoVault,
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
const resetConfirming = ref(false);

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

  try {
    const selectedPath = await pickFolder();
    if (!selectedPath) {
      feedback.value = {
        tone: "info",
        title: "Import cancelled",
        message: "Nothing changed in your notebook.",
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
    message: "Your current notebook was not changed.",
  };
}

function applyImport(replace: boolean): void {
  const result = importReview.value;
  if (!result) return;

  const noteCount = mergeImportedVault(result, replace);
  const warnings = [...result.warnings];
  importReview.value = null;
  importSourcePath.value = "";
  replaceConfirming.value = false;
  resetConfirming.value = false;
  feedback.value = {
    tone: warnings.length ? "warning" : "success",
    title: replace ? "Notebook replaced" : "Vault merged",
    message: `${formatCount(noteCount)} Markdown ${noteCount === 1 ? "note was" : "notes were"} imported from ${result.vaultName || "the selected vault"}.`,
    warnings,
  };
}

async function exportVault(): Promise<void> {
  if (!nativeAvailable || activeTask.value) return;

  activeTask.value = "export";
  feedback.value = null;
  exportResult.value = null;

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
      title: "Could not export the notebook",
      message: errorMessage(error, "The destination folder could not be written."),
    };
  } finally {
    activeTask.value = null;
  }
}

function restoreDemoVault(): void {
  resetDemoVault();
  importReview.value = null;
  importSourcePath.value = "";
  exportResult.value = null;
  replaceConfirming.value = false;
  resetConfirming.value = false;
  feedback.value = {
    tone: "success",
    title: "Demo notebook restored",
    message: "The original Obsidian At Home example notes, folders, templates, and snippets are back.",
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
          Review notebook details, import or export Markdown, and manage the local
          data stored by Obsidian At Home.
        </p>
      </div>

      <div
        class="settings-runtime-badge"
        :class="nativeAvailable ? 'settings-runtime-badge--native' : 'settings-runtime-badge--web'"
      >
        <span class="settings-runtime-badge__dot" aria-hidden="true" />
        {{ nativeAvailable ? "Tauri desktop" : "Browser preview" }}
      </div>
    </header>

    <section class="settings-section settings-section--overview" aria-labelledby="settings-overview-title">
      <div class="settings-section__heading">
        <div>
          <span class="settings-eyebrow">Current notebook</span>
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

    <section class="settings-section settings-section--runtime" aria-labelledby="settings-runtime-title">
      <div class="settings-runtime-intro">
        <span class="settings-runtime-intro__icon">
          <AppIcon name="sparkles" :size="20" />
        </span>
        <div>
          <span class="settings-eyebrow">Desktop app</span>
          <h2 id="settings-runtime-title" class="settings-section__title">A native shell, not Electron</h2>
          <p class="settings-section__description">
            Obsidian At Home uses Vue for its interface, Tauri and Rust for its desktop
            shell, and your system WebView for rendering. It has no bundled Chromium
            runtime, account, or sync service.
          </p>
        </div>
      </div>

      <dl class="settings-runtime-specs">
        <div class="settings-runtime-spec">
          <dt>Interface</dt>
          <dd>Vue 3</dd>
        </div>
        <div class="settings-runtime-spec">
          <dt>Desktop shell</dt>
          <dd>Tauri 2</dd>
        </div>
        <div class="settings-runtime-spec">
          <dt>Native core</dt>
          <dd>Rust</dd>
        </div>
        <div class="settings-runtime-spec">
          <dt>Storage</dt>
          <dd>Local only</dd>
        </div>
      </dl>

      <div v-if="!nativeAvailable" class="settings-native-notice" role="note">
        <AppIcon name="info" :size="17" />
        <p>
          Folder import and export are available in the installed desktop app. This
          browser preview still saves edits locally for interface development.
        </p>
      </div>
    </section>

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
        </article>
      </div>

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
          <span class="settings-import-preview__label">Sample files</span>
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
              @click="applyImport(false)"
            >
              <AppIcon name="plus" :size="16" />
              Merge with notebook
            </button>
            <button
              type="button"
              class="settings-button settings-button--danger-ghost"
              @click="replaceConfirming = true"
            >
              <AppIcon name="refresh" :size="16" />
              Replace notes &amp; folders
            </button>
          </div>
        </div>

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
      </article>
    </section>

    <section class="settings-section settings-section--danger" aria-labelledby="settings-data-title">
      <div class="settings-danger-row">
        <div class="settings-danger-row__icon">
          <AppIcon name="refresh" :size="19" />
        </div>
        <div class="settings-danger-row__copy">
          <span class="settings-eyebrow">Local data</span>
          <h2 id="settings-data-title" class="settings-section__title">Restore demo notebook</h2>
          <p class="settings-section__description">
            Replace this notebook with the original Obsidian At Home sample content. Export first
            if there is anything you want to keep.
          </p>
        </div>
        <button
          v-if="!resetConfirming"
          type="button"
          class="settings-button settings-button--danger-ghost"
          :disabled="activeTask !== null"
          @click="resetConfirming = true"
        >
          Restore demo
        </button>
      </div>

      <div v-if="resetConfirming" class="settings-confirmation settings-confirmation--danger" role="alert">
        <span class="settings-confirmation__icon">
          <AppIcon name="info" :size="18" />
        </span>
        <div class="settings-confirmation__copy">
          <strong>Replace all local notebook data?</strong>
          <p>
            This resets notes, folders, templates, and snippets. It cannot be undone unless
            you have exported a copy.
          </p>
        </div>
        <div class="settings-confirmation__actions">
          <button
            type="button"
            class="settings-button settings-button--danger"
            @click="restoreDemoVault"
          >
            Replace with demo
          </button>
          <button
            type="button"
            class="settings-button settings-button--quiet"
            @click="resetConfirming = false"
          >
            Keep my notebook
          </button>
        </div>
      </div>
    </section>
  </main>
</template>
