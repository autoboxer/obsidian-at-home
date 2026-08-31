use super::*;

#[tauri::command]
pub fn workspace_bootstrap(app: AppHandle, defaults: VaultData) -> Result<BootstrapResult, String> {
    let _guard = lock_workspace_io()?;
    let mut registry = read_registry(&app)?;
    sort_descriptors(&mut registry.recent_vaults);

    let workspace = match registry.active_path.clone() {
        Some(active_path) => {
            let path = PathBuf::from(&active_path);
            if !path.is_dir() {
                registry.active_path = None;
                write_registry(&app, &registry)?;
                None
            } else {
                let canonical = validate_workspace_root_path(&path)?;
                reject_home_vault(&app, &canonical)?;
                let _workspace_guard = lock_workspace_files(&canonical)?;
                let loaded = load_workspace(&canonical, &defaults)?;
                remember_workspace(&mut registry, &loaded.descriptor);
                write_registry(&app, &registry)?;
                Some(loaded)
            }
        }
        None => None,
    };

    sort_descriptors(&mut registry.recent_vaults);
    Ok(BootstrapResult {
        workspace,
        recent_vaults: registry.recent_vaults,
    })
}

#[tauri::command]
pub fn workspace_open(
    app: AppHandle,
    path: String,
    defaults: VaultData,
) -> Result<WorkspaceLoad, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let mut registry = read_registry(&app)?;
    reject_nested_registered_vault(&root, &registry, true)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let loaded = load_workspace(&root, &defaults)?;
    remember_workspace(&mut registry, &loaded.descriptor);
    write_registry(&app, &registry)?;
    Ok(loaded)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_create(
    app: AppHandle,
    parent_path: String,
    name: String,
    mut initial: VaultData,
) -> Result<WorkspaceLoad, String> {
    let _guard = lock_workspace_io()?;
    let parent = validate_parent_directory(&parent_path)?;
    validate_component_name(name.trim(), "vault")?;
    let root = parent.join(name.trim());
    let mut registry = read_registry(&app)?;
    reject_nested_registered_vault(&root, &registry, false)?;
    if root.exists() {
        return Err(format!(
            "A file or folder already exists at {}.",
            root.display()
        ));
    }
    create_directory_durable(&root)
        .map_err(|error| format!("Could not create the vault folder: {error}"))?;
    let root = root
        .canonicalize()
        .map_err(|error| format!("Could not resolve the new vault folder: {error}"))?;
    let _workspace_guard = lock_workspace_files(&root)?;

    initial.name = name.trim().to_owned();
    let expected_revision = revision_for_root(&root)?;
    save_workspace_files(&root, &initial, expected_revision)?;
    let mut loaded = load_workspace(&root, &initial)?;

    match (|| {
        remember_workspace(&mut registry, &loaded.descriptor);
        write_registry(&app, &registry)
    })() {
        Ok(()) => {}
        Err(error) => loaded.warnings.push(format!(
            "The vault was created, but it could not be added to Recents: {error}"
        )),
    }
    Ok(loaded)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_save(
    app: AppHandle,
    path: String,
    vault: VaultData,
    expected_revision: u64,
) -> Result<SaveResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let mut result = save_workspace_files(&root, &vault, expected_revision)?;

    let registry_result = (|| {
        let mut registry = read_registry(&app)?;
        let descriptor = VaultDescriptor {
            name: display_vault_name(&vault.name, &root),
            path: path_string(&root)?,
            last_opened_at: result.saved_at,
        };
        remember_workspace(&mut registry, &descriptor);
        write_registry(&app, &registry)
    })();
    if let Err(error) = registry_result {
        result.warnings.push(format!(
            "The vault was saved, but Recents could not be updated: {error}"
        ));
    }
    Ok(result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_save_with_image_import(
    app: AppHandle,
    path: String,
    vault: VaultData,
    expected_revision: u64,
    transaction_id: String,
) -> Result<WorkspaceImportSaveResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let mut result =
        save_workspace_files_with_image_import(&root, &vault, expected_revision, &transaction_id)?;

    if result.saved {
        let registry_result = (|| {
            let mut registry = read_registry(&app)?;
            let descriptor = VaultDescriptor {
                name: display_vault_name(&vault.name, &root),
                path: path_string(&root)?,
                last_opened_at: result.saved_at,
            };
            remember_workspace(&mut registry, &descriptor);
            write_registry(&app, &registry)
        })();
        if let Err(error) = registry_result {
            result.warnings.push(format!(
                "The vault was saved, but Recents could not be updated: {error}"
            ));
        }
    }

    Ok(result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_archive_note(
    app: AppHandle,
    path: String,
    vault: VaultData,
    note: Note,
    original_folder_path: String,
    editor_position: Option<NoteEditorPosition>,
    expected_revision: u64,
) -> Result<WorkspaceArchiveResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let pending_archive = PendingNoteArchive {
        note,
        original_folder_path,
        editor_position,
    };
    let (mut result, deleted_note) =
        save_workspace_files_with_archive(&root, &vault, expected_revision, Some(pending_archive))?;
    let deleted_note =
        deleted_note.ok_or_else(|| "The note was saved without a recovery snapshot.".to_owned())?;

    let registry_result = (|| {
        let mut registry = read_registry(&app)?;
        let descriptor = VaultDescriptor {
            name: display_vault_name(&vault.name, &root),
            path: path_string(&root)?,
            last_opened_at: result.saved_at,
        };
        remember_workspace(&mut registry, &descriptor);
        write_registry(&app, &registry)
    })();
    if let Err(error) = registry_result {
        result.warnings.push(format!(
            "The vault was saved, but Recents could not be updated: {error}"
        ));
    }

    Ok(WorkspaceArchiveResult {
        deleted_note,
        revision: result.revision,
        saved_at: result.saved_at,
        warnings: result.warnings,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_restore_recently_deleted_note(
    app: AppHandle,
    path: String,
    deleted_note_id: String,
    vault: VaultData,
    expected_revision: u64,
) -> Result<WorkspaceRestoreResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let (state, deleted_note) =
        read_recovery_for_restore(&root, &deleted_note_id, expected_revision)?;
    let (restored_note, preferred_relative_path) =
        build_restored_note(&root, &vault, &state, &deleted_note)?;
    let mut restored_vault = vault;
    restored_vault.notes.push(restored_note.clone());
    restored_vault.active_note_id = Some(restored_note.id.clone());
    restored_vault
        .recent_note_ids
        .retain(|id| id != &restored_note.id);
    restored_vault
        .recent_note_ids
        .insert(0, restored_note.id.clone());
    restored_vault.recent_note_ids.truncate(MAX_RECENT_NOTES);
    restored_vault.selected_folder_id = "all".to_owned();

    let (mut result, prepared_restore) = save_workspace_files_with_restore(
        &root,
        &restored_vault,
        expected_revision,
        PendingNoteRestore {
            deleted_note_id,
            restored_note,
            preferred_relative_path,
        },
    )?;

    let registry_result = (|| {
        let mut registry = read_registry(&app)?;
        let descriptor = VaultDescriptor {
            name: display_vault_name(&restored_vault.name, &root),
            path: path_string(&root)?,
            last_opened_at: result.saved_at,
        };
        remember_workspace(&mut registry, &descriptor);
        write_registry(&app, &registry)
    })();
    if let Err(error) = registry_result {
        result.warnings.push(format!(
            "The vault was saved, but Recents could not be updated: {error}"
        ));
    }

    Ok(WorkspaceRestoreResult {
        restored_note: prepared_restore.restored_note,
        editor_position: prepared_restore.editor_position,
        revision: result.revision,
        saved_at: result.saved_at,
        warnings: result.warnings,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_delete_recently_deleted_notes(
    app: AppHandle,
    path: String,
    deleted_note_ids: Vec<String>,
    expected_revision: u64,
) -> Result<WorkspaceRecoveryMutationResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    remove_recently_deleted_notes(&root, deleted_note_ids, expected_revision, false)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_prune_recently_deleted_notes(
    app: AppHandle,
    path: String,
    expected_revision: u64,
) -> Result<WorkspaceRecoveryMutationResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    remove_recently_deleted_notes(&root, Vec::new(), expected_revision, true)
}

#[tauri::command]
pub fn workspace_forget(app: AppHandle, path: String) -> Result<Vec<VaultDescriptor>, String> {
    let _guard = lock_workspace_io()?;
    let mut registry = read_registry(&app)?;
    let comparison_path = canonical_path_if_available(&path);
    registry
        .recent_vaults
        .retain(|vault| canonical_path_if_available(&vault.path) != comparison_path);
    if registry
        .active_path
        .as_deref()
        .is_some_and(|active| canonical_path_if_available(active) == comparison_path)
    {
        registry.active_path = None;
    }
    sort_descriptors(&mut registry.recent_vaults);
    write_registry(&app, &registry)?;
    Ok(registry.recent_vaults)
}

#[tauri::command]
pub fn workspace_revision(app: AppHandle, path: String) -> Result<u64, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    revision_for_root(&root)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_embed_image_file(
    app: AppHandle,
    path: String,
    source_path: String,
    note_relative_path: String,
    settings: ImageEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedImageResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let source = validate_image_source_file(&source_path)?;
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Image.png")
        .to_owned();
    let bytes = read_image_file(&source)?;
    let existing_relative_path = source
        .strip_prefix(&root)
        .ok()
        .and_then(path_to_slash_string)
        .filter(|relative| validate_image_relative_path(relative).is_ok());

    embed_workspace_image(
        &root,
        &note_relative_path,
        settings,
        &file_name,
        &bytes,
        existing_relative_path.as_deref(),
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_embed_vault_image(
    app: AppHandle,
    path: String,
    image_relative_path: String,
    note_relative_path: String,
    settings: ImageEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedImageResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    validate_image_relative_path(&image_relative_path)?;
    let source = resolve_workspace_image_file(&root, &image_relative_path, false)?;
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Image.png")
        .to_owned();
    let bytes = read_image_file(&source)?;

    embed_workspace_image(
        &root,
        &note_relative_path,
        settings,
        &file_name,
        &bytes,
        Some(&image_relative_path),
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_embed_attachment_file(
    app: AppHandle,
    path: String,
    source_path: String,
    note_relative_path: String,
    settings: AttachmentEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedAttachmentResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let source = validate_attachment_source_file(&source_path)?;
    let existing_relative_path = source
        .strip_prefix(&root)
        .ok()
        .and_then(path_to_slash_string)
        .filter(|relative| validate_attachment_relative_path(relative).is_ok());

    embed_workspace_attachment(
        &root,
        &note_relative_path,
        settings,
        &source,
        existing_relative_path.as_deref(),
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_begin_external_file_upload(
    app: AppHandle,
    path: String,
    file_name: String,
    byte_length: u64,
    kind: ExternalFileUploadKind,
    note_relative_path: String,
    expected_revision: u64,
) -> Result<WorkspaceExternalFileUpload, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    validate_external_file_drop_note(&root, &note_relative_path)?;
    if revision_for_root(&root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before dropping the file."
                .to_owned(),
        );
    }
    let staging_directory = external_file_staging_directory(&app)?;
    begin_external_file_upload(
        &staging_directory,
        file_name,
        byte_length,
        kind,
        root,
        note_relative_path,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_append_external_file_upload(
    request: tauri::ipc::Request<'_>,
) -> Result<u64, String> {
    let encoded_metadata = request
        .headers()
        .get("x-oah-external-file-upload")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "Dropped-file transfer metadata is missing.".to_owned())?;
    let metadata: AppendExternalFileUploadMetadata =
        serde_json::from_str(&percent_decode_utf8(encoded_metadata)?)
            .map_err(|error| format!("Dropped-file transfer metadata is invalid: {error}"))?;
    let bytes: Cow<'_, [u8]> = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => Cow::Borrowed(bytes),
        tauri::ipc::InvokeBody::Json(serde_json::Value::Array(values)) => Cow::Owned(
            values
                .iter()
                .map(|value| {
                    value
                        .as_u64()
                        .filter(|value| *value <= u64::from(u8::MAX))
                        .map(|value| value as u8)
                        .ok_or_else(|| "Dropped-file bytes are invalid.".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        _ => return Err("Dropped-file bytes are missing.".to_owned()),
    };
    append_external_file_upload(&metadata.upload_id, metadata.offset, &bytes)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_cancel_external_file_upload(upload_id: String) -> Result<bool, String> {
    cancel_external_file_upload(&upload_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_finish_external_image_upload(
    app: AppHandle,
    upload_id: String,
    settings: ImageEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedImageResult, String> {
    let staged = finish_external_file_upload(&upload_id, ExternalFileUploadKind::Image)?;
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root_path(&staged.root)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    validate_external_file_drop_note(&root, &staged.note_relative_path)?;
    let source = validate_image_source_path(&staged.path)?;
    let bytes = read_image_file(&source)?;
    embed_workspace_image(
        &root,
        &staged.note_relative_path,
        settings,
        &staged.file_name,
        &bytes,
        None,
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_finish_external_attachment_upload(
    app: AppHandle,
    upload_id: String,
    settings: AttachmentEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedAttachmentResult, String> {
    let staged = finish_external_file_upload(&upload_id, ExternalFileUploadKind::Attachment)?;
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root_path(&staged.root)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    validate_external_file_drop_note(&root, &staged.note_relative_path)?;
    let source = validate_attachment_source_path(&staged.path)?;
    embed_workspace_attachment(
        &root,
        &staged.note_relative_path,
        settings,
        &source,
        None,
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_discard_external_asset(
    app: AppHandle,
    path: String,
    asset_id: String,
    relative_path: String,
    expected_revision: u64,
) -> Result<WorkspaceExternalAssetDiscardResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    discard_workspace_external_asset(&root, &asset_id, &relative_path, expected_revision)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_embed_vault_attachment(
    app: AppHandle,
    path: String,
    attachment_relative_path: String,
    note_relative_path: String,
    settings: AttachmentEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedAttachmentResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    validate_attachment_relative_path(&attachment_relative_path)?;
    let source = resolve_workspace_asset_file(&root, &attachment_relative_path, false)?;

    embed_workspace_attachment(
        &root,
        &note_relative_path,
        settings,
        &source,
        Some(&attachment_relative_path),
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_relocate_image(
    app: AppHandle,
    path: String,
    image_relative_path: String,
    target_relative_path: String,
    asset_id: String,
    note_updates: Vec<WorkspaceImageNoteUpdate>,
    expected_revision: u64,
) -> Result<WorkspaceRelocateImageResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    relocate_workspace_image(
        &root,
        &image_relative_path,
        &target_relative_path,
        &asset_id,
        &note_updates,
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_relocate_attachment(
    app: AppHandle,
    path: String,
    attachment_relative_path: String,
    target_relative_path: String,
    asset_id: String,
    note_updates: Vec<WorkspaceImageNoteUpdate>,
    expected_revision: u64,
) -> Result<WorkspaceRelocateAttachmentResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    relocate_workspace_attachment(
        &root,
        &attachment_relative_path,
        &target_relative_path,
        &asset_id,
        &note_updates,
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_locate_vault_item(
    app: AppHandle,
    path: String,
    kind: WorkspaceVaultItemKind,
    relative_path: String,
    asset_id: Option<String>,
) -> Result<String, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    locate_workspace_vault_item(&root, kind, &relative_path, asset_id.as_deref())
        .map(|(resolved_relative_path, _)| resolved_relative_path)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_show_vault_item_in_folder(
    app: AppHandle,
    path: String,
    kind: WorkspaceVaultItemKind,
    relative_path: String,
    asset_id: Option<String>,
) -> Result<(), String> {
    let target = {
        let _guard = lock_workspace_io()?;
        let root = validate_workspace_root(&path)?;
        reject_home_vault(&app, &root)?;
        let _workspace_guard = lock_workspace_files(&root)?;
        let (_, target) =
            locate_workspace_vault_item(&root, kind, &relative_path, asset_id.as_deref())?;
        target
    };
    app.opener()
        .reveal_item_in_dir(&target)
        .map_err(|error| format!("Could not show the vault item in its folder: {error}"))
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_open_attachment(
    app: AppHandle,
    path: String,
    attachment_relative_path: String,
    asset_id: Option<String>,
) -> Result<(), String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let (_, source) =
        resolve_attachment_action_source(&root, &attachment_relative_path, asset_id.as_deref())?;
    if is_archive_attachment_path(&source) {
        return Err(
            "Archives must be saved to a location outside the vault before opening.".to_owned(),
        );
    }
    if attachment_opening_is_disabled(&source)? {
        return Err("Opening executable or installer attachments is not supported.".to_owned());
    }
    app.opener()
        .open_path(path_string(&source)?, None::<&str>)
        .map_err(|error| format!("Could not open the attachment: {error}"))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn workspace_save_attachment_copy(
    app: AppHandle,
    path: String,
    attachment_relative_path: String,
    asset_id: Option<String>,
    preferred_directory: Option<String>,
) -> Result<Option<WorkspaceAttachmentCopyResult>, String> {
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    save_workspace_attachment_copy(
        &app,
        &root,
        &attachment_relative_path,
        asset_id.as_deref(),
        preferred_directory.as_deref(),
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_embed_image_bytes(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<WorkspaceEmbedImageResult, String> {
    let encoded_metadata = request
        .headers()
        .get("x-oah-image-metadata")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "Image metadata is missing from the clipboard request.".to_owned())?;
    let metadata: EmbedImageBytesMetadata =
        serde_json::from_str(&percent_decode_utf8(encoded_metadata)?)
            .map_err(|error| format!("Image metadata is invalid: {error}"))?;
    let bytes: Cow<'_, [u8]> = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => Cow::Borrowed(bytes),
        tauri::ipc::InvokeBody::Json(serde_json::Value::Array(values)) => Cow::Owned(
            values
                .iter()
                .map(|value| {
                    value
                        .as_u64()
                        .filter(|value| *value <= u64::from(u8::MAX))
                        .map(|value| value as u8)
                        .ok_or_else(|| "Image bytes are invalid.".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        _ => return Err("Image bytes are missing from the clipboard request.".to_owned()),
    };
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&metadata.path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    embed_workspace_image(
        &root,
        &metadata.note_relative_path,
        metadata.settings,
        &metadata.file_name,
        &bytes,
        None,
        metadata.expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_read_image(
    app: AppHandle,
    path: String,
    asset_id: Option<String>,
    note_relative_path: String,
    destination: String,
) -> Result<tauri::ipc::Response, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    read_workspace_image(
        &root,
        asset_id.as_deref(),
        &note_relative_path,
        &destination,
    )
    .map(tauri::ipc::Response::new)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_import_images(
    app: AppHandle,
    path: String,
    source_path: String,
    image_paths: Vec<String>,
    expected_revision: u64,
) -> Result<WorkspaceImportImagesResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let source_root = validate_image_import_root(&source_path)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    begin_workspace_asset_import(&root, &source_root, &image_paths, &[], expected_revision)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_import_assets(
    app: AppHandle,
    path: String,
    source_path: String,
    image_paths: Vec<String>,
    attachment_paths: Vec<String>,
    expected_revision: u64,
) -> Result<WorkspaceImportImagesResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let source_root = validate_image_import_root(&source_path)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    begin_workspace_asset_import(
        &root,
        &source_root,
        &image_paths,
        &attachment_paths,
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_save_editor_positions(
    app: AppHandle,
    path: String,
    positions: BTreeMap<String, NoteEditorPosition>,
    expected_revision: Option<String>,
) -> Result<String, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    save_editor_positions(&root, positions, expected_revision)
}
