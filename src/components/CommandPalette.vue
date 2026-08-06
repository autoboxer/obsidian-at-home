<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { createSearchSnippet, searchNotes } from "../lib";
import {
  createNote,
  folderNameMap,
  selectNote,
  uiState,
  vaultState,
} from "../stores/vault";
import AppIcon from "./AppIcon.vue";

const query = ref("");
const selectedIndex = ref(0);
const input = ref<HTMLInputElement>();

const results = computed(() => query.value.trim()
  ? searchNotes(vaultState.notes, query.value, { folderNames: folderNameMap.value, limit: 9 })
  : [...vaultState.notes]
    .sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updatedAt - a.updatedAt)
    .slice(0, 7)
    .map((note) => ({ note, score: 0, reason: "title" as const, snippet: createSearchSnippet(note.content, [], 110) })),
);

watch(() => uiState.commandOpen, (open) => {
  if (open) {
    query.value = uiState.noteFilter;
    selectedIndex.value = 0;
    nextTick(() => input.value?.focus());
  }
});

watch(query, () => {
  selectedIndex.value = 0;
});

onMounted(() => nextTick(() => input.value?.focus()));

function close(): void {
  uiState.commandOpen = false;
}

function chooseNote(id: string): void {
  selectNote(id);
  uiState.tool = "notes";
  uiState.noteFilter = "";
  close();
}

function create(): void {
  createNote();
  close();
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === "Escape") {
    close();
    return;
  }
  if (event.key === "ArrowDown") {
    event.preventDefault();
    selectedIndex.value = Math.min(results.value.length - 1, selectedIndex.value + 1);
  } else if (event.key === "ArrowUp") {
    event.preventDefault();
    selectedIndex.value = Math.max(0, selectedIndex.value - 1);
  } else if (event.key === "Enter") {
    event.preventDefault();
    const result = results.value[selectedIndex.value];
    if (result) chooseNote(result.note.id);
    else if (!query.value) create();
  }
}

function highlightTitle(title: string): string {
  const needle = query.value.trim();
  if (!needle) return escapeHtml(title);
  const index = title.toLocaleLowerCase().indexOf(needle.toLocaleLowerCase());
  if (index < 0) return escapeHtml(title);
  return `${escapeHtml(title.slice(0, index))}<mark>${escapeHtml(title.slice(index, index + needle.length))}</mark>${escapeHtml(title.slice(index + needle.length))}`;
}

function escapeHtml(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
</script>

<template>
  <div class="modal-backdrop command-backdrop" @mousedown.self="close">
    <section class="command-palette" role="dialog" aria-modal="true" aria-label="Quick search">
      <div class="command-input-wrap">
        <AppIcon name="search" :size="20" />
        <input
          ref="input"
          v-model="query"
          placeholder="Search notes, content, and tags…"
          aria-label="Search all notes"
          @keydown="onKeydown"
        />
        <kbd>esc</kbd>
      </div>

      <div class="command-body">
        <div class="command-section-label">
          <span>{{ query ? `${results.length} best matches` : "Recent & pinned" }}</span>
          <span v-if="query">Searching titles, content, folders, and tags</span>
        </div>
        <div class="command-results">
          <button
            v-for="(result, index) in results"
            :key="result.note.id"
            type="button"
            class="command-result"
            :class="{ active: index === selectedIndex }"
            @mouseenter="selectedIndex = index"
            @click="chooseNote(result.note.id)"
          >
            <span class="command-result-icon"><AppIcon name="file-text" :size="17" /></span>
            <span class="command-result-copy">
              <span class="command-result-title" v-html="highlightTitle(result.note.title || 'Untitled note')" />
              <span>{{ result.snippet || "No content yet" }}</span>
            </span>
            <span class="command-result-meta">
              <span v-if="result.note.tags[0]">#{{ result.note.tags[0] }}</span>
              <AppIcon v-if="index === selectedIndex" name="enter" :size="14" />
            </span>
          </button>
          <div v-if="query && !results.length" class="command-empty">
            <div><AppIcon name="search" :size="22" /></div>
            <strong>No notes found</strong>
            <span>Try a title, phrase, folder, or tag.</span>
          </div>
        </div>
      </div>

      <footer class="command-footer">
        <button type="button" @click="create"><span><AppIcon name="plus" :size="13" /></span> New note</button>
        <div><span><kbd>↑</kbd><kbd>↓</kbd> move</span><span><kbd>↵</kbd> open</span></div>
      </footer>
    </section>
  </div>
</template>
