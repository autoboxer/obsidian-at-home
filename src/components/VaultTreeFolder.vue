<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import {
  createFolder,
  createNote,
  deleteFolder,
  FOLDER_DRAG_MIME,
  folderChildren,
  folderPath,
  moveFolder,
  moveNoteToFolder,
  NOTE_DRAG_MIME,
  renameFolder,
  treeDragState,
  vaultState,
} from "../stores/vault";
import type { Folder, Note } from "../types";
import AppIcon from "./AppIcon.vue";
import VaultTreeNote from "./VaultTreeNote.vue";

defineOptions({ name: "VaultTreeFolder" });

const props = withDefaults(
  defineProps<{
    folder: Folder;
    notes: Note[];
    depth?: number;
    showEmptyFolders?: boolean;
  }>(),
  {
    depth: 0,
    showEmptyFolders: true,
  },
);

const expanded = ref(true);
const dropActive = ref(false);
const dropInvalid = ref(false);
const dragging = ref(false);
const menuOpen = ref(false);
const movePickerOpen = ref(false);
const moveTarget = ref("");
const editing = ref(false);
const editValue = ref(props.folder.name);
const subfolderInputOpen = ref(false);
const subfolderName = ref("");
const menuButton = ref<HTMLButtonElement>();
const menu = ref<HTMLElement>();
const moveSelect = ref<HTMLSelectElement>();
const renameInput = ref<HTMLInputElement>();
const subfolderInput = ref<HTMLInputElement>();
let expandTimer: number | undefined;

const noteFolderIds = computed(() => new Set(props.notes.flatMap((note) => note.folderId ? [note.folderId] : [])));

const directNotes = computed(() => props.notes
  .filter((note) => note.folderId === props.folder.id)
  .sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: "base", numeric: true }) || a.id.localeCompare(b.id)));

const children = computed(() => folderChildren(props.folder.id)
  .filter((folder) => props.showEmptyFolders || branchContainsPassedNote(folder.id)));
const moveDestinations = computed(() => vaultState.folders
  .filter((folder) => folder.id !== props.folder.id && !folderIsWithin(folder.id, props.folder.id))
  .sort((a, b) => folderPath(a.id).localeCompare(folderPath(b.id))));

const hasContents = computed(() => children.value.length > 0 || directNotes.value.length > 0);
const canExpand = computed(() => hasContents.value || subfolderInputOpen.value);
const branchVisible = computed(() => props.showEmptyFolders || hasContents.value);
const passedNoteIds = computed(() => props.notes.map((note) => note.id).sort().join("\u0000"));
const activeNoteFolderId = computed(
  () => vaultState.notes.find((note) => note.id === vaultState.activeNoteId)?.folderId ?? null,
);

watch(
  () => props.folder.name,
  (name) => {
    if (!editing.value) editValue.value = name;
  },
);

watch(
  passedNoteIds,
  () => {
    if (branchContainsPassedNote(props.folder.id)) expanded.value = true;
  },
);

watch(
  () => vaultState.activeNoteId,
  () => {
    const folderId = activeNoteFolderId.value;
    if (folderId && branchContainsFolder(folderId)) expanded.value = true;
  },
  { immediate: true },
);

watch(
  () => treeDragState.folderId,
  (folderId) => {
    if (!folderId) {
      dropActive.value = false;
      dropInvalid.value = false;
      clearExpandTimer();
    }
  },
);

onBeforeUnmount(clearExpandTimer);

function branchContainsPassedNote(folderId: string, visited = new Set<string>()): boolean {
  if (noteFolderIds.value.has(folderId)) return true;
  if (visited.has(folderId)) return false;
  visited.add(folderId);
  return folderChildren(folderId).some((folder) => branchContainsPassedNote(folder.id, visited));
}

function branchContainsFolder(folderId: string): boolean {
  let current = vaultState.folders.find((folder) => folder.id === folderId);
  const visited = new Set<string>();
  while (current && !visited.has(current.id)) {
    if (current.id === props.folder.id) return true;
    visited.add(current.id);
    const parentId = current.parentId;
    current = parentId
      ? vaultState.folders.find((folder) => folder.id === parentId)
      : undefined;
  }
  return false;
}

function isTreeDrag(event: DragEvent): boolean {
  const types = Array.from(event.dataTransfer?.types ?? []);
  return types.includes(NOTE_DRAG_MIME) || types.includes(FOLDER_DRAG_MIME);
}

