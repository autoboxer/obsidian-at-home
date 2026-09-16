import { deleteWorkspaceAsset } from '../services/native';
import type { WorkspaceSaveResult } from '../types';
import { deleteNoteEditorHistory, resetNoteEditorHistory } from './editorHistories';
import {
  planVaultAssetDeletion,
  type VaultAssetDeletionPlan,
  type VaultAssetIdentity,
  type VaultAssetKind
} from './vaultAssets';
import { errorMessage } from './vaultPersistence';
import {
  assetDeletionState,
  canEditVault,
  recentlyDeletedState,
  uiState,
  vaultSession,
  vaultState,
  type ToastTone
} from './vaultState';

interface VaultAssetDeletionDependencies {
  applyVaultMutation: ( mutation: () => void ) => void;
  applyWorkspaceSaveResult: ( result: WorkspaceSaveResult ) => void;
  currentEditorPositionVaultId: () => string;
  flushVault: () => Promise<boolean>;
  getSessionGeneration: () => number;
  notify: ( message: string, tone: ToastTone ) => void;
  runExclusiveVaultDataOperation: <T>( fallback: T, operation: () => Promise<T> ) => Promise<T>;
}

export function createVaultAssetDeletion(
  dependencies: VaultAssetDeletionDependencies
) {
  const {
    applyVaultMutation,
    applyWorkspaceSaveResult,
    currentEditorPositionVaultId,
    flushVault,
    getSessionGeneration,
    notify,
    runExclusiveVaultDataOperation
  } = dependencies;
  let pendingAssetDeletion: { generation: number; signature: string } | null = null;

  function cancelVaultAssetDeletion(): void {
    if ( vaultSession.busy ) {
      return;
    }
    clearAssetDeletionRequest();
  }

  function clearAssetDeletionRequest(): void {
    assetDeletionState.request = null;
    pendingAssetDeletion = null;
  }

  async function requestVaultAssetDeletion(
    kind: VaultAssetKind,
    asset: VaultAssetIdentity
  ): Promise<boolean> {
    if ( assetDeletionState.request ) {
      return false;
    }

    return deleteVaultAsset( kind, asset );
  }

  async function confirmVaultAssetDeletion(): Promise<boolean> {
    const request = assetDeletionState.request;
    const pending = pendingAssetDeletion;
    if ( !request || !pending || pending.generation !== getSessionGeneration() ) {
      clearAssetDeletionRequest();

      return false;
    }

    return deleteVaultAsset( request.kind, request, pending.signature );
  }

  async function deleteVaultAsset(
    kind: VaultAssetKind,
    identity: VaultAssetIdentity,
    confirmedSignature?: string
  ): Promise<boolean> {
    const path = vaultSession.path;
    if ( vaultSession.backend !== 'native' || !path || uiState.vaultChooserOpen ) {
      return false;
    }

    return runExclusiveVaultDataOperation( false, async () => {
      const generation = getSessionGeneration();
      uiState.commandOpen = false;
      if ( !( await flushVault() ) ) {
        clearAssetDeletionRequest();

        return false;
      }
      if ( generation !== getSessionGeneration() || !canEditVault.value ) {
        clearAssetDeletionRequest();

        return false;
      }

      const files = kind === 'image' ? vaultState.imageFiles : vaultState.attachmentFiles;
      // Resolve the selected identity exactly; never choose a case-sensitive sibling.
      const asset = identity.assetId
        ? files.find( ( file ) => file.assetId === identity.assetId )
        : files.find( ( file ) => file.relativePath === identity.relativePath );
      if ( !asset ) {
        clearAssetDeletionRequest();
        notify( 'The file is no longer in this vault. Reload the vault and try again.', 'warning' );

        return false;
      }
      const plan = planVaultAssetDeletion( vaultState, recentlyDeletedState.notes, kind, asset );
      const signature = JSON.stringify([ kind, asset.assetId, asset.relativePath, plan ]);
      if (
        confirmedSignature !== undefined && signature !== confirmedSignature
        || confirmedSignature === undefined && plan.referenceCount > 0
      ) {
        pendingAssetDeletion = { generation, signature };
        assetDeletionState.request = {
          kind,
          assetId: asset.assetId,
          relativePath: asset.relativePath,
          referenceCount: plan.referenceCount,
          recoveryReferenceCount: plan.recoveryReferenceCount,
          changed: confirmedSignature !== undefined
        };

        return false;
      }

      const revision = vaultSession.revision;
      try {
        const result = await deleteWorkspaceAsset(
          path,
          kind,
          asset.relativePath,
          asset.assetId,
          plan.noteUpdates,
          plan.recoveryUpdates,
          revision
        );
        if ( generation !== getSessionGeneration() ) {
          return false;
        }
        applyAssetDeletion( kind, asset, plan, result );
        clearAssetDeletionRequest();
        // A committed deletion can report an old revision if final validation failed.
        // Show the existing recovery UI before another operation can use stale state.
        if ( result.revision === revision ) {
          showAssetDeletionReload( 'The file was deleted, but the vault needs to be reloaded before continuing.' );
        } else {
          const name = asset.relativePath.split( '/' ).at( -1 );
          notify( result.warnings[ 0 ] || `Deleted ${ name }`, result.warnings.length ? 'warning' : 'success' );
        }

        return true;
      } catch ( error ) {
        if ( generation === getSessionGeneration() ) {
          clearAssetDeletionRequest();
          showAssetDeletionReload( errorMessage( error, 'The file could not be deleted. Reload the vault and try again.' ) );
        }

        return false;
      }
    });
  }

  function showAssetDeletionReload( message: string ): void {
    vaultSession.error = message;
    vaultSession.conflict = true;
    uiState.saveStatus = 'error';
    uiState.vaultChooserOpen = true;
    notify( message, 'warning' );
  }

  function applyAssetDeletion(
    kind: VaultAssetKind,
    asset: VaultAssetIdentity,
    plan: VaultAssetDeletionPlan,
    result: WorkspaceSaveResult
  ): void {
    const liveUpdates = new Map( plan.noteUpdates.map( ( update ) => [ update.noteId, update.content ]) );
    const recoveryUpdates = new Map( plan.recoveryUpdates.map( ( update ) => [ update.noteId, update.content ]) );
    const vaultId = currentEditorPositionVaultId();
    applyVaultMutation( () => {
      for ( const note of vaultState.notes ) {
        const content = liveUpdates.get( note.id );
        if ( content !== undefined ) {
          note.content = content;
          note.updatedAt = result.savedAt;
          resetNoteEditorHistory( vaultId, note.id );
        }
      }
      if ( kind === 'image' ) {
        vaultState.imageFiles = vaultState.imageFiles.filter( ( file ) => file.relativePath !== asset.relativePath );
        vaultState.embeddedImages = vaultState.embeddedImages.filter( ( file ) => file.relativePath !== asset.relativePath );
      } else {
        vaultState.attachmentFiles = vaultState.attachmentFiles.filter( ( file ) => file.relativePath !== asset.relativePath );
        vaultState.embeddedAttachments = vaultState.embeddedAttachments.filter( ( file ) => file.relativePath !== asset.relativePath );
      }
    });
    for ( const entry of recentlyDeletedState.notes ) {
      const content = recoveryUpdates.get( entry.id );
      if ( content !== undefined ) {
        entry.note.content = content;
        delete entry.editorPosition;
        deleteNoteEditorHistory( vaultId, entry.note.id );
      }
    }
    uiState.imageRefreshToken += 1;
    uiState.attachmentRefreshToken += 1;
    applyWorkspaceSaveResult( result );
  }

  return {
    cancelVaultAssetDeletion,
    clearAssetDeletionRequest,
    confirmVaultAssetDeletion,
    requestVaultAssetDeletion
  };
}
