use super::*;

pub(in crate::workspace) struct WorkspaceImageImportTransaction {
    pub(in crate::workspace) baseline: Vec<RevisionEntry>,
    pub(in crate::workspace) targets: Vec<TransactionTarget>,
    pub(in crate::workspace) transaction_root: Option<PathBuf>,
}

pub(in crate::workspace) fn stage_workspace_attachment_import(
    root: &Path,
    transaction: &mut WorkspaceImageImportTransaction,
    relative_path: &str,
    source: &Path,
    expected_fingerprint: &FileFingerprint,
) -> Result<(), String> {
    let transaction_root = match transaction.transaction_root.as_ref() {
        Some(transaction_root) => transaction_root,
        None => {
            let created = prepare_transaction_root(root, &new_transaction_id())
                .map_err(|error| format!("Could not prepare the asset import: {error}"))?;
            transaction.transaction_root.insert(created)
        }
    };
    let kind = TransactionTargetKind::Attachment;
    let staged = staged_import_asset_path(transaction_root, relative_path, &kind)?;
    let parent = staged
        .parent()
        .ok_or_else(|| "The staged attachment path has no parent folder.".to_owned())?;
    ensure_private_directory_tree(transaction_root, parent)
        .map_err(|error| format!("Could not prepare an attachment import: {error}"))?;
    let copied_fingerprint = copy_attachment_file_durable(source, &staged)?;
    if copied_fingerprint != *expected_fingerprint {
        let _ = remove_file_durable(&staged);
        return Err(format!(
            "The staged copy of {relative_path} did not match the selected attachment."
        ));
    }
    transaction.targets.push(TransactionTarget {
        relative_path: relative_path.to_owned(),
        fingerprint: copied_fingerprint,
        kind,
    });
    Ok(())
}

pub(in crate::workspace) fn prepare_workspace_image_import(
    root: &Path,
    expected_revision: u64,
) -> Result<WorkspaceImageImportTransaction, String> {
    let baseline = revision_entries_for_root(root)?;
    if revision_for_entries(&baseline) != expected_revision {
        return Err(
            "The vault changed before its assets could be imported. Reload it and try again."
                .to_owned(),
        );
    }
    Ok(WorkspaceImageImportTransaction {
        baseline,
        targets: Vec::new(),
        transaction_root: None,
    })
}

pub(in crate::workspace) fn stage_workspace_image_import(
    root: &Path,
    transaction: &mut WorkspaceImageImportTransaction,
    relative_path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let transaction_root = match transaction.transaction_root.as_ref() {
        Some(transaction_root) => transaction_root,
        None => {
            let created = prepare_transaction_root(root, &new_transaction_id())
                .map_err(|error| format!("Could not prepare the asset import: {error}"))?;
            transaction.transaction_root.insert(created)
        }
    };
    let fingerprint = fingerprint_bytes(bytes);
    let staged = staged_import_image_path(transaction_root, relative_path)?;
    let parent = staged
        .parent()
        .ok_or_else(|| "The staged image path has no parent folder.".to_owned())?;
    ensure_private_directory_tree(transaction_root, parent)
        .map_err(|error| format!("Could not prepare an asset import: {error}"))?;
    atomic_write(&staged, bytes)
        .map_err(|error| format!("Could not stage {relative_path}: {error}"))?;
    if fingerprint_regular_file(&staged)? != Some(fingerprint.clone()) {
        return Err(format!(
            "The staged copy of {relative_path} failed its integrity check."
        ));
    }
    transaction.targets.push(TransactionTarget {
        relative_path: relative_path.to_owned(),
        fingerprint,
        kind: TransactionTargetKind::Image,
    });
    Ok(())
}

