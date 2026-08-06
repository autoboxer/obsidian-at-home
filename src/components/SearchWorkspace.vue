<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import { searchNotes } from "../lib";
import {
  folderNameMap,
  folderPath,
  selectNote,
  uiState,
  vaultState,
} from "../stores/vault";
import AppIcon from "./AppIcon.vue";

const query = ref(uiState.noteFilter);
const input = ref<HTMLInputElement>();
const scope = ref<"all" | "titles" | "content">("all");

const results = computed(() => {
  const rawResults = query.value.trim()
    ? searchNotes(vaultState.notes, query.value, { folderNames: folderNameMap.value, limit: 100 })
    : [...vaultState.notes]
      .sort((a, b) => b.updatedAt - a.updatedAt)
      .map((note) => ({ note, score: 0, snippet: "", reason: "title" as const }));
  if (scope.value === "titles" && query.value) {
    const needle = query.value.toLocaleLowerCase();
    return rawResults.filter((result) => result.note.title.toLocaleLowerCase().includes(needle));
  }
  if (scope.value === "content" && query.value) {
    const needle = query.value.toLocaleLowerCase();
    return rawResults.filter((result) => result.note.content.toLocaleLowerCase().includes(needle));
  }
  return rawResults;
});

onMounted(() => nextTick(() => input.value?.focus()));

function openNote(id: string): void {
  selectNote(id);
  uiState.noteFilter = "";
  uiState.tool = "notes";
}

function formatDate(timestamp: number): string {
  return new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" }).format(timestamp);
}
</script>

<template>
  <main class="search-workspace utility-workspace">
    <div class="utility-page search-page">
      <header class="utility-hero search-hero">
        <span class="utility-eyebrow">Search</span>
        <h1>Search all notes</h1>
        <p>Search titles, content, folders, and tags. Nothing leaves this device.</p>
      </header>

      <div class="search-box-large">
        <AppIcon name="search" :size="23" />
        <input ref="input" v-model="query" placeholder="Search your notes…" aria-label="Search notes" />
        <button v-if="query" type="button" aria-label="Clear search" @click="query = ''"><AppIcon name="x" :size="15" /></button>
        <kbd v-else>⌘ K</kbd>
      </div>

      <div class="search-controls">
        <div class="search-scopes">
          <button v-for="item in ([['all', 'Everywhere'], ['titles', 'Titles'], ['content', 'Content']] as const)" :key="item[0]" type="button" :class="{ active: scope === item[0] }" @click="scope = item[0]">
            {{ item[1] }}
          </button>
        </div>
        <span>{{ results.length }} {{ results.length === 1 ? "result" : "results" }}</span>
      </div>

      <section class="search-results-grid">
        <button v-for="result in results" :key="result.note.id" type="button" class="search-result-card" @click="openNote(result.note.id)">
          <span class="search-result-topline">
            <span>{{ result.note.folderId ? folderPath(result.note.folderId) : "Unfiled" }}</span>
            <span>{{ formatDate(result.note.updatedAt) }}</span>
          </span>
          <strong>{{ result.note.title || "Untitled note" }}</strong>
          <p>{{ result.snippet || result.note.content.replace(/[#*_>`\[\]]/g, " ").replace(/\s+/g, " ").trim().slice(0, 180) || "No content yet" }}</p>
          <span class="search-card-footer">
            <span v-if="result.note.tags.length" class="search-tags">
              <em v-for="tag in result.note.tags.slice(0, 2)" :key="tag">#{{ tag }}</em>
            </span>
            <span class="search-open-hint">Open <AppIcon name="arrow" :size="13" /></span>
          </span>
        </button>

        <div v-if="query && !results.length" class="search-zero-state">
          <div><AppIcon name="search" :size="26" /></div>
          <h2>Nothing matched “{{ query }}”</h2>
          <p>Try fewer words, a folder name, or one of your tags.</p>
        </div>
      </section>
    </div>
  </main>
</template>
