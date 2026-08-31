<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { formatCommandShortcut } from "../lib/keyboard";
import { VAULT_IMAGE_DRAG_MIME } from "../lib/imageEmbeds";
import { VAULT_ATTACHMENT_DRAG_MIME } from "../lib/markdownAttachments";
import {
  createFolder,
  createNote,
  FOLDER_DRAG_MIME,
  moveFolder,
  moveVaultAttachmentToFolder,
  moveVaultImageToFolder,
  moveNoteToFolder,
  NOTE_DRAG_MIME,
  notify,
  openRecentlyDeletedWorkspace,
  openSearchWorkspace,
  recentNotes,
  recentlyDeletedNotes,
  selectFolder,
  treeDragState,
  uiState,
  vaultState,
  vaultTreeItemIsRevealed,
  vaultTreeRevealTarget,
  visibleNotes,
} from "../stores/vault";
import AppIcon from "./AppIcon.vue";
import VaultTreeAttachment from "./VaultTreeAttachment.vue";
import VaultTreeFolder from "./VaultTreeFolder.vue";
import VaultTreeImage from "./VaultTreeImage.vue";
import VaultTreeNote from "./VaultTreeNote.vue";

const createNoteShortcut = formatCommandShortcut("N");
const folderInputOpen = ref(false);
const folderName = ref("");
const folderField = ref<HTMLInputElement>();
const rootExpanded = ref(true);
const rootDropActive = ref(false);
const rootDropInvalid = ref(false);
const fileTree = ref<HTMLElement>();
let revealFrame: number | undefined;

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

const visibleImages = computed(() => {
  if (vaultState.selectedFolderId !== "all") {
    return [];
  }
  const filter = uiState.noteFilter.trim().toLocaleLowerCase();

  return [...vaultState.imageFiles]
    .filter((image) => !filter || image.relativePath.toLocaleLowerCase().includes(filter))
    .sort((left, right) => left.relativePath.localeCompare(right.relativePath));
});

const rootImages = computed(() => visibleImages.value.filter((image) =>
  !image.relativePath.includes("/"),
));

const visibleAttachments = computed(() => {
  if (vaultState.selectedFolderId !== "all") {
    return [];
  }
  const filter = uiState.noteFilter.trim().toLocaleLowerCase();

  return [...vaultState.attachmentFiles]
    .filter((attachment) =>
      !filter || attachment.relativePath.toLocaleLowerCase().includes(filter)
    )
    .sort((left, right) => left.relativePath.localeCompare(right.relativePath));
});

const rootAttachments = computed(() => visibleAttachments.value.filter((attachment) =>
  !attachment.relativePath.includes("/"),
));

const emptyTreeMessage = computed(() => {
  if (vaultState.selectedFolderId === "recent") {
    return "No recent notes";
  }
  if (vaultState.selectedFolderId === "favorites") {
    return "No favorite notes";
  }
  if (uiState.noteFilter.trim()) {
    return "No matching notes";
  }

  return "Create your first note";
});

const treeAriaLabel = computed(() => {
  if (vaultState.selectedFolderId === "recent") {
    return "Recent files";
  }
  if (vaultState.selectedFolderId === "favorites") {
    return "Favorite files";
  }

  return "All files";
});

const emptyTreeIcon = computed(() => {
  if (vaultState.selectedFolderId === "recent") {
    return "clock";
  }
  if (vaultState.selectedFolderId === "favorites") {
    return "star";
  }

  return vaultState.notes.length ? "search" : "file-plus";
});

watch(
  () => [
    treeDragState.noteId,
    treeDragState.folderId,
    treeDragState.imagePath,
    treeDragState.attachmentPath,
  ] as const,
  ([noteId, folderId, imagePath, attachmentPath]) => {
    if (!noteId && !folderId && !imagePath && !attachmentPath) {
      rootDropActive.value = false;
      rootDropInvalid.value = false;
    }
  },
);

watch(
  () => vaultTreeRevealTarget.requestId,
  (requestId) => {
    if (!requestId) {
      return;
    }
    rootExpanded.value = true;
    scheduleRevealTarget(requestId);
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  if (revealFrame !== undefined) {
    window.cancelAnimationFrame(revealFrame);
  }
});

