<script setup lang="ts">
import { computed, nextTick, ref } from 'vue';
import {
  emptyRecentlyDeletedNotes,
  permanentlyDeleteRecentlyDeletedNote,
  recentlyDeletedNotes,
  recentlyDeletedState,
  restoreRecentlyDeletedNote
} from '../stores/vault';
import type { RecentlyDeletedNote } from '../types';
import AppIcon from './AppIcon.vue';

const PAGE_SIZE = 50;

const dateTimeFormatter = new Intl.DateTimeFormat( undefined, {
  dateStyle: 'medium',
  timeStyle: 'short'
});
const visibleCount = ref( PAGE_SIZE );
const workspaceRoot = ref<HTMLElement>();
const pendingAction = ref<{
  id: string | null;
  type: 'restore' | 'delete' | 'empty';
} | null>( null );

const visibleEntries = computed( () =>
  recentlyDeletedNotes.value.slice( 0, visibleCount.value )
);

async function restore( entry: RecentlyDeletedNote ): Promise<void> {
  pendingAction.value = { id: entry.id, type: 'restore' };
  try {
    await restoreRecentlyDeletedNote( entry.id );
  } finally {
    pendingAction.value = null;
  }
}

async function permanentlyDelete( entry: RecentlyDeletedNote ): Promise<void> {
  const title = displayTitle( entry );
  const confirmed = window.confirm(
    `Permanently delete “${ title }”? This cannot be undone.`
  );
  if ( !confirmed ) {
    return;
  }

  const deletedIndex = visibleEntries.value.findIndex( ( candidate ) => candidate.id === entry.id );
  pendingAction.value = { id: entry.id, type: 'delete' };
  try {
    const deleted = await permanentlyDeleteRecentlyDeletedNote( entry.id );
    if ( !deleted ) {
      return;
    }

    await nextTick();
    const nextEntry = visibleEntries.value[ Math.min( deletedIndex, visibleEntries.value.length - 1 ) ];
    if ( nextEntry ) {
      workspaceRoot.value
        ?.querySelector<HTMLButtonElement>( `[data-recovery-id="${ nextEntry.id }"] [data-recovery-action="restore"]` )
        ?.focus();
    }
  } finally {
    pendingAction.value = null;
  }
}

async function emptyRecentlyDeleted(): Promise<void> {
  const count = recentlyDeletedNotes.value.length;
  const noun = count === 1 ? 'note' : 'notes';
  const confirmed = window.confirm(
    `Permanently delete ${ count } ${ noun } from Recently Deleted? This cannot be undone.`
  );
  if ( !confirmed ) {
    return;
  }

  pendingAction.value = { id: null, type: 'empty' };
  try {
    await emptyRecentlyDeletedNotes();
  } finally {
    pendingAction.value = null;
  }
}

function displayTitle( entry: RecentlyDeletedNote ): string {
  return entry.note.title.trim() || 'Untitled note';
}

