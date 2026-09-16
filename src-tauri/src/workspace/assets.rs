use super::*;

mod embedding;
mod external_upload;
pub(crate) mod files;
mod imports;
mod relocation;

pub(in crate::workspace) use embedding::*;
pub(in crate::workspace) use external_upload::*;
pub(in crate::workspace) use files::*;
pub(in crate::workspace) use imports::*;
pub(in crate::workspace) use relocation::*;

pub(in crate::workspace) const MAX_VAULT_ASSETS: usize = 100_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(in crate::workspace) enum VaultAssetKind {
    Image,
    Attachment,
}

impl Default for VaultAssetKind {
    fn default() -> Self {
        Self::Image
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(in crate::workspace) struct StoredVaultAsset {
    #[serde(default)]
    pub(in crate::workspace) kind: VaultAssetKind,
    pub(in crate::workspace) relative_path: String,
    pub(in crate::workspace) media_type: String,
    pub(in crate::workspace) fingerprint: FileFingerprint,
    #[serde(default)]
    pub(in crate::workspace) modified_nanos: u64,
}

#[derive(Debug)]
pub(in crate::workspace) struct PreparedAssetNoteUpdate {
    path: PathBuf,
    expected_content: Vec<u8>,
    content: Vec<u8>,
}

pub(in crate::workspace) fn delete_workspace_asset(
    root: &Path,
    kind: ExternalFileUploadKind,
    relative_path: &str,
    asset_id: Option<&str>,
    note_updates: &[WorkspaceImageNoteUpdate],
    recovery_updates: &[WorkspaceImageNoteUpdate],
    expected_revision: u64,
) -> Result<SaveResult, String> {
    delete_workspace_asset_with_hook(
        root,
        kind,
        relative_path,
        asset_id,
        note_updates,
        recovery_updates,
        expected_revision,
        |_| Ok(()),
    )
}

pub(in crate::workspace) fn delete_workspace_asset_with_hook(
    root: &Path,
    kind: ExternalFileUploadKind,
    relative_path: &str,
    asset_id: Option<&str>,
    note_updates: &[WorkspaceImageNoteUpdate],
    recovery_updates: &[WorkspaceImageNoteUpdate],
    expected_revision: u64,
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
) -> Result<SaveResult, String> {
    let (item_kind, stored_kind) = match kind {
        ExternalFileUploadKind::Image => {
            validate_image_relative_path(relative_path)?;
            (WorkspaceVaultItemKind::Image, VaultAssetKind::Image)
        }
        ExternalFileUploadKind::Attachment => {
            validate_attachment_relative_path(relative_path)?;
            (
                WorkspaceVaultItemKind::Attachment,
                VaultAssetKind::Attachment,
            )
        }
    };
    let mut warnings = WarningCollector::default();
    let (stored_state, state_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_present {
        return Err("Files cannot be deleted while workspace metadata is unreadable or newer than this app.".to_owned());
    }
    let old_state = stored_state.unwrap_or_default();
    recover_workspace_transactions(root, Some(&old_state), &mut warnings)?;
    let baseline = revision_entries_for_root(root)?;
    if revision_for_entries(&baseline) != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before deleting the file."
                .to_owned(),
        );
    }
    let (resolved_path, source) =
        locate_workspace_vault_item(root, item_kind, relative_path, asset_id)?;
    if resolved_path != relative_path {
        return Err("The file moved. Reload the vault before deleting it.".to_owned());
    }
    let fingerprint = fingerprint_attachment_file(&source)?;
    let mut prepared = prepare_asset_note_updates(root, &old_state, note_updates, "file deletion")?;
    let mut next_state = old_state.clone();
    let mut recovery_records = Vec::new();
    if recovery_updates.len() > MAX_RECENTLY_DELETED_NOTES {
        return Err("Too many recovery snapshots were included in the deletion.".to_owned());
    }
    // Refuse to discard files if a recovery snapshot could not be inspected.
    for (id, entry) in &old_state.recently_deleted_notes {
        read_indexed_recently_deleted_note(root, id, entry)?;
    }
    let mut recovery_ids = HashSet::new();
    for update in recovery_updates {
        if !recovery_ids.insert(&update.note_id) {
            return Err("The deletion contains a duplicate recovery update.".to_owned());
        }
        let stored = old_state
            .recently_deleted_notes
            .get(&update.note_id)
            .ok_or_else(|| {
                "A note in Recently Deleted is no longer available. Reload the vault.".to_owned()
            })?;
        let mut deleted_note = read_indexed_recently_deleted_note(root, &update.note_id, stored)?;
        if deleted_note.note.relative_path != update.relative_path
            || deleted_note.note.content != update.expected_content
        {
            return Err(
                "A note in Recently Deleted changed before its reference could be updated."
                    .to_owned(),
            );
        }
        if update.content.len() as u64 > MAX_NOTE_BYTES {
            return Err("An updated recovery note exceeds the note size limit.".to_owned());
        }
        deleted_note.note.content = update.content.clone();
        deleted_note.editor_position = None;
        let content = serde_json::to_vec(&RecentlyDeletedSnapshot {
            version: RECENTLY_DELETED_SNAPSHOT_VERSION,
            deleted_note,
        })
        .map_err(|error| format!("Could not prepare updated recovery references: {error}"))?;
        if content.len() as u64 > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
            return Err("An updated recovery snapshot exceeds its size limit.".to_owned());
        }
        let path = recently_deleted_snapshot_path(root, &update.note_id)?;
        let expected_content = fs::read(&path)
            .map_err(|error| format!("Could not back up a recovery snapshot: {error}"))?;
        if fingerprint_bytes(&expected_content) != stored.fingerprint {
            return Err(
                "A recovery snapshot changed while deletion was being prepared.".to_owned(),
            );
        }
        let updated_fingerprint = fingerprint_bytes(&content);
        next_state
            .recently_deleted_notes
            .get_mut(&update.note_id)
            .unwrap()
            .fingerprint = updated_fingerprint.clone();
        recovery_records.push(AssetDeletionRecoveryUpdate {
            id: update.note_id.clone(),
            original_fingerprint: stored.fingerprint.clone(),
            fingerprint: updated_fingerprint,
        });
        prepared.push(PreparedAssetNoteUpdate {
            path,
            expected_content,
            content,
        });
    }
    validate_recently_deleted_capacity(&next_state.recently_deleted_notes, 0)?;
    let plans = note_updates
        .iter()
        .map(|update| NoteWritePlan {
            id: update.note_id.clone(),
            old_relative_path: Some(update.relative_path.clone()),
            new_relative_path: update.relative_path.clone(),
            content: update.content.clone(),
            needs_write: true,
            preserved_modified_at: None,
        })
        .collect::<Vec<_>>();
    let paths = note_updates
        .iter()
        .map(|update| update.relative_path.clone())
        .collect();
    let (transaction_root, mut manifest) = prepare_transaction(
        root,
        new_transaction_id(),
        &paths,
        &plans,
        &[],
        Vec::new(),
        Vec::new(),
    )?;
    let preparation = (|| {
        let copied =
            copy_attachment_file_durable(&source, &transaction_root.join("deleted-asset"))?;
        if copied != fingerprint {
            return Err(
                "The file changed while its deletion backup was being prepared.".to_owned(),
            );
        }
        for (record, update) in recovery_records
            .iter()
            .zip(prepared.iter().skip(note_updates.len()))
        {
            let backup = asset_deletion_recovery_backup(&transaction_root, &record.id)?;
            ensure_private_directory_tree(&transaction_root, backup.parent().unwrap())
                .map_err(|error| format!("Could not prepare a recovery backup: {error}"))?;
            atomic_write(&backup, &update.expected_content)
                .map_err(|error| format!("Could not back up a recovery snapshot: {error}"))?;
        }
        manifest.asset_deletion = Some(AssetDeletionTransaction {
            kind: stored_kind,
            relative_path: relative_path.to_owned(),
            fingerprint: fingerprint.clone(),
            recovery_updates: recovery_records,
        });
        write_transaction_manifest(&transaction_root, &manifest)?;
        if revision_entries_for_root(root)? != baseline {
            return Err(
                "The vault changed while deletion was being prepared. Reload it and try again."
                    .to_owned(),
            );
        }
        Ok(())
    })();
    if let Err(error) = preparation {
        discard_private_transaction(root, &transaction_root, &mut warnings);
        return Err(error);
    }
    manifest.phase = TransactionPhase::Applying;
    write_transaction_manifest(&transaction_root, &manifest)?;
    next_state
        .assets
        .retain(|_, stored| stored.kind != stored_kind || stored.relative_path != relative_path);
    next_state.version = STATE_VERSION;
    next_state.last_committed_transaction_id = Some(manifest.id.clone());
    let next_bytes = workspace_state_bytes(&next_state)?;
    let next_fingerprint = fingerprint_bytes(&next_bytes);
    let mutation = (|| {
        checkpoint("prepared")?;
        if revision_entries_for_root(root)? != baseline {
            return Err(
                "The vault changed before deletion began. Reload it and try again.".to_owned(),
            );
        }
        for update in &prepared {
            atomic_write_with_precondition(&update.path, &update.content, || {
                validate_asset_reference_parent(root, &update.path).map_err(io::Error::other)?;
                if fs::read(&update.path)? != update.expected_content {
                    return Err(io::Error::other(
                        "A note changed before its reference could be updated.",
                    ));
                }
                Ok(())
            })
            .map_err(|error| format!("Could not mark a deleted file reference: {error}"))?;
        }
        checkpoint("references-written")?;
        ensure_existing_directory_without_symlink(root, source.parent().unwrap())?;
        if fingerprint_attachment_file(&source)? != fingerprint {
            return Err("The file changed while deletion was being saved.".to_owned());
        }
        remove_file_durable(&source)
            .map_err(|error| format!("Could not delete the file: {error}"))?;
        checkpoint("asset-removed")?;
        for update in &prepared {
            if fs::read(&update.path)
                .map_err(|error| format!("Could not verify a reference: {error}"))?
                != update.content
            {
                return Err("A reference changed while deletion was being saved.".to_owned());
            }
        }
        for (id, entry) in &next_state.recently_deleted_notes {
            read_indexed_recently_deleted_note(root, id, entry)?;
        }
        let written_entries = revision_entries_for_root(root)?;
        let changed_paths = note_updates
            .iter()
            .map(|update| format!("F:{}", update.relative_path))
            .chain(std::iter::once(format!("F:{relative_path}")))
            .collect::<HashSet<_>>();
        if !baseline
            .iter()
            .filter(|(path, _)| !changed_paths.contains(path))
            .eq(written_entries
                .iter()
                .filter(|(path, _)| !changed_paths.contains(path)))
            || written_entries
                .iter()
                .any(|(path, _)| path == &format!("F:{relative_path}"))
        {
            return Err(
                "The vault changed while deletion was being saved. Reload it and try again."
                    .to_owned(),
            );
        }
        if let Err(error) = write_loaded_workspace_state_bytes(
            root,
            &next_bytes,
            workspace_state_revision_fingerprint(&baseline).as_ref(),
        ) {
            // A directory-sync failure can follow a successful metadata replacement.
            if fingerprint_regular_file(&workspace_state_path(root))?
                != Some(next_fingerprint.clone())
            {
                return Err(error);
            }
            warnings.push(format!(
                "The deletion was saved, but its metadata could not be synchronized: {error}"
            ));
        }
        Ok(written_entries)
    })();
    let written_entries = match mutation {
        Ok(entries) => entries,
        Err(error) => {
            let restored = rollback_transaction(root, &transaction_root, &manifest, &mut warnings);
            if restored {
                discard_private_transaction(root, &transaction_root, &mut warnings);
            }
            return Err(format!(
                "{error} {}",
                if restored {
                    "The deletion was rolled back. Reload the vault before trying again.".to_owned()
                } else {
                    format!("The deletion could not be fully rolled back. Backups remain at {}. Reload the vault. {}",
                    transaction_root.display(), warnings.finish().join(" "))
                }
            ));
        }
    };
    // Metadata is the commit point. Later failures must never undo committed data.
    if let Err(error) = checkpoint("committed") {
        warnings.push(error);
    }
    manifest.phase = TransactionPhase::Committed;
    if let Err(error) = write_transaction_manifest(&transaction_root, &manifest) {
        warnings.push(format!(
            "Deletion cleanup will be retried when the vault reopens: {error}"
        ));
    } else {
        discard_private_transaction(root, &transaction_root, &mut warnings);
    }
    let revision = match verify_workspace_load_revision(
        root,
        &written_entries,
        Some(&next_fingerprint),
    ) {
        Ok(revision) => revision,
        Err(error) => {
            warnings.push(format!("The file was deleted, but the vault must be reloaded before further edits. {error}"));
            expected_revision
        }
    };
    Ok(SaveResult {
        note_paths: next_state.note_paths,
        revision,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

fn asset_deletion_recovery_backup(transaction_root: &Path, id: &str) -> Result<PathBuf, String> {
    validate_recently_deleted_id(id)?;
    Ok(transaction_root
        .join("recovery-backups")
        .join(format!("{id}.json")))
}

fn validate_asset_reference_parent(root: &Path, path: &Path) -> Result<(), String> {
    if path.parent()
        == Some(
            root.join(STATE_DIRECTORY)
                .join(RECENTLY_DELETED_DIRECTORY)
                .as_path(),
        )
    {
        inspect_recently_deleted_directory(root)?;
        Ok(())
    } else {
        ensure_existing_directory_without_symlink(root, path.parent().unwrap())
    }
}

pub(in crate::workspace) fn rollback_asset_deletion(
    root: &Path,
    transaction_root: &Path,
    deletion: &AssetDeletionTransaction,
    warnings: &mut WarningCollector,
) -> bool {
    let mut restored = true;
    let result = (|| {
        let path = match deletion.kind {
            VaultAssetKind::Image => {
                resolve_workspace_image_file(root, &deletion.relative_path, true)?
            }
            VaultAssetKind::Attachment => {
                resolve_workspace_asset_file(root, &deletion.relative_path, true)?
            }
        };
        match fingerprint_regular_file(&path)? {
            Some(current) if current == deletion.fingerprint => return Ok(()),
            Some(_) => {
                return Err(
                    "The deleted file's path now contains another file; it was retained."
                        .to_owned(),
                )
            }
            None => {}
        }
        let backup = transaction_root.join("deleted-asset");
        if fingerprint_regular_file(&backup)? != Some(deletion.fingerprint.clone()) {
            return Err("The deleted file's backup failed its integrity check.".to_owned());
        }
        ensure_existing_directory_without_symlink(root, path.parent().unwrap())?;
        if copy_attachment_file_durable(&backup, &path)? != deletion.fingerprint {
            return Err("The restored file failed its integrity check.".to_owned());
        }
        Ok(())
    })();
    if let Err(error) = result {
        restored = false;
        warnings.push(error);
    }
    for update in &deletion.recovery_updates {
        let result = (|| {
            if update.original_fingerprint.length > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
                return Err("A recovery backup is unexpectedly large.".to_owned());
            }
            let path = recently_deleted_snapshot_path(root, &update.id)?;
            let current = fingerprint_regular_file(&path)?;
            if current.as_ref() == Some(&update.original_fingerprint) {
                return Ok(());
            }
            if current.is_some() && current.as_ref() != Some(&update.fingerprint) {
                return Err("A recovery snapshot changed after the deletion; the changed snapshot was retained.".to_owned());
            }
            let backup = asset_deletion_recovery_backup(transaction_root, &update.id)?;
            if fingerprint_regular_file(&backup)? != Some(update.original_fingerprint.clone()) {
                return Err("A recovery backup failed its integrity check.".to_owned());
            }
            let bytes = fs::read(&backup)
                .map_err(|error| format!("Could not read a recovery backup: {error}"))?;
            atomic_write_with_precondition(&path, &bytes, || {
                validate_asset_reference_parent(root, &path).map_err(io::Error::other)?;
                if fingerprint_regular_file(&path).map_err(io::Error::other)? != current {
                    return Err(io::Error::other(
                        "The recovery snapshot changed before rollback.",
                    ));
                }
                Ok(())
            })
            .map_err(|error| format!("Could not restore a recovery snapshot: {error}"))
        })();
        if let Err(error) = result {
            restored = false;
            warnings.push(error);
        }
    }
    restored
}

pub(in crate::workspace) fn prepare_asset_note_updates(
    root: &Path,
    state: &WorkspaceState,
    note_updates: &[WorkspaceImageNoteUpdate],
    asset_label: &str,
) -> Result<Vec<PreparedAssetNoteUpdate>, String> {
    if note_updates.len() > MAX_NOTES {
        return Err(format!("Only {MAX_NOTES} notes can be updated at once."));
    }
    let mut seen_note_ids = HashSet::new();
    let mut seen_paths = HashSet::new();
    let mut total_bytes = 0_u64;
    let mut prepared = Vec::with_capacity(note_updates.len());
    for update in note_updates {
        validate_markdown_relative_path(&update.relative_path)?;
        if state.note_paths.get(&update.note_id).map(String::as_str)
            != Some(update.relative_path.as_str())
        {
            return Err(format!(
                "A note path changed before its {asset_label} reference could be updated."
            ));
        }
        if !seen_note_ids.insert(update.note_id.as_str())
            || !seen_paths.insert(portable_path_key(&update.relative_path))
        {
            return Err(format!(
                "The {asset_label} move contains a duplicate note update."
            ));
        }
        if update.content.len() as u64 > MAX_NOTE_BYTES
            || update.expected_content.len() as u64 > MAX_NOTE_BYTES
        {
            return Err(format!(
                "{} is larger than {} MiB and cannot be updated.",
                update.relative_path,
                MAX_NOTE_BYTES / 1024 / 1024,
            ));
        }
        total_bytes = total_bytes.saturating_add(update.content.len() as u64);
        if total_bytes > MAX_TOTAL_NOTE_BYTES {
            return Err(format!(
                "The {asset_label} move would update too much note content at once."
            ));
        }
        let path = resolve_workspace_file(root, &update.relative_path, false)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Could not inspect {}: {error}", update.relative_path))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "{} is not a regular Markdown file.",
                update.relative_path
            ));
        }
        let current = fs::read(&path)
            .map_err(|error| format!("Could not read {}: {error}", update.relative_path))?;
        if current != update.expected_content.as_bytes() {
            return Err(format!(
                "{} changed before its {asset_label} reference could be updated.",
                update.relative_path,
            ));
        }
        prepared.push(PreparedAssetNoteUpdate {
            path,
            expected_content: current,
            content: update.content.as_bytes().to_vec(),
        });
    }
    Ok(prepared)
}

