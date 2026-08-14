<script setup lang="ts">
import { nextTick, ref } from "vue";
import {
  deleteNote,
  NOTE_DRAG_MIME,
  selectNote,
  treeDragState,
  vaultState,
} from "../stores/vault";
import type { Note } from "../types";
import AppIcon from "./AppIcon.vue";

const props = withDefaults(defineProps<{ note: Note; depth?: number }>(), { depth: 0 });
const dragging = ref(false);
const menuOpen = ref(false);
const menuPosition = ref<{ x: number; y: number }>();
const noteButton = ref<HTMLButtonElement>();
const menu = ref<HTMLElement>();

function startDrag(event: DragEvent): void {
  if (!event.dataTransfer) {
    return;
  }
  event.dataTransfer.clearData();
  event.dataTransfer.effectAllowed = "move";
  event.dataTransfer.setData(NOTE_DRAG_MIME, props.note.id);
  event.dataTransfer.setData("text/plain", props.note.title || "Untitled note");
  treeDragState.noteId = props.note.id;
  treeDragState.folderId = null;
  dragging.value = true;
}

function finishDrag(): void {
  dragging.value = false;
  if (treeDragState.noteId === props.note.id) {
    treeDragState.noteId = null;
  }
}

function openContextMenu(event: MouseEvent): void {
  const menuWidth = 174;
  const menuHeight = 54;
  menuPosition.value = {
    x: Math.max(8, Math.min(event.clientX, window.innerWidth - menuWidth - 8)),
    y: Math.max(8, Math.min(event.clientY, window.innerHeight - menuHeight - 8)),
  };
  menuOpen.value = true;
  nextTick(() => menu.value?.querySelector<HTMLButtonElement>("button")?.focus());
}

function closeMenu(restoreFocus = false): void {
  menuOpen.value = false;
  menuPosition.value = undefined;
  if (restoreFocus) {
    nextTick(() => noteButton.value?.focus());
  }
}

function handleFocusOut(event: FocusEvent): void {
  const row = event.currentTarget as HTMLElement;
  if (event.relatedTarget instanceof Node && row.contains(event.relatedTarget)) {
    return;
  }
  closeMenu();
}

function handleMenuKeydown(event: KeyboardEvent): void {
  if (event.key === "Escape") {
    event.preventDefault();
    closeMenu(true);
  }
}

function requestDelete(): void {
  closeMenu();
  const title = props.note.title || "Untitled note";
  if (window.confirm(`Delete “${title}”? It will remain in Recently Deleted for seven days.`)) {
    void deleteNote(props.note.id);
  }
}
</script>

<template>
  <div
    class="vault-tree-note"
    :class="{
      active: vaultState.activeNoteId === note.id,
      'is-dragging': dragging,
    }"
    :style="{ '--tree-depth': depth }"
    :aria-current="vaultState.activeNoteId === note.id ? 'page' : undefined"
    @contextmenu.prevent.stop="openContextMenu"
  >
    <button
      ref="noteButton"
      type="button"
      class="vault-tree-note-main"
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

    <button
      type="button"
      class="vault-tree-note-delete"
      :aria-label="`Delete ${note.title || 'Untitled note'}`"
      title="Delete note"
      @click.stop="requestDelete"
    >
      <AppIcon name="trash" :size="12" />
    </button>

    <div class="vault-tree-note-menu-anchor" @focusout="handleFocusOut">
      <Transition name="popover-fade">
        <div
          v-if="menuOpen"
          ref="menu"
          class="popover-menu tree-context-menu vault-tree-note-popover"
          :style="menuPosition ? { left: `${menuPosition.x}px`, top: `${menuPosition.y}px` } : undefined"
          role="menu"
          :aria-label="`Actions for ${note.title || 'Untitled note'}`"
          @keydown="handleMenuKeydown"
        >
          <button type="button" class="danger" role="menuitem" @click="requestDelete">
            <AppIcon name="trash" :size="14" /> Delete note
          </button>
        </div>
      </Transition>
    </div>
  </div>
</template>
