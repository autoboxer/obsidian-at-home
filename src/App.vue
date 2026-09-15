<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import ActivityRail from './components/ActivityRail.vue';
import AppIcon from './components/AppIcon.vue';
import CommandPalette from './components/CommandPalette.vue';
import EditorWorkspace from './components/EditorWorkspace.vue';
import ExplorerSidebar from './components/ExplorerSidebar.vue';
import LinkInspector from './components/LinkInspector.vue';
import RecentlyDeletedWorkspace from './components/RecentlyDeletedWorkspace.vue';
import SearchWorkspace from './components/SearchWorkspace.vue';
import SettingsView from './components/SettingsView.vue';
import SnippetStudio from './components/SnippetStudio.vue';
import TemplateGallery from './components/TemplateGallery.vue';
import VaultChooser from './components/VaultChooser.vue';
import { applyAppZoom } from './services/native';
import {
  activeNote,
  assetDeletionState,
  cancelVaultAssetDeletion,
  canEditVault,
  confirmVaultAssetDeletion,
  createNote,
  openQuickSearch,
  resetZoom,
  uiState,
  vaultSession,
  vaultState,
  zoomIn,
  zoomOut
} from './stores/vault';
import type { ToolView } from './types';

const editorWorkspace = ref<InstanceType<typeof EditorWorkspace>>();
const commandModalActive = ref( false );
let commandReturnFocus: HTMLElement | null = null;
let commandFocusNoteId: string | null = null;
const assetDeletionDialog = ref<HTMLElement>();
const assetDeletionCancel = ref<HTMLButtonElement>();
let assetDeletionReturnFocus: HTMLElement | null = null;
const assetDeletionName = computed( () => assetDeletionState.request?.relativePath.split( '/' ).at( -1 ) );
const assetDeletionReferenceSummary = computed( () => {
  const request = assetDeletionState.request;
  if ( !request ) {
    return '';
  }
  const recovery = request.recoveryReferenceCount
    ? `, including ${ request.recoveryReferenceCount } in Recently Deleted`
    : '';

  return `This file has ${ request.referenceCount } ${ request.referenceCount === 1 ? 'reference' : 'references' }${ recovery }.`;
});

watch(
  () => assetDeletionState.request,
  async ( request, previous ) => {
    if ( !request ) {
      return;
    }
    if ( !previous ) {
      const row = Array.from( document.querySelectorAll<HTMLElement>( '[data-vault-item-kind]' ) ).find(
        ( element ) => element.dataset.vaultItemKind === request.kind
          && element.dataset.vaultItemRelativePath === request.relativePath
      );
      assetDeletionReturnFocus = row?.querySelector<HTMLElement>( '[data-vault-item-primary]' ) ?? null;
    }
    await nextTick();
    assetDeletionCancel.value?.focus({ preventScroll: true });
  }
);

function restoreAssetDeletionFocus(): void {
  if ( assetDeletionState.request || vaultChooserVisible.value ) {
    return;
  }
  const target = assetDeletionReturnFocus?.isConnected
    ? assetDeletionReturnFocus
    : document.querySelector<HTMLElement>( '.vault-tree-root-main' );
  target?.focus({ preventScroll: true });
  assetDeletionReturnFocus = null;
}

function handleAssetDeletionKeydown( event: KeyboardEvent ): void {
  if ( event.key === 'Escape' ) {
    event.preventDefault();
    cancelVaultAssetDeletion();

    return;
  }
  const isTab = event.key === 'Tab' || event.key === 'Unidentified' && event.code === 'Tab';
  if ( !isTab ) {
    return;
  }
  const buttons = Array.from( assetDeletionDialog.value?.querySelectorAll<HTMLButtonElement>( 'button:not(:disabled)' ) ?? []);
  const first = buttons[ 0 ];
  const last = buttons.at( -1 );
  if ( !first || !last ) {
    event.preventDefault();
    assetDeletionDialog.value?.focus();
  } else if ( event.shiftKey && document.activeElement === first ) {
    event.preventDefault();
    last.focus();
  } else if ( !event.shiftKey && document.activeElement === last ) {
    event.preventDefault();
    first.focus();
  }
}