function startFolderDrag(event: DragEvent): void {
  if (!event.dataTransfer) return;
  closeMenu();
  event.dataTransfer.clearData();
  event.dataTransfer.effectAllowed = "move";
  event.dataTransfer.setData(FOLDER_DRAG_MIME, props.folder.id);
  treeDragState.folderId = props.folder.id;
  dragging.value = true;
}

function finishFolderDrag(): void {
  dragging.value = false;
  dropActive.value = false;
  dropInvalid.value = false;
  if (treeDragState.folderId === props.folder.id) treeDragState.folderId = null;
  clearExpandTimer();
}

function isInvalidFolderTarget(): boolean {
  const draggedFolderId = treeDragState.folderId;
  return draggedFolderId ? folderIsWithin(props.folder.id, draggedFolderId) : false;
}

function folderIsWithin(folderId: string, ancestorId: string): boolean {
  let cursor = vaultState.folders.find((folder) => folder.id === folderId);
  const visited = new Set<string>();
  while (cursor && !visited.has(cursor.id)) {
    if (cursor.id === ancestorId) return true;
    visited.add(cursor.id);
    const parentId = cursor.parentId;
    cursor = parentId
      ? vaultState.folders.find((folder) => folder.id === parentId)
      : undefined;
  }
  return false;
}

function scheduleExpand(): void {
  if (expanded.value || !hasContents.value || expandTimer !== undefined) return;
  expandTimer = window.setTimeout(() => {
    expanded.value = true;
    expandTimer = undefined;
  }, 550);
}

function clearExpandTimer(): void {
  if (expandTimer === undefined) return;
  window.clearTimeout(expandTimer);
  expandTimer = undefined;
}

function handleDragEnter(event: DragEvent): void {
  if (!isTreeDrag(event)) return;
  event.preventDefault();
  event.stopPropagation();
  dropInvalid.value = isInvalidFolderTarget();
  dropActive.value = !dropInvalid.value;
  if (!dropInvalid.value) scheduleExpand();
}

function handleDragOver(event: DragEvent): void {
  if (!isTreeDrag(event)) return;
  event.preventDefault();
  event.stopPropagation();
  dropInvalid.value = isInvalidFolderTarget();
  if (event.dataTransfer) event.dataTransfer.dropEffect = dropInvalid.value ? "none" : "move";
  dropActive.value = !dropInvalid.value;
  if (!dropInvalid.value) scheduleExpand();
}

function handleDragLeave(event: DragEvent): void {
  const row = event.currentTarget as HTMLElement;
  if (event.relatedTarget instanceof Node && row.contains(event.relatedTarget)) return;
  dropActive.value = false;
  dropInvalid.value = false;
  clearExpandTimer();
}

function handleDrop(event: DragEvent): void {
  dropActive.value = false;
  const invalid = dropInvalid.value || isInvalidFolderTarget();
  dropInvalid.value = false;
  clearExpandTimer();
  if (!isTreeDrag(event)) return;
  event.preventDefault();
  event.stopPropagation();
  if (invalid) {
    treeDragState.folderId = null;
    return;
  }
  const noteId = event.dataTransfer?.getData(NOTE_DRAG_MIME).trim();
  const folderId = event.dataTransfer?.getData(FOLDER_DRAG_MIME).trim();
  const moved = noteId
    ? moveNoteToFolder(noteId, props.folder.id)
    : Boolean(folderId && moveFolder(folderId, props.folder.id));
  treeDragState.folderId = null;
  if (moved) expanded.value = true;
}

function toggleExpanded(): void {
  if (canExpand.value) expanded.value = !expanded.value;
}

function handleDisclosureKeydown(event: KeyboardEvent): void {
  if (event.key === "ArrowRight" && canExpand.value && !expanded.value) {
    event.preventDefault();
    expanded.value = true;
  } else if (event.key === "ArrowLeft" && canExpand.value && expanded.value) {
    event.preventDefault();
    expanded.value = false;
  }
}

function toggleMenu(): void {
  if (movePickerOpen.value) closeMovePicker();
  if (menuOpen.value) {
    closeMenu(true);
    return;
  }
  menuOpen.value = true;
  nextTick(() => menu.value?.querySelector<HTMLButtonElement>("button")?.focus());
}