function scheduleRevealTarget(requestId: number, attempt = 0): void {
  if (revealFrame !== undefined) {
    window.cancelAnimationFrame(revealFrame);
  }
  void nextTick(() => {
    revealFrame = window.requestAnimationFrame(() => {
      revealFrame = undefined;
      if (requestId !== vaultTreeRevealTarget.requestId || !currentRevealTargetIsValid()) {
        return;
      }
      const row = findRevealTargetRow();
      if (!row) {
        if (attempt < 12) {
          scheduleRevealTarget(requestId, attempt + 1);
        } else {
          notify("The vault item could not be revealed in the app's file tree.", "warning");
        }

        return;
      }
      const primary = row.querySelector<HTMLButtonElement>("[data-vault-item-primary]");
      primary?.focus({ preventScroll: true });
      row.scrollIntoView({ behavior: "auto", block: "nearest", inline: "nearest" });
    });
  });
}

function currentRevealTargetIsValid(): boolean {
  const kind = vaultTreeRevealTarget.kind;
  if (!kind) {
    return false;
  }

  return vaultTreeItemIsRevealed({
    ...(vaultTreeRevealTarget.assetId
      ? { assetId: vaultTreeRevealTarget.assetId }
      : {}),
    kind,
    relativePath: vaultTreeRevealTarget.relativePath,
  });
}

function findRevealTargetRow(): HTMLElement | undefined {
  const candidates = fileTree.value?.querySelectorAll<HTMLElement>("[data-vault-item-kind]");

  return [...(candidates ?? [])].find((candidate) => {
    if (candidate.dataset.vaultItemKind !== vaultTreeRevealTarget.kind) {
      return false;
    }
    if (vaultTreeRevealTarget.assetId) {
      return candidate.dataset.vaultItemAssetId === vaultTreeRevealTarget.assetId;
    }

    return candidate.dataset.vaultItemRelativePath === vaultTreeRevealTarget.relativePath;
  });
}

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

  return Boolean(
    treeDragState.noteId
    || treeDragState.folderId
    || treeDragState.imagePath
    || treeDragState.attachmentPath
  )
    || types.includes(NOTE_DRAG_MIME)
    || types.includes(FOLDER_DRAG_MIME)
    || types.includes(VAULT_IMAGE_DRAG_MIME)
    || types.includes(VAULT_ATTACHMENT_DRAG_MIME);
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

async function handleRootDrop(event: DragEvent): Promise<void> {
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
    treeDragState.imagePath = null;
    treeDragState.attachmentPath = null;

    return;
  }
  const noteId = event.dataTransfer?.getData(NOTE_DRAG_MIME).trim() || treeDragState.noteId;
  const folderId = event.dataTransfer?.getData(FOLDER_DRAG_MIME).trim() || treeDragState.folderId;
  const imagePath = event.dataTransfer?.getData(VAULT_IMAGE_DRAG_MIME).trim()
    || treeDragState.imagePath;
  const attachmentPath = event.dataTransfer?.getData(VAULT_ATTACHMENT_DRAG_MIME).trim()
    || treeDragState.attachmentPath;
  const image = imagePath
    ? vaultState.imageFiles.find((candidate) => candidate.relativePath === imagePath)
    : undefined;
  const attachment = attachmentPath
    ? vaultState.attachmentFiles.find((candidate) =>
      candidate.relativePath === attachmentPath
    )
    : undefined;
  if (attachment) {
    await moveVaultAttachmentToFolder(attachment, null);
  } else if (image) {
    await moveVaultImageToFolder(image, null);
  } else if (noteId) {
    await moveNoteToFolder(noteId, null);
  } else if (folderId) {
    moveFolder(folderId, null);
  }
  treeDragState.noteId = null;
  treeDragState.folderId = null;
  treeDragState.imagePath = null;
  treeDragState.attachmentPath = null;
  rootExpanded.value = true;
}

