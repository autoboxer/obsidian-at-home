use super::*;

pub(in crate::workspace) fn remove_empty_managed_directories(
    root: &Path,
    old_paths: &BTreeMap<String, String>,
    new_paths: &BTreeMap<String, String>,
    warnings: &mut WarningCollector,
) {
    let desired: HashSet<String> = new_paths
        .values()
        .map(|path| portable_path_key(path))
        .collect();
    let mut obsolete: Vec<&String> = old_paths
        .values()
        .filter(|path| !desired.contains(&portable_path_key(path)))
        .collect();
    obsolete.sort_by_key(|path| std::cmp::Reverse(path.split('/').count()));
    for relative_path in obsolete {
        let Ok(path) = resolve_workspace_directory(root, relative_path) else {
            continue;
        };
        match remove_directory_durable(&path) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    || error.kind() == io::ErrorKind::DirectoryNotEmpty => {}
            Err(error) => warnings.push(format!(
                "Could not remove the empty folder {relative_path}: {error}"
            )),
        }
    }
}

pub(in crate::workspace) fn validate_managed_path_ownership(
    paths: &BTreeMap<String, String>,
) -> Result<(), String> {
    let mut portable_paths = HashSet::new();
    for path in paths.values() {
        validate_markdown_relative_path(path)?;
        if !portable_paths.insert(portable_path_key(path)) {
            return Err(format!(
                "Workspace metadata contains more than one managed note for {path}."
            ));
        }
    }
    Ok(())
}

pub(in crate::workspace) fn validate_save_targets(
    root: &Path,
    plans: &[NoteWritePlan],
    paths_to_replace: &BTreeSet<String>,
    managed_paths: &BTreeMap<String, String>,
) -> Result<(), String> {
    let replace_keys: HashSet<String> = paths_to_replace
        .iter()
        .map(|path| portable_path_key(path))
        .collect();
    let managed_keys: HashSet<String> = managed_paths
        .values()
        .map(|path| portable_path_key(path))
        .collect();
    for plan in plans.iter().filter(|plan| plan.needs_write) {
        let target = resolve_workspace_file(root, &plan.new_relative_path, true)?;
        if fs::symlink_metadata(&target).is_ok() {
            let key = portable_path_key(&plan.new_relative_path);
            if !replace_keys.contains(&key) || !managed_keys.contains(&key) {
                return Err(format!(
                    "Cannot save {:?} because {} already exists and is not owned by this vault.",
                    plan.id, plan.new_relative_path
                ));
            }
        }
    }
    Ok(())
}

pub(in crate::workspace) fn build_folder_case_renames(
    old_paths: &BTreeMap<String, String>,
    new_paths: &BTreeMap<String, String>,
) -> Result<Vec<FolderCaseRename>, String> {
    let mut candidates: Vec<(String, String)> = old_paths
        .iter()
        .filter_map(|(id, old_path)| {
            let new_path = new_paths.get(id)?;
            (old_path != new_path && portable_path_key(old_path) == portable_path_key(new_path))
                .then(|| (old_path.clone(), new_path.clone()))
        })
        .collect();
    candidates.sort_by_key(|(old_path, _)| old_path.split('/').count());

    let mut operations: Vec<FolderCaseRename> = Vec::new();
    for (old_path, new_path) in candidates {
        let mut current_from = old_path;
        for operation in &operations {
            if current_from == operation.from_relative_path {
                current_from = operation.to_relative_path.clone();
                continue;
            }
            let prefix = format!("{}/", operation.from_relative_path);
            if let Some(remainder) = current_from.strip_prefix(&prefix) {
                current_from = format!("{}/{remainder}", operation.to_relative_path);
            }
        }
        if current_from == new_path {
            continue;
        }
        validate_relative_path(&current_from, false)?;
        validate_relative_path(&new_path, false)?;
        if portable_path_key(&current_from) != portable_path_key(&new_path) {
            return Err("A case-only folder rename could not be planned safely.".to_owned());
        }
        operations.push(FolderCaseRename {
            from_relative_path: current_from,
            to_relative_path: new_path,
        });
    }
    Ok(operations)
}

