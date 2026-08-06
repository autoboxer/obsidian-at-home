<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import {
  createFolder,
  createNote,
  folderChildren,
  selectFolder,
  uiState,
  vaultState,
} from "../stores/vault";
import AppIcon from "./AppIcon.vue";
import FolderTreeItem from "./FolderTreeItem.vue";

const folderInputOpen = ref(false);
const folderName = ref("");
const folderField = ref<HTMLInputElement>();

const vaultMonogram = computed(() => {
  const words = vaultState.name.trim().split(/\s+/).filter(Boolean);
  if (!words.length) return "VA";
  if (words.length === 1) return Array.from(words[0]).slice(0, 2).join("").toLocaleUpperCase();
  return `${Array.from(words[0])[0] ?? ""}${Array.from(words.at(-1) ?? "")[0] ?? ""}`.toLocaleUpperCase();
});

const topTags = computed(() => {
  const counts = new Map<string, number>();
  for (const note of vaultState.notes) {
    for (const tag of note.tags) counts.set(tag, (counts.get(tag) ?? 0) + 1);
  }
  return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 8);
});

function openFolderInput(): void {
  folderInputOpen.value = true;
  nextTick(() => folderField.value?.focus());
}

function submitFolder(): void {
  if (folderName.value.trim()) createFolder(folderName.value);
  folderName.value = "";
  folderInputOpen.value = false;
}

function searchTag(tag: string): void {
  uiState.noteFilter = tag;
  uiState.tool = "search";
  uiState.commandOpen = true;
}

function openVaultChooser(): void {
  uiState.commandOpen = false;
  uiState.vaultChooserOpen = true;
}
</script>

<template>
  <aside class="explorer-sidebar">
    <header class="vault-header">
      <button
        type="button"
        class="vault-identity"
        :aria-label="`Switch vault. Current vault: ${vaultState.name}`"
        title="Switch vault"
        @click="openVaultChooser"
      >
        <span class="vault-monogram">{{ vaultMonogram }}</span>
        <span class="vault-identity-copy">
          <strong>{{ vaultState.name }}</strong>
          <small>Switch vault</small>
        </span>
        <AppIcon class="vault-identity-chevron" name="chevron-down" :size="12" />
      </button>
      <button type="button" class="icon-button subtle" title="Hide vault panel" @click="uiState.explorerOpen = false">
        <AppIcon name="sidebar" :size="16" />
      </button>
    </header>

    <button type="button" class="new-note-button" @click="createNote()">
      <span><AppIcon name="plus" :size="16" /></span>
      New note
      <kbd>⌘N</kbd>
    </button>

    <div class="explorer-scroll">
      <section class="explorer-section smart-folders">
        <button type="button" :class="{ active: vaultState.selectedFolderId === 'all' }" @click="selectFolder('all')">
          <AppIcon name="notes" :size="15" />
          <span>All notes</span>
          <small>{{ vaultState.notes.length }}</small>
        </button>
        <button type="button" :class="{ active: vaultState.selectedFolderId === 'favorites' }" @click="selectFolder('favorites')">
          <AppIcon name="star" :size="15" />
          <span>Favorites</span>
          <small>{{ vaultState.notes.filter((note) => note.pinned).length }}</small>
        </button>
        <button type="button" :class="{ active: vaultState.selectedFolderId === 'unfiled' }" @click="selectFolder('unfiled')">
          <AppIcon name="archive" :size="15" />
          <span>Unfiled</span>
          <small>{{ vaultState.notes.filter((note) => !note.folderId).length }}</small>
        </button>
      </section>

      <section class="explorer-section folder-section">
        <div class="section-label">
          <span>Folders</span>
          <button type="button" aria-label="New folder" @click="openFolderInput">
            <AppIcon name="plus" :size="13" />
          </button>
        </div>
        <Transition name="collapse-fade">
          <form v-if="folderInputOpen" class="new-folder-form" @submit.prevent="submitFolder">
            <AppIcon name="folder" :size="14" />
            <input ref="folderField" v-model="folderName" placeholder="Folder name" @blur="submitFolder" @keydown.esc="folderInputOpen = false" />
          </form>
        </Transition>
        <div class="folder-tree">
          <FolderTreeItem v-for="folder in folderChildren(null)" :key="folder.id" :folder="folder" />
        </div>
      </section>

      <Transition name="collapse-fade">
        <section v-if="topTags.length" class="explorer-section tags-section">
          <div class="section-label"><span>Tags</span></div>
          <div class="tag-list">
            <button v-for="([tag, count]) in topTags" :key="tag" type="button" @click="searchTag(tag)">
              <span><em>#</em>{{ tag }}</span><small>{{ count }}</small>
            </button>
          </div>
        </section>
      </Transition>
    </div>

    <footer class="explorer-footer">
      <span>{{ vaultState.notes.length }} notes</span>
    </footer>
  </aside>
</template>
