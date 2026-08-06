<script setup lang="ts">
import { computed } from "vue";
import {
  activeNote,
  backlinks,
  createLinkedNote,
  folderPath,
  outgoingLinks,
  selectNote,
  uiState,
} from "../stores/vault";
import AppIcon from "./AppIcon.vue";

const uniqueOutgoing = computed(() => {
  const seen = new Set<string>();
  return outgoingLinks.value.filter(({ link }) => {
    const key = link.target.toLocaleLowerCase();
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
});

const wordCount = computed(() => {
  const content = activeNote.value?.content.trim() ?? "";
  return content ? content.split(/\s+/).length : 0;
});

function openOutgoing(target: string, noteId?: string): void {
  if (noteId) selectNote(noteId);
  else createLinkedNote(target);
}

function formatDate(timestamp?: number): string {
  if (!timestamp) return "—";
  return new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" }).format(timestamp);
}
</script>

<template>
  <aside class="link-inspector">
    <header class="inspector-header">
      <div>
        <span class="eyebrow">Note context</span>
        <h2>{{ uiState.inspectorTab === 'links' ? 'Connections' : 'Details' }}</h2>
      </div>
      <button type="button" class="icon-button subtle" title="Close context" @click="uiState.contextOpen = false">
        <AppIcon name="x" :size="15" />
      </button>
    </header>

    <div class="inspector-tabs">
      <button type="button" :class="{ active: uiState.inspectorTab === 'links' }" @click="uiState.inspectorTab = 'links'">
        <AppIcon name="link" :size="14" /> Links
      </button>
      <button type="button" :class="{ active: uiState.inspectorTab === 'info' }" @click="uiState.inspectorTab = 'info'">
        <AppIcon name="info" :size="14" /> Info
      </button>
    </div>

    <div v-if="activeNote" class="inspector-scroll">
      <template v-if="uiState.inspectorTab === 'links'">
        <section class="connection-section">
          <div class="connection-heading">
            <span><AppIcon name="link" :size="14" /> Outgoing links</span>
            <small>{{ uniqueOutgoing.length }}</small>
          </div>
          <div v-if="uniqueOutgoing.length" class="connection-list">
            <button
              v-for="item in uniqueOutgoing"
              :key="`${item.link.target}-${item.link.index}`"
              type="button"
              class="connection-card outgoing-card"
              :class="{ unresolved: !item.note }"
              @click="openOutgoing(item.link.target, item.note?.id)"
            >
              <span class="connection-node"><AppIcon :name="item.note ? 'file-text' : 'plus'" :size="14" /></span>
              <span>
                <strong>{{ item.link.display || item.link.target }}</strong>
                <small>{{ item.note ? (item.note.folderId ? folderPath(item.note.folderId) : 'Unfiled') : 'Create this note' }}</small>
              </span>
              <AppIcon name="chevron" :size="13" />
            </button>
          </div>
          <div v-else class="empty-connection">
            <span>Type <code>[[</code> in the editor to connect another note.</span>
          </div>
        </section>

        <section class="connection-section backlink-section">
          <div class="connection-heading">
            <span><AppIcon name="backlinks" :size="15" /> Backlinks</span>
            <small>{{ backlinks.length }}</small>
          </div>
          <div v-if="backlinks.length" class="backlink-list">
            <button v-for="backlink in backlinks" :key="`${backlink.note.id}-${backlink.link.index}`" type="button" class="backlink-card" @click="selectNote(backlink.note.id)">
              <span class="backlink-source">
                <span class="backlink-dot" />
                <strong>{{ backlink.note.title }}</strong>
                <small>{{ backlink.note.folderId ? folderPath(backlink.note.folderId) : 'Unfiled' }}</small>
              </span>
              <span class="backlink-excerpt">{{ backlink.excerpt }}</span>
            </button>
          </div>
          <div v-else class="empty-connection quiet">
            <span>No notes link here yet.</span>
          </div>
        </section>
      </template>

      <template v-else>
        <section class="note-info-card">
          <div class="info-row"><span>Created</span><strong>{{ formatDate(activeNote.createdAt) }}</strong></div>
          <div class="info-row"><span>Updated</span><strong>{{ formatDate(activeNote.updatedAt) }}</strong></div>
          <div class="info-row"><span>Folder</span><strong>{{ activeNote.folderId ? folderPath(activeNote.folderId) : 'Unfiled' }}</strong></div>
          <div class="info-row"><span>Words</span><strong>{{ wordCount.toLocaleString() }}</strong></div>
          <div class="info-row"><span>Characters</span><strong>{{ activeNote.content.length.toLocaleString() }}</strong></div>
        </section>
        <section class="info-tags">
          <div class="connection-heading"><span><AppIcon name="tag" :size="14" /> Tags</span><small>{{ activeNote.tags.length }}</small></div>
          <div v-if="activeNote.tags.length" class="info-tag-cloud">
            <span v-for="tag in activeNote.tags" :key="tag">#{{ tag }}</span>
          </div>
          <div v-else class="empty-connection quiet">No tags on this note.</div>
        </section>
      </template>
    </div>
  </aside>
</template>
