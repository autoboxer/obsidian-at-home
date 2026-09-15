<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { createSearchSnippet, searchNotes } from '../lib';
import {
  canEditVault,
  createNote,
  folderNameMap,
  searchState,
  selectNote,
  uiState,
  vaultState
} from '../stores/vault';
import AppIcon from './AppIcon.vue';

const emit = defineEmits<{ navigate: [ noteId: string ] }>();
const query = computed({
  get: () => searchState.quickQuery,
  set: ( value: string ) => {
    searchState.quickQuery = value;
  }
});
const selectedIndex = ref( 0 );
const input = ref<HTMLInputElement>();
const dialog = ref<HTMLElement>();
let focusFrame: number | undefined;

const results = computed( () => query.value.trim()
  ? searchNotes( vaultState.notes, query.value, { folderNames: folderNameMap.value, limit: 9 })
  : [ ...vaultState.notes ]
    .sort( ( a, b ) => Number( b.pinned ) - Number( a.pinned ) || b.updatedAt - a.updatedAt )
    .slice( 0, 7 )
    .map( ( note ) => ({ note, score: 0, reason: 'title' as const, snippet: createSearchSnippet( note.content, [], 110 ) }) )
);

watch( () => uiState.commandOpen, ( open ) => {
  if ( open ) {
    selectedIndex.value = 0;
    void focusInput();
  }
});

watch( query, () => {
  selectedIndex.value = 0;
});

watch([ results, canEditVault ], async ([ matches ]) => {
  selectedIndex.value = Math.max( 0, Math.min( selectedIndex.value, matches.length - 1 ) );
  await nextTick();
  const focused = document.activeElement;
  if ( !dialog.value?.contains( focused ) || focused?.matches( ':disabled' ) ) {
    void focusInput();
  }
});

onMounted( () => void focusInput() );
onBeforeUnmount( () => {
  if ( focusFrame !== undefined ) {
    window.cancelAnimationFrame( focusFrame );
  }
});

async function focusInput(): Promise<void> {
  await nextTick();
  if ( focusFrame !== undefined ) {
    window.cancelAnimationFrame( focusFrame );
  }
  focusFrame = window.requestAnimationFrame( () => {
    focusFrame = undefined;
    if ( !uiState.commandOpen || !dialog.value?.closest( '[data-modal-scroll-active]' ) ) {
      return;
    }
    input.value?.focus({ preventScroll: true });
    input.value?.select();
  });
}

function close(): void {
  uiState.commandOpen = false;
}

function chooseNote( id: string ): void {
  selectNote( id );
  uiState.tool = 'notes';
  emit( 'navigate', id );
  close();
}

function create(): void {
  if ( !canEditVault.value ) {
    return;
  }
  const note = createNote();
  if ( note ) {
    emit( 'navigate', note.id );
    close();
  }
}

function onKeydown( event: KeyboardEvent ): void {
  if ( event.isComposing ) {
    return;
  }
  if ( event.key === 'ArrowDown' ) {
    event.preventDefault();
    selectedIndex.value = Math.max( 0, Math.min( results.value.length - 1, selectedIndex.value + 1 ) );
  } else if ( event.key === 'ArrowUp' ) {
    event.preventDefault();
    selectedIndex.value = Math.max( 0, selectedIndex.value - 1 );
  } else if ( event.key === 'Enter' ) {
    event.preventDefault();
    const result = results.value[ selectedIndex.value ];
    if ( result ) {
      chooseNote( result.note.id );
    } else if ( !query.value ) {
      create();
    }
  }
}

