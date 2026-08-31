use super::*;
mod editor_positions;
mod frontmatter;
mod recovery;
mod transaction;

pub(in crate::workspace) use editor_positions::*;
pub(in crate::workspace) use frontmatter::*;
pub(in crate::workspace) use recovery::*;
pub(in crate::workspace) use transaction::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredNoteMetadata {
    #[serde(default)]
    pub(super) pinned: bool,
    #[serde(default)]
    pub(super) created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredRecentlyDeletedNote {
    pub(super) deleted_at: u64,
    pub(super) expires_at: u64,
    pub(super) fingerprint: FileFingerprint,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecentlyDeletedSnapshot {
    pub(super) version: u32,
    pub(super) deleted_note: RecentlyDeletedNote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct WorkspaceState {
    #[serde(default = "state_version")]
    pub(super) version: u32,
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) note_paths: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) folder_paths: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) note_metadata: BTreeMap<String, StoredNoteMetadata>,
    #[serde(default)]
    pub(super) templates: Vec<NoteTemplate>,
    #[serde(default)]
    pub(super) snippets: Vec<CssSnippet>,
    #[serde(default)]
    pub(super) active_note_id: Option<String>,
    #[serde(default)]
    pub(super) recent_note_ids: Vec<String>,
    #[serde(default)]
    pub(super) recently_deleted_notes: BTreeMap<String, StoredRecentlyDeletedNote>,
    #[serde(default, alias = "imageAssets")]
    pub(super) assets: BTreeMap<String, StoredVaultAsset>,
    #[serde(default)]
    pub(super) image_embed_settings: ImageEmbedSettings,
    #[serde(default)]
    pub(super) attachment_embed_settings: AttachmentEmbedSettings,
    #[serde(default = "default_folder_selection")]
    pub(super) selected_folder_id: String,
    #[serde(default)]
    pub(super) last_committed_transaction_id: Option<String>,
    #[serde(default)]
    pub(super) last_committed_image_import_id: Option<String>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            name: String::new(),
            note_paths: BTreeMap::new(),
            folder_paths: BTreeMap::new(),
            note_metadata: BTreeMap::new(),
            templates: Vec::new(),
            snippets: Vec::new(),
            active_note_id: None,
            recent_note_ids: Vec::new(),
            recently_deleted_notes: BTreeMap::new(),
            assets: BTreeMap::new(),
            image_embed_settings: ImageEmbedSettings::default(),
            attachment_embed_settings: AttachmentEmbedSettings::default(),
            selected_folder_id: default_folder_selection(),
            last_committed_transaction_id: None,
            last_committed_image_import_id: None,
        }
    }
}