pub(in crate::workspace) fn validate_folder_case_renames(
    root: &Path,
    operations: &[FolderCaseRename],
) -> Result<(), String> {
    for (index, operation) in operations.iter().enumerate() {
        // Nested case-only renames are expressed in the path produced by their
        // parent rename. Map them back to the current on-disk path while doing
        // the preflight checks; apply_transaction performs them in order.
        let source_relative =
            path_before_folder_renames(&operation.from_relative_path, &operations[..index]);
        let target_relative =
            path_before_folder_renames(&operation.to_relative_path, &operations[..index]);
        let source = resolve_workspace_directory(root, &source_relative)?;
        let metadata = fs::symlink_metadata(&source).map_err(|error| {
            format!(
                "Could not inspect the folder {} before renaming it: {error}",
                operation.from_relative_path
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "{} is not a regular folder.",
                operation.from_relative_path
            ));
        }
        if directory_contains_nested_vault(&source) {
            return Err(format!(
                "Cannot rename {} because it contains another vault.",
                operation.from_relative_path
            ));
        }
        let target = resolve_workspace_directory(root, &target_relative)?;
        match fs::symlink_metadata(&target) {
            Ok(target_metadata) if target_metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Cannot rename {} because {} is a symbolic link.",
                    operation.from_relative_path, operation.to_relative_path
                ));
            }
            Ok(target_metadata) if !target_metadata.is_dir() => {
                return Err(format!(
                    "Cannot rename {} because {} is not a folder.",
                    operation.from_relative_path, operation.to_relative_path
                ));
            }
            Ok(_) => {
                let same_location = source
                    .canonicalize()
                    .ok()
                    .zip(target.canonicalize().ok())
                    .is_some_and(|(left, right)| left == right);
                if !same_location {
                    return Err(format!(
                        "Cannot rename {} because {} already exists.",
                        operation.from_relative_path, operation.to_relative_path
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Could not inspect {} before renaming it: {error}",
                    operation.to_relative_path
                ));
            }
        }
    }
    Ok(())
}

pub(in crate::workspace) fn path_before_folder_renames(
    path: &str,
    operations: &[FolderCaseRename],
) -> String {
    let mut current = path.to_owned();
    for operation in operations.iter().rev() {
        if current == operation.to_relative_path {
            current = operation.from_relative_path.clone();
            continue;
        }
        let prefix = format!("{}/", operation.to_relative_path);
        if let Some(remainder) = current.strip_prefix(&prefix) {
            current = format!("{}/{remainder}", operation.from_relative_path);
        }
    }
    current
}

pub(in crate::workspace) fn collect_created_directories<'a>(
    root: &Path,
    desired_paths: impl Iterator<Item = &'a String>,
    case_renames: &[FolderCaseRename],
) -> Result<Vec<String>, String> {
    let rename_targets: HashSet<String> = case_renames
        .iter()
        .map(|operation| portable_path_key(&operation.to_relative_path))
        .collect();
    let mut created = BTreeSet::new();
    for desired_path in desired_paths {
        validate_relative_path(desired_path, false)?;
        let mut prefix = String::new();
        for component in desired_path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if rename_targets.contains(&portable_path_key(&prefix)) {
                continue;
            }
            let path = root.join(checked_relative_path(&prefix, false)?);
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(format!(
                        "Refusing to use the symbolic link {}.",
                        path.display()
                    ));
                }
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => return Err(format!("{} is not a folder.", path.display())),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    created.insert(prefix.clone());
                }
                Err(error) => {
                    return Err(format!("Could not inspect {}: {error}", path.display()));
                }
            }
        }
    }
    Ok(created.into_iter().collect())
}

pub(in crate::workspace) fn prepare_transaction(
    root: &Path,
    id: String,
    paths_to_replace: &BTreeSet<String>,
    plans: &[NoteWritePlan],
    recovery_archives: &[PreparedNoteArchive],
    folder_case_renames: Vec<FolderCaseRename>,
    created_directories: Vec<String>,
) -> Result<(PathBuf, TransactionManifest), String> {
    let transaction_root = prepare_transaction_root(root, &id)?;
    let mut originals = Vec::new();
    for relative_path in paths_to_replace {
        let source = resolve_workspace_file(root, relative_path, true)?;
        let metadata = match fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("Could not inspect {relative_path}: {error}"));
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!("{relative_path} is not a regular Markdown file."));
        }
        let bytes = fs::read(&source)
            .map_err(|error| format!("Could not back up {relative_path}: {error}"))?;
        let backup_relative_path = format!("backups/{relative_path}");
        let backup = transaction_root.join(checked_internal_transaction_path(
            &backup_relative_path,
            true,
        )?);
        if let Some(parent) = backup.parent() {
            ensure_private_directory_tree(&transaction_root, parent)
                .map_err(|error| format!("Could not prepare a note backup: {error}"))?;
        }
        atomic_write(&backup, &bytes)
            .map_err(|error| format!("Could not back up {relative_path}: {error}"))?;
        originals.push(TransactionOriginal {
            relative_path: relative_path.clone(),
            backup_relative_path,
            fingerprint: fingerprint_bytes(&bytes),
        });
    }
    let targets = plans
        .iter()
        .filter(|plan| plan.needs_write)
        .map(|plan| TransactionTarget {
            relative_path: plan.new_relative_path.clone(),
            fingerprint: fingerprint_bytes(plan.content.as_bytes()),
            kind: TransactionTargetKind::Markdown,
        })
        .collect();
    let mut recovery_targets = Vec::with_capacity(recovery_archives.len());
    for archive in recovery_archives {
        validate_recently_deleted_id(&archive.deleted_note.id)?;
        if fingerprint_bytes(&archive.bytes) != archive.fingerprint {
            return Err(
                "A recovery snapshot changed while the save was being prepared.".to_owned(),
            );
        }
        let archived_content_fingerprint =
            fingerprint_bytes(archive.deleted_note.note.content.as_bytes());
        let original = originals
            .iter()
            .find(|original| original.relative_path == archive.deleted_note.note.relative_path)
            .ok_or_else(|| {
                "The note being archived was not included in the save transaction.".to_owned()
            })?;
        if original.fingerprint != archived_content_fingerprint {
            return Err(
                "The note changed while its recovery snapshot was being prepared. Try again."
                    .to_owned(),
            );
        }
        let staged =
            transaction_recovery_snapshot_path(&transaction_root, &archive.deleted_note.id)?;
        if let Some(parent) = staged.parent() {
            ensure_private_directory_tree(&transaction_root, parent)
                .map_err(|error| format!("Could not prepare a recovery snapshot: {error}"))?;
        }
        atomic_write(&staged, &archive.bytes)
            .map_err(|error| format!("Could not stage a recovery snapshot: {error}"))?;
        recovery_targets.push(TransactionRecoveryTarget {
            id: archive.deleted_note.id.clone(),
            fingerprint: archive.fingerprint.clone(),
        });
    }
    let manifest = TransactionManifest {
        version: TRANSACTION_VERSION,
        id,
        phase: TransactionPhase::Prepared,
        originals,
        targets,
        recovery_targets,
        folder_case_renames,
        created_directories,
    };
    write_transaction_manifest(&transaction_root, &manifest)?;
    Ok((transaction_root, manifest))
}