function handleDialogKeydown( event: KeyboardEvent ): void {
  if ( event.isComposing ) {
    return;
  }
  if ( event.key === 'Escape' ) {
    event.preventDefault();
    event.stopPropagation();
    close();

    return;
  }
  // WebKitGTK can report Shift+Tab as Unidentified while retaining its code.
  const isTab = event.key === 'Tab' || event.key === 'Unidentified' && event.code === 'Tab';
  if ( !isTab ) {
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  const controls = Array.from( dialog.value?.querySelectorAll<HTMLElement>(
    'input:not(:disabled), button:not(:disabled)'
  ) ?? []);
  const index = controls.indexOf( document.activeElement as HTMLElement );
  const next = event.shiftKey
    ? ( index <= 0 ? controls.length - 1 : index - 1 )
    : ( index + 1 ) % controls.length;
  ( controls[ next ] ?? dialog.value )?.focus();
}

function highlightTitle( title: string ): string {
  const needle = query.value.trim();
  if ( !needle ) {
    return escapeHtml( title );
  }
  const index = title.toLocaleLowerCase().indexOf( needle.toLocaleLowerCase() );
  if ( index < 0 ) {
    return escapeHtml( title );
  }

  return `${ escapeHtml( title.slice( 0, index ) ) }<mark>${ escapeHtml( title.slice( index, index + needle.length ) ) }</mark>${ escapeHtml( title.slice( index + needle.length ) ) }`;
}

function escapeHtml( value: string ): string {
  return value.replace( /&/g, '&amp;' ).replace( /</g, '&lt;' ).replace( />/g, '&gt;' ).replace( /"/g, '&quot;' );
}
</script>

<template>
  <div
    v-modal-scroll-lock
    class="modal-backdrop command-backdrop"
    data-ui-region="quick-switcher"
    @mousedown.self.prevent="close"
  >
    <section
      ref="dialog"
      class="command-palette"
      role="dialog"
      aria-modal="true"
      aria-label="Quick search"
      tabindex="-1"
      @keydown="handleDialogKeydown"
    >
      <div class="command-input-wrap">
        <AppIcon name="search" :size="20" />
        <input
          ref="input"
          v-model="query"
          placeholder="Search notes, content, and tags…"
          aria-label="Search all notes"
          autocomplete="off"
          autocapitalize="none"
          autocorrect="off"
          spellcheck="false"
          @keydown="onKeydown"
        >
        <kbd>esc</kbd>
      </div>

      <div class="command-body">
        <div class="command-section-label">
          <span>{{ query ? `${results.length} best matches` : "Recent & favorites" }}</span>
          <span v-if="query">Searching titles, content, folders, and tags</span>
        </div>
        <div class="command-results" data-modal-scroll-region>
          <button
            v-for="( result, index ) in results"
            :key="result.note.id"
            type="button"
            class="command-result"
            :class="{ active: index === selectedIndex }"
            @mouseenter="selectedIndex = index"
            @focus="selectedIndex = index"
            @click="chooseNote( result.note.id )"
          >
            <span class="command-result-icon"><AppIcon name="file-text" :size="17" /></span>
            <span class="command-result-copy">
              <!-- eslint-disable-next-line vue/no-v-html -- highlightTitle escapes note text before adding mark tags. -->
              <span class="command-result-title" v-html="highlightTitle( result.note.title || 'Untitled note' )" />
              <span>{{ result.snippet || "No content yet" }}</span>
            </span>
            <span class="command-result-meta">
              <span v-if="result.note.tags[0]">#{{ result.note.tags[0] }}</span>
              <AppIcon
                v-if="index === selectedIndex"
                name="enter"
                :size="14"
              />
            </span>
          </button>
          <div v-if="query && !results.length" class="command-empty">
            <div><AppIcon name="search" :size="22" /></div>
            <strong>No notes found</strong>
            <span>Try a title, phrase, folder, or tag.</span>
          </div>
        </div>
      </div>

      <footer class="command-footer">
        <button
          :disabled="!canEditVault"
          type="button"
          @click="create"
        >
          <span><AppIcon name="plus" :size="13" /></span> New note
        </button>
        <div><span><kbd>↑</kbd><kbd>↓</kbd> move</span><span><kbd>↵</kbd> open</span></div>
      </footer>
    </section>
  </div>
</template>
