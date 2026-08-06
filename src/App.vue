<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from "vue";
import ActivityRail from "./components/ActivityRail.vue";
import AppIcon from "./components/AppIcon.vue";
import CommandPalette from "./components/CommandPalette.vue";
import EditorWorkspace from "./components/EditorWorkspace.vue";
import ExplorerSidebar from "./components/ExplorerSidebar.vue";
import LinkInspector from "./components/LinkInspector.vue";
import NoteList from "./components/NoteList.vue";
import SearchWorkspace from "./components/SearchWorkspace.vue";
import SettingsView from "./components/SettingsView.vue";
import SnippetStudio from "./components/SnippetStudio.vue";
import TemplateGallery from "./components/TemplateGallery.vue";
import { activeNote, createNote, uiState, vaultState } from "./stores/vault";
import type { ToolView } from "./types";

const requestedView = new URLSearchParams(window.location.search).get("view") as ToolView | null;
if (requestedView && ["notes", "search", "templates", "snippets", "settings"].includes(requestedView)) {
  uiState.tool = requestedView;
}

const titlebarContext = computed(() => {
  if (uiState.tool === "notes") return activeNote.value?.title || vaultState.name;
  if (uiState.tool === "search") return "Search";
  if (uiState.tool === "templates") return "Templates";
  if (uiState.tool === "snippets") return "CSS snippets";
  return "Settings";
});

function handleKeyboard(event: KeyboardEvent): void {
  const modifier = event.metaKey || event.ctrlKey;
  const key = event.key.toLocaleLowerCase();

  if (modifier && key === "k") {
    event.preventDefault();
    uiState.commandOpen = !uiState.commandOpen;
    return;
  }
  if (modifier && key === "n") {
    event.preventDefault();
    createNote();
    return;
  }
  if (modifier && event.shiftKey && key === "t") {
    event.preventDefault();
    uiState.tool = "templates";
    return;
  }
  if (modifier && key === "\\") {
    event.preventDefault();
    uiState.explorerOpen = !uiState.explorerOpen;
    return;
  }
  if (event.key === "Escape" && uiState.commandOpen) {
    uiState.commandOpen = false;
    return;
  }

  const target = event.target as HTMLElement;
  const isEditing = target.matches("input, textarea, select, [contenteditable='true']");
  if (!isEditing && event.key === "/" && uiState.tool === "notes") {
    event.preventDefault();
    document.querySelector<HTMLInputElement>(".list-filter input")?.focus();
  }
}

onMounted(() => window.addEventListener("keydown", handleKeyboard));
onBeforeUnmount(() => window.removeEventListener("keydown", handleKeyboard));
</script>

<template>
  <div class="app-frame" :class="`tool-${uiState.tool}`">
    <header class="desktop-titlebar" data-tauri-drag-region>
      <div class="traffic-light-space" data-tauri-drag-region />
      <div class="titlebar-title" data-tauri-drag-region>
        <span>Obsidian At Home</span>
        <i />
        <small>{{ titlebarContext }}</small>
      </div>
      <div class="titlebar-local" data-tauri-drag-region>
        <span /> Local
      </div>
    </header>

    <div class="app-content" :class="{ 'notes-layout': uiState.tool === 'notes' }">
      <ActivityRail />

      <template v-if="uiState.tool === 'notes'">
        <ExplorerSidebar v-if="uiState.explorerOpen" />
        <NoteList />
        <EditorWorkspace />
        <LinkInspector v-if="uiState.contextOpen" />
      </template>
      <SearchWorkspace v-else-if="uiState.tool === 'search'" />
      <TemplateGallery v-else-if="uiState.tool === 'templates'" />
      <SnippetStudio v-else-if="uiState.tool === 'snippets'" />
      <SettingsView v-else />
    </div>

    <CommandPalette v-if="uiState.commandOpen" />

    <Transition name="toast">
      <div v-if="uiState.toast" :key="uiState.toast.id" class="app-toast" :class="`tone-${uiState.toast.tone}`" role="status">
        <span class="toast-icon">
          <AppIcon :name="uiState.toast.tone === 'success' ? 'check' : uiState.toast.tone === 'warning' ? 'info' : 'sparkles'" :size="15" />
        </span>
        <span>{{ uiState.toast.message }}</span>
      </div>
    </Transition>
  </div>
</template>
