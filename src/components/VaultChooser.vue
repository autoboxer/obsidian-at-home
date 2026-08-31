<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import {
  createFilesystemVault,
  openFilesystemVault,
  overwriteFilesystemVault,
  reloadFilesystemVault,
  switchFilesystemVault,
  uiState,
  vaultSession
} from '../stores/vault';
import AppIcon from './AppIcon.vue';

const vaultName = ref( 'Home Vault' );
const nameField = ref<HTMLInputElement>();
const dialog = ref<HTMLElement>();

const firstRun = computed( () => vaultSession.phase !== 'ready' );
const canClose = computed(
  () => vaultSession.phase === 'ready' && !vaultSession.busy && !vaultSession.conflict
);
const nativeAvailable = computed( () => vaultSession.backend === 'native' );
const nameReady = computed( () => vaultName.value.trim().length > 0 );
const errorTitle = computed( () => {
  if ( vaultSession.conflict ) {
    return 'This vault changed on disk';
  }
  if ( vaultSession.phase === 'error' ) {
    return 'Couldn’t open your vaults';
  }

  return 'Couldn’t save or open this vault';
});

function closeChooser(): void {
  if ( canClose.value ) {
    uiState.vaultChooserOpen = false;
  }
}

function handleWindowKeydown( event: KeyboardEvent ): void {
  if ( event.key === 'Escape' ) {
    closeChooser();
  }
}

function handleDialogKeydown( event: KeyboardEvent ): void {
  if ( event.key !== 'Tab' || !dialog.value ) {
    return;
  }
  const focusable = Array.from( dialog.value.querySelectorAll<HTMLElement>(
    "button:not(:disabled), input:not(:disabled), [href], [tabindex]:not([tabindex='-1'])"
  ) ).filter( ( element ) => !element.hasAttribute( 'hidden' ) );
  if ( !focusable.length ) {
    event.preventDefault();
    dialog.value.focus();

    return;
  }
  const first = focusable[ 0 ];
  const last = focusable[ focusable.length - 1 ];
  if ( event.shiftKey && document.activeElement === first ) {
    event.preventDefault();
    last.focus();
  } else if ( !event.shiftKey && document.activeElement === last ) {
    event.preventDefault();
    first.focus();
  }
}

function focusInitialControl(): void {
  nextTick( () => {
    const recoveryAction = dialog.value?.querySelector<HTMLButtonElement>(
      '.vault-chooser-conflict button:not(:disabled)'
    );
    if ( recoveryAction ) {
      recoveryAction.focus();
    } else if ( nameField.value && !nameField.value.disabled ) {
      nameField.value.select();
    } else {
      dialog.value?.focus();
    }
  });
}

watch(
  () => vaultSession.phase,
  focusInitialControl,
  { immediate: true }
);

onMounted( () => {
  window.addEventListener( 'keydown', handleWindowKeydown );
  focusInitialControl();
});
onBeforeUnmount( () => window.removeEventListener( 'keydown', handleWindowKeydown ) );

async function createVault( useLegacy: boolean ): Promise<void> {
  if ( !nativeAvailable.value || vaultSession.busy || !nameReady.value ) {
    return;
  }
  const succeeded = await createFilesystemVault( vaultName.value.trim(), useLegacy );
  if ( succeeded ) {
    closeChooser();
  }
}

async function openVault(): Promise<void> {
  if ( !nativeAvailable.value || vaultSession.busy ) {
    return;
  }
  const succeeded = await openFilesystemVault();
  if ( succeeded ) {
    closeChooser();
  }
}

async function switchVault( path: string ): Promise<void> {
  if ( !nativeAvailable.value || vaultSession.busy || path === vaultSession.path ) {
    return;
  }
  const succeeded = await switchFilesystemVault( path );
  if ( succeeded ) {
    closeChooser();
  }
}

async function resolveConflict( reloadFromDisk: boolean ): Promise<void> {
  if ( vaultSession.busy ) {
    return;
  }
  const succeeded = reloadFromDisk
    ? await reloadFilesystemVault()
    : await overwriteFilesystemVault();
  if ( succeeded ) {
    closeChooser();
  }
}
</script>