pub(in crate::workspace) fn apply_workspace_image_import(
    root: &Path,
    transaction: WorkspaceImageImportTransaction,
    warnings: &mut WarningCollector,
) -> Result<(u64, Option<String>), String> {
    let WorkspaceImageImportTransaction {
        baseline,
        targets,
        transaction_root,
    } = transaction;

    let current = match revision_entries_for_root(root) {
        Ok(current) => current,
        Err(error) => {
            if let Some(transaction_root) = &transaction_root {
                discard_private_transaction(root, transaction_root, warnings);
            }
            return Err(error);
        }
    };
    if current != baseline {
        if let Some(transaction_root) = &transaction_root {
            discard_private_transaction(root, transaction_root, warnings);
        }
        return Err(
            "The vault changed while its assets were being prepared. Reload it and try again."
                .to_owned(),
        );
    }

    let Some(transaction_root) = transaction_root else {
        return Ok((revision_for_entries(&baseline), None));
    };
    if targets.is_empty() {
        discard_private_transaction(root, &transaction_root, warnings);
        return Ok((revision_for_entries(&baseline), None));
    }

    let parent_paths = targets
        .iter()
        .filter_map(|target| {
            Path::new(&target.relative_path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .and_then(path_to_slash_string)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let created_directories = match collect_created_directories(root, parent_paths.iter(), &[]) {
        Ok(created_directories) => created_directories,
        Err(error) => {
            discard_private_transaction(root, &transaction_root, warnings);
            return Err(error);
        }
    };
    let transaction_id = match transaction_root
        .file_name()
        .and_then(|value| value.to_str())
    {
        Some(transaction_id) => transaction_id.to_owned(),
        None => {
            discard_private_transaction(root, &transaction_root, warnings);
            return Err("The asset import transaction ID is invalid.".to_owned());
        }
    };
    let mut manifest = TransactionManifest {
        version: TRANSACTION_VERSION,
        id: transaction_id,
        phase: TransactionPhase::Prepared,
        originals: Vec::new(),
        targets,
        recovery_targets: Vec::new(),
        folder_case_renames: Vec::new(),
        created_directories,
    };
    if let Err(error) = write_transaction_manifest(&transaction_root, &manifest) {
        discard_private_transaction(root, &transaction_root, warnings);
        return Err(error);
    }
    match revision_entries_for_root(root) {
        Ok(current) if current == baseline => {}
        Ok(_) => {
            discard_private_transaction(root, &transaction_root, warnings);
            return Err(
                "The vault changed while its assets were being prepared. Reload it and try again."
                    .to_owned(),
            );
        }
        Err(error) => {
            discard_private_transaction(root, &transaction_root, warnings);
            return Err(error);
        }
    }

    manifest.phase = TransactionPhase::Applying;
    if let Err(error) = write_transaction_manifest(&transaction_root, &manifest) {
        discard_private_transaction(root, &transaction_root, warnings);
        return Err(error);
    }
    let result = (|| {
        if revision_entries_for_root(root)? != baseline {
            return Err(
                "The vault changed before its assets could be committed. Reload it and try again."
                    .to_owned(),
            );
        }
        for target in &manifest.targets {
            let label = match target.kind {
                TransactionTargetKind::Image => "image",
                TransactionTargetKind::Attachment => "attachment",
                TransactionTargetKind::Markdown => {
                    return Err("A Markdown file was included in an asset import.".to_owned())
                }
            };
            ensure_asset_parent(root, &target.relative_path, label)?;
            apply_staged_import_image(root, &transaction_root, target)?;
        }
        let committed_entries = verify_image_import_consistency(root, &baseline, &manifest)?;
        if verify_image_import_consistency(root, &baseline, &manifest)? != committed_entries {
            return Err(
                "The vault changed while its imported assets were being verified. Reload it and try again."
                    .to_owned(),
            );
        }
        Ok(revision_for_entries(&committed_entries))
    })();

    let revision = match result {
        Ok(revision) => revision,
        Err(error) => {
            let recovered = rollback_transaction(root, &transaction_root, &manifest, warnings);
            if recovered {
                discard_private_transaction(root, &transaction_root, warnings);
                return Err(error);
            }
            return Err(format!(
                "{error} The interrupted asset import could not be fully rolled back. Reopen the vault before editing again."
            ));
        }
    };

    Ok((revision, Some(manifest.id)))
}

pub(in crate::workspace) fn pending_workspace_image_import(
    root: &Path,
    transaction_id: &str,
) -> Result<(PathBuf, TransactionManifest), String> {
    let transaction_root = existing_transaction_root(root, transaction_id)?;
    let manifest = read_transaction_manifest(&transaction_root)?
        .ok_or_else(|| "The pending asset import has no transaction manifest.".to_owned())?;
    if manifest.id != transaction_id
        || manifest.version > TRANSACTION_VERSION
        || manifest.phase != TransactionPhase::Applying
        || !manifest.originals.is_empty()
        || !manifest.recovery_targets.is_empty()
        || !manifest.folder_case_renames.is_empty()
        || manifest.targets.is_empty()
        || manifest
            .targets
            .iter()
            .any(|target| target.kind == TransactionTargetKind::Markdown)
    {
        return Err("The pending asset import transaction is invalid.".to_owned());
    }
    for target in &manifest.targets {
        if !import_image_was_applied(&transaction_root, target)? {
            return Err(format!(
                "The pending asset import did not create {}.",
                target.relative_path,
            ));
        }
        let path = resolve_transaction_target_file(root, target, false)?;
        if fingerprint_regular_file(&path)? != Some(target.fingerprint.clone()) {
            return Err(format!(
                "The imported asset {} changed before its notes were saved.",
                target.relative_path,
            ));
        }
    }

    Ok((transaction_root, manifest))
}

pub(in crate::workspace) fn finalize_workspace_image_import(
    root: &Path,
    transaction_id: &str,
    warnings: &mut WarningCollector,
) -> Result<(), String> {
    let (transaction_root, mut manifest) = pending_workspace_image_import(root, transaction_id)?;
    manifest.phase = TransactionPhase::Committed;
    write_transaction_manifest(&transaction_root, &manifest)?;
    discard_private_transaction(root, &transaction_root, warnings);

    Ok(())
}

pub(in crate::workspace) fn rollback_workspace_image_import(
    root: &Path,
    transaction_id: &str,
    warnings: &mut WarningCollector,
) -> Result<bool, String> {
    let (transaction_root, manifest) = pending_workspace_image_import(root, transaction_id)?;
    let recovered = rollback_transaction(root, &transaction_root, &manifest, warnings);
    if recovered {
        discard_private_transaction(root, &transaction_root, warnings);
    }

    Ok(recovered)
}

pub(in crate::workspace) fn staged_import_image_path(
    transaction_root: &Path,
    relative_path: &str,
) -> Result<PathBuf, String> {
    staged_import_asset_path(
        transaction_root,
        relative_path,
        &TransactionTargetKind::Image,
    )
}

pub(in crate::workspace) fn staged_import_asset_path(
    transaction_root: &Path,
    relative_path: &str,
    kind: &TransactionTargetKind,
) -> Result<PathBuf, String> {
    match kind {
        TransactionTargetKind::Image => validate_image_relative_path(relative_path)?,
        TransactionTargetKind::Attachment => validate_attachment_relative_path(relative_path)?,
        TransactionTargetKind::Markdown => {
            return Err("Markdown files cannot be staged as imported assets.".to_owned())
        }
    }
    let internal_path =
        checked_internal_transaction_path(&format!("assets/{relative_path}"), true)?;
    Ok(transaction_root.join(internal_path))
}

pub(in crate::workspace) fn import_asset_applied_marker_path(
    transaction_root: &Path,
    target: &TransactionTarget,
) -> Result<PathBuf, String> {
    match target.kind {
        TransactionTargetKind::Image => validate_image_relative_path(&target.relative_path)?,
        TransactionTargetKind::Attachment => {
            validate_attachment_relative_path(&target.relative_path)?
        }
        TransactionTargetKind::Markdown => {
            return Err("Markdown files do not use asset import markers.".to_owned())
        }
    }
    let internal_path =
        checked_internal_transaction_path(&format!("applied/{}.json", target.relative_path), true)?;
    Ok(transaction_root.join(internal_path))
}

pub(in crate::workspace) fn validate_private_import_directory(
    transaction_root: &Path,
    directory: &Path,
) -> Result<(), String> {
    let relative = directory
        .strip_prefix(transaction_root)
        .map_err(|_| "A private asset import path escaped its transaction.".to_owned())?;
    let root_metadata = fs::symlink_metadata(transaction_root)
        .map_err(|error| format!("Could not inspect the asset import transaction: {error}"))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("The asset import transaction is not a regular folder.".to_owned());
    }
    let mut current = transaction_root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("Could not inspect a private asset import folder: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("A private asset import path is not a regular folder.".to_owned());
        }
    }
    Ok(())
}

pub(in crate::workspace) fn mark_import_image_applied(
    transaction_root: &Path,
    target: &TransactionTarget,
) -> Result<(), String> {
    let marker = import_asset_applied_marker_path(transaction_root, target)?;
    let parent = marker
        .parent()
        .ok_or_else(|| "The asset import marker has no parent folder.".to_owned())?;
    ensure_private_directory_tree(transaction_root, parent)
        .map_err(|error| format!("Could not prepare an asset import marker: {error}"))?;
    let bytes = serde_json::to_vec(&target.fingerprint)
        .map_err(|error| format!("Could not encode an asset import marker: {error}"))?;
    atomic_write(&marker, &bytes)
        .map_err(|error| format!("Could not save an asset import marker: {error}"))?;
    if fingerprint_regular_file(&marker)? != Some(fingerprint_bytes(&bytes)) {
        return Err("An asset import marker failed its integrity check.".to_owned());
    }
    Ok(())
}

pub(in crate::workspace) fn import_image_was_applied(
    transaction_root: &Path,
    target: &TransactionTarget,
) -> Result<bool, String> {
    let marker = import_asset_applied_marker_path(transaction_root, target)?;
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!("Could not inspect an asset import marker: {error}"));
        }
    };
    let marker_parent = marker
        .parent()
        .ok_or_else(|| "The asset import marker has no parent folder.".to_owned())?;
    validate_private_import_directory(transaction_root, marker_parent)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 1024 {
        return Err("An asset import marker is unsafe or unexpectedly large.".to_owned());
    }
    let bytes = fs::read(&marker)
        .map_err(|error| format!("Could not read an asset import marker: {error}"))?;
    let fingerprint: FileFingerprint = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse an asset import marker: {error}"))?;
    if fingerprint != target.fingerprint {
        return Err("An asset import marker does not match its target.".to_owned());
    }
    Ok(true)
}