const requestedView = new URLSearchParams( window.location.search ).get( 'view' ) as ToolView | null;
if ( requestedView && [ 'notes', 'search', 'templates', 'snippets', 'settings' ].includes( requestedView ) ) {
  uiState.tool = requestedView;
}

const vaultChooserVisible = computed(
  () => vaultSession.phase !== 'loading'
    && ( vaultSession.phase !== 'ready' || uiState.vaultChooserOpen )
);
const readOnlyReason = computed( () => (
  vaultSession.phase === 'ready' && vaultSession.access.mode === 'read-only'
    ? vaultSession.access.reason
    : null
) );
const appInteractionBlocked = computed(
  () => vaultSession.phase !== 'ready'
    || uiState.vaultChooserOpen
    || vaultSession.busy
    || assetDeletionState.request !== null
);

watch( () => uiState.commandOpen, ( open ) => {
  if ( !open ) {
    // Opening and closing in one update may never mount a leaving transition.
    void nextTick( () => {
      if ( commandModalActive.value && !document.querySelector( '.command-backdrop' ) ) {
        void finishCommandClose();
      }
    });

    return;
  }
  // Capture the invoking control before rendering inert blurs the background.
  const focused = document.activeElement;
  if ( focused instanceof HTMLElement && focused !== document.body && !focused.closest( '.command-backdrop, [inert]' ) ) {
    commandReturnFocus = focused;
  } else if ( !commandModalActive.value ) {
    commandReturnFocus = null;
  }
  commandFocusNoteId = null;
  commandModalActive.value = true;
}, { flush: 'sync' });

watch( appInteractionBlocked, ( blocked ) => {
  if ( blocked ) {
    // Vault recovery and other blocking operations own focus from this point.
    uiState.commandOpen = false;
    commandReturnFocus = null;
    commandFocusNoteId = null;
  }
});

function commandNavigated( noteId: string ): void {
  commandFocusNoteId = noteId;
  commandReturnFocus = null;
}

function restoreCommandFocus(): void {
  if ( commandModalActive.value || uiState.commandOpen || appInteractionBlocked.value ) {
    return;
  }
  if ( document.querySelector( '[data-modal-scroll-active]' ) ) {
    commandReturnFocus = null;
    commandFocusNoteId = null;

    return;
  }
  if ( commandFocusNoteId && (
    uiState.tool !== 'notes'
    || uiState.notesView !== 'editor'
    || activeNote.value?.id !== commandFocusNoteId
  ) ) {
    commandFocusNoteId = null;

    return;
  }
  const previous = commandReturnFocus;
  if ( !previous && !commandFocusNoteId ) {
    return;
  }
  const target = [
    previous,
    document.querySelector<HTMLElement>( '.source-editor .cm-content' ),
    commandFocusNoteId ? null : document.querySelector<HTMLElement>( '.rail-nav .rail-button.active' )
  ].find( ( element ) => element?.isConnected
    && element.getClientRects().length
    // An outgoing sidebar/tool can still be connected during its transition.
    && !element.closest( '[inert], :disabled, [hidden], .panel-left-leave-active, .workspace-switch-leave-active' ) );
  target?.focus({ preventScroll: true });
  commandReturnFocus = null;
  // A tool transition can still be mounting the chosen note's editor.
  if ( target ) {
    commandFocusNoteId = null;
  }
}

async function finishCommandClose(): Promise<void> {
  if ( uiState.commandOpen ) {
    return;
  }
  commandModalActive.value = false;
  await nextTick();
  restoreCommandFocus();
}

function runToastAction(): void {
  const action = uiState.toast?.action;
  uiState.toast = null;
  action?.run();
}

watch(
  () => uiState.zoom,
  ( zoom ) => void applyAppZoom( zoom ),
  { immediate: true }
);

const titlebarContext = computed( () => {
  if ( uiState.tool === 'notes' ) {
    if ( uiState.notesView === 'recently-deleted' ) {
      return 'Recently Deleted';
    }

    return activeNote.value?.title || vaultState.name;
  }
  if ( uiState.tool === 'search' ) {
    return 'Search';
  }
  if ( uiState.tool === 'templates' ) {
    return 'Templates';
  }
  if ( uiState.tool === 'snippets' ) {
    return 'CSS snippets';
  }

  return 'Settings';
});