pub(in crate::workspace) fn read_workspace_image(
    root: &Path,
    asset_id: Option<&str>,
    note_relative_path: &str,
    destination: &str,
) -> Result<Vec<u8>, String> {
    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present && asset_id.is_some() {
        return Err("Workspace metadata is unreadable or newer than this app.".to_owned());
    }
    // Reading an explicit Markdown path needs no metadata write or stable-ID
    // lookup. Keep those images visible in a read-only vault as well.
    let mut state = stored_state.unwrap_or_default();
    let valid_asset_id = asset_id.filter(|id| is_valid_asset_id(id));
    let tracked_asset_id = valid_asset_id.filter(|id| {
        state
            .assets
            .get(*id)
            .is_some_and(|asset| asset.kind == VaultAssetKind::Image)
    });
    if let Some(relative_path) = tracked_asset_id
        .and_then(|id| state.assets.get(id))
        .map(|asset| asset.relative_path.as_str())
    {
        if let Ok(bytes) = read_relative_workspace_image(root, relative_path) {
            return Ok(bytes);
        }
    }
    if tracked_asset_id.is_some() {
        let _ = reconcile_image_assets(root, &mut state.assets, &mut warnings);
        if let Some(relative_path) = tracked_asset_id
            .and_then(|id| state.assets.get(id))
            .map(|asset| asset.relative_path.as_str())
        {
            if let Ok(bytes) = read_relative_workspace_image(root, relative_path) {
                return Ok(bytes);
            }
        }
    }

    let relative_path = resolve_markdown_image_path(note_relative_path, destination)?;
    read_relative_workspace_image(root, &relative_path)
}