<template>
  <div
    v-modal-scroll-lock
    class="modal-backdrop vault-chooser-backdrop"
    data-ui-region="vault-chooser"
    @keydown.esc="closeChooser"
    @mousedown.self="closeChooser"
  >
    <section
      ref="dialog"
      class="vault-chooser-dialog"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      aria-labelledby="vault-chooser-title"
      aria-describedby="vault-chooser-description"
      @keydown="handleDialogKeydown"
    >
      <header class="vault-chooser-header">
        <span class="vault-chooser-mark" aria-hidden="true">
          <AppIcon name="folder-open" :size="22" />
        </span>
        <div class="vault-chooser-heading">
          <span class="utility-eyebrow">Markdown vaults</span>
          <h2 id="vault-chooser-title">
            {{ firstRun ? "Choose where your notes live" : "Manage your vaults" }}
          </h2>
          <p id="vault-chooser-description">
            Each vault is a folder of Markdown files that you can open with other tools.
          </p>
        </div>
        <button
          v-if="vaultSession.phase === 'ready'"
          type="button"
          class="icon-button vault-chooser-close"
          aria-label="Close vault chooser"
          :disabled="!canClose"
          @click="closeChooser"
        >
          <AppIcon name="x" :size="17" />
        </button>
      </header>

      <div
        v-if="vaultSession.phase === 'loading'"
        class="vault-chooser-loading"
        role="status"
      >
        <AppIcon
          class="vault-chooser-spinner"
          name="refresh"
          :size="22"
        />
        <strong>Opening your vault…</strong>
        <span>Reading Markdown files and rebuilding the note index.</span>
      </div>

      <div
        v-else
        class="vault-chooser-body"
        data-modal-scroll-region
      >
        <div
          v-if="vaultSession.error"
          class="vault-chooser-alert vault-chooser-alert--error"
          role="alert"
        >
          <AppIcon name="info" :size="18" />
          <div>
            <strong>{{ errorTitle }}</strong>
            <p>{{ vaultSession.error }}</p>
          </div>
        </div>

        <article
          v-if="vaultSession.conflict"
          class="vault-chooser-conflict"
          role="alert"
        >
          <div>
            <strong>Choose which version to keep</strong>
            <p>
              Reload uses the Markdown files currently on disk and discards unsaved edits in the app.
              Keep app version writes the open notes over changed managed files.
            </p>
          </div>
          <div class="vault-chooser-conflict-actions">
            <button
              type="button"
              class="secondary-button"
              :disabled="vaultSession.busy"
              @click="resolveConflict( true )"
            >
              Reload from disk
            </button>
            <button
              type="button"
              class="settings-button settings-button--danger-ghost"
              :disabled="vaultSession.busy"
              @click="resolveConflict( false )"
            >
              Keep app version
            </button>
          </div>
        </article>

        <article
          v-else-if="vaultSession.error && vaultSession.path && uiState.saveStatus === 'error'"
          class="vault-chooser-conflict"
          role="alert"
        >
          <div>
            <strong>Unsaved app edits are still open</strong>
            <p>
              Fix the problem and edit again to retry, or reload the Markdown files from disk and
              discard the unsaved app edits.
            </p>
          </div>
          <div class="vault-chooser-conflict-actions">
            <button
              type="button"
              class="primary-action-button"
              :disabled="vaultSession.busy"
              @click="closeChooser"
            >
              Return to editor
            </button>
            <button
              type="button"
              class="secondary-button"
              :disabled="vaultSession.busy"
              @click="resolveConflict( true )"
            >
              Discard and reload
            </button>
          </div>
        </article>

        <details v-if="vaultSession.warnings.length" class="vault-chooser-warnings">
          <summary>
            {{ vaultSession.warnings.length }}
            {{ vaultSession.warnings.length === 1 ? "file warning" : "file warnings" }}
          </summary>
          <ul>
            <li v-for="( warning, index ) in vaultSession.warnings" :key="`${index}-${warning}`">
              {{ warning }}
            </li>
          </ul>
        </details>

        <div
          v-if="!nativeAvailable"
          class="vault-chooser-alert vault-chooser-alert--info"
          role="note"
        >
          <AppIcon name="info" :size="18" />
          <div>
            <strong>Folder access requires the desktop app</strong>
            <p>The browser preview cannot create or open filesystem vaults.</p>
          </div>
        </div>

        <div v-if="vaultSession.path" class="vault-chooser-current">
          <span>Current vault</span>
          <code :title="vaultSession.path">{{ vaultSession.path }}</code>
        </div>

        <label class="vault-chooser-name-field">
          <span>Vault name</span>
          <input
            ref="nameField"
            v-model="vaultName"
            type="text"
            maxlength="96"
            autocomplete="off"
            :disabled="vaultSession.busy"
            placeholder="Home Vault"
            @keydown.enter.prevent="createVault( false )"
          >
          <small>Used when creating a folder or saving your existing notes.</small>
        </label>

        <article v-if="vaultSession.legacyAvailable" class="vault-chooser-migration">
          <span class="vault-chooser-card-icon vault-chooser-card-icon--migration">
            <AppIcon name="export" :size="21" />
          </span>
          <div>
            <strong>Keep your existing notes</strong>
            <p>Write the notes from the previous app storage into a folder as Markdown files.</p>
          </div>
          <button
            type="button"
            class="primary-action-button vault-chooser-action"
            :disabled="vaultSession.busy || !nativeAvailable || !nameReady"
            @click="createVault( true )"
          >
            <AppIcon name="folder-open" :size="16" />
            Save existing notes to a folder
          </button>
        </article>

        <div class="vault-chooser-action-grid">
          <article class="vault-chooser-card">
            <span class="vault-chooser-card-icon">
              <AppIcon name="plus" :size="21" />
            </span>
            <div>
              <strong>Create a vault</strong>
              <p>Choose a parent folder and create a new Markdown vault inside it.</p>
            </div>
            <button
              type="button"
              class="primary-action-button vault-chooser-action"
              :disabled="vaultSession.busy || !nativeAvailable || !nameReady"
              @click="createVault( false )"
            >
              <AppIcon name="plus" :size="16" />
              Create vault
            </button>
          </article>

          <article class="vault-chooser-card vault-chooser-card--open">
            <span class="vault-chooser-card-icon vault-chooser-card-icon--open">
              <AppIcon name="folder-open" :size="21" />
            </span>
            <div>
              <strong>Open a folder as a vault</strong>
              <p>Use an existing folder and save changes directly to its Markdown files.</p>
            </div>
            <button
              type="button"
              class="secondary-button vault-chooser-action"
              :disabled="vaultSession.busy || !nativeAvailable"
              @click="openVault"
            >
              <AppIcon name="folder-open" :size="16" />
              Open folder
            </button>
          </article>
        </div>

        <section
          v-if="vaultSession.recentVaults.length"
          class="vault-chooser-recents"
          aria-labelledby="recent-vaults-title"
        >
          <div class="vault-chooser-section-heading">
            <span id="recent-vaults-title">Recent vaults</span>
            <small>{{ vaultSession.recentVaults.length }}</small>
          </div>
          <div class="vault-chooser-recent-list">
            <button
              v-for="vault in vaultSession.recentVaults"
              :key="vault.path"
              type="button"
              class="vault-chooser-recent"
              :class="{ active: vault.path === vaultSession.path }"
              :disabled="vaultSession.busy || !nativeAvailable || vault.path === vaultSession.path"
              @click="switchVault( vault.path )"
            >
              <span class="vault-chooser-recent-icon">
                <AppIcon name="folder" :size="16" />
              </span>
              <span class="vault-chooser-recent-copy">
                <strong>{{ vault.name }}</strong>
                <code :title="vault.path">{{ vault.path }}</code>
              </span>
              <span v-if="vault.path === vaultSession.path" class="vault-chooser-current-badge">
                <AppIcon name="check" :size="13" />
                Open
              </span>
              <AppIcon
                v-else
                name="arrow"
                :size="15"
              />
            </button>
          </div>
        </section>
      </div>

      <footer class="vault-chooser-footer">
        <span
          v-if="vaultSession.busy"
          class="vault-chooser-busy"
          role="status"
        >
          <AppIcon
            class="vault-chooser-spinner"
            name="refresh"
            :size="14"
          />
          Working with your vault…
        </span>
        <span v-else>Obsidian At Home does not upload or sync your vault.</span>
        <button
          v-if="vaultSession.phase === 'ready'"
          type="button"
          class="secondary-button vault-chooser-done"
          :disabled="!canClose"
          @click="closeChooser"
        >
          Done
        </button>
      </footer>
    </section>
  </div>
</template>