pub(in crate::workspace) fn apply_staged_import_image(
    root: &Path,
    transaction_root: &Path,
    target: &TransactionTarget,
) -> Result<(), String> {
    let staged = staged_import_asset_path(transaction_root, &target.relative_path, &target.kind)?;
    let staged_parent = staged
        .parent()
        .ok_or_else(|| "The staged asset has no parent folder.".to_owned())?;
    validate_private_import_directory(transaction_root, staged_parent)?;
    let metadata = fs::symlink_metadata(&staged).map_err(|error| {
        format!(
            "Could not inspect staged asset {}: {error}",
            target.relative_path
        )
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() != target.fingerprint.length
    {
        return Err(format!(
            "The staged copy of {} is unsafe or incomplete.",
            target.relative_path
        ));
    }
    let source = File::open(&staged).map_err(|error| {
        format!(
            "Could not open staged asset {}: {error}",
            target.relative_path
        )
    })?;
    let opened_metadata = source.metadata().map_err(|error| {
        format!(
            "Could not inspect staged asset {}: {error}",
            target.relative_path
        )
    })?;
    if !opened_metadata.is_file() || opened_metadata.len() != target.fingerprint.length {
        return Err(format!(
            "The staged copy of {} changed.",
            target.relative_path
        ));
    }

    let destination = resolve_transaction_target_file(root, target, true)?;
    if let Some(parent) = destination.parent() {
        ensure_existing_directory_without_symlink(root, parent)?;
    }
    let mut destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| {
            format!(
                "Could not import {} without overwriting another file: {error}",
                target.relative_path
            )
        })?;
    let copy_result = (|| -> io::Result<()> {
        let copied = io::copy(
            &mut source.take(target.fingerprint.length.saturating_add(1)),
            &mut destination_file,
        )?;
        if copied != target.fingerprint.length {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the staged asset length changed",
            ));
        }
        destination_file.flush()?;
        destination_file.sync_all()?;
        if let Some(parent) = destination.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    })();
    drop(destination_file);
    if let Err(error) = copy_result {
        let _ = remove_file_durable(&destination);
        return Err(format!(
            "Could not import {}: {error}",
            target.relative_path
        ));
    }
    if fingerprint_regular_file(&destination)? != Some(target.fingerprint.clone()) {
        return Err(format!(
            "The imported copy of {} failed its integrity check.",
            target.relative_path
        ));
    }
    if let Err(error) = mark_import_image_applied(transaction_root, target) {
        if fingerprint_regular_file(&destination)? == Some(target.fingerprint.clone()) {
            let _ = remove_file_durable(&destination);
        }
        return Err(error);
    }

    Ok(())
}

