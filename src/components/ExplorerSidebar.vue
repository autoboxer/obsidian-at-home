<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import {
  createFolder,
  createNote,
  FOLDER_DRAG_MIME,
  moveFolder,
  moveNoteToFolder,
  NOTE_DRAG_MIME,
  openSearchWorkspace,
  selectFolder,
  treeDragState,
  uiState,
  vaultState,
  visibleNotes,
} from "../stores/vault";
import AppIcon from "./AppIcon.vue";
import RecentNotesSection from "./RecentNotesSection.vue";
import VaultTreeFolder from "./VaultTreeFolder.vue";
import VaultTreeNote from "./VaultTreeNote.vue";

const folderInputOpen = ref(false);
const folderName = ref("");
const folderField = ref<HTMLInputElement>();
const rootExpanded = ref(true);
const rootDropActive = ref(false);
const rootDropInvalid = ref(false);

const vaultMonogram = computed(() => {
  const words = vaultState.name.trim().split(/\s+/).filter(Boolean);
  if (!words.length) {
    return "VA";
  }
  if (words.length === 1) {
    return Array.from(words[0]).slice(0, 2).join("").toLocaleUpperCase();
  }

  return `${Array.from(words[0])[0] ?? ""}${Array.from(words.at(-1) ?? "")[0] ?? ""}`.toLocaleUpperCase();
});

const topTags = computed(() => {
  const counts = new Map<string, { tag: string; count: number }>();
  for (const note of vaultState.notes) {
    const noteTags = new Set<string>();
    for (const tag of note.tags) {
      const key = normalizeTagKey(tag);
      if (noteTags.has(key)) {
        continue;
      }
      noteTags.add(key);
      const existing = counts.get(key);
      if (existing) {
        existing.count += 1;
      } else {
        counts.set(key, { tag, count: 1 });
      }
    }
  }

  return [...counts.values()]
    .map(({ tag, count }) => [tag, count] as const)
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .slice(0, 8);
});

function normalizeTagKey(tag: string): string {
  return tag
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLocaleLowerCase()
    .trim();
}

const rootFolders = computed(() => [...vaultState.folders]
  .filter((folder) => folder.parentId === null)
  .sort((a, b) => a.name.localeCompare(b.name)));

const showEmptyFolders = computed(() =>
  vaultState.selectedFolderId === "all" && !uiState.noteFilter.trim(),
);

const rootNotes = computed(() => [...visibleNotes.value]
  .filter((note) => note.folderId === null)
  .sort((a, b) => a.title.localeCompare(b.title)));

const emptyTreeMessage = computed(() => {
  if (vaultState.selectedFolderId === "favorites") {
    return "No favorite notes";
  }
  if (uiState.noteFilter.trim()) {
    return "No matching notes";
  }

  return "Create your first note";
});

const treeAriaLabel = computed(() => {
  if (vaultState.selectedFolderId === "favorites") {
    return "Favorite files";
  }

  return "All files";
});

watch(
  () => [treeDragState.noteId, treeDragState.folderId] as const,
  ([noteId, folderId]) => {
    if (!noteId && !folderId) {
      rootDropActive.value = false;
      rootDropInvalid.value = false;
    }
  },
);

function openFolderInput(): void {
  folderInputOpen.value = true;
  nextTick(() => {
    folderField.value?.focus();
    folderField.value?.select();
  });
}

function cancelFolderInput(): void {
  folderName.value = "";
  folderInputOpen.value = false;
}

function submitFolder(): void {
  if (folderName.value.trim()) {
    createFolder(folderName.value);
    rootExpanded.value = true;
  }
  folderName.value = "";
  folderInputOpen.value = false;
}

function searchTag(tag: string): void {
  openSearchWorkspace({ query: tag, scope: "tags", exactTag: tag });
}

function openVaultChooser(): void {
  uiState.commandOpen = false;
  uiState.vaultChooserOpen = true;
}

function isTreeDrag(event: DragEvent): boolean {
  const types = Array.from(event.dataTransfer?.types ?? []);

  return Boolean(treeDragState.noteId || treeDragState.folderId)
    || types.includes(NOTE_DRAG_MIME)
    || types.includes(FOLDER_DRAG_MIME);
}

