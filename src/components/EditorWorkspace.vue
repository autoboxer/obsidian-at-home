<script setup lang="ts">
import { openUrl } from "@tauri-apps/plugin-opener";
import { computed, nextTick, ref, watch } from "vue";
import { leadingFrontmatterEnd } from "../lib/frontmatter";
import {
  findMarkdownHeading,
  parseMarkdownHeadingTarget,
} from "../lib/headingLinks";
import { resolveWikiLink } from "../lib/wikiLinks";
import {
  editorPositionVaultId,
  getNoteEditorPosition,
  setNoteEditorPosition,
} from "../stores/editorPositions";
import {
  activeNote,
  backNavigationNote,
  canNavigateBack,
  canNavigateForward,
  createFolder,
  createLinkedNote,
  createNote,
  deleteNote,
  folderPath,
  forwardNavigationNote,
  navigateBack,
  navigateForward,
  notify,
  selectNote,
  togglePinned,
  uiState,
  updateNote,
  vaultSession,
  vaultState,
} from "../stores/vault";
import type { NoteEditorPosition } from "../types";
import AppIcon from "./AppIcon.vue";
import SourceEditor from "./SourceEditor.vue";

const tagInputOpen = ref(false);
const tagInput = ref("");
const tagField = ref<HTMLInputElement>();
const tagSuggestionIndex = ref(-1);
const noteMenuOpen = ref(false);
const quickFolderOpen = ref(false);
const quickFolderName = ref("");
const quickFolderField = ref<HTMLInputElement>();
const quickFolderButton = ref<HTMLButtonElement>();
const sourceEditor = ref<{
  focusDocumentOffset: (offset: number) => boolean;
}>();

const noteTitles = computed(() => vaultState.notes.map((note) => note.title));
const positionVaultId = computed(() => editorPositionVaultId(vaultSession.backend, vaultSession.path));
const editorKey = computed(() => JSON.stringify([
  positionVaultId.value,
  activeNote.value?.id ?? null,
]));
const sortedFolders = computed(() => [...vaultState.folders].sort((a, b) => folderPath(a.id).localeCompare(folderPath(b.id))));
const tagSuggestions = computed(() => {
  const query = normalizeTag(tagInput.value).toLocaleLowerCase();
  const appliedTags = new Set(activeNote.value?.tags.map((tag) => tag.toLocaleLowerCase()) ?? []);
  const uniqueTags = new Map<string, string>();

  for (const note of vaultState.notes) {
    for (const tag of note.tags) {
      if (!uniqueTags.has(tag.toLocaleLowerCase())) {
        uniqueTags.set(tag.toLocaleLowerCase(), tag);
      }
    }
  }

  return [...uniqueTags.values()]
    .filter((tag) => !appliedTags.has(tag.toLocaleLowerCase()))
    .filter((tag) => !query || tag.toLocaleLowerCase().includes(query))
    .sort((a, b) => {
      const aStartsWithQuery = query && a.toLocaleLowerCase().startsWith(query) ? 0 : 1;
      const bStartsWithQuery = query && b.toLocaleLowerCase().startsWith(query) ? 0 : 1;

      return aStartsWithQuery - bStartsWithQuery || a.localeCompare(b);
    })
    .slice(0, 7);
});
const wordCount = computed(() => {
  const content = activeNote.value?.content ?? "";

  return content.trim() ? content.trim().split(/\s+/).length : 0;
});
const characterCount = computed(() => activeNote.value?.content.length ?? 0);
const hasFrontmatter = computed(() => (
  activeNote.value
    ? leadingFrontmatterEnd(activeNote.value.content) !== undefined
    : false
));
const backNavigationLabel = computed(() => backNavigationNote.value
  ? `Back to “${backNavigationNote.value.title.trim() || "Untitled note"}”`
  : "No previous note");
