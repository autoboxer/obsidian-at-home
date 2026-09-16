import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { createEmptyVault, createSeedVault } from '../data/seed';
import { readBrowserWorkspace, type StoredBrowserWorkspace } from '../services/browserWorkspace';
import {
  bootstrapWorkspace,
  createWorkspace,
  forgetWorkspace,
  getWorkspaceRevision,
  isTauri,
  openWorkspace,
  pickFolder
} from '../services/native';
import type { RecentlyDeletedNote, VaultData, VaultDescriptor, WorkspaceLoad } from '../types';
import {
  flushNoteEditorPositions,
  hasPendingNoteEditorPositions,
  initializeNoteEditorPositions
} from './editorPositions';
import {
  errorMessage,
  normalizeVault,
  safeStorageGet,
  safeStorageSet,
  type createVaultChangeTracker
} from './vaultPersistence';
import type { createVaultRecovery } from './vaultRecovery';
import { assetDeletionState, uiState, vaultSession, vaultState, type ToastTone } from './vaultState';

const LEGACY_MIGRATED_KEY = 'obsidian-at-home.vault.filesystem-migrated.v1';
const EXTERNAL_CHECK_DELAY = 3_000;

interface VaultLifecycleDependencies {
  advanceSessionGeneration: () => void;
  applyWorkspace: ( workspace: WorkspaceLoad, recentVaults?: VaultDescriptor[]) => void;
  currentEditorPositionVaultId: () => string;
  flushVault: () => Promise<boolean>;
  getSessionGeneration: () => number;
  hasRecoverySaveInFlight: () => boolean;
  hasSaveInFlight: () => boolean;
  hydrateVault: ( vault: Partial<VaultData> ) => void;
  markVaultInitialized: () => void;
  notify: ( message: string, tone: ToastTone ) => void;
  persistBrowserWorkspace: ( vault: VaultData, notes: RecentlyDeletedNote[]) => boolean;
  recovery: Pick<ReturnType<typeof createVaultRecovery>,
    | 'hydrateRecentlyDeletedNotes'
    | 'pruneExpiredRecentlyDeletedNotes'
    | 'scheduleRecentlyDeletedExpiry'
    | 'snapshotRecentlyDeletedNotes'
  >;
  resetNoteNavigation: () => void;
  snapshotVault: () => VaultData;
  vaultChanges: Pick<ReturnType<typeof createVaultChangeTracker>,
    'acknowledge' | 'hasChanges' | 'reset' | 'version'
  >;
}