pub(super) fn load_workspace(root: &Path, defaults: &VaultData) -> Result<WorkspaceLoad, String> {
    let root = validate_workspace_root_path(root)?;
    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(&root, &mut warnings);
    let state_was_present = stored_state.is_some();
    if state_was_present || !state_file_was_present {
        recover_workspace_transactions(&root, stored_state.as_ref(), &mut warnings)?;
    } else {
        warnings.push(
            "Save transactions were not recovered because workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    let mut state = stored_state.unwrap_or_default();
    let (scanned_notes, scanned_folders, scanned_images, scanned_attachments) =
        scan_workspace_files(&root, &mut warnings)?;

    let mut used_note_ids = HashSet::new();
    let note_id_by_path = reverse_valid_paths(&state.note_paths, "note", &mut warnings);
    let mut notes = Vec::with_capacity(scanned_notes.len());
    let mut note_paths = BTreeMap::new();
    let mut note_metadata = BTreeMap::new();

    for scanned in scanned_notes {
        let id = note_id_by_path
            .get(&scanned.relative_path)
            .filter(|id| used_note_ids.insert((*id).clone()))
            .cloned()
            .unwrap_or_else(|| fresh_id("note", &scanned.relative_path, &mut used_note_ids));
        let metadata = state.note_metadata.get(&id).cloned().unwrap_or_default();
        let title = Path::new(&scanned.relative_path)
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Untitled note")
            .to_owned();
        note_paths.insert(id.clone(), scanned.relative_path.clone());
        note_metadata.insert(
            id.clone(),
            StoredNoteMetadata {
                pinned: metadata.pinned,
                created_at: if metadata.created_at > 0 {
                    metadata.created_at
                } else {
                    scanned.created_at
                },
            },
        );
        notes.push(Note {
            id,
            relative_path: scanned.relative_path,
            title,
            content: scanned.content,
            folder_id: None,
            tags: scanned.tags,
            pinned: metadata.pinned,
            created_at: if metadata.created_at > 0 {
                metadata.created_at
            } else {
                scanned.created_at
            },
            updated_at: scanned.updated_at,
        });
    }

    let mut used_folder_ids = HashSet::new();
    let folder_id_by_path = reverse_valid_paths(&state.folder_paths, "folder", &mut warnings);
    let mut folder_ids = HashMap::new();
    let mut folder_created_at = HashMap::new();
    for scanned in &scanned_folders {
        let id = folder_id_by_path
            .get(&scanned.relative_path)
            .filter(|id| used_folder_ids.insert((*id).clone()))
            .cloned()
            .unwrap_or_else(|| fresh_id("folder", &scanned.relative_path, &mut used_folder_ids));
        folder_ids.insert(scanned.relative_path.clone(), id);
        folder_created_at.insert(scanned.relative_path.clone(), scanned.created_at);
    }

    let mut folders = Vec::with_capacity(scanned_folders.len());
    let mut folder_paths = BTreeMap::new();
    for scanned in scanned_folders {
        let id = folder_ids
            .get(&scanned.relative_path)
            .expect("scanned folder should have an ID")
            .clone();
        let relative = Path::new(&scanned.relative_path);
        let name = relative
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Folder")
            .to_owned();
        let parent_id = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .and_then(path_to_slash_string)
            .and_then(|parent| folder_ids.get(&parent).cloned());
        folder_paths.insert(id.clone(), scanned.relative_path.clone());
        folders.push(Folder {
            id,
            name,
            parent_id,
            created_at: *folder_created_at
                .get(&scanned.relative_path)
                .unwrap_or(&now_millis()),
        });
    }

    for note in &mut notes {
        note.folder_id = Path::new(&note.relative_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .and_then(path_to_slash_string)
            .and_then(|parent| folder_ids.get(&parent).cloned());
    }

    notes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    folders.sort_by(|left, right| folder_paths.get(&left.id).cmp(&folder_paths.get(&right.id)));

    let note_ids: HashSet<&str> = notes.iter().map(|note| note.id.as_str()).collect();
    let state_ids_are_trustworthy = state_was_present || !state_file_was_present;
    let (editor_positions, editor_positions_writable, editor_positions_revision) =
        if state_ids_are_trustworthy {
            load_editor_positions(&root, &note_ids, &mut warnings)
        } else {
            (BTreeMap::new(), false, None)
        };
    let active_note_id = state
        .active_note_id
        .filter(|id| note_ids.contains(id.as_str()))
        .or_else(|| notes.first().map(|note| note.id.clone()));
    let recent_note_ids =
        normalize_recent_note_ids(&state.recent_note_ids, active_note_id.as_deref(), &note_ids);
    let selected_folder_id = if is_virtual_folder_selection(&state.selected_folder_id) {
        state.selected_folder_id.clone()
    } else {
        "all".to_owned()
    };
    let templates = if state_was_present {
        state.templates.clone()
    } else {
        defaults.templates.clone()
    };
    let snippets = if state_was_present {
        state.snippets.clone()
    } else {
        defaults.snippets.clone()
    };
    let embedded_images = reconcile_image_assets(&root, &mut state.assets, &mut warnings);
    let tracked_image_ids = embedded_images
        .iter()
        .map(|image| (portable_path_key(&image.relative_path), image.id.clone()))
        .collect::<HashMap<_, _>>();
    let image_files = scanned_images
        .into_iter()
        .map(|image| VaultImageFile {
            asset_id: tracked_image_ids
                .get(&portable_path_key(&image.relative_path))
                .cloned(),
            relative_path: image.relative_path,
            media_type: image.media_type,
        })
        .collect();
    let embedded_attachments = reconcile_attachment_assets(&root, &mut state.assets, &mut warnings);
    let tracked_attachment_ids = embedded_attachments
        .iter()
        .map(|attachment| {
            (
                portable_path_key(&attachment.relative_path),
                attachment.id.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    let attachment_files = scanned_attachments
        .into_iter()
        .map(|attachment| VaultAttachmentFile {
            asset_id: tracked_attachment_ids
                .get(&portable_path_key(&attachment.relative_path))
                .cloned(),
            relative_path: attachment.relative_path,
            media_type: attachment.media_type,
            byte_length: attachment.byte_length,
            opening_disabled: attachment.opening_disabled,
        })
        .collect();
    let image_embed_settings = match normalize_image_embed_settings(&state.image_embed_settings) {
        Ok(settings) => settings,
        Err(error) => {
            warnings.push(format!("Reset invalid image embed settings: {error}"));
            ImageEmbedSettings::default()
        }
    };
    let attachment_embed_settings =
        match normalize_attachment_embed_settings(&state.attachment_embed_settings) {
            Ok(settings) => settings,
            Err(error) => {
                warnings.push(format!("Reset invalid attachment embed settings: {error}"));
                AttachmentEmbedSettings::default()
            }
        };
    let vault_name = display_vault_name(
        if state_was_present && !state.name.trim().is_empty() {
            &state.name
        } else {
            ""
        },
        &root,
    );
    let mut recently_deleted_state = state.recently_deleted_notes.clone();
    let now = now_millis();
    if recently_deleted_state
        .values()
        .any(|entry| now >= entry.expires_at)
    {
        let expired_ids = recently_deleted_state
            .iter()
            .filter_map(|(id, entry)| (now >= entry.expires_at).then(|| id.clone()))
            .collect::<Vec<_>>();
        for id in expired_ids {
            let entry = recently_deleted_state
                .get(&id)
                .expect("expired recovery entry should still exist")
                .clone();
            if remove_expired_recovery_snapshot(&root, &id, &entry, &mut warnings) {
                recently_deleted_state.remove(&id);
            }
        }
    }
    let recently_deleted_notes =
        load_recently_deleted_notes(&root, &recently_deleted_state, &mut warnings);

    state = WorkspaceState {
        version: STATE_VERSION,
        name: vault_name.clone(),
        note_paths,
        folder_paths,
        note_metadata,
        templates: templates.clone(),
        snippets: snippets.clone(),
        active_note_id: active_note_id.clone(),
        recent_note_ids: recent_note_ids.clone(),
        recently_deleted_notes: recently_deleted_state,
        assets: state.assets.clone(),
        image_embed_settings: image_embed_settings.clone(),
        attachment_embed_settings: attachment_embed_settings.clone(),
        selected_folder_id: selected_folder_id.clone(),
        last_committed_transaction_id: state.last_committed_transaction_id.clone(),
        last_committed_image_import_id: state.last_committed_image_import_id.clone(),
    };
    let mut state_was_written = false;
    if state_was_present || !state_file_was_present {
        match write_workspace_state(&root, &state) {
            Ok(()) => state_was_written = true,
            Err(error) => warnings.push(format!("Could not save workspace metadata: {error}")),
        }
    } else {
        warnings.push(
            "Workspace metadata was not replaced because the existing file could not be read."
                .to_owned(),
        );
    }
    if state_was_written {
        cleanup_orphaned_recovery_snapshots(
            &root,
            &state.recently_deleted_notes,
            &HashSet::new(),
            &mut warnings,
        );
    }

    let opened_at = now_millis();
    let revision = revision_for_root(&root)?;
    Ok(WorkspaceLoad {
        vault: VaultData {
            name: vault_name.clone(),
            notes,
            folders,
            templates,
            snippets,
            active_note_id,
            recent_note_ids,
            selected_folder_id,
            embedded_images,
            image_files,
            image_embed_settings,
            embedded_attachments,
            attachment_files,
            attachment_embed_settings,
        },
        descriptor: VaultDescriptor {
            name: vault_name,
            path: path_string(&root)?,
            last_opened_at: opened_at,
        },
        recently_deleted_notes,
        editor_positions,
        editor_positions_revision,
        editor_positions_writable,
        warnings: warnings.finish(),
        revision,
    })
}

pub(super) fn save_workspace_files(
    root: &Path,
    vault: &VaultData,
    expected_revision: u64,
) -> Result<SaveResult, String> {
    save_workspace_files_with_archive(root, vault, expected_revision, None)
        .map(|(result, _)| result)
}

pub(super) fn save_workspace_files_with_image_import(
    root: &Path,
    vault: &VaultData,
    expected_revision: u64,
    transaction_id: &str,
) -> Result<WorkspaceImportSaveResult, String> {
    pending_workspace_image_import(root, transaction_id)?;
    let result = save_workspace_files_with_recovery(
        root,
        vault,
        expected_revision,
        None,
        None,
        Some(transaction_id),
    );
    match result {
        Ok((mut result, _, _)) => {
            let mut cleanup_warnings = WarningCollector::default();
            if let Err(error) =
                finalize_workspace_image_import(root, transaction_id, &mut cleanup_warnings)
            {
                result.warnings.push(format!(
                    "The imported assets were saved, but their transaction cleanup will be retried when the vault reopens: {error}"
                ));
            }
            result.warnings.extend(cleanup_warnings.finish());

            Ok(WorkspaceImportSaveResult {
                saved: true,
                error: None,
                note_paths: result.note_paths,
                revision: result.revision,
                saved_at: result.saved_at,
                warnings: result.warnings,
            })
        }
        Err(error) => {
            let mut state_warnings = WarningCollector::default();
            let (state, state_file_was_present) = read_workspace_state(root, &mut state_warnings);
            let committed = state.as_ref().is_some_and(|state| {
                state.last_committed_image_import_id.as_deref() == Some(transaction_id)
            });
            if committed {
                let mut cleanup_warnings = WarningCollector::default();
                if let Err(cleanup_error) =
                    finalize_workspace_image_import(root, transaction_id, &mut cleanup_warnings)
                {
                    cleanup_warnings.push(format!(
                        "The committed asset import will be cleaned up when the vault reopens: {cleanup_error}"
                    ));
                }
                let state = state.expect("committed state should be available");
                let mut warnings = state_warnings.finish();
                warnings.extend(cleanup_warnings.finish());
                warnings.push(format!(
                    "The import was saved, but its final verification reported: {error}"
                ));

                return Ok(WorkspaceImportSaveResult {
                    saved: true,
                    error: None,
                    note_paths: state.note_paths,
                    revision: revision_for_root(root)?,
                    saved_at: now_millis(),
                    warnings,
                });
            }
            if state.is_none() && state_file_was_present {
                return Err(format!(
                    "{error} The asset import could not be rolled back because workspace metadata became unreadable. Reopen the vault before editing again."
                ));
            }

            let mut rollback_warnings = WarningCollector::default();
            let recovered =
                rollback_workspace_image_import(root, transaction_id, &mut rollback_warnings)?;
            if !recovered {
                return Err(format!(
                    "{error} The imported assets could not be fully rolled back. Reopen the vault before editing again."
                ));
            }
            let revision = revision_for_root(root)?;
            if revision_for_root(root)? != revision {
                return Err(format!(
                    "{error} The vault changed while the failed asset import was being rolled back. Reload it before editing again."
                ));
            }
            let mut warnings = state_warnings.finish();
            warnings.extend(rollback_warnings.finish());

            Ok(WorkspaceImportSaveResult {
                saved: false,
                error: Some(error),
                note_paths: BTreeMap::new(),
                revision,
                saved_at: now_millis(),
                warnings,
            })
        }
    }
}

pub(super) fn save_workspace_files_with_archive(
    root: &Path,
    vault: &VaultData,
    expected_revision: u64,
    pending_archive: Option<PendingNoteArchive>,
) -> Result<(SaveResult, Option<RecentlyDeletedNote>), String> {
    save_workspace_files_with_recovery(root, vault, expected_revision, pending_archive, None, None)
        .map(|(result, deleted_note, _)| (result, deleted_note))
}

pub(super) fn save_workspace_files_with_restore(
    root: &Path,
    vault: &VaultData,
    expected_revision: u64,
    pending_restore: PendingNoteRestore,
) -> Result<(SaveResult, PreparedNoteRestore), String> {
    let (result, _, prepared_restore) = save_workspace_files_with_recovery(
        root,
        vault,
        expected_revision,
        None,
        Some(pending_restore),
        None,
    )?;
    let prepared_restore = prepared_restore
        .ok_or_else(|| "The note was restored without recovery metadata.".to_owned())?;

    Ok((result, prepared_restore))
}

pub(super) fn save_workspace_files_with_recovery(
    root: &Path,
    vault: &VaultData,
    expected_revision: u64,
    pending_archive: Option<PendingNoteArchive>,
    pending_restore: Option<PendingNoteRestore>,
    pending_image_import_id: Option<&str>,
) -> Result<
    (
        SaveResult,
        Option<RecentlyDeletedNote>,
        Option<PreparedNoteRestore>,
    ),
    String,
> {
    if pending_archive.is_some() && pending_restore.is_some() {
        return Err("A note cannot be archived and restored in the same save.".to_owned());
    }
    let root = validate_workspace_root_path(root)?;
    let mut warnings = WarningCollector::default();
    let state_path = workspace_state_path(&root);
    let expected_state_fingerprint = fingerprint_regular_file(&state_path)?;
    let (old_state, state_file_was_present) = read_workspace_state(&root, &mut warnings);
    if fingerprint_regular_file(&state_path)? != expected_state_fingerprint {
        return Err(
            "Workspace metadata changed while it was being read. Reload the vault before saving."
                .to_owned(),
        );
    }
    if old_state.is_none() && state_file_was_present {
        return Err(
            "The existing .obsidian-at-home/state.json file could not be read. Move or repair it before saving so it is not overwritten."
                .to_owned(),
        );
    }
    let old_state = old_state.unwrap_or_default();
    recover_workspace_transactions_except(
        &root,
        Some(&old_state),
        pending_image_import_id,
        &mut warnings,
    )?;
    if revision_for_root(&root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before saving so those changes are not overwritten."
                .to_owned(),
        );
    }

    let desired_folder_paths = build_folder_paths(&vault.folders)?;
    let prepared_restore = pending_restore
        .map(|restore| {
            prepare_note_restore(&root, vault, &old_state, &desired_folder_paths, restore)
        })
        .transpose()?;
    let preferred_new_paths = prepared_restore
        .as_ref()
        .map(|restore| {
            BTreeMap::from([(
                restore.restored_note.id.clone(),
                restore.restored_note.relative_path.clone(),
            )])
        })
        .unwrap_or_default();
    let plans = build_note_write_plans(
        &root,
        vault,
        &old_state,
        &desired_folder_paths,
        &preferred_new_paths,
    )?;
    if revision_for_root(&root)? != expected_revision {
        return Err(
            "The vault changed while it was being saved. Reload it before trying again.".to_owned(),
        );
    }

    let mut paths_to_replace = BTreeSet::new();
    for (id, old_relative_path) in &old_state.note_paths {
        let new_path = plans
            .iter()
            .find(|plan| plan.id == *id)
            .map(|plan| plan.new_relative_path.as_str());
        if new_path != Some(old_relative_path.as_str()) {
            paths_to_replace.insert(old_relative_path.clone());
        }
    }
    for plan in &plans {
        if plan.needs_write {
            if let Some(old_relative_path) = &plan.old_relative_path {
                paths_to_replace.insert(old_relative_path.clone());
            }
        }
    }
    validate_managed_path_ownership(&old_state.note_paths)?;
    validate_save_targets(&root, &plans, &paths_to_replace, &old_state.note_paths)?;
    let folder_case_renames =
        build_folder_case_renames(&old_state.folder_paths, &desired_folder_paths)?;
    validate_folder_case_renames(&root, &folder_case_renames)?;
    let created_directories =
        collect_created_directories(&root, desired_folder_paths.values(), &folder_case_renames)?;
    let baseline = note_file_stamps(&root)?;
    let consistency = build_save_consistency(&baseline, &paths_to_replace, &plans)?;
    if revision_for_root(&root)? != expected_revision {
        return Err(
            "The vault changed while the save was being prepared. Reload it before trying again."
                .to_owned(),
        );
    }

    let saved_at = now_millis();
    let prepared_archive = pending_archive
        .map(|archive| prepare_note_archive(&root, vault, &old_state, archive, saved_at))
        .transpose()?;
    let mut note_paths = BTreeMap::new();
    let mut note_metadata = BTreeMap::new();
    for (note, plan) in vault.notes.iter().zip(plans.iter()) {
        note_paths.insert(note.id.clone(), plan.new_relative_path.clone());
        note_metadata.insert(
            note.id.clone(),
            StoredNoteMetadata {
                pinned: note.pinned,
                created_at: if note.created_at > 0 {
                    note.created_at
                } else {
                    saved_at
                },
            },
        );
    }
    let note_ids: HashSet<&str> = vault.notes.iter().map(|note| note.id.as_str()).collect();
    let recent_note_ids = normalize_recent_note_ids(
        &vault.recent_note_ids,
        vault.active_note_id.as_deref(),
        &note_ids,
    );
    let mut recently_deleted_notes = old_state.recently_deleted_notes.clone();
    if let Some(archive) = &prepared_archive {
        recently_deleted_notes.insert(
            archive.deleted_note.id.clone(),
            StoredRecentlyDeletedNote {
                deleted_at: archive.deleted_note.deleted_at,
                expires_at: archive.deleted_note.expires_at,
                fingerprint: archive.fingerprint.clone(),
            },
        );
    }
    if let Some(restore) = &prepared_restore {
        recently_deleted_notes.remove(&restore.recovery_id);
    }
    let mut state = WorkspaceState {
        version: STATE_VERSION,
        name: display_vault_name(&vault.name, &root),
        note_paths,
        folder_paths: desired_folder_paths,
        note_metadata,
        templates: vault.templates.clone(),
        snippets: vault.snippets.clone(),
        active_note_id: vault.active_note_id.clone(),
        recent_note_ids,
        recently_deleted_notes,
        assets: old_state.assets.clone(),
        image_embed_settings: normalize_image_embed_settings(&vault.image_embed_settings)?,
        attachment_embed_settings: normalize_attachment_embed_settings(
            &vault.attachment_embed_settings,
        )?,
        selected_folder_id: vault.selected_folder_id.clone(),
        last_committed_transaction_id: old_state.last_committed_transaction_id.clone(),
        last_committed_image_import_id: pending_image_import_id
            .map(str::to_owned)
            .or_else(|| old_state.last_committed_image_import_id.clone()),
    };

    let needs_transaction = prepared_archive.is_some()
        || !paths_to_replace.is_empty()
        || plans.iter().any(|plan| plan.needs_write)
        || !folder_case_renames.is_empty()
        || !created_directories.is_empty();
    if needs_transaction {
        let transaction_id = new_transaction_id();
        let recovery_archives = prepared_archive
            .as_ref()
            .map(std::slice::from_ref)
            .unwrap_or(&[]);
        let (transaction_root, mut manifest) = prepare_transaction(
            &root,
            transaction_id,
            &paths_to_replace,
            &plans,
            recovery_archives,
            folder_case_renames,
            created_directories,
        )?;
        if revision_for_root(&root)? != expected_revision {
            discard_private_transaction(&root, &transaction_root, &mut warnings);

            return Err(
                "The vault changed while the save transaction was being prepared. Reload it before trying again."
                    .to_owned(),
            );
        }
        manifest.phase = TransactionPhase::Applying;
        write_transaction_manifest(&transaction_root, &manifest)?;

        if let Err(error) =
            apply_transaction(&root, &transaction_root, &manifest, &plans, &mut warnings)
        {
            let recovered =
                rollback_transaction(&root, &transaction_root, &manifest, &mut warnings);
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(error);
        }
        if let Err(error) = verify_save_consistency(&root, &consistency) {
            let recovered =
                rollback_transaction(&root, &transaction_root, &manifest, &mut warnings);
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(error);
        }
        if let Err(error) = verify_applied_recovery_targets(&root, &manifest.recovery_targets) {
            let recovered =
                rollback_transaction(&root, &transaction_root, &manifest, &mut warnings);
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(error);
        }
        if fingerprint_regular_file(&state_path)? != expected_state_fingerprint {
            let recovered =
                rollback_transaction(&root, &transaction_root, &manifest, &mut warnings);
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(
                "Workspace metadata changed outside Obsidian At Home while saving. Reload before editing again."
                    .to_owned(),
            );
        }
        if let Some(restore) = &prepared_restore {
            if let Err(error) =
                verify_recovery_snapshot_target(&root, &restore.recovery_id, &restore.fingerprint)
            {
                let recovered =
                    rollback_transaction(&root, &transaction_root, &manifest, &mut warnings);
                if recovered {
                    discard_private_transaction(&root, &transaction_root, &mut warnings);
                }

                return Err(error);
            }
        }

        state.last_committed_transaction_id = Some(manifest.id.clone());
        if let Err(error) = write_workspace_state(&root, &state) {
            let recovered =
                rollback_transaction(&root, &transaction_root, &manifest, &mut warnings);
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(format!("Could not save workspace metadata: {error}"));
        }
        finalize_committed_recovery_targets(
            &root,
            &transaction_root,
            &manifest.recovery_targets,
        )
        .map_err(|error| {
            format!(
                "The vault was saved, but its recovery snapshot could not be finalized. Reopen the vault before editing again. {error}"
            )
        })?;
        // The state file is the commit boundary. Persist the same fact in the
        // manifest before cleanup so an undeletable old transaction can never
        // be mistaken for an uncommitted save after a later transaction.
        manifest.phase = TransactionPhase::Committed;
        let transaction_was_finalized =
            match write_transaction_manifest(&transaction_root, &manifest) {
                Ok(()) => true,
                Err(error) if prepared_restore.is_some() => {
                    warnings.push(format!(
                    "The note was restored, but its save transaction will be cleaned up the next \
                     time the vault opens: {error}"
                ));
                    false
                }
                Err(error) => {
                    return Err(format!(
                    "The vault was saved, but its transaction could not be finalized. Reopen the \
                     vault before editing again. {error}"
                ));
                }
            };
        if let Err(error) = verify_save_consistency(&root, &consistency) {
            if prepared_restore.is_some() {
                warnings.push(format!(
                    "The note was restored, but the vault changed as the restore was committed. \
                     Reload before editing again. {error}"
                ));
            } else {
                return Err(format!(
                    "The vault changed as the save was committed. Reload before editing again. \
                     {error}"
                ));
            }
        }
        if transaction_was_finalized {
            discard_private_transaction(&root, &transaction_root, &mut warnings);
        }
    } else {
        verify_save_consistency(&root, &consistency)?;
        if fingerprint_regular_file(&state_path)? != expected_state_fingerprint {
            return Err(
                "Workspace metadata changed outside Obsidian At Home while saving. Reload before editing again."
                    .to_owned(),
            );
        }
        if let Some(restore) = &prepared_restore {
            verify_recovery_snapshot_target(&root, &restore.recovery_id, &restore.fingerprint)?;
        }
        write_workspace_state(&root, &state)?;
    }

    if let Some(restore) = &prepared_restore {
        remove_recovery_snapshot_if_matches(
            &root,
            &restore.recovery_id,
            &restore.fingerprint,
            &mut warnings,
        );
    }

    remove_empty_managed_directories(
        &root,
        &old_state.folder_paths,
        &state.folder_paths,
        &mut warnings,
    );
    let revision = if prepared_restore.is_some() {
        if let Err(error) = verify_save_consistency(&root, &consistency) {
            warnings.push(format!(
                "The note was restored, but the vault changed immediately afterward. Reload \
                 before editing again. {error}"
            ));
        }
        let revision = match revision_for_root(&root) {
            Ok(revision) => revision,
            Err(error) => {
                warnings.push(format!(
                    "The note was restored, but the new vault revision could not be read. Reload \
                     before editing again. {error}"
                ));
                expected_revision
            }
        };
        if let Err(error) = verify_save_consistency(&root, &consistency) {
            warnings.push(format!(
                "The note was restored, but the vault no longer matches the committed restore. \
                 Reload before editing again. {error}"
            ));
        }
        match revision_for_root(&root) {
            Ok(current_revision) if current_revision != revision => warnings.push(
                "The note was restored, but the vault changed immediately afterward. Reload \
                 before editing again."
                    .to_owned(),
            ),
            Ok(_) => {}
            Err(error) => warnings.push(format!(
                "The note was restored, but its revision could not be confirmed. Reload before \
                 editing again. {error}"
            )),
        }
        revision
    } else {
        verify_save_consistency(&root, &consistency)?;
        let revision = revision_for_root(&root)?;
        verify_save_consistency(&root, &consistency)?;
        if revision_for_root(&root)? != revision {
            return Err(
                "The vault changed immediately after saving. Reload it before editing again."
                    .to_owned(),
            );
        }
        revision
    };
    let deleted_note = prepared_archive.map(|archive| archive.deleted_note);
    Ok((
        SaveResult {
            note_paths: state.note_paths.clone(),
            revision,
            saved_at,
            warnings: warnings.finish(),
        },
        deleted_note,
        prepared_restore,
    ))
}

pub(super) fn build_note_write_plans(
    root: &Path,
    vault: &VaultData,
    old_state: &WorkspaceState,
    folder_paths: &BTreeMap<String, String>,
    preferred_new_paths: &BTreeMap<String, String>,
) -> Result<Vec<NoteWritePlan>, String> {
    if vault.notes.len() > MAX_NOTES {
        return Err(format!(
            "A vault can contain at most {MAX_NOTES} Markdown notes."
        ));
    }
    let mut plans = Vec::with_capacity(vault.notes.len());
    let mut desired_paths = HashSet::new();
    let mut note_ids = HashSet::new();
    let mut total_note_bytes = 0_u64;

    for note in &vault.notes {
        if note.id.trim().is_empty() || !note_ids.insert(note.id.clone()) {
            return Err("Every note must have a unique, non-empty ID.".to_owned());
        }
        if note.content.len() as u64 > MAX_NOTE_BYTES {
            return Err(format!(
                "The note {:?} is larger than {} MiB.",
                note.title,
                MAX_NOTE_BYTES / 1024 / 1024
            ));
        }
        let folder_path = match note.folder_id.as_deref() {
            Some(folder_id) => folder_paths.get(folder_id).ok_or_else(|| {
                format!(
                    "The note {:?} refers to a folder that does not exist.",
                    note.title
                )
            })?,
            None => "",
        };
        let old_relative_path = old_state.note_paths.get(&note.id).cloned();
        let extension = old_relative_path
            .as_deref()
            .and_then(|path| Path::new(path).extension())
            .and_then(|value| value.to_str())
            .filter(|value| value.eq_ignore_ascii_case("markdown"))
            .unwrap_or("md");
        let stem = safe_file_stem(&note.title, "Untitled note");

        let preserve_old_name = old_relative_path.as_deref().is_some_and(|old_path| {
            let path = Path::new(old_path);
            let old_folder = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .and_then(path_to_slash_string)
                .unwrap_or_default();
            let old_title = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            old_folder == folder_path && old_title == note.title
        });
        let preferred_new_path = old_relative_path
            .is_none()
            .then(|| preferred_new_paths.get(&note.id))
            .flatten();
        let preserved_modified_at = preferred_new_path.map(|_| note.updated_at);
        let new_relative_path = if let Some(preferred_path) = preferred_new_path {
            validate_markdown_relative_path(preferred_path)?;
            let preferred = Path::new(preferred_path);
            let preferred_folder = preferred
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .and_then(path_to_slash_string)
                .unwrap_or_default();
            let preferred_title = preferred
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if preferred_folder != folder_path || preferred_title != note.title {
                return Err(
                    "The restored note path does not match its title and folder.".to_owned(),
                );
            }
            preferred_path.clone()
        } else if preserve_old_name {
            old_relative_path
                .clone()
                .expect("preserved path should exist")
        } else if folder_path.is_empty() {
            format!("{stem}.{extension}")
        } else {
            format!("{folder_path}/{stem}.{extension}")
        };
        validate_markdown_relative_path(&new_relative_path)?;
        let portable_key = new_relative_path.to_lowercase();
        if !desired_paths.insert(portable_key) {
            return Err(format!(
                "More than one note would be saved as {new_relative_path}. Rename one of them first."
            ));
        }

        let old_content = match old_relative_path.as_deref() {
            Some(old_path) => {
                let path = resolve_workspace_file(root, old_path, true)?;
                match fs::read_to_string(&path) {
                    Ok(content) => Some(content),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    Err(error) => {
                        return Err(format!("Could not read {old_path} before saving: {error}"));
                    }
                }
            }
            None => None,
        };
        let content = content_with_requested_tags(note, old_content.as_deref())?;
        if content.len() as u64 > MAX_NOTE_BYTES {
            return Err(format!(
                "The note {:?} is too large after writing its frontmatter.",
                note.title
            ));
        }
        total_note_bytes = total_note_bytes.saturating_add(content.len() as u64);
        if total_note_bytes > MAX_TOTAL_NOTE_BYTES {
            return Err(format!(
                "The vault contains more than {} MiB of Markdown text.",
                MAX_TOTAL_NOTE_BYTES / 1024 / 1024
            ));
        }
        let needs_write = match old_relative_path.as_deref() {
            Some(old_path) if old_path == new_relative_path => old_content
                .as_deref()
                .is_none_or(|existing| existing.as_bytes() != content.as_bytes()),
            _ => true,
        };
        plans.push(NoteWritePlan {
            id: note.id.clone(),
            old_relative_path,
            new_relative_path,
            content,
            needs_write,
            preserved_modified_at,
        });
    }
    Ok(plans)
}

pub(super) fn build_folder_paths(folders: &[Folder]) -> Result<BTreeMap<String, String>, String> {
    let by_id: HashMap<&str, &Folder> = folders
        .iter()
        .map(|folder| (folder.id.as_str(), folder))
        .collect();
    if by_id.len() != folders.len() || by_id.contains_key("") {
        return Err("Every folder must have a unique, non-empty ID.".to_owned());
    }

    fn resolve(
        id: &str,
        by_id: &HashMap<&str, &Folder>,
        result: &mut BTreeMap<String, String>,
        visiting: &mut HashSet<String>,
    ) -> Result<String, String> {
        if let Some(path) = result.get(id) {
            return Ok(path.clone());
        }
        if !visiting.insert(id.to_owned()) {
            return Err("The folder tree contains a cycle.".to_owned());
        }
        let folder = by_id
            .get(id)
            .ok_or_else(|| "A folder refers to a parent that does not exist.".to_owned())?;
        validate_component_name(&folder.name, "folder")?;
        let path = match folder.parent_id.as_deref() {
            Some(parent_id) => format!(
                "{}/{}",
                resolve(parent_id, by_id, result, visiting)?,
                folder.name.trim()
            ),
            None => folder.name.trim().to_owned(),
        };
        validate_relative_path(&path, false)?;
        visiting.remove(id);
        result.insert(id.to_owned(), path.clone());
        Ok(path)
    }

    let mut result = BTreeMap::new();
    for folder in folders {
        resolve(&folder.id, &by_id, &mut result, &mut HashSet::new())?;
    }
    let mut portable_paths = HashSet::new();
    for path in result.values() {
        if !portable_paths.insert(path.to_lowercase()) {
            return Err(format!("More than one folder would be saved as {path}."));
        }
    }
    Ok(result)
}

pub(super) fn scan_workspace_files(
    root: &Path,
    warnings: &mut WarningCollector,
) -> Result<
    (
        Vec<ScannedNote>,
        Vec<ScannedFolder>,
        Vec<ScannedImage>,
        Vec<ScannedAttachment>,
    ),
    String,
> {
    let mut notes = Vec::new();
    let mut folders = Vec::new();
    let mut images = Vec::new();
    let mut attachments = Vec::new();
    let mut total_bytes = 0_u64;
    let walker = WalkDir::new(root)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
        .filter_entry(should_visit_workspace_entry);

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("Could not inspect a vault entry: {error}"));
                continue;
            }
        };
        if entry.depth() == 0 || entry.file_type().is_symlink() {
            continue;
        }
        let relative_path = match entry
            .path()
            .strip_prefix(root)
            .ok()
            .and_then(path_to_slash_string)
        {
            Some(path)
                if validate_relative_path(
                    &path,
                    entry.file_type().is_file() && is_markdown_path(entry.path()),
                )
                .is_ok() =>
            {
                path
            }
            _ => {
                warnings.push(format!(
                    "Skipped a vault entry with an unsupported path: {}",
                    entry.path().display()
                ));
                continue;
            }
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!("Could not inspect {relative_path}: {error}"));
                continue;
            }
        };
        if entry.file_type().is_dir() {
            folders.push(ScannedFolder {
                relative_path,
                created_at: metadata_time_millis(&metadata, true),
            });
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let markdown = is_markdown_path(entry.path());
        if !markdown
            && workspace_asset_limit_reached(images.len(), attachments.len(), MAX_VAULT_ASSETS)
        {
            warnings.push(format!(
                "Only the first {MAX_VAULT_ASSETS} asset files are shown in the vault."
            ));
            continue;
        }
        if is_supported_image_path(entry.path()) {
            if metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES {
                continue;
            }
            let Some(media_type) = Path::new(&relative_path)
                .extension()
                .and_then(|value| value.to_str())
                .and_then(image_media_type_for_extension)
            else {
                continue;
            };
            images.push(ScannedImage {
                relative_path,
                media_type: media_type.to_owned(),
            });
            continue;
        }
        if !markdown {
            if metadata.len() > MAX_ATTACHMENT_BYTES {
                continue;
            }
            attachments.push(ScannedAttachment {
                relative_path,
                media_type: attachment_media_type_for_path(entry.path()).to_owned(),
                byte_length: metadata.len(),
                opening_disabled: attachment_opening_is_disabled(entry.path()).unwrap_or(true),
            });
            continue;
        }
        if notes.len() >= MAX_NOTES {
            warnings.push(format!("Stopped after {MAX_NOTES} Markdown notes."));
            break;
        }
        if metadata.len() > MAX_NOTE_BYTES {
            warnings.push(format!(
                "Skipped {relative_path} because it is larger than {} MiB.",
                MAX_NOTE_BYTES / 1024 / 1024
            ));
            continue;
        }
        if total_bytes.saturating_add(metadata.len()) > MAX_TOTAL_NOTE_BYTES {
            warnings.push(format!(
                "Stopped after reading {} MiB of Markdown notes.",
                MAX_TOTAL_NOTE_BYTES / 1024 / 1024
            ));
            break;
        }
        let content = match fs::read_to_string(entry.path()) {
            Ok(content) => content,
            Err(error) => {
                warnings.push(format!(
                    "Skipped {relative_path} because it is not readable UTF-8: {error}"
                ));
                continue;
            }
        };
        total_bytes += metadata.len();
        let tags = parse_frontmatter_tags(&content);
        notes.push(ScannedNote {
            relative_path,
            content,
            created_at: metadata_time_millis(&metadata, true),
            updated_at: metadata_time_millis(&metadata, false),
            tags,
        });
    }

    notes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    folders.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    images.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    attachments.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok((notes, folders, images, attachments))
}