const forwardNavigationLabel = computed(() => forwardNavigationNote.value
  ? `Forward to “${forwardNavigationNote.value.title.trim() || "Untitled note"}”`
  : "No next note");

function setTitle(event: Event): void {
  if (!activeNote.value) {
    return;
  }
  updateNote(activeNote.value.id, { title: (event.target as HTMLInputElement).value });
}

function setContent(content: string): void {
  if (activeNote.value) {
    updateNote(activeNote.value.id, { content });
  }
}

function savedEditorPosition(noteId: string, content: string): NoteEditorPosition | undefined {
  return getNoteEditorPosition(positionVaultId.value, noteId, content);
}

function rememberEditorPosition(
  vaultId: string,
  noteId: string,
  position: NoteEditorPosition,
): void {
  if (
    vaultId === positionVaultId.value
    && !vaultState.notes.some((note) => note.id === noteId)
  ) {
    return;
  }

  setNoteEditorPosition(vaultId, noteId, position);
}

async function openRenderedLink(href: string): Promise<void> {
  const headingTarget = parseMarkdownHeadingTarget(href);
  if (headingTarget) {
    await openHeadingLink(headingTarget.noteTarget, headingTarget.heading);

    return;
  }

  try {
    await openUrl(href);
  } catch {
    notify("Could not open that link", "warning");
  }
}

async function openWikiLink(target: string, heading?: string): Promise<void> {
  if (!heading) {
    createLinkedNote(target);

    return;
  }

  await openHeadingLink(target, heading);
}

async function openHeadingLink(target: string, heading: string): Promise<void> {
  const note = resolveWikiLink(target, vaultState.notes, activeNote.value);
  if (!note) {
    notify(`Could not find note “${linkedNoteLabel(target)}”`, "warning");

    return;
  }

  const match = findMarkdownHeading(note.content, heading);
  if (!match) {
    notify(`Could not find heading “${heading}” in “${note.title}”`, "warning");

    return;
  }

  if (note.id !== activeNote.value?.id) {
    selectNote(note.id);
    await nextTick();
  }

  if (!sourceEditor.value?.focusDocumentOffset(match.contentFrom)) {
    notify("Could not move to that heading", "warning");
  }
}

function linkedNoteLabel(target: string): string {
  return target.trim().replace(/\.md$/i, "") || "current note";
}

function setFolder(event: Event): void {
  if (!activeNote.value) {
    return;
  }
  const value = (event.target as HTMLSelectElement).value;
  updateNote(activeNote.value.id, { folderId: value || null });
}

function openTagInput(): void {
  tagInput.value = "";
  tagSuggestionIndex.value = -1;
  tagInputOpen.value = true;
  nextTick(() => {
    tagField.value?.focus();
    tagField.value?.select();
  });
}