pub(in crate::workspace) fn apply_transaction(
    root: &Path,
    transaction_root: &Path,
    manifest: &TransactionManifest,
    plans: &[NoteWritePlan],
    warnings: &mut WarningCollector,
) -> Result<(), String> {
    apply_recovery_targets(root, transaction_root, &manifest.recovery_targets)?;

    for original in &manifest.originals {
        let source = resolve_workspace_file(root, &original.relative_path, true)?;
        let current = fingerprint_regular_file(&source)?
            .ok_or_else(|| format!("{} disappeared while saving.", original.relative_path))?;
        if current != original.fingerprint {
            return Err(format!(
                "{} changed in another app while saving. Reload the vault before trying again.",
                original.relative_path
            ));
        }
        remove_file_durable(&source)
            .map_err(|error| format!("Could not replace {}: {error}", original.relative_path))?;
    }

    for operation in &manifest.folder_case_renames {
        let source = resolve_workspace_directory(root, &operation.from_relative_path)?;
        let target = root.join(checked_relative_path(&operation.to_relative_path, false)?);
        rename_durable(&source, &target).map_err(|error| {
            format!(
                "Could not rename {} to {}: {error}",
                operation.from_relative_path, operation.to_relative_path
            )
        })?;
    }
    for relative_path in &manifest.created_directories {
        ensure_directory_path(root, relative_path)?;
    }
    for plan in plans.iter().filter(|plan| plan.needs_write) {
        let target = resolve_workspace_file(root, &plan.new_relative_path, true)?;
        if fs::symlink_metadata(&target).is_ok() {
            return Err(format!(
                "{} appeared while saving. It was not overwritten.",
                plan.new_relative_path
            ));
        }
        if let Some(parent) = target.parent() {
            ensure_existing_directory_without_symlink(root, parent)?;
        }
        atomic_write(&target, plan.content.as_bytes())
            .map_err(|error| format!("Could not save {}: {error}", plan.new_relative_path))?;
        if let Some(modified_at) = plan.preserved_modified_at {
            if let Err(error) = set_file_modified_millis(&target, modified_at) {
                warnings.push(format!(
                    "The note was restored, but the modified time for {} could not be preserved: \
                     {error}",
                    plan.new_relative_path,
                ));
            }
        }
    }
    Ok(())
}