pub(super) fn read_workspace_state(
    root: &Path,
    warnings: &mut WarningCollector,
) -> (Option<WorkspaceState>, bool) {
    let path = workspace_state_path(root);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return (None, false),
        Err(error) => {
            warnings.push(format!("Could not inspect workspace metadata: {error}"));

            return (None, true);
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        warnings.push("Ignored workspace metadata because it is not a regular file.".to_owned());

        return (None, true);
    }
    if metadata.len() > 64 * 1024 * 1024 {
        warnings.push("Ignored workspace metadata because it is unexpectedly large.".to_owned());

        return (None, true);
    }
    match fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            serde_json::from_slice::<WorkspaceState>(&bytes).map_err(|error| error.to_string())
        }) {
        Ok(state) if state.version <= STATE_VERSION => (Some(state), true),
        Ok(state) => {
            warnings.push(format!(
                "Workspace metadata uses version {}, but this app supports up to version {STATE_VERSION}. It was opened read-only and was not changed.",
                state.version
            ));
            (None, true)
        }
        Err(error) => {
            warnings.push(format!("Ignored invalid workspace metadata: {error}"));
            (None, true)
        }
    }
}

pub(super) fn write_workspace_state(root: &Path, state: &WorkspaceState) -> Result<(), String> {
    let directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &directory)?;
    let mut bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("Could not encode workspace metadata: {error}"))?;
    bytes.push(b'\n');
    atomic_write(&directory.join(STATE_FILE), &bytes)
        .map_err(|error| format!("Could not write workspace metadata: {error}"))
}

pub(super) fn lock_workspace_files(root: &Path) -> Result<File, String> {
    let file = open_workspace_lock_file(root)?;
    file.lock()
        .map_err(|error| format!("Could not lock the vault: {error}"))?;

    Ok(file)
}

pub(super) fn open_workspace_lock_file(root: &Path) -> Result<File, String> {
    let directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &directory)?;
    let path = directory.join(WORKSPACE_LOCK_FILE);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("The workspace lock is not a regular file.".to_owned());
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not inspect the workspace lock: {error}")),
    }

    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|error| format!("Could not open the workspace lock: {error}"))
}

pub(super) fn lock_editor_positions(root: &Path) -> Result<File, String> {
    let directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &directory)?;
    let path = directory.join(EDITOR_POSITIONS_LOCK_FILE);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("The editor-position lock is not a regular file.".to_owned());
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Could not inspect the editor-position lock: {error}"
            ));
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|error| format!("Could not open the editor-position lock: {error}"))?;
    file.lock()
        .map_err(|error| format!("Could not lock editor positions: {error}"))?;

    Ok(file)
}