function normalizeTag(value: string): string {
  return value.trim().replace(/^#/, "").replace(/\s+/g, "-");
}

function addTag(suggestedTag?: string): void {
  if (!activeNote.value) {
    return;
  }

  const tag = normalizeTag(suggestedTag ?? tagInput.value);
  const alreadyApplied = activeNote.value.tags.some(
    (candidate) => candidate.toLocaleLowerCase() === tag.toLocaleLowerCase(),
  );
  if (tag && !alreadyApplied) {
    updateNote(activeNote.value.id, { tags: [...activeNote.value.tags, tag] });
  }

  tagInput.value = "";
  tagSuggestionIndex.value = -1;
  tagInputOpen.value = false;
}

function submitTag(): void {
  const suggestion = tagSuggestionIndex.value >= 0
    ? tagSuggestions.value[tagSuggestionIndex.value]
    : undefined;
  addTag(suggestion);
}

function cancelTagInput(): void {
  tagInput.value = "";
  tagSuggestionIndex.value = -1;
  tagInputOpen.value = false;
}

function handleTagKeydown(event: KeyboardEvent): void {
  if (event.key === "ArrowDown" && tagSuggestions.value.length) {
    event.preventDefault();
    tagSuggestionIndex.value = (tagSuggestionIndex.value + 1) % tagSuggestions.value.length;
  } else if (event.key === "ArrowUp" && tagSuggestions.value.length) {
    event.preventDefault();
    tagSuggestionIndex.value = tagSuggestionIndex.value <= 0
      ? tagSuggestions.value.length - 1
      : tagSuggestionIndex.value - 1;
  }
}

function removeTag(tag: string): void {
  if (activeNote.value) {
    updateNote(activeNote.value.id, { tags: activeNote.value.tags.filter((candidate) => candidate !== tag) });
  }
}

function requestDelete(): void {
  if (!activeNote.value) {
    return;
  }
  noteMenuOpen.value = false;
  if (window.confirm(`Delete “${activeNote.value.title || "Untitled note"}”? It will remain in Recently Deleted for seven days.`)) {
    void deleteNote(activeNote.value.id);
  }
}

function toggleFrontmatter(): void {
  uiState.frontmatterVisible = !uiState.frontmatterVisible;
}

function openQuickFolder(): void {
  quickFolderOpen.value = true;
  nextTick(() => quickFolderField.value?.focus());
}

function closeQuickFolder(restoreFocus = false): void {
  quickFolderOpen.value = false;
  quickFolderName.value = "";
  if (restoreFocus) {
    nextTick(() => quickFolderButton.value?.focus());
  }
}

function submitQuickFolder(): void {
  const name = quickFolderName.value.trim();
  if (name) {
    createFolder(name);
  }
  closeQuickFolder(true);
}

function handleQuickFolderFocusOut(event: FocusEvent): void {
  const form = event.currentTarget as HTMLElement;
  const next = event.relatedTarget;
  if (!(next instanceof Node) || !form.contains(next)) {
    closeQuickFolder();
  }
}

watch(
  () => uiState.explorerOpen,
  (open) => {
    if (open) {
      closeQuickFolder();
    }
  },
);

watch(
  () => activeNote.value?.id,
  () => {
    noteMenuOpen.value = false;
  },
);

watch(tagInput, () => {
  tagSuggestionIndex.value = -1;
});
</script>

<template>
  <main class="editor-workspace" data-ui-region="editor" data-editor-view="live">
    <header class="editor-toolbar">
      <div class="editor-crumbs">
        <button
          class="icon-button explorer-toggle"
          type="button"
          :class="{ active: uiState.explorerOpen }"
          :title="uiState.explorerOpen ? 'Hide vault panel' : 'Show vault panel'"
          :aria-label="uiState.explorerOpen ? 'Hide vault panel' : 'Show vault panel'"
          :aria-pressed="uiState.explorerOpen"
          @click="uiState.explorerOpen = !uiState.explorerOpen"
        >
          <AppIcon name="sidebar" :size="17" />
        </button>
        <div v-if="activeNote" class="note-navigation" role="group" aria-label="Note history">
          <button
            class="icon-button subtle"
            type="button"
            :disabled="!canNavigateBack"
            :title="backNavigationLabel"
            :aria-label="backNavigationLabel"
            @click="navigateBack"
          >
            <AppIcon name="history-back" :size="15" />
          </button>
          <button
            class="icon-button subtle"
            type="button"
            :disabled="!canNavigateForward"
            :title="forwardNavigationLabel"
            :aria-label="forwardNavigationLabel"
            @click="navigateForward"
          >
            <AppIcon name="history-forward" :size="15" />
          </button>
        </div>
        <Transition name="chip-swap">
          <div v-if="!uiState.explorerOpen" class="vault-hidden-actions">
            <button
              type="button"
              class="icon-button subtle"
              aria-label="Create note"
              title="Create note · ⌘N"
              @click="createNote()"
            >
              <AppIcon name="file-plus" :size="15" />
            </button>
            <div class="menu-anchor">
              <button
                ref="quickFolderButton"
                type="button"
                class="icon-button subtle"
                aria-label="Create folder"
                title="Create folder"
                :aria-expanded="quickFolderOpen"
                @mousedown.prevent
                @click="quickFolderOpen ? closeQuickFolder(true) : openQuickFolder()"
              >
                <AppIcon name="folder-plus" :size="15" />
              </button>
              <Transition name="popover-fade">
                <form
                  v-if="quickFolderOpen"
                  class="popover-menu quick-folder-popover"
                  @submit.prevent="submitQuickFolder"
                  @focusout="handleQuickFolderFocusOut"
                  @keydown.esc.prevent="closeQuickFolder(true)"
                >
                  <strong>New folder</strong>
                  <div class="quick-folder-entry">
                    <input
                      ref="quickFolderField"
                      v-model="quickFolderName"
                      type="text"
                      maxlength="120"
                      autocomplete="off"
                      aria-label="Folder name"
                      placeholder="Folder name"
                    />
                    <button type="submit" :disabled="!quickFolderName.trim()" aria-label="Create folder">
                      <AppIcon name="arrow" :size="14" />
                    </button>
                  </div>
                </form>
              </Transition>
            </div>
          </div>
        </Transition>
        <template v-if="activeNote">
          <span class="crumb-vault">{{ vaultState.name }}</span>
          <AppIcon name="chevron" :size="12" />
          <span v-if="activeNote.folderId" class="crumb-folder">{{ folderPath(activeNote.folderId) }}</span>
          <AppIcon v-if="activeNote.folderId" name="chevron" :size="12" />
          <span class="crumb-note">{{ activeNote.title || "Untitled note" }}</span>
        </template>
      </div>

      <div v-if="activeNote" class="editor-toolbar-actions">
        <button
          class="icon-button"
          type="button"
          :class="{ active: activeNote.pinned }"
          :aria-label="activeNote.pinned ? 'Remove from favorites' : 'Favorite'"
          :title="activeNote.pinned ? 'Remove from favorites' : 'Favorite'"
          @click="togglePinned(activeNote.id)"
        >
          <AppIcon name="star" :size="16" />
        </button>
        <button
          v-if="hasFrontmatter || uiState.frontmatterVisible"
          class="icon-button frontmatter-toggle"
          type="button"
          :class="{ active: uiState.frontmatterVisible }"
          :aria-label="uiState.frontmatterVisible ? 'Hide frontmatter' : 'Show frontmatter'"
          :title="uiState.frontmatterVisible ? 'Hide frontmatter' : 'Show frontmatter'"
          :aria-pressed="uiState.frontmatterVisible"
          @click="toggleFrontmatter"
        >
          <AppIcon name="code" :size="16" />
        </button>
        <button
          class="icon-button context-toggle"
          type="button"
          :class="{ active: uiState.contextOpen }"
          :aria-label="uiState.contextOpen ? 'Hide note context' : 'Show note context'"
          :title="uiState.contextOpen ? 'Hide note context' : 'Show note context'"
          @click="uiState.contextOpen = !uiState.contextOpen"
        >
          <AppIcon name="panel-right" :size="17" />
        </button>
        <div class="menu-anchor">
          <button class="icon-button" type="button" title="More actions" @click="noteMenuOpen = !noteMenuOpen">
            <AppIcon name="more" :size="18" />
          </button>
          <Transition name="popover-fade">
            <div v-if="noteMenuOpen" class="popover-menu compact-menu">
              <button type="button" class="danger" @click="requestDelete">
                <AppIcon name="trash" :size="15" /> Delete note
              </button>
            </div>
          </Transition>
        </div>
      </div>
    </header>

    <template v-if="activeNote">
      <section class="editor-document">
        <div class="document-heading">
          <input
            class="note-title-input"
            data-ui-region="note-title"
            :value="activeNote.title"
            aria-label="Note title"
            placeholder="Untitled note"
            @input="setTitle"
          />
          <div class="note-properties">
            <label class="property-control folder-property">
              <AppIcon name="folder" :size="14" />
              <select :value="activeNote.folderId ?? ''" aria-label="Move note to folder" @change="setFolder">
                <option value="">Vault root</option>
                <option v-for="folder in sortedFolders" :key="folder.id" :value="folder.id">
                  {{ folderPath(folder.id) }}
                </option>
              </select>
            </label>

            <span v-for="tag in activeNote.tags" :key="tag" class="tag-chip">
              <span>#</span>{{ tag }}
              <button type="button" :aria-label="`Remove ${tag} tag`" @click="removeTag(tag)">
                <AppIcon name="x" :size="10" />
              </button>
            </span>
            <Transition name="chip-swap" mode="out-in">
              <form v-if="tagInputOpen" key="tag-input" class="inline-tag-form" @submit.prevent="submitTag">
                <span>#</span>
                <input
                  ref="tagField"
                  v-model="tagInput"
                  placeholder="tag"
                  autocomplete="off"
                  autocapitalize="none"
                  autocorrect="off"
                  spellcheck="false"
                  role="combobox"
                  aria-autocomplete="list"
                  aria-controls="tag-suggestions"
                  :aria-expanded="tagSuggestions.length > 0"
                  :aria-activedescendant="tagSuggestionIndex >= 0 ? `tag-suggestion-${tagSuggestionIndex}` : undefined"
                  @blur="addTag()"
                  @keydown="handleTagKeydown"
                  @keydown.esc.prevent="cancelTagInput"
                />
                <div v-if="tagSuggestions.length" id="tag-suggestions" class="tag-suggestions" role="listbox">
                  <button
                    v-for="(tag, index) in tagSuggestions"
                    :id="`tag-suggestion-${index}`"
                    :key="tag"
                    type="button"
                    role="option"
                    :aria-selected="index === tagSuggestionIndex"
                    :class="{ active: index === tagSuggestionIndex }"
                    @mouseenter="tagSuggestionIndex = index"
                    @mousedown.prevent="addTag(tag)"
                  >
                    <span>#</span>{{ tag }}
                  </button>
                </div>
              </form>
              <button v-else key="tag-button" type="button" class="add-tag-button" @click="openTagInput">
                <AppIcon name="plus" :size="12" /> Add tag
              </button>
            </Transition>
          </div>
        </div>

        <div class="editor-canvas" data-editor-pane="live">
          <SourceEditor
            :key="editorKey"
            ref="sourceEditor"
            :initial-position="savedEditorPosition(activeNote.id, activeNote.content)"
            :model-value="activeNote.content"
            :note-id="activeNote.id"
            :note-titles="noteTitles"
            :show-frontmatter="uiState.frontmatterVisible"
            :vault-id="positionVaultId"
            @editor-position="rememberEditorPosition"
            @open-link="openRenderedLink"
            @open-wiki="openWikiLink"
            @update:model-value="setContent"
          />
        </div>
      </section>

      <footer class="editor-statusbar">
        <div>
          <span class="status-dot" :class="uiState.saveStatus" />
          <span v-if="uiState.saveStatus === 'saving'">Saving…</span>
          <span v-else-if="uiState.saveStatus === 'error'">Couldn’t save</span>
          <span v-else>Saved</span>
        </div>
        <div class="status-stats">
          <span>Ln {{ activeNote.content.slice(0, activeNote.content.length).split('\n').length }}</span>
          <span>{{ wordCount }} words</span>
          <span>{{ characterCount.toLocaleString() }} characters</span>
          <span>Markdown</span>
        </div>
      </footer>
    </template>

    <div v-else class="empty-editor">
      <div class="empty-glyph"><AppIcon name="file-plus" :size="28" /></div>
      <h2>No note selected</h2>
      <p>Select a note or create a new one.</p>
    </div>
  </main>
</template>
