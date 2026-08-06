<script setup lang="ts">
import { ref } from "vue";
import { NOTE_DRAG_MIME, selectNote, treeDragState, vaultState } from "../stores/vault";
import type { Note } from "../types";
import AppIcon from "./AppIcon.vue";

const props = withDefaults(defineProps<{ note: Note; depth?: number }>(), { depth: 0 });
const dragging = ref(false);

function startDrag(event: DragEvent): void {
  if (!event.dataTransfer) return;
  event.dataTransfer.clearData();
  event.dataTransfer.effectAllowed = "move";
  event.dataTransfer.setData(NOTE_DRAG_MIME, props.note.id);
  treeDragState.folderId = null;
  dragging.value = true;
}

function finishDrag(): void {
  dragging.value = false;
}
</script>

<template>
  <button
    type="button"
    class="vault-tree-note"
    :class="{
      active: vaultState.activeNoteId === note.id,
      'is-dragging': dragging,
    }"
    :style="{ '--tree-depth': depth }"
    :aria-current="vaultState.activeNoteId === note.id ? 'page' : undefined"
    draggable="true"
    @click="selectNote(note.id)"
    @dragstart="startDrag"
    @dragend="finishDrag"
  >
    <AppIcon class="vault-tree-note-icon" name="file-text" :size="14" />
    <span class="vault-tree-note-title">{{ note.title || "Untitled note" }}</span>
    <span v-if="note.pinned" class="vault-tree-note-favorite" title="Favorite" aria-label="Favorite">
      <AppIcon name="star" :size="11" />
    </span>
  </button>
</template>