function closeMenu(restoreFocus = false): void {
  menuOpen.value = false;
  if (restoreFocus) nextTick(() => menuButton.value?.focus());
}

function openMovePicker(): void {
  closeMenu();
  moveTarget.value = props.folder.parentId ?? "";
  movePickerOpen.value = true;
  nextTick(() => moveSelect.value?.focus());
}

function closeMovePicker(restoreFocus = false): void {
  movePickerOpen.value = false;
  if (restoreFocus) nextTick(() => menuButton.value?.focus());
}

function submitMove(): void {
  const parentId = moveTarget.value || null;
  if (parentId !== props.folder.parentId) moveFolder(props.folder.id, parentId);
  closeMovePicker(true);
}

function handleMoveFocusOut(event: FocusEvent): void {
  const form = event.currentTarget as HTMLElement;
  if (event.relatedTarget instanceof Node && form.contains(event.relatedTarget)) return;
  closeMovePicker();
}

function handleMenuFocusOut(event: FocusEvent): void {
  const anchor = event.currentTarget as HTMLElement;
  if (event.relatedTarget instanceof Node && anchor.contains(event.relatedTarget)) return;
  closeMenu();
}

function handleMenuKeydown(event: KeyboardEvent): void {
  if (event.key === "Escape") {
    event.preventDefault();
    closeMenu(true);
    return;
  }

  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  const items = Array.from(menu.value?.querySelectorAll<HTMLButtonElement>("[role='menuitem']") ?? []);
  if (!items.length) return;
  event.preventDefault();
  const currentIndex = items.indexOf(document.activeElement as HTMLButtonElement);
  let nextIndex = currentIndex;
  if (event.key === "Home") nextIndex = 0;
  else if (event.key === "End") nextIndex = items.length - 1;
  else if (event.key === "ArrowDown") nextIndex = (currentIndex + 1) % items.length;
  else nextIndex = (currentIndex - 1 + items.length) % items.length;
  items[nextIndex]?.focus();
}

function beginRename(): void {
  closeMenu();
  editValue.value = props.folder.name;
  editing.value = true;
  nextTick(() => {
    renameInput.value?.focus();
    renameInput.value?.select();
  });
}

function saveRename(): void {
  if (!editing.value) return;
  renameFolder(props.folder.id, editValue.value);
  editValue.value = props.folder.name;
  editing.value = false;
}

function cancelRename(): void {
  editValue.value = props.folder.name;
  editing.value = false;
}

function openSubfolderInput(): void {
  closeMenu();
  subfolderInputOpen.value = true;
  expanded.value = true;
  nextTick(() => subfolderInput.value?.focus());
}

function closeSubfolderInput(): void {
  subfolderInputOpen.value = false;
  subfolderName.value = "";
}

function submitSubfolder(): void {
  const name = subfolderName.value.trim();
  if (name) createFolder(name, props.folder.id);
  closeSubfolderInput();
}

function addNoteInside(): void {
  closeMenu();
  createNote(props.folder.id);
  expanded.value = true;
}

function removeFolder(): void {
  closeMenu();
  if (window.confirm(`Remove the folder “${props.folder.name}”? Its contents will move up one level.`)) {
    deleteFolder(props.folder.id);
  }
}
</script>