function handleRootDragEnter(event: DragEvent): void {
  if (!isTreeDrag(event)) {
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  rootDropInvalid.value = isInvalidRootFolderDrop();
  rootDropActive.value = !rootDropInvalid.value;
}

function handleRootDragOver(event: DragEvent): void {
  if (!isTreeDrag(event)) {
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  rootDropInvalid.value = isInvalidRootFolderDrop();
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = rootDropInvalid.value ? "none" : "move";
  }
  rootDropActive.value = !rootDropInvalid.value;
}

function handleRootDragLeave(event: DragEvent): void {
  const target = event.currentTarget as HTMLElement;
  if (event.relatedTarget instanceof Node && target.contains(event.relatedTarget)) {
    return;
  }
  rootDropActive.value = false;
  rootDropInvalid.value = false;
}

function handleRootDrop(event: DragEvent): void {
  rootDropActive.value = false;
  const invalid = rootDropInvalid.value || isInvalidRootFolderDrop();
  rootDropInvalid.value = false;
  if (!isTreeDrag(event)) {
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  if (invalid) {
    treeDragState.noteId = null;
    treeDragState.folderId = null;

    return;
  }
  const noteId = event.dataTransfer?.getData(NOTE_DRAG_MIME).trim() || treeDragState.noteId;
  const folderId = event.dataTransfer?.getData(FOLDER_DRAG_MIME).trim() || treeDragState.folderId;
  if (noteId) {
    moveNoteToFolder(noteId, null);
  } else if (folderId) {
    moveFolder(folderId, null);
  }
  treeDragState.noteId = null;
  treeDragState.folderId = null;
  rootExpanded.value = true;
}

function isInvalidRootFolderDrop(): boolean {
  if (!treeDragState.folderId) {
    return false;
  }

  return vaultState.folders.find((folder) => folder.id === treeDragState.folderId)?.parentId === null;
}

function handleRootKeydown(event: KeyboardEvent): void {
  if (event.key === "ArrowRight" && !rootExpanded.value) {
    event.preventDefault();
    rootExpanded.value = true;
  } else if (event.key === "ArrowLeft" && rootExpanded.value) {
    event.preventDefault();
    rootExpanded.value = false;
  }
}
</script>

<template>
  <aside class="explorer-sidebar" data-ui-region="vault-panel">
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
      <button
        type="button"
        class="icon-button subtle"
        aria-label="Hide vault panel"
        title="Hide vault panel"
        @click="uiState.explorerOpen = false"
      >
        <AppIcon name="x" :size="15" />
      </button>
    </header>

    <label class="vault-tree-filter">
      <AppIcon name="search" :size="14" />
      <input v-model="uiState.noteFilter" placeholder="Filter files…" aria-label="Filter vault files" />
      <button v-if="uiState.noteFilter" type="button" aria-label="Clear filter" @click="uiState.noteFilter = ''">
        <AppIcon name="x" :size="12" />
      </button>
    </label>

    <div class="explorer-create-actions">
      <button type="button" class="explorer-create-button" title="New note · ⌘N" @click="createNote()">
        <span><AppIcon name="file-plus" :size="15" /></span>
        New note
      </button>
      <button type="button" class="explorer-create-button" title="New folder" @click="openFolderInput">
        <span><AppIcon name="folder-plus" :size="15" /></span>
        New folder
      </button>
    </div>

    <Transition name="collapse-fade">
      <form v-if="folderInputOpen" class="new-folder-form explorer-new-folder-form" @submit.prevent="submitFolder">
        <AppIcon name="folder" :size="14" />
        <input
          ref="folderField"
          v-model="folderName"
          placeholder="Folder name"
          aria-label="Folder name"
          autocomplete="off"
          @blur="submitFolder"
          @keydown.esc.prevent="cancelFolderInput"
        />
      </form>
    </Transition>

    <div class="explorer-scroll">
      <RecentNotesSection />

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
      </section>

      <section class="explorer-section vault-tree-section" aria-labelledby="vault-tree-heading">
        <div class="section-label">
          <span id="vault-tree-heading">Files</span>
          <div class="section-label-actions">
            <button type="button" aria-label="New note" title="New note · ⌘N" @click="createNote()">
              <AppIcon name="file-plus" :size="14" />
            </button>
            <button type="button" aria-label="New folder" title="New folder" @click="openFolderInput">
              <AppIcon name="folder-plus" :size="14" />
            </button>
          </div>
        </div>

        <div class="vault-file-tree" :aria-label="treeAriaLabel">
          <div
            class="vault-tree-root-row"
            :class="{ 'drop-active': rootDropActive, 'drop-invalid': rootDropInvalid }"
            @dragenter="handleRootDragEnter"
            @dragover="handleRootDragOver"
            @dragleave="handleRootDragLeave"
            @drop="handleRootDrop"
          >
            <button
              type="button"
              class="vault-tree-root-main"
              :aria-expanded="rootExpanded"
              title="Vault root · Drop notes or folders here to move them to the root"
              @click="rootExpanded = !rootExpanded"
              @keydown="handleRootKeydown"
            >
              <AppIcon :name="rootExpanded ? 'chevron-down' : 'chevron'" :size="11" />
              <AppIcon :name="rootExpanded ? 'folder-open' : 'folder'" :size="14" />
              <span>Vault root</span>
            </button>
            <small>Drop here</small>
          </div>

          <Transition name="collapse-fade">
            <div v-if="rootExpanded" class="vault-tree-root-children">
              <VaultTreeFolder
                v-for="folder in rootFolders"
                :key="folder.id"
                :folder="folder"
                :notes="visibleNotes"
                :depth="1"
                :show-empty-folders="showEmptyFolders"
              />
              <VaultTreeNote v-for="note in rootNotes" :key="note.id" :note="note" :depth="1" />

              <div v-if="!visibleNotes.length" class="vault-tree-empty">
                <AppIcon
                  :name="vaultState.selectedFolderId === 'favorites' ? 'star' : vaultState.notes.length ? 'search' : 'file-plus'"
                  :size="18"
                />
                <span>{{ emptyTreeMessage }}</span>
              </div>
            </div>
          </Transition>
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
