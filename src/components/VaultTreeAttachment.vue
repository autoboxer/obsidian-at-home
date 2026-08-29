<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import {
  markdownAttachmentIsArchive,
  markdownAttachmentIsExecutable,
  VAULT_ATTACHMENT_DRAG_MIME,
} from "../lib/markdownAttachments";
import {
  activateVaultAttachment,
  isMirrorManagedAttachment,
  renameVaultAttachment,
  requestInsertVaultAttachment,
  treeDragState,
} from "../stores/vault";
import type { VaultAttachmentFile } from "../types";
import AppIcon from "./AppIcon.vue";

const props = withDefaults(
  defineProps<{ attachment: VaultAttachmentFile; depth?: number }>(),
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

const fileName = computed(() =>
  props.attachment.relativePath.split("/").at(-1) || "Attachment"
);
const mirrorManaged = computed(() =>
  isMirrorManagedAttachment(props.attachment.relativePath)
);
const archive = computed(() => markdownAttachmentIsArchive(
  props.attachment.relativePath,
  props.attachment.mediaType,
));
const executable = computed(() =>
  markdownAttachmentIsExecutable(
    props.attachment.relativePath,
    props.attachment.openingDisabled,
  )
);
const actionLabel = computed(() => {
  if (executable.value) {
    return "Opening unavailable";
  }

  return archive.value ? "Save archive as…" : "Open file";
});
const rowTitle = computed(() => mirrorManaged.value
  ? `${props.attachment.relativePath} · Press Enter to embed · Mirrored attachments follow their note folders`
  : `${props.attachment.relativePath} · Press Enter to embed or drag into the editor or onto a folder`);

function startDrag(event: DragEvent): void {
  if (!event.dataTransfer) {
    return;
  }
  closeMenu();
  event.dataTransfer.clearData();
  event.dataTransfer.effectAllowed = "copyMove";
  event.dataTransfer.setData(VAULT_ATTACHMENT_DRAG_MIME, props.attachment.relativePath);
  event.dataTransfer.setData("text/plain", fileName.value);
  treeDragState.noteId = null;
  treeDragState.folderId = null;
  treeDragState.imagePath = null;
  treeDragState.attachmentPath = props.attachment.relativePath;
  dragging.value = true;
}

function finishDrag(): void {
  dragging.value = false;
  if (treeDragState.attachmentPath === props.attachment.relativePath) {
    treeDragState.attachmentPath = null;
  }
}

function insertIntoActiveNote(): void {
  closeMenu();
  requestInsertVaultAttachment(props.attachment);
}

function activateAttachment(): void {
  closeMenu();
  void activateVaultAttachment(props.attachment);
}

function beginRename(): void {
  if (mirrorManaged.value) {
    return;
  }
  closeMenu();
  editValue.value = fileName.value;
  editing.value = true;
  nextTick(() => {
    renameInput.value?.focus();
    const extensionStart = editValue.value.lastIndexOf(".");
    renameInput.value?.setSelectionRange(
      0,
      extensionStart > 0 ? extensionStart : editValue.value.length,
    );
  });
}

function saveRename(): void {
  if (!editing.value) {
    return;
  }
  const requestedName = editValue.value;
  editing.value = false;
  if (requestedName !== fileName.value) {
    void renameVaultAttachment(props.attachment, requestedName);
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
    class="vault-tree-attachment"
    :class="{ 'is-dragging': dragging, 'is-mirror-managed': mirrorManaged }"
    :style="{ '--tree-depth': depth }"
    :title="rowTitle"
    @contextmenu.prevent.stop="openContextMenu"
  >
    <template v-if="!editing">
      <button
        type="button"
        class="vault-tree-attachment-main"
        draggable="true"
        :aria-label="`Insert ${fileName} into the active note`"
        @click="insertIntoActiveNote"
        @dragstart="startDrag"
        @dragend="finishDrag"
      >
        <AppIcon class="vault-tree-attachment-icon" name="paperclip" :size="14" />
        <span class="vault-tree-attachment-title">{{ fileName }}</span>
        <AppIcon v-if="mirrorManaged" class="vault-tree-attachment-lock" name="lock" :size="11" />
      </button>
      <div class="vault-tree-attachment-menu-anchor" @focusout="handleMenuFocusOut">
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
            :aria-label="`Actions for ${fileName}`"
            @keydown.esc.prevent="closeMenu(true)"
          >
            <button type="button" role="menuitem" @click="insertIntoActiveNote">
              <AppIcon name="paperclip" :size="14" />
              Insert into active note
            </button>
            <button
              type="button"
              role="menuitem"
              :disabled="executable"
              @click="activateAttachment"
            >
              <AppIcon :name="archive ? 'export' : 'arrow'" :size="14" />
              {{ actionLabel }}
            </button>
            <button type="button" role="menuitem" :disabled="mirrorManaged" @click="beginRename">
              <AppIcon :name="mirrorManaged ? 'lock' : 'edit'" :size="14" />
              {{ mirrorManaged ? "Managed by mirror setting" : "Rename" }}
            </button>
          </div>
        </Transition>
      </div>
    </template>
    <form v-else class="vault-tree-attachment-rename" @submit.prevent="saveRename">
      <AppIcon name="paperclip" :size="14" />
      <input
        ref="renameInput"
        v-model="editValue"
        type="text"
        maxlength="180"
        aria-label="Attachment file name"
        @blur="saveRename"
        @keydown.esc.prevent.stop="cancelRename"
      />
    </form>
  </div>
</template>