export function createVaultLifecycle( dependencies: VaultLifecycleDependencies ) {
  const {
    advanceSessionGeneration,
    applyWorkspace,
    currentEditorPositionVaultId,
    flushVault,
    getSessionGeneration,
    hasRecoverySaveInFlight,
    hasSaveInFlight,
    hydrateVault,
    markVaultInitialized,
    notify,
    persistBrowserWorkspace,
    recovery,
    resetNoteNavigation,
    snapshotVault,
    vaultChanges
  } = dependencies;
  const {
    hydrateRecentlyDeletedNotes,
    pruneExpiredRecentlyDeletedNotes,
    scheduleRecentlyDeletedExpiry,
    snapshotRecentlyDeletedNotes
  } = recovery;
  let externalCheckTimer: ReturnType<typeof setInterval> | undefined;
  let checkingExternalChanges = false;
  let initializePromise: Promise<void> | null = null;
  let closeHandlerInstalled = false;
  let closingAfterSave = false;

  function initializeVault(): Promise<void> {
    if ( initializePromise ) {
      return initializePromise;
    }
    initializePromise = initializeVaultStorage();

    return initializePromise;
  }

  async function initializeVaultStorage(): Promise<void> {
    vaultSession.error = null;
    vaultSession.phase = 'loading';
    vaultSession.access = { mode: 'read-write' };

    if ( !isTauri() ) {
      let storedVault: StoredBrowserWorkspace | null;
      try {
        storedVault = readStoredVault();
      } catch ( error ) {
        hydrateVault( createEmptyVault() );
        hydrateRecentlyDeletedNotes([]);
        resetNoteNavigation();
        vaultSession.backend = 'browser';
        vaultSession.phase = 'error';
        vaultSession.error = errorMessage( error, 'Saved browser notes could not be read safely.' );
        markVaultInitialized();
        installVaultLifecycleHandlers();

        return;
      }
      const browserVault = storedVault?.vault ?? createSeedVault();
      hydrateVault( browserVault );
      hydrateRecentlyDeletedNotes( storedVault?.recentlyDeletedNotes ?? []);
      initializeNoteEditorPositions( 'browser', null, vaultState.notes );
      resetNoteNavigation();
      vaultSession.backend = 'browser';
      vaultSession.phase = 'ready';
      vaultSession.path = null;
      vaultSession.recentVaults = [];
      vaultSession.legacyAvailable = false;
      vaultSession.revision = 0;
      vaultSession.conflict = false;
      vaultSession.warnings = [];
      markVaultInitialized();
      vaultChanges.acknowledge( vaultChanges.version );
      if ( storedVault?.needsRewrite ) {
        persistBrowserWorkspace( snapshotVault(), snapshotRecentlyDeletedNotes() );
      }
      scheduleRecentlyDeletedExpiry();
      void pruneExpiredRecentlyDeletedNotes();
      installVaultLifecycleHandlers();

      return;
    }

    vaultSession.backend = 'native';
    let legacy: StoredBrowserWorkspace | null = null;
    try {
      legacy = readStoredVault();
    } catch {
      // A newer browser workspace remains untouched and unavailable for migration
    }
    vaultSession.legacyAvailable = Boolean(
      legacy && safeStorageGet( LEGACY_MIGRATED_KEY ) !== legacy.migrationFingerprint
    );

    try {
      const result = await bootstrapWorkspace( createEmptyVault() );
      vaultSession.recentVaults = result.recentVaults;
      if ( result.workspace ) {
        applyWorkspace( result.workspace, result.recentVaults );
      } else {
        hydrateVault( createEmptyVault() );
        hydrateRecentlyDeletedNotes([]);
        resetNoteNavigation();
        vaultSession.phase = 'needs-vault';
        vaultSession.path = null;
        vaultSession.revision = 0;
        vaultSession.conflict = false;
        vaultSession.warnings = [];
        uiState.vaultChooserOpen = true;
      }
    } catch ( error ) {
      hydrateVault( createEmptyVault() );
      hydrateRecentlyDeletedNotes([]);
      resetNoteNavigation();
      vaultSession.phase = 'error';
      vaultSession.error = errorMessage( error, 'The vault list could not be opened.' );
      uiState.vaultChooserOpen = true;
    } finally {
      markVaultInitialized();
      vaultChanges.acknowledge( vaultChanges.version );
      installVaultLifecycleHandlers();
    }
  }

  async function createFilesystemVault( name: string, useLegacy = false ): Promise<boolean> {
    if ( vaultSession.backend !== 'native' || vaultSession.busy ) {
      return false;
    }
    const cleanName = name.trim();
    if ( !cleanName ) {
      return false;
    }

    vaultSession.busy = true;
    vaultSession.error = null;
    try {
      if ( !( await flushBeforeVaultChange() ) ) {
        return false;
      }

      const parentPath = await pickFolder();
      if ( !parentPath ) {
        return false;
      }
      const legacy = useLegacy ? readStoredVault() : null;
      if ( useLegacy && !legacy ) {
        throw new Error( 'The previous notes could not be read from app storage.' );
      }
      const initial = legacy?.vault ?? createSeedVault();
      const workspace = await createWorkspace( parentPath, cleanName, initial );
      applyWorkspace( workspace );

      if ( useLegacy && legacy ) {
        safeStorageSet( LEGACY_MIGRATED_KEY, legacy.migrationFingerprint );
        vaultSession.legacyAvailable = false;
        notify( `Saved ${ legacy.vault.notes.length } ${ legacy.vault.notes.length === 1 ? 'note' : 'notes' } as Markdown files`, 'success' );
      } else {
        notify( `Created ${ workspace.descriptor.name }`, 'success' );
      }

      return true;
    } catch ( error ) {
      setVaultError( error, 'The vault could not be created.' );

      return false;
    } finally {
      vaultSession.busy = false;
      scheduleRecentlyDeletedExpiry();
    }
  }

  async function openFilesystemVault(): Promise<boolean> {
    if ( vaultSession.backend !== 'native' || vaultSession.busy ) {
      return false;
    }

    vaultSession.busy = true;
    vaultSession.error = null;
    try {
      if ( !( await flushBeforeVaultChange() ) ) {
        return false;
      }

      const path = await pickFolder();
      if ( !path ) {
        return false;
      }
      const workspace = await openWorkspace( path, createEmptyVault() );
      applyWorkspace( workspace );
      notify( `Opened ${ workspace.descriptor.name }`, 'success' );

      return true;
    } catch ( error ) {
      setVaultError( error, 'That folder could not be opened as a vault.' );

      return false;
    } finally {
      vaultSession.busy = false;
      scheduleRecentlyDeletedExpiry();
    }
  }

  async function switchFilesystemVault( path: string ): Promise<boolean> {
    if ( vaultSession.backend !== 'native' || vaultSession.busy || path === vaultSession.path ) {
      return path === vaultSession.path;
    }

    vaultSession.busy = true;
    vaultSession.error = null;
    try {
      if ( !( await flushBeforeVaultChange() ) ) {
        return false;
      }

      const workspace = await openWorkspace( path, createEmptyVault() );
      applyWorkspace( workspace );
      notify( `Switched to ${ workspace.descriptor.name }`, 'success' );

      return true;
    } catch ( error ) {
      setVaultError( error, 'That recent vault is no longer available.' );

      return false;
    } finally {
      vaultSession.busy = false;
      scheduleRecentlyDeletedExpiry();
    }
  }

  async function forgetCurrentVault(): Promise<boolean> {
    const path = vaultSession.path;
    if ( vaultSession.backend !== 'native' || !path || vaultSession.busy ) {
      return false;
    }

    vaultSession.busy = true;
    vaultSession.error = null;
    try {
      if ( !( await flushBeforeVaultChange() ) ) {
        return false;
      }

      const recentVaults = await forgetWorkspace( path );
      advanceSessionGeneration();
      vaultSession.recentVaults = recentVaults;
      vaultSession.path = null;
      vaultSession.revision = 0;
      vaultSession.conflict = false;
      vaultSession.warnings = [];
      vaultSession.phase = 'needs-vault';
      vaultSession.access = { mode: 'read-write' };
      hydrateVault( createEmptyVault() );
      hydrateRecentlyDeletedNotes([]);
      resetNoteNavigation();
      vaultChanges.reset();
      uiState.vaultChooserOpen = true;
      notify( 'Vault forgotten; its files are still on disk', 'neutral' );

      return true;
    } catch ( error ) {
      setVaultError( error, 'The vault could not be removed from the recent list.' );

      return false;
    } finally {
      vaultSession.busy = false;
      scheduleRecentlyDeletedExpiry();
    }
  }

  async function showCurrentVaultInFolder(): Promise<void> {
    if ( !vaultSession.path || vaultSession.backend !== 'native' ) {
      return;
    }
    try {
      await revealItemInDir( vaultSession.path );
    } catch ( error ) {
      setVaultError( error, 'The vault folder could not be shown.' );
      throw error;
    }
  }

  async function reloadFilesystemVault(): Promise<boolean> {
    const path = vaultSession.path;
    if ( vaultSession.backend !== 'native' || !path || vaultSession.busy ) {
      return false;
    }

    vaultSession.busy = true;
    try {
      await flushNoteEditorPositions( currentEditorPositionVaultId() );
      const workspace = await openWorkspace( path, createEmptyVault() );
      applyWorkspace( workspace );
      notify( 'Reloaded the vault from disk', 'success' );

      return true;
    } catch ( error ) {
      setVaultError( error, 'The vault could not be reloaded from disk.' );

      return false;
    } finally {
      vaultSession.busy = false;
      scheduleRecentlyDeletedExpiry();
    }
  }

  function readStoredVault(): StoredBrowserWorkspace | null {
    return readBrowserWorkspace( normalizeVault );
  }

  async function flushBeforeVaultChange(): Promise<boolean> {
    if ( vaultChanges.hasChanges ) {
      if ( vaultSession.phase !== 'ready' ) {
        vaultSession.error = 'Choose a vault before saving changes.';

        return false;
      }
      if ( !( await flushVault() ) ) {
        vaultSession.error = 'Save the current changes before switching vaults.';

        return false;
      }
    }
    const positionsSaved = await flushNoteEditorPositions( currentEditorPositionVaultId() );
    if ( !positionsSaved ) {
      vaultSession.error = 'Save the current document position before switching vaults.';
    }

    return positionsSaved;
  }

  function installVaultLifecycleHandlers(): void {
    if ( typeof window === 'undefined' || externalCheckTimer ) {
      return;
    }

    window.addEventListener( 'blur', () => void flushApplicationState() );
    window.addEventListener( 'focus', () => {
      void ( async () => {
        await refreshWorkspaceFromDisk();
        await pruneExpiredRecentlyDeletedNotes();
      })();
    });
    window.addEventListener( 'beforeunload', () => {
      void flushVault();
      void flushNoteEditorPositions();
    });
    document.addEventListener( 'visibilitychange', () => {
      if ( document.visibilityState === 'hidden' ) {
        void flushApplicationState();
      } else {
        void ( async () => {
          await refreshWorkspaceFromDisk();
          await pruneExpiredRecentlyDeletedNotes();
        })();
      }
    });

    if ( vaultSession.backend === 'native' ) {
      void installNativeCloseHandler();
    }

    externalCheckTimer = setInterval(
      () => void refreshWorkspaceFromDisk(),
      EXTERNAL_CHECK_DELAY
    );
  }

  async function flushApplicationState(): Promise<void> {
    await flushVault();
    await flushNoteEditorPositions();
  }

  async function installNativeCloseHandler(): Promise<void> {
    if ( closeHandlerInstalled ) {
      return;
    }
    closeHandlerInstalled = true;
    const appWindow = getCurrentWindow();
    try {
      await appWindow.onCloseRequested( async ( event ) => {
        if ( closingAfterSave ) {
          return;
        }
        if ( vaultSession.busy ) {
          event.preventDefault();
          notify( 'Wait for the current vault action to finish before closing', 'warning' );

          return;
        }
        if (
          !vaultChanges.hasChanges
          && !hasSaveInFlight()
          && !hasPendingNoteEditorPositions()
        ) {
          return;
        }
        event.preventDefault();
        const saved = await flushVault();
        if ( !saved ) {
          notify( vaultSession.error || 'Save the current changes before closing', 'warning' );

          return;
        }
        const positionsSaved = await flushNoteEditorPositions();
        if ( !positionsSaved ) {
          notify( 'Notes are saved, but document positions could not be saved', 'warning' );
        }
        closingAfterSave = true;
        await appWindow.destroy();
      });
    } catch ( error ) {
      closeHandlerInstalled = false;
      vaultSession.error = errorMessage( error, 'Could not install the safe-close handler.' );
    }
  }

  async function refreshWorkspaceFromDisk(): Promise<void> {
    const path = vaultSession.path;
    if (
      vaultSession.backend !== 'native'
      || vaultSession.phase !== 'ready'
      || !path
      || vaultSession.busy
      || assetDeletionState.request !== null
      || uiState.vaultChooserOpen
      || checkingExternalChanges
      || document.visibilityState === 'hidden'
      || hasSaveInFlight()
      || hasRecoverySaveInFlight()
      || vaultChanges.hasChanges
    ) {
      return;
    }

    checkingExternalChanges = true;
    const generation = getSessionGeneration();
    try {
      const revision = await getWorkspaceRevision( path );
      if ( revision === vaultSession.revision ) {
        return;
      }
      await flushNoteEditorPositions( currentEditorPositionVaultId() );
      const workspace = await openWorkspace( path, createEmptyVault() );
      if (
        generation !== getSessionGeneration()
        || path !== vaultSession.path
        || vaultSession.busy
        || assetDeletionState.request !== null
        || hasRecoverySaveInFlight()
        || vaultChanges.hasChanges
      ) {
        return;
      }
      applyWorkspace( workspace );
      notify( 'Reloaded changes from the vault folder', 'neutral' );
    } catch ( error ) {
      if ( generation === getSessionGeneration() && path === vaultSession.path ) {
        vaultSession.error = errorMessage( error, 'The vault folder could not be checked for changes.' );
      }
    } finally {
      checkingExternalChanges = false;
    }
  }

  function setVaultError( error: unknown, fallback: string ): void {
    vaultSession.error = errorMessage( error, fallback );
    vaultSession.conflict = false;
  }

  return {
    createFilesystemVault,
    flushApplicationState,
    forgetCurrentVault,
    initializeVault,
    openFilesystemVault,
    reloadFilesystemVault,
    showCurrentVaultInFolder,
    switchFilesystemVault
  };
}
