<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import { VAULT_IMAGE_DRAG_MIME } from "../lib/imageEmbeds";
import {
  renameVaultImage,
  requestInsertVaultImage,
  showVaultItemInFolder,
  treeDragState,
} from "../stores/vault";
import type { VaultImageFile } from "../types";
import AppIcon from "./AppIcon.vue";

const props = withDefaults(
  defineProps<{ image: VaultImageFile; depth?: number }>(),
  { depth: 0 },
);
const dragging = ref(false);
const editing = ref(false);
const editValue = ref("");
const renameInput = ref<HTMLInputElement>();
const menuOpen = ref(false);
const menuPosition = ref<{ x: number; y: number }>();
const menu = ref<HTMLElement>();
const menuButton = ref<HTMLButtonElement>();

const fileName = computed(() => props.image.relativePath.split("/").at(-1) || "Image");
const rowTitle = computed(() =>
  `${props.image.relativePath} · Press Enter to embed or drag onto a note or folder`
);

function startDrag(event: DragEvent): void {
  if (!event.dataTransfer) {
    return;
  }
  closeMenu();
  event.dataTransfer.clearData();
  event.dataTransfer.effectAllowed = "copyMove";
  event.dataTransfer.setData(VAULT_IMAGE_DRAG_MIME, props.image.relativePath);
  event.dataTransfer.setData("text/plain", fileName.value);
  treeDragState.noteId = null;
  treeDragState.folderId = null;
  treeDragState.attachmentPath = null;
  treeDragState.imagePath = props.image.relativePath;
  dragging.value = true;
}

function finishDrag(): void {
  dragging.value = false;
  if (treeDragState.imagePath === props.image.relativePath) {
    treeDragState.imagePath = null;
  }
}

function insertIntoActiveNote(): void {
  closeMenu();
  requestInsertVaultImage(props.image);
}

function showInFolder(): void {
  closeMenu();
  void showVaultItemInFolder({
    assetId: props.image.assetId,
    kind: "image",
    relativePath: props.image.relativePath,
  });
}

function beginRename(): void {
  closeMenu();
  editValue.value = fileName.value;
  editing.value = true;
  nextTick(() => {
    renameInput.value?.focus();
    const extensionStart = editValue.value.lastIndexOf(".");
    renameInput.value?.setSelectionRange(0, extensionStart > 0 ? extensionStart : editValue.value.length);
  });
}

function saveRename(): void {
  if (!editing.value) {
    return;
  }
  const requestedName = editValue.value;
  editing.value = false;
  if (requestedName !== fileName.value) {
    void renameVaultImage(props.image, requestedName);
  }
}

function cancelRename(): void {
  editing.value = false;
  editValue.value = fileName.value;
}

function toggleMenu(): void {
  if (menuOpen.value) {
    closeMenu(true);
    return;
  }
  menuPosition.value = undefined;
  menuOpen.value = true;
  nextTick(() => menu.value?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus());
}

function openContextMenu(event: MouseEvent): void {
  const menuWidth = 190;
  const menuHeight = 128;
  menuPosition.value = {
    x: Math.max(8, Math.min(event.clientX, window.innerWidth - menuWidth - 8)),
    y: Math.max(8, Math.min(event.clientY, window.innerHeight - menuHeight - 8)),
  };
  menuOpen.value = true;
  nextTick(() => menu.value?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus());
}

function closeMenu(restoreFocus = false): void {
  menuOpen.value = false;
  menuPosition.value = undefined;
  if (restoreFocus) {
    nextTick(() => menuButton.value?.focus());
  }
}

function handleMenuFocusOut(event: FocusEvent): void {
  const anchor = event.currentTarget as HTMLElement;
  if (event.relatedTarget instanceof Node && anchor.contains(event.relatedTarget)) {
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
  >
    <template v-if="!editing">
      <button
        type="button"
        class="vault-tree-image-main"
        draggable="true"
        :aria-label="`Insert ${fileName} into the active note`"
        @click="insertIntoActiveNote"
        @dragstart="startDrag"
        @dragend="finishDrag"
      >
        <AppIcon class="vault-tree-image-icon" name="image" :size="14" />
        <span class="vault-tree-image-title">{{ fileName }}</span>
      </button>
      <div class="vault-tree-image-menu-anchor" @focusout="handleMenuFocusOut">
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
            :aria-label="`Actions for ${fileName}`"
            @keydown.esc.prevent="closeMenu(true)"
          >
            <button type="button" role="menuitem" @click="insertIntoActiveNote">
              <AppIcon name="image" :size="14" />
              Insert into active note
            </button>
            <button type="button" role="menuitem" @click="beginRename">
              <AppIcon name="edit" :size="14" />
              Rename
            </button>
            <button type="button" role="menuitem" @click="showInFolder">
              <AppIcon name="folder-open" :size="14" />
              Show in folder
            </button>
          </div>
        </Transition>
      </div>
    </template>
    <form v-else class="vault-tree-image-rename" @submit.prevent="saveRename">
      <AppIcon name="image" :size="14" />
      <input
        ref="renameInput"
        v-model="editValue"
        type="text"
        maxlength="180"
        aria-label="Image file name"
        @blur="saveRename"
        @keydown.esc.prevent.stop="cancelRename"
      />
    </form>
  </div>
</template>