pub(in crate::workspace) fn apply_recovery_targets(
    root: &Path,
    transaction_root: &Path,
    targets: &[TransactionRecoveryTarget],
) -> Result<(), String> {
    if targets.is_empty() {
        return Ok(());
    }
    ensure_recently_deleted_directory(root)?;

    for target in targets {
        let bytes = read_staged_recovery_snapshot(transaction_root, target)?;
        let destination = recently_deleted_snapshot_path(root, &target.id)?;
        match fs::symlink_metadata(&destination) {
            Ok(_) => {
                return Err(format!(
                    "A recovery snapshot already exists for {}.",
                    target.id,
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("Could not inspect a recovery snapshot: {error}"));
            }
        }
        atomic_write(&destination, &bytes)
            .map_err(|error| format!("Could not save a recovery snapshot: {error}"))?;
        if fingerprint_regular_file(&destination)? != Some(target.fingerprint.clone()) {
            return Err("A recovery snapshot changed while it was being saved.".to_owned());
        }
    }

    Ok(())
}

pub(in crate::workspace) fn finalize_committed_recovery_targets(
    root: &Path,
    transaction_root: &Path,
    targets: &[TransactionRecoveryTarget],
) -> Result<(), String> {
    if targets.is_empty() {
        return Ok(());
    }
    ensure_recently_deleted_directory(root)?;

    for target in targets {
        let destination = recently_deleted_snapshot_path(root, &target.id)?;
        match fingerprint_regular_file(&destination)? {
            Some(fingerprint) if fingerprint == target.fingerprint => continue,
            Some(_) => {
                return Err(format!(
                    "Recovery snapshot {} changed before its save was finalized.",
                    target.id,
                ));
            }
            None => {}
        }

        let bytes = read_staged_recovery_snapshot(transaction_root, target)?;
        atomic_write(&destination, &bytes)
            .map_err(|error| format!("Could not finalize a recovery snapshot: {error}"))?;
    }

    Ok(())
}

pub(in crate::workspace) fn read_staged_recovery_snapshot(
    transaction_root: &Path,
    target: &TransactionRecoveryTarget,
) -> Result<Vec<u8>, String> {
    validate_recently_deleted_id(&target.id)?;
    if target.fingerprint.length > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
        return Err("A staged recovery snapshot is unexpectedly large.".to_owned());
    }

    let transaction_metadata = fs::symlink_metadata(transaction_root)
        .map_err(|error| format!("Could not inspect a save transaction: {error}"))?;
    if transaction_metadata.file_type().is_symlink() || !transaction_metadata.is_dir() {
        return Err("The save transaction is not a regular folder.".to_owned());
    }

    let recovery_directory = transaction_root.join("recoveries");
    let recovery_metadata = fs::symlink_metadata(&recovery_directory)
        .map_err(|error| format!("Could not inspect staged recovery snapshots: {error}"))?;
    if recovery_metadata.file_type().is_symlink() || !recovery_metadata.is_dir() {
        return Err("The staged recovery snapshot path is not a regular folder.".to_owned());
    }

    let path = transaction_recovery_snapshot_path(transaction_root, &target.id)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Could not inspect a staged recovery snapshot: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("A staged recovery snapshot is not a regular file.".to_owned());
    }
    if metadata.len() != target.fingerprint.length {
        return Err("A staged recovery snapshot does not match its manifest.".to_owned());
    }

    let file = File::open(&path)
        .map_err(|error| format!("Could not open a staged recovery snapshot: {error}"))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect an open recovery snapshot: {error}"))?;
    if !opened_metadata.is_file() || opened_metadata.len() != target.fingerprint.length {
        return Err("A staged recovery snapshot changed while it was being opened.".to_owned());
    }
    let read_limit = target
        .fingerprint
        .length
        .checked_add(1)
        .ok_or_else(|| "A staged recovery snapshot is too large to read safely.".to_owned())?;
    let mut bytes = Vec::with_capacity(target.fingerprint.length as usize);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read a staged recovery snapshot: {error}"))?;
    if bytes.len() as u64 != target.fingerprint.length {
        return Err("A staged recovery snapshot changed while it was being read.".to_owned());
    }
    if fingerprint_bytes(&bytes) != target.fingerprint {
        return Err("A staged recovery snapshot failed its integrity check.".to_owned());
    }

    Ok(bytes)
}

pub(in crate::workspace) fn verify_applied_recovery_targets(
    root: &Path,
    targets: &[TransactionRecoveryTarget],
) -> Result<(), String> {
    for target in targets {
        let path = recently_deleted_snapshot_path(root, &target.id)?;
        if fingerprint_regular_file(&path)? != Some(target.fingerprint.clone()) {
            return Err(format!(
                "Recovery snapshot {} changed while the note was being archived.",
                target.id,
            ));
        }
    }

    Ok(())
}

pub(in crate::workspace) fn rollback_recovery_targets(
    root: &Path,
    targets: &[TransactionRecoveryTarget],
    warnings: &mut WarningCollector,
) -> bool {
    let mut recovered = true;
    for target in targets {
        let path = match recently_deleted_snapshot_path(root, &target.id) {
            Ok(path) => path,
            Err(error) => {
                warnings.push(error);
                recovered = false;
                continue;
            }
        };
        match fingerprint_regular_file(&path) {
            Ok(Some(fingerprint)) if fingerprint == target.fingerprint => {
                if let Err(error) = remove_file_durable(&path) {
                    warnings.push(format!(
                        "Could not remove an uncommitted recovery snapshot: {error}"
                    ));
                    recovered = false;
                }
            }
            Ok(Some(_)) => {
                warnings.push(format!(
                    "Did not remove recovery snapshot {} because it changed after the interrupted save.",
                    target.id,
                ));
                recovered = false;
            }
            Ok(None) => {}
            Err(error) => {
                warnings.push(error);
                recovered = false;
            }
        }
    }

    recovered
}

