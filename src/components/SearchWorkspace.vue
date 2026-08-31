<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { searchNotes, shortcutCommandKey } from '../lib';
import {
  folderNameMap,
  folderPath,
  searchState,
  selectNote,
  uiState,
  vaultState
} from '../stores/vault';
import type { SearchScope } from '../types';
import AppIcon from './AppIcon.vue';

const input = ref<HTMLInputElement>();
const commandKey = shortcutCommandKey();
const searchScopes: Array<{ id: SearchScope; label: string }> = [
  { id: 'all', label: 'Everywhere' },
  { id: 'titles', label: 'Titles' },
  { id: 'content', label: 'Content' },
  { id: 'tags', label: 'Tags' }
];
const query = computed({
  get: () => searchState.query,
  set: ( value: string ) => {
    searchState.query = value;
    searchState.exactTag = null;
    if ( !value.trim() ) {
      searchState.scope = 'all';
    }
  }
});
const scope = computed( () => searchState.scope );

const results = computed( () => {
  if ( !query.value.trim() ) {
    return [ ...vaultState.notes ]
      .sort( ( a, b ) => b.updatedAt - a.updatedAt )
      .map( ( note ) => ({ note, score: 0, snippet: '', reason: 'title' as const }) );
  }

  return searchNotes( vaultState.notes, query.value, {
    folderNames: folderNameMap.value,
    limit: 100,
    scope: scope.value,
    exactTag: searchState.exactTag ?? undefined
  });
});

const emptyHint = computed( () => {
  if ( searchState.exactTag ) {
    return 'No notes use this tag yet.';
  }
  if ( scope.value === 'tags' ) {
    return 'Try another tag or fewer characters.';
  }
  if ( scope.value === 'titles' ) {
    return 'Try fewer words or search everywhere.';
  }
  if ( scope.value === 'content' ) {
    return 'Try a shorter phrase or search everywhere.';
  }

  return 'Try fewer words, a folder name, or one of your tags.';
});

onMounted( () => void focusInput() );

watch(
  () => uiState.commandOpen,
  ( commandOpen ) => {
    if ( !commandOpen ) {
      void focusInput();
    }
  }
);

watch(
  () => searchState.focusRequest,
  () => void focusInput()
);

async function focusInput(): Promise<void> {
  await nextTick();
  window.requestAnimationFrame( () => {
    if ( !uiState.commandOpen ) {
      input.value?.focus({ preventScroll: true });
    }
  });
}

function setScope( nextScope: SearchScope ): void {
  if ( nextScope === searchState.scope ) {
    void focusInput();

    return;
  }
  searchState.scope = nextScope;
  searchState.exactTag = null;
  void focusInput();
}

function clearSearch(): void {
  searchState.query = '';
  searchState.scope = 'all';
  searchState.exactTag = null;
  void focusInput();
}

function openNote( id: string ): void {
  selectNote( id );
  uiState.tool = 'notes';
}

function visibleResultTags( tags: string[]): string[] {
  if ( scope.value !== 'tags' || !query.value.trim() ) {
    return tags.slice( 0, 2 );
  }

  return [ ...tags ]
    .sort( ( a, b ) => Number( tagMatches( b ) ) - Number( tagMatches( a ) ) )
    .slice( 0, 3 );
}

function tagMatches( tag: string ): boolean {
  if ( scope.value !== 'tags' || !query.value.trim() ) {
    return false;
  }

  const tagValue = normalizeTag( tag );
  const exactTag = searchState.exactTag ? normalizeTag( searchState.exactTag ) : '';

  return exactTag
    ? tagValue === exactTag
    : tagValue.includes( normalizeTag( query.value ) );
}

function normalizeTag( tag: string ): string {
  return tag
    .normalize( 'NFKD' )
    .replace( /[\u0300-\u036f]/g, '' )
    .toLocaleLowerCase()
    .trim();
}

function formatDate( timestamp: number ): string {
  return new Intl.DateTimeFormat( 'en', { month: 'short', day: 'numeric', year: 'numeric' }).format( timestamp );
}
</script>

<template>
  <main class="search-workspace utility-workspace" data-ui-region="search">
    <div class="utility-page search-page">
      <header class="utility-hero search-hero">
        <span class="utility-eyebrow">Search</span>
        <h1>Search all notes</h1>
        <p>Search titles, content, folders, and tags.</p>
      </header>

      <div class="search-box-large">
        <AppIcon name="search" :size="23" />
        <input
          ref="input"
          v-model="query"
          placeholder="Search titles, content, folders, or tags…"
          aria-label="Search notes"
          autocomplete="off"
          autocapitalize="none"
          autocorrect="off"
          spellcheck="false"
        >
        <button
          v-if="query"
          type="button"
          aria-label="Clear search"
          @click="clearSearch"
        >
          <AppIcon name="x" :size="15" />
        </button>
        <span v-else class="search-quick-hint">Quick search <kbd>{{ commandKey }} O</kbd></span>
      </div>

      <div class="search-controls">
        <div class="search-scopes">
          <button
            v-for="item in searchScopes"
            :key="item.id"
            type="button"
            :class="{ active: scope === item.id }"
            :aria-pressed="scope === item.id"
            @click="setScope( item.id )"
          >
            {{ item.label }}
          </button>
        </div>
        <span>{{ results.length }} {{ results.length === 1 ? "result" : "results" }}</span>
      </div>

      <section class="search-results-grid">
        <button
          v-for="result in results"
          :key="result.note.id"
          type="button"
          class="search-result-card"
          @click="openNote( result.note.id )"
        >
          <span class="search-result-topline">
            <span>{{ result.note.folderId ? folderPath( result.note.folderId ) : "Vault root" }}</span>
            <span>{{ formatDate( result.note.updatedAt ) }}</span>
          </span>
          <strong>{{ result.note.title || "Untitled note" }}</strong>
          <p>{{ result.snippet || result.note.content.replace( /[#*_>`\[\]]/g, " " ).replace( /\s+/g, " " ).trim().slice( 0, 180 ) || "No content yet" }}</p>
          <span class="search-card-footer">
            <span v-if="result.note.tags.length" class="search-tags">
              <em
                v-for="tag in visibleResultTags( result.note.tags )"
                :key="tag"
                :class="{ matched: tagMatches( tag ) }"
              >#{{ tag }}</em>
            </span>
            <span class="search-open-hint">Open <AppIcon name="arrow" :size="13" /></span>
          </span>
        </button>

        <div v-if="query.trim() && !results.length" class="search-zero-state">
          <div><AppIcon name="search" :size="26" /></div>
          <h2>Nothing matched “{{ query }}”</h2>
          <p>{{ emptyHint }}</p>
        </div>
      </section>
    </div>
  </main>
</template>