<template>
  <div
    v-if="branchVisible"
    class="vault-tree-folder"
    :class="{ 'is-expanded': expanded, 'is-empty': !hasContents, 'is-dragging': dragging }"
  >
    <div
      class="vault-tree-folder-row"
      :class="{ 'drop-active': dropActive, 'drop-invalid': dropInvalid }"
      :style="{ '--tree-depth': depth }"
      @dragenter="handleDragEnter"
      @dragover="handleDragOver"
      @dragleave="handleDragLeave"
      @drop="handleDrop"
    >
      <button
        v-if="!editing"
        type="button"
        class="vault-tree-folder-main"
        draggable="true"
        :aria-expanded="canExpand ? expanded : undefined"
        @click="toggleExpanded"
        @keydown="handleDisclosureKeydown"
        @dragstart="startFolderDrag"
        @dragend="finishFolderDrag"
      >
        <AppIcon
          class="vault-tree-folder-chevron"
          :class="{ invisible: !canExpand }"
          :name="expanded ? 'chevron-down' : 'chevron'"
          :size="11"
        />
        <AppIcon :name="expanded && canExpand ? 'folder-open' : 'folder'" :size="14" />
        <span class="vault-tree-folder-name">{{ folder.name }}</span>
      </button>

      <form v-else class="vault-tree-folder-rename" @submit.prevent="saveRename">
        <input
          ref="renameInput"
          v-model="editValue"
          type="text"
          maxlength="120"
          aria-label="Folder name"
          @blur="saveRename"
          @keydown.esc.prevent.stop="cancelRename"
        />
      </form>

      <button
        v-if="!editing"
        type="button"
        class="vault-tree-folder-add"
        :aria-label="`New folder inside ${folder.name}`"
        :title="`New folder inside ${folder.name}`"
        @click.stop="openSubfolderInput"
      >
        <AppIcon name="folder-plus" :size="13" />
      </button>

      <div v-if="!editing" class="vault-tree-folder-menu-anchor" @focusout="handleMenuFocusOut">
        <button
          ref="menuButton"
          type="button"
          class="vault-tree-folder-more"
          :aria-label="`Actions for ${folder.name}`"
          aria-haspopup="menu"
          :aria-expanded="menuOpen || movePickerOpen"
          @click.stop="toggleMenu"
        >
          <AppIcon name="more" :size="14" />
        </button>
        <Transition name="popover-fade">
          <div
            v-if="menuOpen"
            ref="menu"
            class="popover-menu vault-tree-folder-popover"
            role="menu"
            :aria-label="`Actions for ${folder.name}`"
            @keydown="handleMenuKeydown"
          >
            <button type="button" role="menuitem" @click="beginRename">
              <AppIcon name="edit" :size="14" /> Rename
            </button>
            <button type="button" role="menuitem" @click="addNoteInside">
              <AppIcon name="file-plus" :size="14" /> New note inside
            </button>
            <button type="button" role="menuitem" @click="openSubfolderInput">
              <AppIcon name="folder-plus" :size="14" /> New folder inside
            </button>
            <button type="button" role="menuitem" @click="openMovePicker">
              <AppIcon name="arrow" :size="14" /> Move folder…
            </button>
            <button type="button" class="danger" role="menuitem" @click="removeFolder">
              <AppIcon name="trash" :size="14" /> Remove
            </button>
          </div>
        </Transition>
        <Transition name="popover-fade">
          <form
            v-if="movePickerOpen"
            class="popover-menu vault-tree-move-popover"
            @submit.prevent="submitMove"
            @focusout="handleMoveFocusOut"
            @keydown.esc.prevent="closeMovePicker(true)"
          >
            <strong>Move {{ folder.name }}</strong>
            <label>
              <span>Destination</span>
              <select ref="moveSelect" v-model="moveTarget" aria-label="Move folder to">
                <option value="">Vault root</option>
                <option v-for="destination in moveDestinations" :key="destination.id" :value="destination.id">
                  {{ folderPath(destination.id) }}
                </option>
              </select>
            </label>
            <div class="vault-tree-move-actions">
              <button type="button" @click="closeMovePicker(true)">Cancel</button>
              <button type="submit" :disabled="moveTarget === (folder.parentId ?? '')">Move</button>
            </div>
          </form>
        </Transition>
      </div>
    </div>

    <Transition name="collapse-fade">
      <div v-if="expanded && canExpand" class="vault-tree-folder-children">
        <Transition name="collapse-fade">
          <form
            v-if="subfolderInputOpen"
            class="vault-tree-subfolder-form"
            :style="{ '--tree-depth': depth + 1 }"
            @submit.prevent="submitSubfolder"
          >
            <AppIcon name="folder" :size="14" />
            <input
              ref="subfolderInput"
              v-model="subfolderName"
              type="text"
              maxlength="120"
              autocomplete="off"
              placeholder="Folder name"
              :aria-label="`New folder inside ${folder.name}`"
              @blur="submitSubfolder"
              @keydown.esc.prevent.stop="closeSubfolderInput"
            />
          </form>
        </Transition>
        <VaultTreeFolder
          v-for="child in children"
          :key="child.id"
          :folder="child"
          :notes="notes"
          :depth="depth + 1"
          :show-empty-folders="showEmptyFolders"
        />
        <VaultTreeNote
          v-for="note in directNotes"
          :key="note.id"
          :note="note"
          :depth="depth + 1"
        />
      </div>
    </Transition>
  </div>
</template>
