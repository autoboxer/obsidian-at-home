use super::*;

mod external_upload;
pub(crate) mod files;
mod imports;

pub(in crate::workspace) use external_upload::*;
pub(in crate::workspace) use files::*;
pub(in crate::workspace) use imports::*;

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

pub(in crate::workspace) fn normalize_image_embed_settings(
    settings: &ImageEmbedSettings,
) -> Result<ImageEmbedSettings, String> {
    match settings.location {
        ImageEmbedLocation::VaultRoot => Ok(ImageEmbedSettings::default()),
        ImageEmbedLocation::NoteFolder => Ok(ImageEmbedSettings {
            location: ImageEmbedLocation::NoteFolder,
            folder_path: String::new(),
        }),
        ImageEmbedLocation::SpecifiedFolder => {
            let folder_path = settings.folder_path.trim().trim_matches('/').to_owned();
            if folder_path.is_empty() {
                return Err("Choose a vault-relative folder for embedded images.".to_owned());
            }
            validate_relative_path(&folder_path, false)?;
            Ok(ImageEmbedSettings {
                location: ImageEmbedLocation::SpecifiedFolder,
                folder_path,
            })
        }
    }
}

pub(in crate::workspace) fn image_destination_folder(
    note_relative_path: &str,
    settings: &ImageEmbedSettings,
) -> Result<String, String> {
    validate_markdown_relative_path(note_relative_path)?;
    let settings = normalize_image_embed_settings(settings)?;
    match settings.location {
        ImageEmbedLocation::VaultRoot => Ok(String::new()),
        ImageEmbedLocation::NoteFolder => Ok(Path::new(note_relative_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .and_then(path_to_slash_string)
            .unwrap_or_default()),
        ImageEmbedLocation::SpecifiedFolder => Ok(settings.folder_path),
    }
}

pub(in crate::workspace) fn normalize_attachment_embed_settings(
    settings: &AttachmentEmbedSettings,
) -> Result<AttachmentEmbedSettings, String> {
    match settings.location {
        ImageEmbedLocation::VaultRoot => Ok(AttachmentEmbedSettings::default()),
        ImageEmbedLocation::NoteFolder => Ok(AttachmentEmbedSettings {
            location: ImageEmbedLocation::NoteFolder,
            folder_path: String::new(),
        }),
        ImageEmbedLocation::SpecifiedFolder => {
            let folder_path = settings.folder_path.trim().trim_matches('/').to_owned();
            if folder_path.is_empty() {
                return Err("Choose a vault-relative folder for embedded files.".to_owned());
            }
            validate_relative_path(&folder_path, false)?;
            Ok(AttachmentEmbedSettings {
                location: ImageEmbedLocation::SpecifiedFolder,
                folder_path,
            })
        }
    }
}

pub(in crate::workspace) fn attachment_destination_folder(
    note_relative_path: &str,
    settings: &AttachmentEmbedSettings,
) -> Result<String, String> {
    validate_markdown_relative_path(note_relative_path)?;
    let settings = normalize_attachment_embed_settings(settings)?;
    match settings.location {
        ImageEmbedLocation::VaultRoot => Ok(String::new()),
        ImageEmbedLocation::NoteFolder => Ok(Path::new(note_relative_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .and_then(path_to_slash_string)
            .unwrap_or_default()),
        ImageEmbedLocation::SpecifiedFolder => Ok(settings.folder_path),
    }
}

pub(in crate::workspace) fn embed_workspace_image(
    root: &Path,
    note_relative_path: &str,
    settings: ImageEmbedSettings,
    file_name: &str,
    bytes: &[u8],
    existing_relative_path: Option<&str>,
    expected_revision: u64,
) -> Result<WorkspaceEmbedImageResult, String> {
    let settings = normalize_image_embed_settings(&settings)?;
    let destination_folder = image_destination_folder(note_relative_path, &settings)?;
    let (media_type, extension) = validate_image_bytes_impl(bytes, Some(file_name))?;
    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err(
            "Embedded images cannot be changed while workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    recover_workspace_transactions(root, stored_state.as_ref(), &mut warnings)?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before embedding the image."
                .to_owned(),
        );
    }

    let mut state = stored_state.unwrap_or_default();
    let existing_relative_path = existing_relative_path
        .map(str::to_owned)
        .filter(|path| validate_image_relative_path(path).is_ok());
    let fingerprint = fingerprint_bytes(bytes);

    if let Some(relative_path) = existing_relative_path.as_deref() {
        if let Some((id, stored)) = state.assets.iter_mut().find(|(_, stored)| {
            stored.kind == VaultAssetKind::Image
                && portable_path_key(&stored.relative_path) == portable_path_key(relative_path)
        }) {
            if stored.relative_path != relative_path {
                let old_path = resolve_workspace_image_file(root, &stored.relative_path, true)?;
                if old_path.exists() {
                    return Err(format!(
                        "The vault contains image paths that differ only by letter case near {relative_path}."
                    ));
                }
                stored.relative_path = relative_path.to_owned();
            }
            stored.media_type = media_type.to_owned();
            stored.fingerprint = fingerprint;
            stored.modified_nanos = image_modified_nanos_for_path(root, relative_path)?;
            let id = id.clone();
            state.version = STATE_VERSION;
            state.image_embed_settings = settings;
            write_workspace_state(root, &state)?;

            return Ok(WorkspaceEmbedImageResult {
                image: EmbeddedImage {
                    id,
                    relative_path: relative_path.to_owned(),
                    media_type: media_type.to_owned(),
                },
                revision: revision_for_root(root)?,
                saved_at: now_millis(),
                warnings: warnings.finish(),
            });
        }
    }

    if state.assets.len() >= MAX_VAULT_ASSETS {
        return Err(format!(
            "This vault already tracks the maximum of {MAX_VAULT_ASSETS} embedded images."
        ));
    }

    let mut wrote_image = false;
    let relative_path = if let Some(relative_path) = existing_relative_path {
        relative_path
    } else {
        if !destination_folder.is_empty() {
            ensure_directory_path(root, &destination_folder)?;
        }
        let safe_name = safe_image_file_name(file_name, extension);
        let relative_path = unique_image_relative_path(root, &destination_folder, &safe_name)?;
        let target = resolve_workspace_image_file(root, &relative_path, true)?;
        atomic_write(&target, bytes)
            .map_err(|error| format!("Could not save the embedded image: {error}"))?;
        wrote_image = true;
        relative_path
    };

    let mut used_ids = state.assets.keys().cloned().collect::<HashSet<_>>();
    let id_seed = format!(
        "{relative_path}:{}:{}:{}",
        fingerprint.length,
        fingerprint.hash,
        now_millis(),
    );
    let id = fresh_id("image", &id_seed, &mut used_ids);
    let modified_nanos = match image_modified_nanos_for_path(root, &relative_path) {
        Ok(modified_nanos) => modified_nanos,
        Err(error) => {
            if wrote_image {
                if let Ok(target) = resolve_workspace_image_file(root, &relative_path, false) {
                    let _ = remove_file_durable(&target);
                }
            }
            return Err(error);
        }
    };
    state.version = STATE_VERSION;
    state.image_embed_settings = settings;
    state.assets.insert(
        id.clone(),
        StoredVaultAsset {
            kind: VaultAssetKind::Image,
            relative_path: relative_path.clone(),
            media_type: media_type.to_owned(),
            fingerprint,
            modified_nanos,
        },
    );

    if let Err(error) = write_workspace_state(root, &state) {
        if wrote_image {
            if let Ok(target) = resolve_workspace_image_file(root, &relative_path, false) {
                let _ = remove_file_durable(&target);
            }
        }
        return Err(error);
    }

    Ok(WorkspaceEmbedImageResult {
        image: EmbeddedImage {
            id,
            relative_path,
            media_type: media_type.to_owned(),
        },
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

pub(in crate::workspace) fn embed_workspace_attachment(
    root: &Path,
    note_relative_path: &str,
    settings: AttachmentEmbedSettings,
    source: &Path,
    existing_relative_path: Option<&str>,
    expected_revision: u64,
) -> Result<WorkspaceEmbedAttachmentResult, String> {
    let settings = normalize_attachment_embed_settings(&settings)?;
    let destination_folder = attachment_destination_folder(note_relative_path, &settings)?;
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "The selected attachment name is not valid Unicode.".to_owned())?;
    let safe_name = safe_attachment_file_name(file_name)?;
    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err(
            "Embedded files cannot be changed while workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    recover_workspace_transactions(root, stored_state.as_ref(), &mut warnings)?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before embedding the file."
                .to_owned(),
        );
    }

    let mut state = stored_state.unwrap_or_default();
    let existing_relative_path = existing_relative_path
        .map(str::to_owned)
        .filter(|path| validate_attachment_relative_path(path).is_ok());
    if let Some(relative_path) = existing_relative_path.as_deref() {
        if let Some((id, stored)) = state.assets.iter_mut().find(|(_, stored)| {
            stored.kind == VaultAssetKind::Attachment
                && portable_path_key(&stored.relative_path) == portable_path_key(relative_path)
        }) {
            if stored.relative_path != relative_path {
                let old_path = resolve_workspace_asset_file(root, &stored.relative_path, true)?;
                if old_path.exists() {
                    return Err(format!(
                        "The vault contains attachment paths that differ only by letter case near {relative_path}."
                    ));
                }
                stored.relative_path = relative_path.to_owned();
            }
            let fingerprint = fingerprint_attachment_file(source)?;
            stored.media_type = attachment_media_type_for_path(Path::new(relative_path)).to_owned();
            stored.fingerprint = fingerprint.clone();
            stored.modified_nanos = file_modified_nanos_for_path(source)?;
            let attachment = EmbeddedAttachment {
                id: id.clone(),
                relative_path: relative_path.to_owned(),
                media_type: stored.media_type.clone(),
                byte_length: fingerprint.length,
                opening_disabled: attachment_opening_is_disabled(source)?,
            };
            state.version = STATE_VERSION;
            state.attachment_embed_settings = settings;
            write_workspace_state(root, &state)?;

            return Ok(WorkspaceEmbedAttachmentResult {
                attachment,
                revision: revision_for_root(root)?,
                saved_at: now_millis(),
                warnings: warnings.finish(),
            });
        }
    }

    if state.assets.len() >= MAX_VAULT_ASSETS {
        return Err(format!(
            "This vault already tracks the maximum of {MAX_VAULT_ASSETS} embedded files."
        ));
    }

    let mut copied_attachment = false;
    let (relative_path, fingerprint) = if let Some(relative_path) = existing_relative_path {
        (relative_path, fingerprint_attachment_file(source)?)
    } else {
        if !destination_folder.is_empty() {
            ensure_directory_path(root, &destination_folder)?;
        }
        let relative_path = unique_attachment_relative_path(root, &destination_folder, &safe_name)?;
        let target = resolve_workspace_asset_file(root, &relative_path, true)?;
        let fingerprint = copy_attachment_file_durable(source, &target)?;
        copied_attachment = true;
        (relative_path, fingerprint)
    };
    let stored_path = resolve_workspace_asset_file(root, &relative_path, false)?;
    let modified_nanos = file_modified_nanos_for_path(&stored_path)?;
    let opening_disabled = attachment_opening_is_disabled(&stored_path)?;
    let media_type = attachment_media_type_for_path(Path::new(&relative_path)).to_owned();
    let mut used_ids = state.assets.keys().cloned().collect::<HashSet<_>>();
    let id_seed = format!(
        "{relative_path}:{}:{}:{}",
        fingerprint.length,
        fingerprint.hash,
        now_millis(),
    );
    let id = fresh_id("asset", &id_seed, &mut used_ids);
    state.version = STATE_VERSION;
    state.attachment_embed_settings = settings;
    state.assets.insert(
        id.clone(),
        StoredVaultAsset {
            kind: VaultAssetKind::Attachment,
            relative_path: relative_path.clone(),
            media_type: media_type.clone(),
            fingerprint: fingerprint.clone(),
            modified_nanos,
        },
    );
    if let Err(error) = write_workspace_state(root, &state) {
        if copied_attachment {
            if let Ok(target) = resolve_workspace_asset_file(root, &relative_path, false) {
                let _ = remove_file_durable(&target);
            }
        }
        return Err(error);
    }

    Ok(WorkspaceEmbedAttachmentResult {
        attachment: EmbeddedAttachment {
            id,
            relative_path,
            media_type,
            byte_length: fingerprint.length,
            opening_disabled,
        },
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

pub(in crate::workspace) fn discard_workspace_external_asset(
    root: &Path,
    asset_id: &str,
    relative_path: &str,
    expected_revision: u64,
) -> Result<WorkspaceExternalAssetDiscardResult, String> {
    if !is_valid_asset_id(asset_id) {
        return Err("The dropped file has an invalid stable ID.".to_owned());
    }

    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err(
            "The dropped file cannot be cleaned up while workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    let old_state = stored_state.unwrap_or_default();
    recover_workspace_transactions(root, Some(&old_state), &mut warnings)?;
    if revision_for_root(root)? != expected_revision {
        warnings.push(
            "The vault changed before the unused dropped file could be removed; the file was retained."
                .to_owned(),
        );

        return retained_external_asset_result(root, &old_state, warnings);
    }

    let Some(stored) = old_state.assets.get(asset_id) else {
        return Err("The dropped file's stable record is no longer available.".to_owned());
    };
    if stored.relative_path != relative_path {
        warnings.push(
            "The dropped file moved before cleanup, so it was retained at its current location."
                .to_owned(),
        );

        return retained_external_asset_result(root, &old_state, warnings);
    }
    let source = match stored.kind {
        VaultAssetKind::Image => {
            validate_image_relative_path(relative_path)?;
            resolve_workspace_image_file(root, relative_path, false)?
        }
        VaultAssetKind::Attachment => {
            validate_attachment_relative_path(relative_path)?;
            resolve_workspace_asset_file(root, relative_path, false)?
        }
    };
    if workspace_asset_is_referenced(root, &old_state, stored.kind, asset_id)? {
        warnings.push(
            "The dropped file is already referenced by a saved note, so it was retained."
                .to_owned(),
        );

        return retained_external_asset_result(root, &old_state, warnings);
    }
    if !workspace_asset_matches_stored(&source, stored)? {
        warnings.push(
            "The dropped file changed before cleanup, so the modified file was retained."
                .to_owned(),
        );

        return retained_external_asset_result(root, &old_state, warnings);
    }

    let mut next_state = old_state.clone();
    next_state.version = STATE_VERSION;
    next_state.assets.remove(asset_id);
    write_workspace_state(root, &next_state)?;

    let cleanup_result = (|| {
        if !workspace_asset_matches_stored(&source, stored)? {
            return Err("The dropped file changed while cleanup was being committed.".to_owned());
        }
        remove_file_durable(&source)
            .map_err(|error| format!("Could not remove the unused dropped file: {error}"))
    })();
    if let Err(error) = cleanup_result {
        write_workspace_state(root, &old_state).map_err(|rollback_error| {
            format!(
                "{error} Its stable record could not be restored: {rollback_error}. Reopen the vault before editing again."
            )
        })?;
        warnings.push(format!("{error} The file was retained."));

        return retained_external_asset_result(root, &old_state, warnings);
    }

    Ok(WorkspaceExternalAssetDiscardResult {
        discarded: true,
        note_paths: next_state.note_paths,
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

pub(in crate::workspace) fn retained_external_asset_result(
    root: &Path,
    state: &WorkspaceState,
    warnings: WarningCollector,
) -> Result<WorkspaceExternalAssetDiscardResult, String> {
    Ok(WorkspaceExternalAssetDiscardResult {
        discarded: false,
        note_paths: state.note_paths.clone(),
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

pub(in crate::workspace) fn workspace_asset_matches_stored(
    path: &Path,
    stored: &StoredVaultAsset,
) -> Result<bool, String> {
    let fingerprint = match stored.kind {
        VaultAssetKind::Image => fingerprint_bytes(&read_image_file(path)?),
        VaultAssetKind::Attachment => fingerprint_attachment_file(path)?,
    };
    Ok(fingerprint == stored.fingerprint
        && file_modified_nanos_for_path(path)? == stored.modified_nanos)
}

pub(in crate::workspace) fn workspace_asset_is_referenced(
    root: &Path,
    state: &WorkspaceState,
    kind: VaultAssetKind,
    asset_id: &str,
) -> Result<bool, String> {
    let fragment = match kind {
        VaultAssetKind::Image => format!("#oah-image={asset_id}"),
        VaultAssetKind::Attachment => format!("#oah-asset={asset_id}"),
    };
    for relative_path in state.note_paths.values() {
        validate_markdown_relative_path(relative_path)?;
        let path = resolve_workspace_file(root, relative_path, false)?;
        let content = fs::read_to_string(&path).map_err(|error| {
            format!(
                "Could not check {} for dropped-file references: {error}",
                path.display(),
            )
        })?;
        if content.contains(&fragment) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Debug)]
pub(in crate::workspace) struct PreparedAssetNoteUpdate {
    path: PathBuf,
    expected_content: Vec<u8>,
    content: Vec<u8>,
}

pub(in crate::workspace) fn relocate_workspace_image(
    root: &Path,
    image_relative_path: &str,
    target_relative_path: &str,
    asset_id: &str,
    note_updates: &[WorkspaceImageNoteUpdate],
    expected_revision: u64,
) -> Result<WorkspaceRelocateImageResult, String> {
    validate_image_relative_path(image_relative_path)?;
    validate_image_relative_path(target_relative_path)?;
    if image_relative_path == target_relative_path {
        return Err("The image is already at that path.".to_owned());
    }
    if !is_valid_asset_id(asset_id) {
        return Err("The image has an invalid stable ID.".to_owned());
    }

    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err(
            "Images cannot be reorganized while workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    let old_state = stored_state.unwrap_or_default();
    recover_workspace_transactions(root, Some(&old_state), &mut warnings)?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before moving the image."
                .to_owned(),
        );
    }
    let source = resolve_workspace_image_file(root, image_relative_path, false)?;
    let bytes = read_image_file(&source)?;
    let (media_type, _) = validate_image_bytes_impl(&bytes, Some(target_relative_path))?;
    let target = resolve_workspace_image_file(root, target_relative_path, true)?;
    let target_parent = target
        .parent()
        .ok_or_else(|| "The image destination has no parent folder.".to_owned())?;
    match fs::symlink_metadata(target_parent) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("The image destination is not a regular vault folder.".to_owned());
        }
        Ok(_) => {}
        Err(error) => {
            return Err(format!(
                "Could not inspect the image destination folder: {error}"
            ));
        }
    }
    let case_only_rename =
        portable_path_key(image_relative_path) == portable_path_key(target_relative_path);
    if !case_only_rename && image_path_exists_portably(root, target_relative_path)? {
        return Err(format!(
            "A file named {target_relative_path} already exists."
        ));
    }

    if let Some(stored) = old_state.assets.get(asset_id) {
        if stored.kind != VaultAssetKind::Image {
            return Err("The stable image record refers to a different file type.".to_owned());
        }
        if portable_path_key(&stored.relative_path) != portable_path_key(image_relative_path) {
            return Err(
                "The stable image record no longer points to that file. Reload the vault."
                    .to_owned(),
            );
        }
    } else {
        if old_state.assets.len() >= MAX_VAULT_ASSETS {
            return Err(format!(
                "This vault already tracks the maximum of {MAX_VAULT_ASSETS} embedded images."
            ));
        }
        if old_state.assets.values().any(|stored| {
            stored.kind == VaultAssetKind::Image
                && portable_path_key(&stored.relative_path)
                    == portable_path_key(image_relative_path)
        }) {
            return Err("The image's stable record changed. Reload the vault.".to_owned());
        }
    }

    let prepared_updates = prepare_asset_note_updates(root, &old_state, note_updates, "image")?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed while the image move was being prepared. Reload it and try again."
                .to_owned(),
        );
    }
    for update in &prepared_updates {
        if fs::read(&update.path)
            .map_err(|error| format!("Could not recheck a note before moving the image: {error}"))?
            != update.expected_content
        {
            return Err(
                "A note changed before the image could be moved. Reload the vault and try again."
                    .to_owned(),
            );
        }
    }

    relocate_asset_file_durable(&source, &target)
        .map_err(|error| format!("Could not move the image to {target_relative_path}: {error}"))?;

    let mut applied_note_count = 0_usize;
    for update in &prepared_updates {
        if let Err(error) = atomic_write(&update.path, &update.content) {
            let rollback_error = rollback_asset_relocation(
                &source,
                &target,
                &prepared_updates[..=applied_note_count],
                None,
                root,
            );
            return Err(format!(
                "Could not update image references: {error}{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
        applied_note_count += 1;
    }

    let modified_nanos = match image_modified_nanos_for_path(root, target_relative_path) {
        Ok(value) => value,
        Err(error) => {
            let rollback_error =
                rollback_asset_relocation(&source, &target, &prepared_updates, None, root);
            return Err(format!(
                "Could not verify the moved image: {error}{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
    };
    for update in &prepared_updates {
        let verification_error = match fs::read(&update.path) {
            Ok(content) if content == update.content => None,
            Ok(_) => Some("an image reference did not match the requested content".to_owned()),
            Err(error) => Some(format!("an image reference could not be read: {error}")),
        };
        if let Some(verification_error) = verification_error {
            let rollback_error =
                rollback_asset_relocation(&source, &target, &prepared_updates, None, root);
            return Err(format!(
                "The move could not be verified because {verification_error}.{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
    }

    let mut state = old_state.clone();
    state.version = STATE_VERSION;
    state.assets.insert(
        asset_id.to_owned(),
        StoredVaultAsset {
            kind: VaultAssetKind::Image,
            relative_path: target_relative_path.to_owned(),
            media_type: media_type.to_owned(),
            fingerprint: fingerprint_bytes(&bytes),
            modified_nanos,
        },
    );
    if let Err(error) = write_workspace_state(root, &state) {
        let rollback_error =
            rollback_asset_relocation(&source, &target, &prepared_updates, Some(&old_state), root);
        return Err(format!(
            "Could not update the stable image record: {error}{}",
            rollback_error
                .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                .unwrap_or_default(),
        ));
    }

    Ok(WorkspaceRelocateImageResult {
        image: EmbeddedImage {
            id: asset_id.to_owned(),
            relative_path: target_relative_path.to_owned(),
            media_type: media_type.to_owned(),
        },
        previous_relative_path: image_relative_path.to_owned(),
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

pub(in crate::workspace) fn relocate_workspace_attachment(
    root: &Path,
    attachment_relative_path: &str,
    target_relative_path: &str,
    asset_id: &str,
    note_updates: &[WorkspaceImageNoteUpdate],
    expected_revision: u64,
) -> Result<WorkspaceRelocateAttachmentResult, String> {
    validate_attachment_relative_path(attachment_relative_path)?;
    validate_attachment_relative_path(target_relative_path)?;
    if attachment_relative_path == target_relative_path {
        return Err("The attachment is already at that path.".to_owned());
    }
    if !is_valid_asset_id(asset_id) {
        return Err("The attachment has an invalid stable ID.".to_owned());
    }

    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err(
            "Attachments cannot be reorganized while workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    let old_state = stored_state.unwrap_or_default();
    recover_workspace_transactions(root, Some(&old_state), &mut warnings)?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before moving the attachment."
                .to_owned(),
        );
    }
    let source = resolve_workspace_asset_file(root, attachment_relative_path, false)?;
    let fingerprint = fingerprint_attachment_file(&source)?;
    let media_type = attachment_media_type_for_path(Path::new(target_relative_path));
    let target = resolve_workspace_asset_file(root, target_relative_path, true)?;
    let target_parent = target
        .parent()
        .ok_or_else(|| "The attachment destination has no parent folder.".to_owned())?;
    match fs::symlink_metadata(target_parent) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("The attachment destination is not a regular vault folder.".to_owned());
        }
        Ok(_) => {}
        Err(error) => {
            return Err(format!(
                "Could not inspect the attachment destination folder: {error}"
            ));
        }
    }
    let case_only_rename =
        portable_path_key(attachment_relative_path) == portable_path_key(target_relative_path);
    if !case_only_rename && asset_path_exists_portably(root, target_relative_path)? {
        return Err(format!(
            "A file named {target_relative_path} already exists."
        ));
    }

    if let Some(stored) = old_state.assets.get(asset_id) {
        if stored.kind != VaultAssetKind::Attachment {
            return Err("The stable attachment record refers to a different file type.".to_owned());
        }
        if portable_path_key(&stored.relative_path) != portable_path_key(attachment_relative_path) {
            return Err(
                "The stable attachment record no longer points to that file. Reload the vault."
                    .to_owned(),
            );
        }
    } else {
        if old_state.assets.len() >= MAX_VAULT_ASSETS {
            return Err(format!(
                "This vault already tracks the maximum of {MAX_VAULT_ASSETS} embedded assets."
            ));
        }
        if old_state.assets.values().any(|stored| {
            stored.kind == VaultAssetKind::Attachment
                && portable_path_key(&stored.relative_path)
                    == portable_path_key(attachment_relative_path)
        }) {
            return Err("The attachment's stable record changed. Reload the vault.".to_owned());
        }
    }

    let prepared_updates =
        prepare_asset_note_updates(root, &old_state, note_updates, "attachment")?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed while the attachment move was being prepared. Reload it and try again."
                .to_owned(),
        );
    }
    for update in &prepared_updates {
        if fs::read(&update.path).map_err(|error| {
            format!("Could not recheck a note before moving the attachment: {error}")
        })? != update.expected_content
        {
            return Err(
                "A note changed before the attachment could be moved. Reload the vault and try again."
                    .to_owned(),
            );
        }
    }

    relocate_asset_file_durable(&source, &target).map_err(|error| {
        format!("Could not move the attachment to {target_relative_path}: {error}")
    })?;

    let mut applied_note_count = 0_usize;
    for update in &prepared_updates {
        if let Err(error) = atomic_write(&update.path, &update.content) {
            let rollback_error = rollback_asset_relocation(
                &source,
                &target,
                &prepared_updates[..=applied_note_count],
                None,
                root,
            );
            return Err(format!(
                "Could not update attachment references: {error}{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
        applied_note_count += 1;
    }

    let target_fingerprint = match fingerprint_attachment_file(&target) {
        Ok(value) if value == fingerprint => value,
        Ok(_) => {
            let rollback_error =
                rollback_asset_relocation(&source, &target, &prepared_updates, None, root);
            return Err(format!(
                "The moved attachment failed its integrity check.{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
        Err(error) => {
            let rollback_error =
                rollback_asset_relocation(&source, &target, &prepared_updates, None, root);
            return Err(format!(
                "Could not verify the moved attachment: {error}{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
    };
    let modified_nanos = match file_modified_nanos_for_path(&target) {
        Ok(value) => value,
        Err(error) => {
            let rollback_error =
                rollback_asset_relocation(&source, &target, &prepared_updates, None, root);
            return Err(format!(
                "Could not inspect the moved attachment: {error}{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
    };
    for update in &prepared_updates {
        let verification_error = match fs::read(&update.path) {
            Ok(content) if content == update.content => None,
            Ok(_) => Some("an attachment reference did not match the requested content".to_owned()),
            Err(error) => Some(format!(
                "an attachment reference could not be read: {error}"
            )),
        };
        if let Some(verification_error) = verification_error {
            let rollback_error =
                rollback_asset_relocation(&source, &target, &prepared_updates, None, root);
            return Err(format!(
                "The move could not be verified because {verification_error}.{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
    }

    let mut state = old_state.clone();
    state.version = STATE_VERSION;
    state.assets.insert(
        asset_id.to_owned(),
        StoredVaultAsset {
            kind: VaultAssetKind::Attachment,
            relative_path: target_relative_path.to_owned(),
            media_type: media_type.to_owned(),
            fingerprint: target_fingerprint.clone(),
            modified_nanos,
        },
    );
    if let Err(error) = write_workspace_state(root, &state) {
        let rollback_error =
            rollback_asset_relocation(&source, &target, &prepared_updates, Some(&old_state), root);
        return Err(format!(
            "Could not update the stable attachment record: {error}{}",
            rollback_error
                .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                .unwrap_or_default(),
        ));
    }

    Ok(WorkspaceRelocateAttachmentResult {
        attachment: EmbeddedAttachment {
            id: asset_id.to_owned(),
            relative_path: target_relative_path.to_owned(),
            media_type: media_type.to_owned(),
            byte_length: target_fingerprint.length,
            opening_disabled: attachment_opening_is_disabled(&target)?,
        },
        previous_relative_path: attachment_relative_path.to_owned(),
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
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

pub(in crate::workspace) fn relocate_asset_file_durable(
    source: &Path,
    target: &Path,
) -> io::Result<()> {
    if source == target {
        return Ok(());
    }
    if source
        .to_string_lossy()
        .eq_ignore_ascii_case(&target.to_string_lossy())
    {
        let parent = source
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "asset has no parent"))?;
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".oah-asset-rename-{}-{counter}.tmp",
            std::process::id(),
        ));
        rename_durable(source, &temporary)?;
        if let Err(error) = rename_durable(&temporary, target) {
            let _ = rename_durable(&temporary, source);
            return Err(error);
        }
        return Ok(());
    }
    rename_durable(source, target)
}

pub(in crate::workspace) fn rollback_asset_relocation(
    source: &Path,
    target: &Path,
    applied_updates: &[PreparedAssetNoteUpdate],
    old_state: Option<&WorkspaceState>,
    root: &Path,
) -> Option<String> {
    let mut errors = Vec::new();
    for update in applied_updates.iter().rev() {
        if let Err(error) = atomic_write(&update.path, &update.expected_content) {
            errors.push(format!("could not restore a note: {error}"));
        }
    }
    if let Err(error) = relocate_asset_file_durable(target, source) {
        errors.push(format!("could not restore the asset: {error}"));
    }
    if let Some(state) = old_state {
        if let Err(error) = write_workspace_state(root, state) {
            errors.push(format!("could not restore asset metadata: {error}"));
        }
    }
    (!errors.is_empty()).then(|| errors.join("; "))
}

pub(in crate::workspace) fn read_workspace_image(
    root: &Path,
    asset_id: Option<&str>,
    note_relative_path: &str,
    destination: &str,
) -> Result<Vec<u8>, String> {
    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err("Workspace metadata is unreadable or newer than this app.".to_owned());
    }
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
