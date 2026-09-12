<script setup lang="ts">
import { computed, nextTick, ref } from 'vue';
import { VAULT_IMAGE_DRAG_MIME } from '../lib/imageEmbeds';
import {
  canEditVault,
  renameVaultImage,
  showVaultItemInFolder,
  treeDragState
} from '../stores/vault';
import type { VaultImageFile } from '../types';
import AppIcon from './AppIcon.vue';

const props = withDefaults(
  defineProps<{ image: VaultImageFile; depth?: number }>(),
  { depth: 0 }
);
const dragging = ref( false );
const editing = ref( false );
const editValue = ref( '' );
const renameInput = ref<HTMLInputElement>();
const menuOpen = ref( false );
const menuPosition = ref<{ x: number; y: number }>();
const menu = ref<HTMLElement>();
const mainButton = ref<HTMLButtonElement>();
const menuButton = ref<HTMLButtonElement>();
const menuTrigger = ref<HTMLButtonElement>();

const fileName = computed( () => props.image.relativePath.split( '/' ).at( -1 ) || 'Image' );
const rowTitle = computed( () =>
  !canEditVault.value ? props.image.relativePath : `${ props.image.relativePath } · Drag into the editor to insert or onto a folder to move`
);

function startDrag( event: DragEvent ): void {
  if ( !canEditVault.value ) {
    event.preventDefault();

    return;
  }
  if ( !event.dataTransfer ) {
    return;
  }
  closeMenu();
  event.dataTransfer.clearData();
  event.dataTransfer.effectAllowed = 'copyMove';
  event.dataTransfer.setData( VAULT_IMAGE_DRAG_MIME, props.image.relativePath );
  event.dataTransfer.setData( 'text/plain', fileName.value );
  treeDragState.noteId = null;
  treeDragState.folderId = null;
  treeDragState.attachmentPath = null;
  treeDragState.imagePath = props.image.relativePath;
  dragging.value = true;
}

function finishDrag(): void {
  dragging.value = false;
  if ( treeDragState.imagePath === props.image.relativePath ) {
    treeDragState.imagePath = null;
  }
}

function showInFolder(): void {
  closeMenu();
  void showVaultItemInFolder({
    assetId: props.image.assetId,
    kind: 'image',
    relativePath: props.image.relativePath
  });
}

function beginRename(): void {
  if ( !canEditVault.value ) {
    return;
  }
  closeMenu();
  editValue.value = fileName.value;
  editing.value = true;
  nextTick( () => {
    renameInput.value?.focus();
    const extensionStart = editValue.value.lastIndexOf( '.' );
    renameInput.value?.setSelectionRange( 0, extensionStart > 0 ? extensionStart : editValue.value.length );
  });
}

function saveRename(): void {
  if ( !editing.value ) {
    return;
  }
  const requestedName = editValue.value;
  editing.value = false;
  if ( requestedName !== fileName.value ) {
    void renameVaultImage( props.image, requestedName );
  }
}

function cancelRename(): void {
  editing.value = false;
  editValue.value = fileName.value;
}

function toggleMenu( event: MouseEvent ): void {
  if ( menuOpen.value ) {
    closeMenu( true );
    return;
  }
  menuTrigger.value = event.currentTarget as HTMLButtonElement;
  menuPosition.value = undefined;
  menuOpen.value = true;
  nextTick( () => menu.value?.focus() );
}

function openContextMenu( event: MouseEvent ): void {
  menuTrigger.value = event.target instanceof Node && menuButton.value?.contains( event.target )
    ? menuButton.value
    : mainButton.value;
  const menuWidth = 190;
  const menuHeight = 76;
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
    nextTick( () => menuTrigger.value?.focus() );
  }
}

function handleMenuKeydown( event: KeyboardEvent ): void {
  if ( event.key === 'Escape' ) {
    event.preventDefault();
    event.stopPropagation();
    closeMenu( true );

    return;
  }
  if (
    ( event.key === 'Tab' || ( event.key === 'Unidentified' && event.code === 'Tab' ) )
    && event.shiftKey
    && (
      document.activeElement === menu.value
      || document.activeElement === menu.value?.querySelector( 'button:not(:disabled)' )
    )
  ) {
    event.preventDefault();
    closeMenu( true );

    return;
  }
  if ( ![ 'ArrowDown', 'ArrowUp', 'Home', 'End' ].includes( event.key ) ) {
    return;
  }
  const items = Array.from( menu.value?.querySelectorAll<HTMLButtonElement>( '[role="menuitem"]:not(:disabled)' ) ?? []);
  if ( !items.length ) {
    return;
  }
  event.preventDefault();
  const currentIndex = items.indexOf( document.activeElement as HTMLButtonElement );
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

function handleMenuFocusOut( event: FocusEvent ): void {
  const anchor = event.currentTarget as HTMLElement;
  if ( event.relatedTarget instanceof Node && anchor.contains( event.relatedTarget ) ) {
    return;
  }
  closeMenu();
}
</script>

<template>
  <div
    class="vault-tree-image"
    :class="{ 'is-dragging': dragging }"
    :style="{ '--tree-depth': depth }"
    :title="rowTitle"
    @contextmenu.prevent.stop="openContextMenu"
    @focusout="handleMenuFocusOut"
  >
    <template v-if="!editing">
      <button
        ref="mainButton"
        type="button"
        class="vault-tree-image-main"
        :draggable="canEditVault"
        :aria-label="`Actions for ${fileName}`"
        aria-haspopup="menu"
        :aria-expanded="menuOpen"
        @click="toggleMenu"
        @dragstart="startDrag"
        @dragend="finishDrag"
      >
        <AppIcon
          class="vault-tree-image-icon"
          name="image"
          :size="14"
        />
        <span class="vault-tree-image-title">{{ fileName }}</span>
      </button>
      <div class="vault-tree-image-menu-anchor">
        <button
          ref="menuButton"
          type="button"
          class="vault-tree-image-more"
          :aria-label="`Actions for ${fileName}`"
          aria-haspopup="menu"
          :aria-expanded="menuOpen"
          @click.stop="toggleMenu"
        >
          <AppIcon name="more" :size="14" />
        </button>
        <Transition name="popover-fade">
          <div
            v-if="menuOpen"
            ref="menu"
            class="popover-menu vault-tree-image-popover"
            :class="{ 'tree-context-menu': menuPosition }"
            :style="menuPosition ? { left: `${menuPosition.x}px`, top: `${menuPosition.y}px`, right: 'auto' } : undefined"
            role="menu"
            tabindex="-1"
            :aria-label="`Actions for ${fileName}`"
            @keydown="handleMenuKeydown"
          >
            <button
              :disabled="!canEditVault"
              type="button"
              role="menuitem"
              @click="beginRename"
            >
              <AppIcon name="edit" :size="14" />
              Rename
            </button>
            <button
              type="button"
              role="menuitem"
              @click="showInFolder"
            >
              <AppIcon name="folder-open" :size="14" />
              Show in folder
            </button>
          </div>
        </Transition>
      </div>
    </template>
    <form
      v-else
      class="vault-tree-image-rename"
      @submit.prevent="saveRename"
    >
      <AppIcon name="image" :size="14" />
      <input
        ref="renameInput"
        v-model="editValue"
        :disabled="!canEditVault"
        type="text"
        maxlength="180"
        aria-label="Image file name"
        @blur="saveRename"
        @keydown.esc.prevent.stop="cancelRename"
      >
    </form>
  </div>
</template>