function contentPreview( content: string ): string {
  return content
    .slice( 0, 2_000 )
    .replace( /[#*_>`[\]]/g, ' ' )
    .replace( /\s+/g, ' ' )
    .trim()
    .slice( 0, 220 ) || 'No content';
}

function originalLocation( entry: RecentlyDeletedNote ): string {
  const fileName = entry.note.relativePath.split( '/' ).pop() || `${ displayTitle( entry ) }.md`;
  const folder = entry.originalFolderPath || 'Vault root';

  return `${ folder }/${ fileName }`;
}

function formatDateTime( timestamp: number ): string {
  const date = new Date( timestamp );

  return Number.isNaN( date.getTime() ) ? 'Unknown date' : dateTimeFormatter.format( date );
}

function dateTimeValue( timestamp: number ): string | undefined {
  const date = new Date( timestamp );

  return Number.isNaN( date.getTime() ) ? undefined : date.toISOString();
}

function isPending( entry: RecentlyDeletedNote, type: 'restore' | 'delete' ): boolean {
  return pendingAction.value?.id === entry.id && pendingAction.value.type === type;
}

function loadMore(): void {
  visibleCount.value += PAGE_SIZE;
}
</script>

<template>
  <main
    ref="workspaceRoot"
    class="recently-deleted-workspace utility-workspace"
    data-ui-region="recently-deleted"
    aria-labelledby="recently-deleted-title"
    :aria-busy="recentlyDeletedState.busy"
  >
    <div class="utility-page recently-deleted-page">
      <header class="utility-header-row recently-deleted-header" data-ui-region="recently-deleted-header">
        <div class="utility-hero compact">
          <span class="utility-eyebrow">Recovery</span>
          <h1 id="recently-deleted-title">
            Recently Deleted
          </h1>
          <p>Restore deleted notes or remove them permanently. Notes expire automatically after seven days.</p>
        </div>

        <button
          v-if="recentlyDeletedNotes.length"
          type="button"
          class="secondary-button recently-deleted-empty-button"
          data-recovery-action="empty"
          :disabled="recentlyDeletedState.busy"
          @click="emptyRecentlyDeleted"
        >
          <AppIcon name="trash" :size="15" />
          {{ pendingAction?.type === "empty" ? "Emptying…" : "Empty Recently Deleted" }}
        </button>
      </header>

      <div class="recently-deleted-summary" aria-live="polite">
        <span v-if="visibleEntries.length < recentlyDeletedNotes.length">
          Showing {{ visibleEntries.length }} of {{ recentlyDeletedNotes.length }} notes
        </span>
        <span v-else>{{ recentlyDeletedNotes.length }} {{ recentlyDeletedNotes.length === 1 ? "note" : "notes" }}</span>
        <span v-if="recentlyDeletedState.busy" class="recently-deleted-progress">
          <AppIcon name="refresh" :size="13" />
          Updating…
        </span>
      </div>

      <p
        v-if="recentlyDeletedState.error"
        class="recently-deleted-error"
        role="alert"
      >
        <AppIcon name="info" :size="16" />
        {{ recentlyDeletedState.error }}
      </p>

      <ol
        v-if="recentlyDeletedNotes.length"
        class="recently-deleted-list"
        data-ui-region="recently-deleted-list"
      >
        <li v-for="entry in visibleEntries" :key="entry.id">
          <article
            class="recently-deleted-card"
            :class="{ 'is-favorite': entry.note.pinned }"
            :data-recovery-id="entry.id"
            data-ui-region="recently-deleted-note"
          >
            <header class="recently-deleted-card-header">
              <div class="recently-deleted-card-title">
                <span
                  v-if="entry.note.pinned"
                  class="recently-deleted-favorite"
                  title="Favorite note"
                >
                  <AppIcon name="star" :size="14" />
                  <span>Favorite</span>
                </span>
                <h2>{{ displayTitle( entry ) }}</h2>
              </div>

              <time :datetime="dateTimeValue( entry.deletedAt )">
                Deleted {{ formatDateTime( entry.deletedAt ) }}
              </time>
            </header>

            <p class="recently-deleted-preview">
              {{ contentPreview( entry.note.content ) }}
            </p>

            <dl class="recently-deleted-details">
              <div>
                <dt>Original location</dt>
                <dd :title="originalLocation( entry )">
                  {{ originalLocation( entry ) }}
                </dd>
              </div>
              <div>
                <dt>Expires</dt>
                <dd>
                  <time :datetime="dateTimeValue( entry.expiresAt )">
                    {{ formatDateTime( entry.expiresAt ) }}
                  </time>
                </dd>
              </div>
            </dl>

            <footer class="recently-deleted-actions" data-ui-region="recently-deleted-actions">
              <button
                type="button"
                class="primary-action-button small"
                :disabled="recentlyDeletedState.busy"
                :aria-label="`Restore ${displayTitle( entry )}`"
                data-recovery-action="restore"
                @click="restore( entry )"
              >
                <AppIcon name="refresh" :size="14" />
                {{ isPending( entry, "restore" ) ? "Restoring…" : "Restore" }}
              </button>
              <button
                type="button"
                class="secondary-button recently-deleted-delete-button"
                :disabled="recentlyDeletedState.busy"
                :aria-label="`Permanently delete ${displayTitle( entry )}`"
                data-recovery-action="delete"
                @click="permanentlyDelete( entry )"
              >
                <AppIcon name="trash" :size="14" />
                {{ isPending( entry, "delete" ) ? "Deleting…" : "Delete Permanently" }}
              </button>
            </footer>
          </article>
        </li>
      </ol>

      <button
        v-if="visibleEntries.length < recentlyDeletedNotes.length"
        type="button"
        class="secondary-button recently-deleted-load-more"
        data-recovery-action="load-more"
        :disabled="recentlyDeletedState.busy"
        @click="loadMore"
      >
        Load more
      </button>
    </div>
  </main>
</template>