pub(in crate::workspace) fn verify_image_import_consistency(
    root: &Path,
    baseline: &[RevisionEntry],
    manifest: &TransactionManifest,
) -> Result<Vec<RevisionEntry>, String> {
    for target in &manifest.targets {
        let destination = resolve_transaction_target_file(root, target, false)?;
        if fingerprint_regular_file(&destination)? != Some(target.fingerprint.clone()) {
            return Err(format!(
                "{} changed while its asset import was being committed.",
                target.relative_path
            ));
        }
    }

    let current = revision_entries_for_root(root)?;
    let allowed_labels = manifest
        .targets
        .iter()
        .map(|target| format!("F:{}", target.relative_path))
        .chain(
            manifest
                .created_directories
                .iter()
                .map(|directory| format!("D:{directory}")),
        )
        .collect::<HashSet<_>>();
    if allowed_labels
        .iter()
        .any(|label| !current.iter().any(|entry| &entry.0 == label))
    {
        return Err(
            "The vault changed while its asset folders were being committed. Reload it and try again."
                .to_owned(),
        );
    }
    let unaffected = current
        .iter()
        .filter(|entry| !allowed_labels.contains(&entry.0))
        .cloned()
        .collect::<Vec<_>>();
    if unaffected != baseline {
        return Err(
            "The vault changed outside Obsidian At Home during the asset import. Reload it before editing again."
                .to_owned(),
        );
    }

    Ok(current)
}