pub(in crate::workspace) fn resolve_transaction_target_file(
    root: &Path,
    target: &TransactionTarget,
    allow_missing: bool,
) -> Result<PathBuf, String> {
    match target.kind {
        TransactionTargetKind::Markdown => {
            resolve_workspace_file(root, &target.relative_path, allow_missing)
        }
        TransactionTargetKind::Image => {
            resolve_workspace_image_file(root, &target.relative_path, allow_missing)
        }
        TransactionTargetKind::Attachment => {
            resolve_workspace_asset_file(root, &target.relative_path, allow_missing)
        }
    }
}

pub(in crate::workspace) fn rollback_transaction(
    root: &Path,
    transaction_root: &Path,
    manifest: &TransactionManifest,
    warnings: &mut WarningCollector,
) -> bool {
    let mut recovered = rollback_recovery_targets(root, &manifest.recovery_targets, warnings);
    for target in manifest.targets.iter().rev() {
        if matches!(
            target.kind,
            TransactionTargetKind::Image | TransactionTargetKind::Attachment
        ) {
            match import_image_was_applied(transaction_root, target) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    warnings.push(error);
                    recovered = false;
                    continue;
                }
            }
        }
        let Ok(path) = resolve_transaction_target_file(root, target, true) else {
            recovered = false;
            continue;
        };
        match fingerprint_regular_file(&path) {
            Ok(Some(current)) if current == target.fingerprint => {
                if let Err(error) = remove_file_durable(&path) {
                    warnings.push(format!(
                        "Could not remove the partial save {}: {error}",
                        target.relative_path
                    ));
                    recovered = false;
                }
            }
            Ok(Some(_)) => {
                warnings.push(format!(
                    "Did not remove {} because it changed after the interrupted save.",
                    target.relative_path
                ));
                recovered = false;
            }
            Ok(None) => {}
            Err(error) => {
                warnings.push(error);
                recovered = false;
            }
        }
    }

    for (index, operation) in manifest.folder_case_renames.iter().enumerate().rev() {
        if !rollback_folder_case_rename(
            root,
            operation,
            &manifest.folder_case_renames[..index],
            warnings,
        ) {
            recovered = false;
        }
    }
    for original in &manifest.originals {
        let backup = match resolve_transaction_backup(transaction_root, original) {
            Ok(path) => path,
            Err(error) => {
                warnings.push(error);
                recovered = false;
                continue;
            }
        };
        let bytes = match fs::read(&backup) {
            Ok(bytes) if fingerprint_bytes(&bytes) == original.fingerprint => bytes,
            Ok(_) => {
                warnings.push(format!(
                    "The backup for {} did not match its manifest.",
                    original.relative_path
                ));
                recovered = false;
                continue;
            }
            Err(error) => {
                warnings.push(format!(
                    "Could not read the backup for {}: {error}",
                    original.relative_path
                ));
                recovered = false;
                continue;
            }
        };
        let original_path = match resolve_workspace_file(root, &original.relative_path, true) {
            Ok(path) => path,
            Err(error) => {
                warnings.push(error);
                recovered = false;
                continue;
            }
        };
        match fingerprint_regular_file(&original_path) {
            Ok(Some(current)) if current == original.fingerprint => {}
            Ok(Some(_)) => {
                warnings.push(format!(
                    "Did not restore {} because another file now occupies that path.",
                    original.relative_path
                ));
                recovered = false;
            }
            Ok(None) => {
                if let Some(parent) = original_path.parent() {
                    if let Err(error) = ensure_existing_directory_without_symlink(root, parent) {
                        warnings.push(error);
                        recovered = false;
                        continue;
                    }
                }
                if let Err(error) = atomic_write(&original_path, &bytes) {
                    warnings.push(format!(
                        "Could not restore {}: {error}",
                        original.relative_path
                    ));
                    recovered = false;
                }
            }
            Err(error) => {
                warnings.push(error);
                recovered = false;
            }
        }
    }
    remove_created_directories(root, &manifest.created_directories, warnings);
    recovered
}

