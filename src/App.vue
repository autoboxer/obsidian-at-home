<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from "vue";
import ActivityRail from "./components/ActivityRail.vue";
import AppIcon from "./components/AppIcon.vue";
import CommandPalette from "./components/CommandPalette.vue";
import EditorWorkspace from "./components/EditorWorkspace.vue";
import ExplorerSidebar from "./components/ExplorerSidebar.vue";
import LinkInspector from "./components/LinkInspector.vue";
import SearchWorkspace from "./components/SearchWorkspace.vue";
import SettingsView from "./components/SettingsView.vue";
import SnippetStudio from "./components/SnippetStudio.vue";
import TemplateGallery from "./components/TemplateGallery.vue";
import VaultChooser from "./components/VaultChooser.vue";
import { activeNote, createNote, uiState, vaultSession, vaultState } from "./stores/vault";
import type { ToolView } from "./types";

const requestedView = new URLSearchParams(window.location.search).get("view") as ToolView | null;
if (requestedView && ["notes", "search", "templates", "snippets", "settings"].includes(requestedView)) {
  uiState.tool = requestedView;
}

const vaultChooserVisible = computed(
  () => vaultSession.phase !== "ready" || uiState.vaultChooserOpen,
);

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
  const target = event.target;
  const isEditing = target instanceof HTMLElement
    && target.matches("input, textarea, select, [contenteditable='true']");
  const appShortcut = (
    modifier && ["k", "n", "\\"].includes(key)
    || modifier && event.shiftKey && key === "t"
    || !modifier && !isEditing && key === "/"
  );

  if (vaultSession.phase !== "ready" || uiState.vaultChooserOpen) {
    if (appShortcut) event.preventDefault();
    return;
  }

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
    if (uiState.tool !== "notes") {
      uiState.tool = "notes";
      uiState.explorerOpen = true;
    } else {
      uiState.explorerOpen = !uiState.explorerOpen;
    }
    return;
  }
  if (event.key === "Escape" && uiState.commandOpen) {
    uiState.commandOpen = false;
    return;
  }

  if (!isEditing && event.key === "/" && uiState.tool === "notes") {
    event.preventDefault();
    document.querySelector<HTMLInputElement>(".vault-tree-filter input")?.focus();
  }
}

onMounted(() => window.addEventListener("keydown", handleKeyboard));
onBeforeUnmount(() => window.removeEventListener("keydown", handleKeyboard));
</script>

<template>
  <div class="app-frame" :class="`tool-${uiState.tool}`">
    <header class="desktop-titlebar" data-tauri-drag-region :inert="vaultChooserVisible">
      <div class="traffic-light-space" data-tauri-drag-region />
      <div class="titlebar-title" data-tauri-drag-region>
        <span>Obsidian At Home</span>
        <i />
        <small>{{ titlebarContext }}</small>
      </div>
      <div data-tauri-drag-region />
    </header>

    <div class="app-content" :inert="vaultChooserVisible">
      <ActivityRail />

      <Transition name="workspace-switch" mode="out-in">
        <div v-if="uiState.tool === 'notes'" key="notes" class="notes-workspace">
          <Transition name="panel-left">
            <ExplorerSidebar v-if="uiState.explorerOpen" />
          </Transition>
          <EditorWorkspace />
          <Transition name="panel-right">
            <LinkInspector v-if="uiState.contextOpen" />
          </Transition>
        </div>
        <SearchWorkspace v-else-if="uiState.tool === 'search'" key="search" />
        <TemplateGallery v-else-if="uiState.tool === 'templates'" key="templates" />
        <SnippetStudio v-else-if="uiState.tool === 'snippets'" key="snippets" />
        <SettingsView v-else-if="uiState.tool === 'settings'" key="settings" />
      </Transition>
    </div>

    <Transition name="overlay-fade">
      <CommandPalette v-if="uiState.commandOpen" />
    </Transition>

    <Transition name="overlay-fade">
      <VaultChooser v-if="vaultChooserVisible" />
    </Transition>

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
