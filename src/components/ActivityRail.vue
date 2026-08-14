<script setup lang="ts">
import appIcon from "../assets/app-icon.png";
import { computed, ref } from "vue";
import {
  MAX_ZOOM,
  MIN_ZOOM,
  openSearchWorkspace,
  resetZoom,
  uiState,
  zoomIn,
  zoomOut,
} from "../stores/vault";
import type { ToolView } from "../types";
import AppIcon from "./AppIcon.vue";
import KeyboardShortcutsReference from "./KeyboardShortcutsReference.vue";

const primaryTools: Array<{ id: ToolView; label: string; icon: string; shortcut?: string }> = [
  { id: "notes", label: "Notes", icon: "notes" },
  { id: "search", label: "Search", icon: "search" },
  { id: "templates", label: "Templates", icon: "templates" },
  { id: "snippets", label: "CSS snippets", icon: "snippets" },
];

const zoomMenuOpen = ref(false);
const zoomPercent = computed(() => Math.round(uiState.zoom * 100));

function selectTool(tool: ToolView): void {
  if (tool === "search") {
    openSearchWorkspace();

    return;
  }
  uiState.tool = tool;
  if (tool === "notes") {
    uiState.notesView = "editor";
  }
}

function handleZoomFocusOut(event: FocusEvent): void {
  const zoomControl = event.currentTarget as HTMLElement;
  if (!(event.relatedTarget instanceof Node) || !zoomControl.contains(event.relatedTarget)) {
    zoomMenuOpen.value = false;
  }
}
</script>

<template>
  <aside class="activity-rail" data-ui-region="activity-rail">
    <button class="rail-brand" type="button" title="Obsidian At Home" @click="selectTool('notes')">
      <img :src="appIcon" alt="" />
    </button>

    <nav class="rail-nav" aria-label="Workspace">
      <button
        v-for="tool in primaryTools"
        :key="tool.id"
        type="button"
        class="rail-button"
        :class="{ active: uiState.tool === tool.id }"
        :aria-label="tool.label"
        :title="tool.shortcut ? `${tool.label} · ${tool.shortcut}` : tool.label"
        @click="selectTool(tool.id)"
      >
        <AppIcon :name="tool.icon" :size="19" />
        <span class="rail-tooltip">{{ tool.label }}<kbd v-if="tool.shortcut">{{ tool.shortcut }}</kbd></span>
      </button>
    </nav>

    <div class="rail-spacer" />
    <KeyboardShortcutsReference />
    <div class="rail-zoom-control" @focusout="handleZoomFocusOut" @keydown.esc="zoomMenuOpen = false">
      <button
        type="button"
        class="rail-button"
        :class="{ active: zoomMenuOpen || uiState.zoom !== 1 }"
        :aria-expanded="zoomMenuOpen"
        :aria-label="`Zoom controls. Current zoom ${zoomPercent}%`"
        :title="`Zoom ${zoomPercent}% · ⌘+ / ⌘− / ⌘0`"
        @click="zoomMenuOpen = !zoomMenuOpen"
      >
        <AppIcon name="zoom-in" :size="18" />
      </button>
      <Transition name="popover-fade">
        <div v-if="zoomMenuOpen" class="rail-zoom-popover">
          <span>App zoom</span>
          <div class="rail-zoom-actions">
            <button type="button" :disabled="uiState.zoom <= MIN_ZOOM" aria-label="Zoom out" title="Zoom out · ⌘−" @click="zoomOut">
              <AppIcon name="minus" :size="14" />
            </button>
            <button type="button" aria-label="Reset zoom" title="Reset zoom · ⌘0" @click="resetZoom">
              {{ zoomPercent }}%
            </button>
            <button type="button" :disabled="uiState.zoom >= MAX_ZOOM" aria-label="Zoom in" title="Zoom in · ⌘+" @click="zoomIn">
              <AppIcon name="plus" :size="14" />
            </button>
          </div>
          <small>⌘/Ctrl + · − · 0</small>
        </div>
      </Transition>
    </div>
    <button
      type="button"
      class="rail-button"
      :class="{ active: uiState.tool === 'settings' }"
      aria-label="Settings"
      title="Settings"
      @click="selectTool('settings')"
    >
      <AppIcon name="settings" :size="19" />
      <span class="rail-tooltip">Settings</span>
    </button>
  </aside>
</template>
