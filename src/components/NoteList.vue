<script setup lang="ts">
import { computed } from "vue";
import { createSearchSnippet, parseWikiLinks } from "../lib";
import {
  folderPath,
  selectNote,
  togglePinned,
  uiState,
  vaultState,
  visibleNotes,
} from "../stores/vault";
import AppIcon from "./AppIcon.vue";

const heading = computed(() => {
  const selected = vaultState.selectedFolderId;
  if (selected === "all") return "All notes";
  if (selected === "favorites") return "Favorites";
  if (selected === "unfiled") return "Unfiled";
  return vaultState.folders.find((folder) => folder.id === selected)?.name ?? "Notes";
});

function cleanExcerpt(content: string): string {
  return createSearchSnippet(content, uiState.noteFilter || [], 132) || "No content yet";
}

function relativeTime(timestamp: number): string {
  const difference = Date.now() - timestamp;
  const minutes = Math.floor(difference / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  return new Intl.DateTimeFormat("en", { month: "short", day: "numeric" }).format(timestamp);
}
</script>

<template>
  <aside class="note-list-panel">
    <header class="note-list-header">
      <div>
        <span class="eyebrow">Notes</span>
        <h2>{{ heading }}</h2>
      </div>
      <span class="note-total">{{ visibleNotes.length }}</span>
    </header>

    <label class="list-filter">
      <AppIcon name="search" :size="15" />
      <input v-model="uiState.noteFilter" placeholder="Filter this list…" aria-label="Filter notes" />
      <button v-if="uiState.noteFilter" type="button" aria-label="Clear filter" @click="uiState.noteFilter = ''">
        <AppIcon name="x" :size="12" />
      </button>
      <kbd v-else>/</kbd>
    </label>

    <div class="note-list-scroll">
      <button
        v-for="note in visibleNotes"
        :key="note.id"
        type="button"
        class="note-card"
        :class="{ active: vaultState.activeNoteId === note.id }"
        @click="selectNote(note.id)"
      >
        <span class="note-card-accent" />
        <span class="note-card-topline">
          <span v-if="note.folderId" class="note-folder">{{ folderPath(note.folderId) }}</span>
          <span v-else class="note-folder">Unfiled</span>
          <span>{{ relativeTime(note.updatedAt) }}</span>
        </span>
        <span class="note-card-title-row">
          <strong>{{ note.title || "Untitled note" }}</strong>
          <span
            v-if="note.pinned"
            class="pinned-indicator"
            role="button"
            title="Unpin note"
            @click.stop="togglePinned(note.id)"
          ><AppIcon name="pin" :size="12" /></span>
        </span>
        <span class="note-excerpt">{{ cleanExcerpt(note.content) }}</span>
        <span class="note-card-meta">
          <span v-if="note.tags.length" class="mini-tag"><em>#</em>{{ note.tags[0] }}</span>
          <span v-if="parseWikiLinks(note.content).length" class="mini-links">
            <AppIcon name="link" :size="11" /> {{ parseWikiLinks(note.content).length }}
          </span>
        </span>
      </button>

      <div v-if="!visibleNotes.length" class="empty-note-list">
        <div><AppIcon name="search" :size="20" /></div>
        <strong>No notes here</strong>
        <span>Try another folder or clear the filter.</span>
      </div>
    </div>
  </aside>
</template>