function isInvalidRootFolderDrop(): boolean {
  if (treeDragState.attachmentPath) {
    const attachment = vaultState.attachmentFiles.find((candidate) =>
      candidate.relativePath === treeDragState.attachmentPath
    );
    if (attachment) {
      return !attachment.relativePath.includes("/");
    }
  }
  if (treeDragState.imagePath) {
    const image = vaultState.imageFiles.find((candidate) =>
      candidate.relativePath === treeDragState.imagePath
    );
    if (image) {
      return !image.relativePath.includes("/");
    }
  }
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
      <button type="button" class="explorer-create-button" :title="`New note · ${createNoteShortcut}`" @click="createNote()">
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
      <section class="explorer-section smart-folders">
        <button type="button" :class="{ active: uiState.notesView === 'editor' && vaultState.selectedFolderId === 'all' }" @click="selectFolder('all')">
          <AppIcon name="notes" :size="15" />
          <span>All notes</span>
          <small>{{ vaultState.notes.length }}</small>
        </button>
        <button type="button" :class="{ active: uiState.notesView === 'editor' && vaultState.selectedFolderId === 'recent' }" @click="selectFolder('recent')">
          <AppIcon name="clock" :size="15" />
          <span>Recent notes</span>
          <small>{{ recentNotes.length }}</small>
        </button>
        <button type="button" :class="{ active: uiState.notesView === 'editor' && vaultState.selectedFolderId === 'favorites' }" @click="selectFolder('favorites')">
          <AppIcon name="star" :size="15" />
          <span>Favorites</span>
          <small>{{ vaultState.notes.filter((note) => note.pinned).length }}</small>
        </button>
        <button
          v-if="recentlyDeletedNotes.length"
          type="button"
          :class="{ active: uiState.notesView === 'recently-deleted' }"
          @click="openRecentlyDeletedWorkspace"
        >
          <AppIcon name="trash" :size="15" />
          <span>Recently Deleted</span>
          <small>{{ recentlyDeletedNotes.length }}</small>
        </button>
      </section>

      <section class="explorer-section vault-tree-section" aria-labelledby="vault-tree-heading">
        <div class="section-label">
          <span id="vault-tree-heading">Files</span>
          <div class="section-label-actions">
            <button type="button" aria-label="New note" :title="`New note · ${createNoteShortcut}`" @click="createNote()">
              <AppIcon name="file-plus" :size="14" />
            </button>
            <button type="button" aria-label="New folder" title="New folder" @click="openFolderInput">
              <AppIcon name="folder-plus" :size="14" />
            </button>
          </div>
        </div>

        <div ref="fileTree" class="vault-file-tree" :aria-label="treeAriaLabel">
          <template v-if="vaultState.selectedFolderId === 'recent'">
            <VaultTreeNote
              v-for="note in visibleNotes"
              :key="note.id"
              :note="note"
            />

            <div v-if="!visibleNotes.length" class="vault-tree-empty">
              <AppIcon :name="emptyTreeIcon" :size="18" />
              <span>{{ emptyTreeMessage }}</span>
            </div>
          </template>

          <template v-else>
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
                title="Vault root · Drop notes, folders, images, or attachments here to move them to the root"
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
                  :images="visibleImages"
                  :attachments="visibleAttachments"
                  :depth="1"
                  :show-empty-folders="showEmptyFolders"
                />
                <VaultTreeNote v-for="note in rootNotes" :key="note.id" :note="note" :depth="1" />
                <VaultTreeImage
                  v-for="image in rootImages"
                  :key="image.relativePath"
                  :image="image"
                  :depth="1"
                />
                <VaultTreeAttachment
                  v-for="attachment in rootAttachments"
                  :key="attachment.relativePath"
                  :attachment="attachment"
                  :depth="1"
                />

                <div
                  v-if="!visibleNotes.length && !visibleImages.length && !visibleAttachments.length"
                  class="vault-tree-empty"
                >
                  <AppIcon :name="emptyTreeIcon" :size="18" />
                  <span>{{ emptyTreeMessage }}</span>
                </div>
              </div>
            </Transition>
          </template>
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
      <span>
        {{ vaultState.notes.length }} notes · {{ vaultState.imageFiles.length }} images ·
        {{ vaultState.attachmentFiles.length }} attachments
      </span>
    </footer>
  </aside>
</template>
