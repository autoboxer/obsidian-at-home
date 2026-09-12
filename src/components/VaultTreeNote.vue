<script setup lang="ts">
import { nextTick, ref } from 'vue';
import {
  canEditVault,
  deleteNote,
  NOTE_DRAG_MIME,
  selectNote,
  showVaultItemInFolder,
  treeDragState,
  updateNote,
  vaultState
} from '../stores/vault';
import type { Note } from '../types';
import AppIcon from './AppIcon.vue';

const props = withDefaults( defineProps<{ note: Note; depth?: number }>(), { depth: 0 });
const dragging = ref( false );
const menuOpen = ref( false );
const menuPosition = ref<{ x: number; y: number }>();
const noteButton = ref<HTMLButtonElement>();
const menu = ref<HTMLElement>();
const editing = ref( false );
const editValue = ref( '' );
const renameInput = ref<HTMLInputElement>();

function startDrag( event: DragEvent ): void {
  if ( !canEditVault.value ) {
    event.preventDefault();

    return;
  }
  if ( !event.dataTransfer ) {
    return;
  }
  event.dataTransfer.clearData();
  event.dataTransfer.effectAllowed = 'copyMove';
  event.dataTransfer.setData( NOTE_DRAG_MIME, props.note.id );
  event.dataTransfer.setData( 'text/plain', props.note.title || 'Untitled note' );
  treeDragState.noteId = props.note.id;
  treeDragState.folderId = null;
  treeDragState.imagePath = null;
  treeDragState.attachmentPath = null;
  dragging.value = true;
}

function finishDrag(): void {
  dragging.value = false;
  if ( treeDragState.noteId === props.note.id ) {
    treeDragState.noteId = null;
  }
}

function openContextMenu( event: Pick<MouseEvent, 'clientX' | 'clientY'> ): void {
  if ( editing.value ) {
    return;
  }
  const menuWidth = 174;
  const menuHeight = 124;
  menuPosition.value = {
    x: Math.max( 8, Math.min( event.clientX, window.innerWidth - menuWidth - 8 ) ),
    y: Math.max( 8, Math.min( event.clientY, window.innerHeight - menuHeight - 8 ) )
  };
  menuOpen.value = true;
  nextTick( () => menu.value?.focus() );
}

function closeMenu( restoreFocus = false ): void {
  menuOpen.value = false;
  menuPosition.value = undefined;
  if ( restoreFocus ) {
    nextTick( () => noteButton.value?.focus() );
  }
}

function handleFocusOut( event: FocusEvent ): void {
  const row = event.currentTarget as HTMLElement;
  if ( event.relatedTarget instanceof Node && row.contains( event.relatedTarget ) ) {
    return;
  }
  closeMenu();
}

function handleMenuKeydown( event: KeyboardEvent ): void {
  if ( event.key === 'Escape' ) {
    event.preventDefault();
    event.stopPropagation();
    closeMenu( true );

    return;
  }
  const items = Array.from( menu.value?.querySelectorAll<HTMLButtonElement>( '[role="menuitem"]:not(:disabled)' ) ?? []);
  const currentIndex = items.indexOf( document.activeElement as HTMLButtonElement );
  if (
    ( event.key === 'Tab' || ( event.key === 'Unidentified' && event.code === 'Tab' ) )
    && event.shiftKey
    && currentIndex <= 0
  ) {
    event.preventDefault();
    closeMenu( true );

    return;
  }
  if ( !items.length || ![ 'ArrowDown', 'ArrowUp', 'Home', 'End' ].includes( event.key ) ) {
    return;
  }
  event.preventDefault();
  let nextIndex: number;
  if ( event.key === 'Home' ) {
    nextIndex = 0;
  } else if ( event.key === 'End' || ( event.key === 'ArrowUp' && currentIndex < 0 ) ) {
    nextIndex = items.length - 1;
  } else if ( event.key === 'ArrowDown' ) {
    nextIndex = ( currentIndex + 1 ) % items.length;
  } else {
    nextIndex = ( currentIndex - 1 + items.length ) % items.length;
  }
  items[ nextIndex ]?.focus();
}

function beginRename(): void {
  if ( !canEditVault.value ) {
    return;
  }
  closeMenu();
  editValue.value = props.note.title;
  editing.value = true;
  nextTick( () => {
    renameInput.value?.focus();
    renameInput.value?.select();
  });
}

