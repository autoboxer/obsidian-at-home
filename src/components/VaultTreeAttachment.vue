<script setup lang="ts">
import { computed, nextTick, ref } from 'vue';
import {
  markdownAttachmentIsArchive,
  markdownAttachmentIsExecutable,
  VAULT_ATTACHMENT_DRAG_MIME
} from '../lib/markdownAttachments';
import {
  canEditVault,
  activateVaultAttachment,
  renameVaultAttachment,
  showVaultItemInFolder,
  treeDragState,
  vaultTreeItemIsRevealed
} from '../stores/vault';
import type { VaultAttachmentFile } from '../types';
import AppIcon from './AppIcon.vue';

const props = withDefaults(
  defineProps<{ attachment: VaultAttachmentFile; depth?: number }>(),
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

const fileName = computed( () =>
  props.attachment.relativePath.split( '/' ).at( -1 ) || 'Attachment'
);
const archive = computed( () => markdownAttachmentIsArchive(
  props.attachment.relativePath,
  props.attachment.mediaType
) );
const executable = computed( () =>
  markdownAttachmentIsExecutable(
    props.attachment.relativePath,
    props.attachment.openingDisabled
  )
);
const actionLabel = computed( () => {
  if ( executable.value ) {
    return 'Opening unavailable';
  }

  return archive.value ? 'Save archive as…' : 'Open file';
});
const rowTitle = computed( () =>
  !canEditVault.value ? props.attachment.relativePath : `${ props.attachment.relativePath } · Drag into the editor to insert or onto a folder to move`
);
const revealed = computed( () => vaultTreeItemIsRevealed({
  assetId: props.attachment.assetId,
  kind: 'attachment',
  relativePath: props.attachment.relativePath
}) );

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
  event.dataTransfer.setData( VAULT_ATTACHMENT_DRAG_MIME, props.attachment.relativePath );
  event.dataTransfer.setData( 'text/plain', fileName.value );
  treeDragState.noteId = null;
  treeDragState.folderId = null;
  treeDragState.imagePath = null;
  treeDragState.attachmentPath = props.attachment.relativePath;
  dragging.value = true;
}

function finishDrag(): void {
  dragging.value = false;
  if ( treeDragState.attachmentPath === props.attachment.relativePath ) {
    treeDragState.attachmentPath = null;
  }
}

function activateAttachment(): void {
  closeMenu();
  void activateVaultAttachment( props.attachment );
}

function showInFolder(): void {
  closeMenu();
  void showVaultItemInFolder({
    assetId: props.attachment.assetId,
    kind: 'attachment',
    relativePath: props.attachment.relativePath
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
    renameInput.value?.setSelectionRange(
      0,
      extensionStart > 0 ? extensionStart : editValue.value.length
    );
  });
}

function saveRename(): void {
  if ( !editing.value ) {
    return;
  }
  const requestedName = editValue.value;
  editing.value = false;
  if ( requestedName !== fileName.value ) {
    void renameVaultAttachment( props.attachment, requestedName );
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
  const menuHeight = 108;
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
    class="vault-tree-attachment"
    :class="{ 'is-dragging': dragging, 'is-revealed': revealed }"
    data-vault-item-kind="attachment"
    :data-vault-item-asset-id="attachment.assetId"
    :data-vault-item-relative-path="attachment.relativePath"
    :style="{ '--tree-depth': depth }"
    :title="rowTitle"
    @contextmenu.prevent.stop="openContextMenu"
    @focusout="handleMenuFocusOut"
  >
    <template v-if="!editing">
      <button
        ref="mainButton"
        type="button"
        class="vault-tree-attachment-main"
        data-vault-item-primary
        :draggable="canEditVault"
        :aria-current="revealed ? 'true' : undefined"
        :aria-label="`Actions for ${fileName}`"
        aria-haspopup="menu"
        :aria-expanded="menuOpen"
        @click="toggleMenu"
        @dragstart="startDrag"
        @dragend="finishDrag"
      >
        <AppIcon
          class="vault-tree-attachment-icon"
          name="paperclip"
          :size="14"
        />
        <span class="vault-tree-attachment-title">{{ fileName }}</span>
      </button>
      <div class="vault-tree-attachment-menu-anchor">
        <button
          ref="menuButton"
          type="button"
          class="vault-tree-attachment-more"
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
            class="popover-menu vault-tree-attachment-popover"
            :class="{ 'tree-context-menu': menuPosition }"
            :style="menuPosition ? { left: `${menuPosition.x}px`, top: `${menuPosition.y}px`, right: 'auto' } : undefined"
            role="menu"
            tabindex="-1"
            :aria-label="`Actions for ${fileName}`"
            @keydown="handleMenuKeydown"
          >
            <button
              type="button"
              role="menuitem"
              :disabled="executable"
              @click="activateAttachment"
            >
              <AppIcon :name="archive ? 'export' : 'arrow'" :size="14" />
              {{ actionLabel }}
            </button>
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
      class="vault-tree-attachment-rename"
      @submit.prevent="saveRename"
    >
      <AppIcon name="paperclip" :size="14" />
      <input
        ref="renameInput"
        v-model="editValue"
        :disabled="!canEditVault"
        type="text"
        maxlength="180"
        aria-label="Attachment file name"
        @blur="saveRename"
        @keydown.esc.prevent.stop="cancelRename"
      >
    </form>
  </div>
</template>
