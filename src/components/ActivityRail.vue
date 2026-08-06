<script setup lang="ts">
import appIcon from "../assets/app-icon.png";
import { uiState, vaultState } from "../stores/vault";
import type { ToolView } from "../types";
import AppIcon from "./AppIcon.vue";

const primaryTools: Array<{ id: ToolView; label: string; icon: string; shortcut?: string }> = [
  { id: "notes", label: "Notes", icon: "notes" },
  { id: "search", label: "Search", icon: "search", shortcut: "⌘K" },
  { id: "templates", label: "Templates", icon: "templates" },
  { id: "snippets", label: "CSS snippets", icon: "snippets" },
];

function selectTool(tool: ToolView): void {
  uiState.tool = tool;
  if (tool === "search") uiState.commandOpen = true;
}
</script>

<template>
  <aside class="activity-rail">
    <button class="rail-brand" type="button" title="Obsidian At Home" @click="selectTool('notes')">
      <img :src="appIcon" alt="" />
    </button>

    <nav class="rail-nav" aria-label="Workspace">
      <button
        v-for="tool in primaryTools"
        :key="tool.id"
        type="button"
        class="rail-button"
        :class="{ active: uiState.tool === tool.id || (tool.id === 'search' && uiState.commandOpen) }"
        :aria-label="tool.label"
        :title="tool.shortcut ? `${tool.label} · ${tool.shortcut}` : tool.label"
        @click="selectTool(tool.id)"
      >
        <AppIcon :name="tool.icon" :size="19" />
        <span class="rail-tooltip">{{ tool.label }}<kbd v-if="tool.shortcut">{{ tool.shortcut }}</kbd></span>
      </button>
    </nav>

    <div class="rail-spacer" />
    <div class="rail-vault-status" :title="`${vaultState.notes.length} notes stored locally`">
      <span class="local-status-dot" />
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