pub(in crate::workspace) fn read_relative_workspace_image(
    root: &Path,
    relative_path: &str,
) -> Result<Vec<u8>, String> {
    let path = resolve_workspace_image_file(root, relative_path, false)?;
    let bytes = read_image_file(&path)?;
    validate_image_bytes_impl(&bytes, Some(relative_path))?;
    Ok(bytes)
}

pub(in crate::workspace) fn reconcile_image_assets(
    root: &Path,
    assets: &mut BTreeMap<String, StoredVaultAsset>,
    warnings: &mut WarningCollector,
) -> Vec<EmbeddedImage> {
    assets.retain(|_, asset| !is_finder_metadata_path(Path::new(&asset.relative_path)));
    if assets.len() > MAX_VAULT_ASSETS {
        warnings.push(format!(
            "Only the first {MAX_VAULT_ASSETS} embedded image records were loaded."
        ));
        let retained = assets
            .keys()
            .take(MAX_VAULT_ASSETS)
            .cloned()
            .collect::<HashSet<_>>();
        assets.retain(|id, _| retained.contains(id));
    }
    let invalid_ids = assets
        .iter()
        .filter_map(|(id, asset)| {
            let path_is_invalid = match asset.kind {
                VaultAssetKind::Image => {
                    validate_image_relative_path(&asset.relative_path).is_err()
                }
                VaultAssetKind::Attachment => {
                    validate_attachment_relative_path(&asset.relative_path).is_err()
                }
            };
            (!is_valid_asset_id(id) || path_is_invalid).then(|| id.clone())
        })
        .collect::<Vec<_>>();
    for id in invalid_ids {
        assets.remove(&id);
        warnings.push("Ignored an invalid embedded image record.".to_owned());
    }

    let mut assigned_paths = HashSet::new();
    let mut missing_ids = Vec::new();
    for (id, asset) in assets
        .iter_mut()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Image)
    {
        let path = match resolve_workspace_image_file(root, &asset.relative_path, false) {
            Ok(path) => path,
            Err(_) => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => metadata,
            _ => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let expected_media_type = Path::new(&asset.relative_path)
            .extension()
            .and_then(|value| value.to_str())
            .and_then(image_media_type_for_extension);
        let modified_nanos = image_modified_nanos(&metadata);
        if asset.modified_nanos != 0
            && asset.modified_nanos == modified_nanos
            && asset.fingerprint.length == metadata.len()
            && expected_media_type == Some(asset.media_type.as_str())
        {
            assigned_paths.insert(portable_path_key(&asset.relative_path));
            continue;
        }
        match read_image_file(&path) {
            Ok(bytes) => match validate_image_bytes_impl(&bytes, Some(&asset.relative_path)) {
                Ok((media_type, _)) => {
                    asset.media_type = media_type.to_owned();
                    asset.fingerprint = fingerprint_bytes(&bytes);
                    asset.modified_nanos = modified_nanos;
                    assigned_paths.insert(portable_path_key(&asset.relative_path));
                }
                Err(_) => missing_ids.push(id.clone()),
            },
            Err(_) => missing_ids.push(id.clone()),
        }
    }

    if !missing_ids.is_empty() {
        let missing_lengths = missing_ids
            .iter()
            .filter_map(|id| assets.get(id).map(|asset| asset.fingerprint.length))
            .collect::<HashSet<_>>();
        let mut candidates: HashMap<FileFingerprint, Vec<(String, String, u64)>> = HashMap::new();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(128)
            .into_iter()
            .filter_entry(should_visit_workspace_entry)
            .filter_map(Result::ok)
        {
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let Some(relative_path) = entry
                .path()
                .strip_prefix(root)
                .ok()
                .and_then(path_to_slash_string)
            else {
                continue;
            };
            if assigned_paths.contains(&portable_path_key(&relative_path))
                || validate_image_relative_path(&relative_path).is_err()
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !missing_lengths.contains(&metadata.len()) {
                continue;
            }
            let Ok(bytes) = read_image_file(entry.path()) else {
                continue;
            };
            let Ok((media_type, _)) = validate_image_bytes_impl(&bytes, Some(&relative_path))
            else {
                continue;
            };
            let modified_nanos = image_modified_nanos(&metadata);
            candidates
                .entry(fingerprint_bytes(&bytes))
                .or_default()
                .push((relative_path, media_type.to_owned(), modified_nanos));
        }

        for id in missing_ids {
            let Some(asset) = assets.get_mut(&id) else {
                continue;
            };
            let Some(matches) = candidates.get(&asset.fingerprint) else {
                warnings.push(format!(
                    "Could not find the embedded image {}.",
                    asset.relative_path
                ));
                continue;
            };
            if matches.len() != 1 {
                warnings.push(format!(
                    "Could not uniquely locate the moved embedded image {}.",
                    asset.relative_path,
                ));
                continue;
            }
            let (relative_path, media_type, modified_nanos) = matches[0].clone();
            asset.relative_path = relative_path.clone();
            asset.media_type = media_type;
            asset.modified_nanos = modified_nanos;
            assigned_paths.insert(portable_path_key(&relative_path));
            candidates.remove(&asset.fingerprint);
        }
    }

    assets
        .iter()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Image)
        .map(|(id, asset)| EmbeddedImage {
            id: id.clone(),
            relative_path: asset.relative_path.clone(),
            media_type: asset.media_type.clone(),
        })
        .collect()
}