function saveRename( restoreFocus = false ): void {
  if ( !editing.value ) {
    return;
  }
  editing.value = false;
  if ( editValue.value !== props.note.title ) {
    updateNote( props.note.id, { title: editValue.value });
  }
  if ( restoreFocus ) {
    nextTick( () => noteButton.value?.focus() );
  }
}

function cancelRename(): void {
  editing.value = false;
  nextTick( () => noteButton.value?.focus() );
}

function handleRowKeydown( event: KeyboardEvent ): void {
  if ( event.key === 'F2' && !event.altKey && !event.ctrlKey && !event.metaKey ) {
    event.preventDefault();
    beginRename();

    return;
  }
  if ( event.key === 'ContextMenu' || ( event.key === 'F10' && event.shiftKey ) ) {
    event.preventDefault();
    const bounds = noteButton.value?.getBoundingClientRect();
    if ( bounds ) {
      openContextMenu({ clientX: bounds.left, clientY: bounds.bottom });
    }
  }
}

function requestDelete(): void {
  if ( !canEditVault.value ) {
    return;
  }
  closeMenu();
  const title = props.note.title || 'Untitled note';
  if ( window.confirm( `Delete “${ title }”? It will remain in Recently Deleted for seven days.` ) ) {
    void deleteNote( props.note.id );
  }
}

function showInFolder(): void {
  closeMenu();
  void showVaultItemInFolder({
    itemId: props.note.id,
    kind: 'note',
    relativePath: props.note.relativePath
  });
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
    @focusout="handleFocusOut"
  >
    <button
      v-if="!editing"
      ref="noteButton"
      type="button"
      class="vault-tree-note-main"
      :draggable="canEditVault"
      aria-haspopup="menu"
      :aria-expanded="menuOpen"
      @click="selectNote( note.id )"
      @keydown="handleRowKeydown"
      @dragstart="startDrag"
      @dragend="finishDrag"
    >
      <AppIcon
        class="vault-tree-note-icon"
        name="file-text"
        :size="14"
      />
      <span class="vault-tree-note-title">{{ note.title || "Untitled note" }}</span>
      <span
        v-if="note.pinned"
        class="vault-tree-note-favorite"
        title="Favorite"
        aria-label="Favorite"
      >
        <AppIcon name="star" :size="11" />
      </span>
    </button>

    <form
      v-else
      class="vault-tree-note-rename"
      @submit.prevent="saveRename( true )"
    >
      <AppIcon name="file-text" :size="14" />
      <input
        ref="renameInput"
        v-model="editValue"
        :disabled="!canEditVault"
        type="text"
        aria-label="Note name"
        @blur="saveRename()"
        @keydown.esc.prevent.stop="cancelRename"
      >
    </form>

    <button
      v-if="!editing"
      :disabled="!canEditVault"
      type="button"
      class="vault-tree-note-delete"
      :aria-label="`Delete ${note.title || 'Untitled note'}`"
      title="Delete note"
      @click.stop="requestDelete"
    >
      <AppIcon name="trash" :size="12" />
    </button>

    <div class="vault-tree-note-menu-anchor">
      <Transition name="popover-fade">
        <div
          v-if="menuOpen"
          ref="menu"
          class="popover-menu tree-context-menu vault-tree-note-popover"
          :style="menuPosition ? { left: `${menuPosition.x}px`, top: `${menuPosition.y}px` } : undefined"
          role="menu"
          tabindex="-1"
          :aria-label="`Actions for ${note.title || 'Untitled note'}`"
          @keydown="handleMenuKeydown"
        >
          <button
            :disabled="!canEditVault"
            type="button"
            role="menuitem"
            @click="beginRename"
          >
            <AppIcon name="edit" :size="14" /> Rename
          </button>
          <button
            type="button"
            role="menuitem"
            @click="showInFolder"
          >
            <AppIcon name="folder-open" :size="14" /> Show in folder
          </button>
          <button
            :disabled="!canEditVault"
            type="button"
            class="danger"
            role="menuitem"
            @click="requestDelete"
          >
            <AppIcon name="trash" :size="14" /> Delete note
          </button>
        </div>
      </Transition>
    </div>
  </div>
</template>