function handleKeyboard( event: KeyboardEvent ): void {
  const modifier = event.metaKey || event.ctrlKey;
  const key = event.key.toLocaleLowerCase();
  const target = event.target;
  const isEditing = target instanceof Element
    && Boolean( target.closest(
      "input, textarea, select, [contenteditable]:not([contenteditable='false'])"
    ) );
  const appShortcut = (
    modifier && [ 'n', 'o', '\\' ].includes( key )
    || modifier && event.shiftKey && key === 't'
    || !modifier && !isEditing && key === '/'
  );
  const zoomShortcut = modifier
    && !event.altKey
    && [ '+', '=', '-', '_', '0' ].includes( event.key );

  if ( document.documentElement.hasAttribute( 'data-modal-scroll-lock' ) ) {
    if ( appShortcut || zoomShortcut ) {
      event.preventDefault();
    }

    return;
  }

  if ( zoomShortcut ) {
    event.preventDefault();
    if ( event.key === '0' ) {
      resetZoom();
    } else if ( event.key === '-' || event.key === '_' ) {
      zoomOut();
    } else {
      zoomIn();
    }

    return;
  }

  if (
    vaultSession.phase !== 'ready'
    || uiState.vaultChooserOpen
    || vaultSession.busy
  ) {
    if ( appShortcut ) {
      event.preventDefault();
    }

    return;
  }

  if ( modifier && key === 'o' ) {
    event.preventDefault();
    if ( uiState.commandOpen ) {
      uiState.commandOpen = false;
    } else {
      openQuickSearch();
    }

    return;
  }
  if ( modifier && key === 'n' ) {
    event.preventDefault();
    createNote();

    return;
  }
  if ( modifier && event.shiftKey && key === 't' ) {
    event.preventDefault();
    uiState.tool = 'templates';

    return;
  }
  if ( modifier && key === '\\' ) {
    event.preventDefault();
    if ( uiState.tool !== 'notes' ) {
      uiState.tool = 'notes';
      uiState.notesView = 'editor';
      uiState.explorerOpen = true;
    } else {
      uiState.explorerOpen = !uiState.explorerOpen;
    }

    return;
  }
  if ( event.key === 'Escape' && uiState.commandOpen ) {
    event.preventDefault();
    event.stopPropagation();
    uiState.commandOpen = false;

    return;
  }

  if ( !isEditing && event.key === '/' && uiState.tool === 'notes' ) {
    event.preventDefault();
    document.querySelector<HTMLInputElement>( '.vault-tree-filter input' )?.focus();
  }
}

function preventExternalFileNavigation( event: DragEvent ): void {
  if ( Array.from( event.dataTransfer?.types ?? []).includes( 'Files' ) ) {
    event.preventDefault();
  }
}

onMounted( () => {
  window.addEventListener( 'keydown', handleKeyboard );
  window.addEventListener( 'dragover', preventExternalFileNavigation, true );
  window.addEventListener( 'drop', preventExternalFileNavigation, true );
});
onBeforeUnmount( () => {
  window.removeEventListener( 'keydown', handleKeyboard );
  window.removeEventListener( 'dragover', preventExternalFileNavigation, true );
  window.removeEventListener( 'drop', preventExternalFileNavigation, true );
});
</script>

