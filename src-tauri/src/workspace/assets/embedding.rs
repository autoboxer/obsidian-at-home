use super::*;

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
