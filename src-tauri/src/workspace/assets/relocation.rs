use super::*;

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
