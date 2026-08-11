<script setup lang="ts">
import {
  folderPath,
  recentNotes,
  selectNote,
  vaultState,
} from "../stores/vault";
import type { Note } from "../types";
import AppIcon from "./AppIcon.vue";

function noteTitle(note: Note): string {
  return note.title.trim() || "Untitled note";
}

function noteLocation(note: Note): string {
  return note.folderId ? folderPath(note.folderId) : "Vault root";
}

function noteTooltip(note: Note): string {
  return `${noteTitle(note)} · ${noteLocation(note)}`;
}
</script>

<template>
  <Transition name="collapse-fade">
    <section v-if="recentNotes.length" class="explorer-section recent-notes-section" aria-labelledby="recent-notes-heading">
      <div class="section-label">
        <span id="recent-notes-heading">Recent notes</span>
        <small>{{ recentNotes.length }}</small>
      </div>

      <TransitionGroup name="recent-list" tag="div" class="recent-note-list">
        <button
          v-for="note in recentNotes"
          :key="note.id"
          type="button"
          class="recent-note"
          :class="{ active: vaultState.activeNoteId === note.id }"
          :title="noteTooltip(note)"
          :aria-label="`Open ${noteTitle(note)} in ${noteLocation(note)}`"
          :aria-current="vaultState.activeNoteId === note.id ? 'page' : undefined"
          @click="selectNote(note.id)"
        >
          <AppIcon name="file-text" :size="13" />
          <span>{{ noteTitle(note) }}</span>
          <small v-if="note.folderId">{{ folderPath(note.folderId) }}</small>
        </button>
      </TransitionGroup>
    </section>
  </Transition>
</template>