pub(in crate::workspace) fn ensure_asset_parent(
    root: &Path,
    relative_path: &str,
    asset_label: &str,
) -> Result<(), String> {
    let parent = Path::new(relative_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let Some(parent) = parent else {
        return Ok(());
    };
    let mut current = root.to_path_buf();
    for component in parent.components() {
        let name = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| format!("An {asset_label} folder name is not valid Unicode."))?;
        let mut case_collision = false;
        let mut exact_match = false;
        for entry in fs::read_dir(&current)
            .map_err(|error| format!("Could not inspect {}: {error}", current.display()))?
        {
            let entry = entry
                .map_err(|error| format!("Could not inspect {}: {error}", current.display()))?;
            let entry_name = entry.file_name();
            if entry_name.to_string_lossy().eq_ignore_ascii_case(name) {
                if entry_name == component.as_os_str() {
                    exact_match = true;
                } else {
                    case_collision = true;
                }
                break;
            }
        }
        if case_collision {
            return Err(format!(
                "a folder differing only by letter case already exists near {relative_path}."
            ));
        }
        current.push(component.as_os_str());
        if exact_match {
            let metadata = fs::symlink_metadata(&current)
                .map_err(|error| format!("Could not inspect {}: {error}", current.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "Refusing to follow the symbolic link {}.",
                    current.display()
                ));
            }
            if !metadata.is_dir() {
                return Err(format!("{} is not a folder.", current.display()));
            }
        } else {
            create_directory_durable(&current).map_err(|error| {
                format!(
                    "Could not create the {asset_label} folder {}: {error}",
                    current.display()
                )
            })?;
        }
    }
    Ok(())
}