pub(in crate::workspace) fn reconcile_attachment_assets(
    root: &Path,
    assets: &mut BTreeMap<String, StoredVaultAsset>,
    warnings: &mut WarningCollector,
) -> Vec<EmbeddedAttachment> {
    let mut assigned_paths = HashSet::new();
    let mut missing_ids = Vec::new();
    for (id, asset) in assets
        .iter_mut()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Attachment)
    {
        let path = match resolve_workspace_asset_file(root, &asset.relative_path, false) {
            Ok(path) => path,
            Err(_) => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata)
                if !metadata.file_type().is_symlink()
                    && metadata.is_file()
                    && metadata.len() <= MAX_ATTACHMENT_BYTES =>
            {
                metadata
            }
            _ => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let media_type = attachment_media_type_for_path(Path::new(&asset.relative_path));
        let modified_nanos = image_modified_nanos(&metadata);
        if asset.modified_nanos != 0
            && asset.modified_nanos == modified_nanos
            && asset.fingerprint.length == metadata.len()
            && asset.media_type == media_type
        {
            assigned_paths.insert(portable_path_key(&asset.relative_path));
            continue;
        }
        match fingerprint_attachment_file(&path) {
            Ok(fingerprint) => {
                asset.media_type = media_type.to_owned();
                asset.fingerprint = fingerprint;
                asset.modified_nanos = modified_nanos;
                assigned_paths.insert(portable_path_key(&asset.relative_path));
            }
            Err(_) => missing_ids.push(id.clone()),
        }
    }

    if !missing_ids.is_empty() {
        let missing_lengths = missing_ids
            .iter()
            .filter_map(|id| assets.get(id).map(|asset| asset.fingerprint.length))
            .collect::<HashSet<_>>();
        let mut candidates: HashMap<FileFingerprint, Vec<(String, String, u64)>> = HashMap::new();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(128)
            .into_iter()
            .filter_entry(should_visit_workspace_entry)
            .filter_map(Result::ok)
        {
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let Some(relative_path) = entry
                .path()
                .strip_prefix(root)
                .ok()
                .and_then(path_to_slash_string)
            else {
                continue;
            };
            if assigned_paths.contains(&portable_path_key(&relative_path))
                || validate_attachment_relative_path(&relative_path).is_err()
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !missing_lengths.contains(&metadata.len()) {
                continue;
            }
            let Ok(fingerprint) = fingerprint_attachment_file(entry.path()) else {
                continue;
            };
            let modified_nanos = image_modified_nanos(&metadata);
            candidates.entry(fingerprint).or_default().push((
                relative_path.clone(),
                attachment_media_type_for_path(Path::new(&relative_path)).to_owned(),
                modified_nanos,
            ));
        }

        for id in missing_ids {
            let Some(asset) = assets.get_mut(&id) else {
                continue;
            };
            let Some(matches) = candidates.get(&asset.fingerprint) else {
                warnings.push(format!(
                    "Could not find the embedded attachment {}.",
                    asset.relative_path,
                ));
                continue;
            };
            if matches.len() != 1 {
                warnings.push(format!(
                    "Could not uniquely locate the moved embedded attachment {}.",
                    asset.relative_path,
                ));
                continue;
            }
            let (relative_path, media_type, modified_nanos) = matches[0].clone();
            asset.relative_path = relative_path.clone();
            asset.media_type = media_type;
            asset.modified_nanos = modified_nanos;
            assigned_paths.insert(portable_path_key(&relative_path));
            candidates.remove(&asset.fingerprint);
        }
    }

    assets
        .iter()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Attachment)
        .map(|(id, asset)| EmbeddedAttachment {
            id: id.clone(),
            relative_path: asset.relative_path.clone(),
            media_type: asset.media_type.clone(),
            byte_length: asset.fingerprint.length,
            opening_disabled: attachment_opening_is_disabled(
                &root.join(Path::new(&asset.relative_path)),
            )
            .unwrap_or(true),
        })
        .collect()
}

pub(in crate::workspace) fn workspace_asset_limit_reached(
    image_count: usize,
    attachment_count: usize,
    limit: usize,
) -> bool {
    image_count.saturating_add(attachment_count) >= limit
}
