use super::*;

mod planning;
mod transaction;

pub(in crate::workspace) use planning::*;
pub(in crate::workspace) use transaction::*;

pub(in crate::workspace) fn begin_workspace_asset_import(
    root: &Path,
    source_root: &Path,
    image_paths: &[String],
    attachment_paths: &[String],
    expected_revision: u64,
) -> Result<WorkspaceImportImagesResult, String> {
    if image_paths.len().saturating_add(attachment_paths.len()) > MAX_VAULT_ASSETS {
        return Err(format!(
            "Only {MAX_VAULT_ASSETS} asset files can be imported at once."
        ));
    }
    let mut transaction = prepare_workspace_image_import(root, expected_revision)?;

    let mut unique_paths = HashSet::new();
    for relative_path in image_paths {
        validate_image_relative_path(relative_path)?;
        if !unique_paths.insert(portable_path_key(relative_path)) {
            return Err(format!(
                "The asset import contains a duplicate path near {relative_path}."
            ));
        }
    }
    for relative_path in attachment_paths {
        validate_attachment_relative_path(relative_path)?;
        if !unique_paths.insert(portable_path_key(relative_path)) {
            return Err(format!(
                "The asset import contains a duplicate path near {relative_path}."
            ));
        }
    }

    let mut image_files = Vec::new();
    let mut attachment_files = Vec::new();
    let mut path_mappings = BTreeMap::new();
    let mut reserved_paths = HashSet::new();
    let mut warnings = WarningCollector::default();
    for relative_path in image_paths {
        let source = match resolve_image_import_source(source_root, relative_path) {
            Ok(source) => source,
            Err(error) => {
                match workspace_image_import_path(root, relative_path, None, &reserved_paths) {
                    Ok((target_path, _)) => {
                        reserved_paths.insert(portable_path_key(&target_path));
                        path_mappings.insert(relative_path.clone(), target_path);
                    }
                    Err(path_error) => warnings.push(format!(
                        "Could not reserve a safe path for {relative_path}: {path_error}"
                    )),
                }
                warnings.push(format!("Skipped {relative_path}: {error}"));
                continue;
            }
        };
        let (bytes, media_type) = match read_image_file(&source).and_then(|bytes| {
            validate_image_bytes_impl(&bytes, Some(relative_path))
                .map(|(media_type, _)| (bytes, media_type.to_owned()))
        }) {
            Ok(image) => image,
            Err(error) => {
                match workspace_image_import_path(root, relative_path, None, &reserved_paths) {
                    Ok((target_path, _)) => {
                        reserved_paths.insert(portable_path_key(&target_path));
                        path_mappings.insert(relative_path.clone(), target_path);
                    }
                    Err(path_error) => warnings.push(format!(
                        "Could not reserve a safe path for {relative_path}: {path_error}"
                    )),
                }
                warnings.push(format!("Skipped {relative_path}: {error}"));
                continue;
            }
        };
        let (target_path, reuse_existing) =
            match workspace_image_import_path(root, relative_path, Some(&bytes), &reserved_paths) {
                Ok(target) => target,
                Err(error) => {
                    warnings.push(format!(
                        "Could not reserve a safe path for {relative_path}: {error}"
                    ));
                    continue;
                }
            };
        reserved_paths.insert(portable_path_key(&target_path));
        path_mappings.insert(relative_path.clone(), target_path.clone());
        if target_path != *relative_path {
            warnings.push(format!(
                "Imported {relative_path} as {target_path} to avoid an existing vault path."
            ));
        }
        if reuse_existing {
            image_files.push(VaultImageFile {
                asset_id: None,
                relative_path: target_path,
                media_type,
            });
            continue;
        }

        if let Err(error) =
            stage_workspace_image_import(root, &mut transaction, &target_path, &bytes)
        {
            warnings.push(format!("Skipped {relative_path}: {error}"));
            continue;
        }
        image_files.push(VaultImageFile {
            asset_id: None,
            relative_path: target_path,
            media_type,
        });
    }

    for relative_path in attachment_paths {
        let source = match resolve_attachment_import_source(source_root, relative_path) {
            Ok(source) => source,
            Err(error) => {
                match workspace_attachment_import_path(root, relative_path, None, &reserved_paths) {
                    Ok((target_path, _)) => {
                        reserved_paths.insert(portable_path_key(&target_path));
                        path_mappings.insert(relative_path.clone(), target_path);
                    }
                    Err(path_error) => warnings.push(format!(
                        "Could not reserve a safe path for {relative_path}: {path_error}"
                    )),
                }
                warnings.push(format!("Skipped {relative_path}: {error}"));
                continue;
            }
        };
        let fingerprint = match fingerprint_attachment_file(&source) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                match workspace_attachment_import_path(root, relative_path, None, &reserved_paths) {
                    Ok((target_path, _)) => {
                        reserved_paths.insert(portable_path_key(&target_path));
                        path_mappings.insert(relative_path.clone(), target_path);
                    }
                    Err(path_error) => warnings.push(format!(
                        "Could not reserve a safe path for {relative_path}: {path_error}"
                    )),
                }
                warnings.push(format!("Skipped {relative_path}: {error}"));
                continue;
            }
        };
        let (target_path, reuse_existing) = match workspace_attachment_import_path(
            root,
            relative_path,
            Some(&fingerprint),
            &reserved_paths,
        ) {
            Ok(target) => target,
            Err(error) => {
                warnings.push(format!(
                    "Could not reserve a safe path for {relative_path}: {error}"
                ));
                continue;
            }
        };
        reserved_paths.insert(portable_path_key(&target_path));
        path_mappings.insert(relative_path.clone(), target_path.clone());
        if target_path != *relative_path {
            warnings.push(format!(
                "Imported {relative_path} as {target_path} to avoid an existing vault path."
            ));
        }
        let media_type = attachment_media_type_for_path(Path::new(&target_path)).to_owned();
        let opening_disabled = if reuse_existing {
            resolve_workspace_asset_file(root, &target_path, false)
                .and_then(|path| attachment_opening_is_disabled(&path))
                .unwrap_or(true)
        } else {
            attachment_opening_is_disabled(&source).unwrap_or(true)
        };
        if !reuse_existing {
            if let Err(error) = stage_workspace_attachment_import(
                root,
                &mut transaction,
                &target_path,
                &source,
                &fingerprint,
            ) {
                warnings.push(format!("Skipped {relative_path}: {error}"));
                continue;
            }
        }
        attachment_files.push(VaultAttachmentFile {
            asset_id: None,
            relative_path: target_path,
            media_type,
            byte_length: fingerprint.length,
            opening_disabled,
        });
    }

    if let Some(missing_path) = image_paths
        .iter()
        .chain(attachment_paths.iter())
        .find(|path| !path_mappings.contains_key(path.as_str()))
    {
        if let Some(transaction_root) = transaction.transaction_root.take() {
            discard_private_transaction(root, &transaction_root, &mut warnings);
        }
        return Err(format!(
            "A safe destination could not be reserved for {missing_path}."
        ));
    }

    let (revision, transaction_id) =
        apply_workspace_image_import(root, transaction, &mut warnings)?;
    Ok(WorkspaceImportImagesResult {
        image_count: image_files.len(),
        image_files,
        attachment_count: attachment_files.len(),
        attachment_files,
        path_mappings,
        transaction_id,
        revision,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

#[cfg(test)]
pub(in crate::workspace) fn begin_workspace_image_import(
    root: &Path,
    source_root: &Path,
    image_paths: &[String],
    expected_revision: u64,
) -> Result<WorkspaceImportImagesResult, String> {
    begin_workspace_asset_import(root, source_root, image_paths, &[], expected_revision)
}

#[cfg(test)]
pub(in crate::workspace) fn import_workspace_images(
    root: &Path,
    source_root: &Path,
    image_paths: &[String],
    expected_revision: u64,
) -> Result<WorkspaceImportImagesResult, String> {
    let mut result =
        begin_workspace_image_import(root, source_root, image_paths, expected_revision)?;
    if let Some(transaction_id) = result.transaction_id.take() {
        let mut warnings = WarningCollector::default();
        finalize_workspace_image_import(root, &transaction_id, &mut warnings)?;
        result.warnings.extend(warnings.finish());
    }

    Ok(result)
}