pub(in crate::workspace) fn rollback_folder_case_rename(
    root: &Path,
    operation: &FolderCaseRename,
    prior_operations: &[FolderCaseRename],
    warnings: &mut WarningCollector,
) -> bool {
    let Ok(from) = resolve_workspace_directory(root, &operation.from_relative_path) else {
        return false;
    };
    let Ok(to_relative) = checked_relative_path(&operation.to_relative_path, false) else {
        return false;
    };
    let to = root.join(to_relative);
    let from_exists = from.is_dir();
    let to_exists = to.is_dir();
    if from_exists && !to_exists {
        return true;
    }
    if !to_exists {
        let original_relative =
            path_before_folder_renames(&operation.from_relative_path, prior_operations);
        if original_relative != operation.from_relative_path {
            if let Ok(original) = resolve_workspace_directory(root, &original_relative) {
                if fs::symlink_metadata(original)
                    .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
                    .unwrap_or(false)
                {
                    return true;
                }
            }
        }
        warnings.push(format!(
            "Could not find {} while recovering a folder rename.",
            operation.to_relative_path
        ));

        return false;
    }
    if from_exists {
        let same_location = from
            .canonicalize()
            .ok()
            .zip(to.canonicalize().ok())
            .is_some_and(|(left, right)| left == right);
        if !same_location {
            warnings.push(format!(
                "Did not restore {} because both folder names now exist.",
                operation.from_relative_path
            ));

            return false;
        }
    }
    if let Err(error) = rename_durable(&to, &from) {
        warnings.push(format!(
            "Could not restore folder {}: {error}",
            operation.from_relative_path
        ));

        return false;
    }
    true
}

pub(in crate::workspace) fn remove_created_directories(
    root: &Path,
    directories: &[String],
    warnings: &mut WarningCollector,
) {
    let mut directories = directories.to_vec();
    directories.sort_by_key(|path| std::cmp::Reverse(path.split('/').count()));
    for relative_path in directories {
        let Ok(path) = resolve_workspace_directory(root, &relative_path) else {
            continue;
        };
        match remove_directory_durable(&path) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    || error.kind() == io::ErrorKind::DirectoryNotEmpty => {}
            Err(error) => warnings.push(format!(
                "Could not remove temporary folder {relative_path}: {error}"
            )),
        }
    }
}

pub(in crate::workspace) fn recover_workspace_transactions(
    root: &Path,
    state: Option<&WorkspaceState>,
    warnings: &mut WarningCollector,
) -> Result<(), String> {
    recover_workspace_transactions_except(root, state, None, warnings)
}

pub(in crate::workspace) fn recover_workspace_transactions_except(
    root: &Path,
    state: Option<&WorkspaceState>,
    retained_transaction_id: Option<&str>,
    warnings: &mut WarningCollector,
) -> Result<(), String> {
    let transactions_root = root.join(STATE_DIRECTORY).join(TRANSACTIONS_DIRECTORY);
    let metadata = match fs::symlink_metadata(&transactions_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Could not inspect save transactions: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The save transactions path is not a regular folder.".to_owned());
    }
    let entries = fs::read_dir(&transactions_root)
        .map_err(|error| format!("Could not read save transactions: {error}"))?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("Could not inspect a save transaction: {error}"));
                continue;
            }
        };
        let transaction_root = entry.path();
        let metadata = match fs::symlink_metadata(&transaction_root) {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!("Could not inspect a save transaction: {error}"));
                continue;
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            warnings.push(format!(
                "Ignored an unsafe save transaction at {}.",
                transaction_root.display()
            ));
            continue;
        }
        let mut manifest = match read_transaction_manifest(&transaction_root) {
            Ok(Some(manifest)) => manifest,
            Ok(None) => {
                warnings.push(
                    "Removed an incomplete transaction that had not changed the vault.".to_owned(),
                );
                discard_private_transaction(root, &transaction_root, warnings);
                continue;
            }
            Err(error) => {
                warnings.push(error);
                continue;
            }
        };
        if manifest.version > TRANSACTION_VERSION {
            warnings.push(format!(
                "A save transaction uses unsupported version {} and was left untouched.",
                manifest.version
            ));
            continue;
        }
        if entry.file_name().to_string_lossy() != manifest.id {
            warnings.push(
                "A save transaction ID did not match its folder and was left untouched.".to_owned(),
            );
            continue;
        }
        if retained_transaction_id == Some(manifest.id.as_str()) {
            continue;
        }
        let committed = state.is_some_and(|state| {
            state.last_committed_transaction_id.as_deref() == Some(manifest.id.as_str())
                || state.last_committed_image_import_id.as_deref() == Some(manifest.id.as_str())
        });
        if committed && manifest.phase != TransactionPhase::Committed {
            manifest.phase = TransactionPhase::Committed;
            write_transaction_manifest(&transaction_root, &manifest).map_err(|error| {
                format!(
                    "A committed save could not be finalized safely. Repair permissions for {} and reopen the vault. {error}",
                    transaction_root.display()
                )
            })?;
        }
        if committed || manifest.phase == TransactionPhase::Committed {
            let recovery_targets = manifest
                .recovery_targets
                .iter()
                .filter(|target| {
                    state
                        .and_then(|state| state.recently_deleted_notes.get(&target.id))
                        .is_some_and(|stored| stored.fingerprint == target.fingerprint)
                })
                .cloned()
                .collect::<Vec<_>>();
            finalize_committed_recovery_targets(root, &transaction_root, &recovery_targets)?;
            discard_private_transaction(root, &transaction_root, warnings);
            warnings.push("Finished cleaning up a previously committed save.".to_owned());
            continue;
        }
        if manifest.phase == TransactionPhase::Prepared {
            discard_private_transaction(root, &transaction_root, warnings);
            continue;
        }
        let recovered = rollback_transaction(root, &transaction_root, &manifest, warnings);
        if recovered {
            discard_private_transaction(root, &transaction_root, warnings);
            warnings.push("Recovered an interrupted save without changing the vault.".to_owned());
        } else {
            return Err(format!(
                "An interrupted save could not be fully recovered. Its backups remain at {}. Resolve the conflicting files before reopening this vault.",
                transaction_root.display()
            ));
        }
    }
    Ok(())
}

