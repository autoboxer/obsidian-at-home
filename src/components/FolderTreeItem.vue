<script setup lang="ts">
import { computed, ref } from "vue";
import {
  createFolder,
  deleteFolder,
  folderChildren,
  noteCountForFolder,
  renameFolder,
  selectFolder,
  vaultState,
} from "../stores/vault";
import type { Folder } from "../types";
import AppIcon from "./AppIcon.vue";

defineOptions({ name: "FolderTreeItem" });

const props = withDefaults(defineProps<{ folder: Folder; depth?: number }>(), { depth: 0 });
const expanded = ref(true);
const menuOpen = ref(false);
const editing = ref(false);
const editValue = ref(props.folder.name);

const children = computed(() => folderChildren(props.folder.id));
const hasChildren = computed(() => children.value.length > 0);

function saveRename(): void {
  renameFolder(props.folder.id, editValue.value);
  editing.value = false;
}

function addSubfolder(): void {
  menuOpen.value = false;
  const name = window.prompt(`New folder inside ${props.folder.name}`);
  if (name) {
    createFolder(name, props.folder.id);
    expanded.value = true;
  }
}

function removeFolder(): void {
  menuOpen.value = false;
  if (window.confirm(`Remove the folder “${props.folder.name}”? Its notes will become unfiled.`)) {
    deleteFolder(props.folder.id);
  }
}
</script>

<template>
  <div class="folder-tree-node">
    <div
      class="folder-row"
      :class="{ active: vaultState.selectedFolderId === folder.id }"
      :style="{ '--tree-depth': depth }"
    >
      <button
        type="button"
        class="folder-chevron"
        :class="{ invisible: !hasChildren }"
        :aria-label="expanded ? 'Collapse folder' : 'Expand folder'"
        @click.stop="expanded = !expanded"
      >
        <AppIcon :name="expanded ? 'chevron-down' : 'chevron'" :size="12" />
      </button>
      <button v-if="!editing" type="button" class="folder-main" @click="selectFolder(folder.id)">
        <AppIcon :name="expanded && hasChildren ? 'folder-open' : 'folder'" :size="15" />
        <span>{{ folder.name }}</span>
        <span class="folder-count">{{ noteCountForFolder(folder.id) }}</span>
      </button>
      <form v-else class="folder-rename-form" @submit.prevent="saveRename">
        <input v-model="editValue" autofocus @blur="saveRename" @keydown.esc="editing = false" />
      </form>
      <div class="folder-menu-anchor">
        <button type="button" class="folder-more" aria-label="Folder actions" @click.stop="menuOpen = !menuOpen">
          <AppIcon name="more" :size="14" />
        </button>
        <Transition name="popover-fade">
          <div v-if="menuOpen" class="popover-menu folder-popover">
            <button type="button" @click="menuOpen = false; editing = true; editValue = folder.name">
              <AppIcon name="edit" :size="14" /> Rename
            </button>
            <button type="button" @click="addSubfolder">
              <AppIcon name="folder" :size="14" /> New subfolder
            </button>
            <button type="button" class="danger" @click="removeFolder">
              <AppIcon name="trash" :size="14" /> Remove
            </button>
          </div>
        </Transition>
      </div>
    </div>
    <Transition name="collapse-fade">
      <div v-if="expanded && hasChildren" class="folder-children">
        <FolderTreeItem
          v-for="child in children"
          :key="child.id"
          :folder="child"
          :depth="depth + 1"
        />
      </div>
    </Transition>
  </div>
</template>
