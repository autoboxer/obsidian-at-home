<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import {
  activeNote,
  createLinkedNote,
  deleteNote,
  folderPath,
  setEditorMode,
  togglePinned,
  uiState,
  updateNote,
  vaultState,
} from "../stores/vault";
import type { EditorMode } from "../types";
import AppIcon from "./AppIcon.vue";
import MarkdownPreview from "./MarkdownPreview.vue";
import SourceEditor from "./SourceEditor.vue";

const tagInputOpen = ref(false);
const tagInput = ref("");
const tagField = ref<HTMLInputElement>();
const noteMenuOpen = ref(false);

const noteTitles = computed(() => vaultState.notes.map((note) => note.title));
const sortedFolders = computed(() => [...vaultState.folders].sort((a, b) => folderPath(a.id).localeCompare(folderPath(b.id))));
const wordCount = computed(() => {
  const content = activeNote.value?.content ?? "";
  return content.trim() ? content.trim().split(/\s+/).length : 0;
});
const characterCount = computed(() => activeNote.value?.content.length ?? 0);

const modes: Array<{ id: EditorMode; label: string; icon: string }> = [
  { id: "source", label: "Source", icon: "code" },
  { id: "split", label: "Split", icon: "columns" },
  { id: "reading", label: "Read", icon: "eye" },
];

function setTitle(event: Event): void {
  if (!activeNote.value) return;
  updateNote(activeNote.value.id, { title: (event.target as HTMLInputElement).value });
}

function setContent(content: string): void {
  if (activeNote.value) updateNote(activeNote.value.id, { content });
}

function setFolder(event: Event): void {
  if (!activeNote.value) return;
  const value = (event.target as HTMLSelectElement).value;
  updateNote(activeNote.value.id, { folderId: value || null });
}

function openTagInput(): void {
  tagInputOpen.value = true;
  nextTick(() => tagField.value?.focus());
}

function addTag(): void {
  if (!activeNote.value) return;
  const tag = tagInput.value.trim().replace(/^#/, "").replace(/\s+/g, "-");
  if (tag && !activeNote.value.tags.includes(tag)) {
    updateNote(activeNote.value.id, { tags: [...activeNote.value.tags, tag] });
  }
  tagInput.value = "";
  tagInputOpen.value = false;
}

function removeTag(tag: string): void {
  if (activeNote.value) {
    updateNote(activeNote.value.id, { tags: activeNote.value.tags.filter((candidate) => candidate !== tag) });
  }
}

function requestDelete(): void {
  if (!activeNote.value) return;
  noteMenuOpen.value = false;
  if (window.confirm(`Delete “${activeNote.value.title || "Untitled note"}”? This cannot be undone.`)) {
    deleteNote(activeNote.value.id);
  }
}
</script>

<template>
  <main class="editor-workspace">
    <template v-if="activeNote">
      <header class="editor-toolbar">
        <div class="editor-crumbs">
          <button class="icon-button mobile-explorer-toggle" type="button" title="Toggle explorer" @click="uiState.explorerOpen = !uiState.explorerOpen">
            <AppIcon name="sidebar" :size="17" />
          </button>
          <span class="crumb-vault">{{ vaultState.name }}</span>
          <AppIcon name="chevron" :size="12" />
          <span v-if="activeNote.folderId" class="crumb-folder">{{ folderPath(activeNote.folderId) }}</span>
          <AppIcon v-if="activeNote.folderId" name="chevron" :size="12" />
          <span class="crumb-note">{{ activeNote.title || "Untitled note" }}</span>
        </div>

        <div class="editor-toolbar-actions">
          <div class="mode-switcher" aria-label="Editor view">
            <button
              v-for="mode in modes"
              :key="mode.id"
              type="button"
              :class="{ active: vaultState.editorMode === mode.id }"
              :title="mode.label"
              @click="setEditorMode(mode.id)"
            >
              <AppIcon :name="mode.icon" :size="15" />
              <span>{{ mode.label }}</span>
            </button>
          </div>
          <button class="icon-button" type="button" :class="{ active: activeNote.pinned }" :title="activeNote.pinned ? 'Unpin note' : 'Pin note'" @click="togglePinned(activeNote.id)">
            <AppIcon name="pin" :size="16" />
          </button>
          <button class="icon-button" type="button" :class="{ active: uiState.contextOpen }" title="Toggle context" @click="uiState.contextOpen = !uiState.contextOpen">
            <AppIcon name="panel-right" :size="17" />
          </button>
          <div class="menu-anchor">
            <button class="icon-button" type="button" title="More actions" @click="noteMenuOpen = !noteMenuOpen">
              <AppIcon name="more" :size="18" />
            </button>
            <div v-if="noteMenuOpen" class="popover-menu compact-menu">
              <button type="button" class="danger" @click="requestDelete">
                <AppIcon name="trash" :size="15" /> Delete note
              </button>
            </div>
          </div>
        </div>
      </header>

      <section class="editor-document">
        <div class="document-heading">
          <input
            class="note-title-input"
            :value="activeNote.title"
            aria-label="Note title"
            placeholder="Untitled note"
            @input="setTitle"
          />
          <div class="note-properties">
            <label class="property-control folder-property">
              <AppIcon name="folder" :size="14" />
              <select :value="activeNote.folderId ?? ''" aria-label="Move note to folder" @change="setFolder">
                <option value="">Unfiled</option>
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
            <form v-if="tagInputOpen" class="inline-tag-form" @submit.prevent="addTag">
              <span>#</span>
              <input ref="tagField" v-model="tagInput" placeholder="tag" @blur="addTag" @keydown.esc="tagInputOpen = false" />
            </form>
            <button v-else type="button" class="add-tag-button" @click="openTagInput">
              <AppIcon name="plus" :size="12" /> Add tag
            </button>
          </div>
        </div>

        <div class="editor-canvas" :class="`mode-${vaultState.editorMode}`">
          <div v-if="vaultState.editorMode !== 'reading'" class="editor-pane editor-page">
            <SourceEditor
              :model-value="activeNote.content"
              :note-titles="noteTitles"
              @update:model-value="setContent"
            />
          </div>
          <div v-if="vaultState.editorMode !== 'source'" class="preview-pane preview-page">
            <MarkdownPreview :content="activeNote.content" @open-wiki="createLinkedNote" />
          </div>
        </div>
      </section>

      <footer class="editor-statusbar">
        <div>
          <span class="status-dot" :class="uiState.saveStatus" />
          <span v-if="uiState.saveStatus === 'saving'">Saving…</span>
          <span v-else-if="uiState.saveStatus === 'error'">Couldn’t save</span>
          <span v-else>Saved locally</span>
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