<template>
  <div
    class="app-frame"
    :class="[ `tool-${uiState.tool}`, { 'is-read-only': readOnlyReason !== null }]"
    :data-app-view="uiState.tool"
    data-ui-region="app"
  >
    <header
      class="desktop-titlebar"
      data-ui-region="titlebar"
      data-tauri-drag-region
      :inert="appInteractionBlocked || commandModalActive"
    >
      <div class="traffic-light-space" data-tauri-drag-region />
      <div class="titlebar-title" data-tauri-drag-region>
        <span>Obsidian At Home</span>
        <i />
        <small>{{ titlebarContext }}</small>
      </div>
      <div data-tauri-drag-region />
    </header>

    <div
      v-if="readOnlyReason !== null"
      class="vault-access-banner"
      data-ui-region="vault-access"
      role="status"
    >
      <strong>Read only</strong>
      <span>{{ readOnlyReason }}</span>
    </div>

    <div class="app-content" :inert="appInteractionBlocked || commandModalActive">
      <ActivityRail />

      <Transition
        name="workspace-switch"
        mode="out-in"
        @after-enter="restoreCommandFocus"
      >
        <div
          v-if="uiState.tool === 'notes'"
          key="notes"
          class="notes-workspace"
          :data-note-view="uiState.notesView"
        >
          <Transition name="panel-left">
            <ExplorerSidebar v-if="uiState.explorerOpen" />
          </Transition>
          <RecentlyDeletedWorkspace v-if="uiState.notesView === 'recently-deleted'" />
          <template v-else>
            <EditorWorkspace ref="editorWorkspace" />
            <Transition name="panel-right">
              <LinkInspector
                v-if="uiState.contextOpen"
                @open-note-link="editorWorkspace?.openNoteLink( $event )"
              />
            </Transition>
          </template>
        </div>
        <SearchWorkspace v-else-if="uiState.tool === 'search'" key="search" />
        <TemplateGallery v-else-if="uiState.tool === 'templates'" key="templates" />
        <SnippetStudio v-else-if="uiState.tool === 'snippets'" key="snippets" />
        <SettingsView v-else-if="uiState.tool === 'settings'" key="settings" />
      </Transition>
    </div>

    <Transition name="overlay-fade" @after-leave="finishCommandClose">
      <CommandPalette
        v-if="uiState.commandOpen"
        :inert="appInteractionBlocked"
        @navigate="commandNavigated"
      />
    </Transition>

    <Transition name="overlay-fade">
      <VaultChooser v-if="vaultChooserVisible" />
    </Transition>

    <Transition
      name="overlay-fade"
      @after-enter="assetDeletionCancel?.focus( { preventScroll: true } )"
      @after-leave="restoreAssetDeletionFocus"
    >
      <div
        v-if="assetDeletionState.request"
        v-modal-scroll-lock
        class="modal-backdrop"
        data-ui-region="asset-deletion-dialog"
        @mousedown.self.prevent="cancelVaultAssetDeletion"
      >
        <section
          ref="assetDeletionDialog"
          class="editor-modal asset-deletion-dialog"
          data-modal-scroll-region
          role="dialog"
          aria-modal="true"
          aria-labelledby="asset-deletion-title"
          aria-describedby="asset-deletion-description"
          :aria-busy="vaultSession.busy"
          tabindex="-1"
          @keydown="handleAssetDeletionKeydown"
        >
          <header>
            <h2 id="asset-deletion-title">
              Delete {{ assetDeletionName }}?
            </h2>
          </header>
          <div id="asset-deletion-description" class="modal-fields">
            <p v-if="assetDeletionState.request.changed" role="status">
              The file or its references changed. Review the updated details before deleting.
            </p>
            <p>
              {{ assetDeletionReferenceSummary }}
              Deleting it permanently removes the file from your vault.
            </p>
            <p>
              Each reference will become “File Deleted”, including in notes restored from Recently Deleted.
            </p>
          </div>
          <footer>
            <button
              ref="assetDeletionCancel"
              type="button"
              class="secondary-button"
              :disabled="vaultSession.busy"
              @click="cancelVaultAssetDeletion"
            >
              Cancel
            </button>
            <button
              type="button"
              class="settings-button settings-button--danger"
              :disabled="vaultSession.busy || !canEditVault"
              @click="confirmVaultAssetDeletion"
            >
              {{ vaultSession.busy ? 'Deleting…' : 'Delete file and replace references' }}
            </button>
          </footer>
        </section>
      </div>
    </Transition>

    <Transition name="toast">
      <div
        v-if="uiState.toast"
        :key="uiState.toast.id"
        class="app-toast"
        :class="`tone-${uiState.toast.tone}`"
        data-ui-region="notification"
        :inert="appInteractionBlocked || commandModalActive"
        role="status"
      >
        <span class="toast-icon">
          <AppIcon :name="uiState.toast.tone === 'success' ? 'check' : uiState.toast.tone === 'warning' ? 'info' : 'sparkles'" :size="15" />
        </span>
        <span>{{ uiState.toast.message }}</span>
        <button
          v-if="uiState.toast.action"
          type="button"
          class="app-toast-action"
          @click="runToastAction"
        >
          {{ uiState.toast.action.label }}
        </button>
      </div>
    </Transition>
  </div>
</template>