pub(in crate::workspace) fn write_transaction_manifest(
    transaction_root: &Path,
    manifest: &TransactionManifest,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("Could not encode the save transaction: {error}"))?;
    if bytes.len() as u64 > MAX_TRANSACTION_MANIFEST_BYTES {
        return Err("The save transaction is too large to recover safely.".to_owned());
    }
    bytes.push(b'\n');
    atomic_write(&transaction_root.join(TRANSACTION_MANIFEST_FILE), &bytes)
        .map_err(|error| format!("Could not write the save transaction: {error}"))
}

pub(in crate::workspace) fn read_transaction_manifest(
    transaction_root: &Path,
) -> Result<Option<TransactionManifest>, String> {
    let path = transaction_root.join(TRANSACTION_MANIFEST_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not inspect a save manifest: {error}")),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_TRANSACTION_MANIFEST_BYTES
    {
        return Err("A save transaction manifest is unsafe or unexpectedly large.".to_owned());
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("Could not read a save transaction: {error}"))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("Could not parse a save transaction: {error}"))
}

pub(in crate::workspace) fn discard_private_transaction(
    root: &Path,
    transaction_root: &Path,
    warnings: &mut WarningCollector,
) {
    let expected_parent = root.join(STATE_DIRECTORY).join(TRANSACTIONS_DIRECTORY);
    if transaction_root.parent() != Some(expected_parent.as_path()) {
        warnings.push("Refused to clean a transaction outside the private save folder.".to_owned());

        return;
    }
    let mut entries = Vec::new();
    for entry in WalkDir::new(transaction_root)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
    {
        match entry {
            Ok(entry) => entries.push(entry),
            Err(error) => warnings.push(format!("Could not inspect transaction cleanup: {error}")),
        }
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.depth()));
    for entry in entries {
        let path = entry.path();
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                warnings.push(format!("Could not inspect {}: {error}", path.display()));
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            warnings.push(format!(
                "Refused to follow a symbolic link at {}.",
                path.display()
            ));
            continue;
        }
        let result = if metadata.is_dir() {
            remove_directory_durable(path)
        } else if metadata.is_file() {
            remove_file_durable(path)
        } else {
            continue;
        };
        if let Err(error) = result {
            if error.kind() != io::ErrorKind::DirectoryNotEmpty
                && error.kind() != io::ErrorKind::NotFound
            {
                warnings.push(format!("Could not clean {}: {error}", path.display()));
            }
        }
    }
    let _ = remove_directory_durable(&expected_parent);
}

pub(in crate::workspace) fn resolve_transaction_backup(
    transaction_root: &Path,
    original: &TransactionOriginal,
) -> Result<PathBuf, String> {
    let relative = checked_internal_transaction_path(&original.backup_relative_path, true)?;
    let path = transaction_root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Could not inspect a transaction backup: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("A transaction backup is not a regular file.".to_owned());
    }
    Ok(path)
}

pub(in crate::workspace) fn checked_internal_transaction_path(
    path: &str,
    file: bool,
) -> Result<PathBuf, String> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains('\\') {
        return Err("A transaction path is invalid.".to_owned());
    }
    let mut result = PathBuf::new();
    let components: Vec<&str> = path.split('/').collect();
    for (index, component) in components.iter().enumerate() {
        if component.is_empty() || *component == "." || *component == ".." {
            return Err("A transaction path contains unsafe segments.".to_owned());
        }
        if component.len() > 255 || component.chars().any(char::is_control) {
            return Err("A transaction path contains an unsupported segment.".to_owned());
        }
        if !file || index + 1 < components.len() {
            if component.contains('/') || component.contains('\\') {
                return Err("A transaction folder path is unsafe.".to_owned());
            }
        }
        result.push(component);
    }
    Ok(result)
}

pub(in crate::workspace) fn build_save_consistency(
    baseline: &BTreeMap<String, FileStamp>,
    paths_to_replace: &BTreeSet<String>,
    plans: &[NoteWritePlan],
) -> Result<SaveConsistency, String> {
    let mut unaffected = baseline.clone();
    for path in paths_to_replace {
        unaffected.remove(&portable_path_key(path));
    }
    let targets: Vec<TransactionTarget> = plans
        .iter()
        .filter(|plan| plan.needs_write)
        .map(|plan| TransactionTarget {
            relative_path: plan.new_relative_path.clone(),
            fingerprint: fingerprint_bytes(plan.content.as_bytes()),
            kind: TransactionTargetKind::Markdown,
        })
        .collect();
    for target in &targets {
        unaffected.remove(&portable_path_key(&target.relative_path));
    }
    Ok(SaveConsistency {
        unaffected,
        targets,
    })
}

pub(in crate::workspace) fn verify_save_consistency(
    root: &Path,
    expected: &SaveConsistency,
) -> Result<(), String> {
    let current = note_file_stamps(root)?;
    let mut expected_keys: HashSet<String> = expected.unaffected.keys().cloned().collect();
    for (path, stamp) in &expected.unaffected {
        if current.get(path) != Some(stamp) {
            return Err(
                "A Markdown file changed outside Obsidian At Home while the vault was being saved. Reload before editing again."
                    .to_owned(),
            );
        }
    }
    for target in &expected.targets {
        let key = portable_path_key(&target.relative_path);
        expected_keys.insert(key);
        let path = resolve_workspace_file(root, &target.relative_path, true)?;
        if fingerprint_regular_file(&path)? != Some(target.fingerprint.clone()) {
            return Err(format!(
                "{} changed outside Obsidian At Home while the vault was being saved. Reload before editing again.",
                target.relative_path
            ));
        }
    }
    let current_keys: HashSet<String> = current.keys().cloned().collect();
    if current_keys != expected_keys {
        return Err(
            "Markdown files were added, removed, or renamed outside Obsidian At Home while saving. Reload before editing again."
                .to_owned(),
        );
    }
    Ok(())
}

pub(in crate::workspace) fn note_file_stamps(
    root: &Path,
) -> Result<BTreeMap<String, FileStamp>, String> {
    let mut result = BTreeMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
        .filter_entry(should_visit_workspace_entry)
    {
        let entry = entry.map_err(|error| format!("Could not inspect the vault: {error}"))?;
        if entry.file_type().is_symlink()
            || !entry.file_type().is_file()
            || !is_markdown_path(entry.path())
        {
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
        if validate_markdown_relative_path(&relative_path).is_err() {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| format!("Could not inspect {relative_path}: {error}"))?;
        let fingerprint = fingerprint_regular_file(entry.path())?
            .ok_or_else(|| format!("{relative_path} disappeared while its content was read."))?;
        let stamp = FileStamp {
            length: fingerprint.length,
            modified_nanos: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0),
            content_hash: Some(fingerprint.hash),
        };
        if result
            .insert(portable_path_key(&relative_path), stamp)
            .is_some()
        {
            return Err(format!(
                "The vault contains paths that differ only by letter case near {relative_path}."
            ));
        }
    }
    Ok(result)
}

pub(in crate::workspace) fn fingerprint_regular_file(
    path: &Path,
) -> Result<Option<FileFingerprint>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not inspect {}: {error}", path.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{} is not a regular file.", path.display()));
    }
    let expected_modified = metadata.modified().ok();
    let mut file =
        File::open(path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if !opened_metadata.is_file()
        || opened_metadata.len() != metadata.len()
        || opened_metadata.modified().ok() != expected_modified
    {
        return Err(format!(
            "{} changed while it was being opened.",
            path.display()
        ));
    }
    let mut hash = 0xcbf29ce484222325_u64;
    let mut length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(read as u64)
            .ok_or_else(|| format!("{} is too large to fingerprint.", path.display()))?;
        fnv_update(&mut hash, &buffer[..read]);
    }
    let final_metadata = file
        .metadata()
        .map_err(|error| format!("Could not recheck {}: {error}", path.display()))?;
    if length != metadata.len()
        || final_metadata.len() != metadata.len()
        || final_metadata.modified().ok() != expected_modified
    {
        return Err(format!(
            "{} changed while it was being fingerprinted.",
            path.display()
        ));
    }
    Ok(Some(FileFingerprint { length, hash }))
}

pub(in crate::workspace) fn fingerprint_bytes(bytes: &[u8]) -> FileFingerprint {
    let mut hash = 0xcbf29ce484222325_u64;
    fnv_update(&mut hash, bytes);
    FileFingerprint {
        length: bytes.len() as u64,
        hash,
    }
}

pub(in crate::workspace) fn editor_positions_revision(fingerprint: &FileFingerprint) -> String {
    format!("{}:{:016x}", fingerprint.length, fingerprint.hash)
}

pub(in crate::workspace) fn portable_path_key(path: &str) -> String {
    path.to_lowercase()
}

pub(in crate::workspace) fn new_transaction_id() -> String {
    format!(
        "{}-{}-{}",
        now_millis(),
        std::process::id(),
        TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}
