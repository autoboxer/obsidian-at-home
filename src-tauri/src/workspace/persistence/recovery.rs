use super::*;

pub(in crate::workspace) fn prepare_note_archive(
    root: &Path,
    vault: &VaultData,
    old_state: &WorkspaceState,
    mut pending: PendingNoteArchive,
    deleted_at: u64,
) -> Result<PreparedNoteArchive, String> {
    if pending.note.id.trim().is_empty() {
        return Err("The deleted note has an invalid ID.".to_owned());
    }
    if pending.note.content.len() as u64 > MAX_NOTE_BYTES {
        return Err(format!(
            "The note {:?} is larger than {} MiB and cannot be archived.",
            pending.note.title,
            MAX_NOTE_BYTES / 1024 / 1024,
        ));
    }
    if pending
        .editor_position
        .as_ref()
        .is_some_and(|position| !is_valid_editor_position(position))
    {
        return Err("The deleted note has an invalid editor position.".to_owned());
    }
    if vault.notes.iter().any(|note| note.id == pending.note.id) {
        return Err("Remove the note from the live vault before archiving it.".to_owned());
    }

    let removed_note_ids = old_state
        .note_paths
        .keys()
        .filter(|id| !vault.notes.iter().any(|note| note.id == id.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    if removed_note_ids != [pending.note.id.as_str()] {
        return Err(
            "Exactly one saved note must be removed when creating a recovery snapshot.".to_owned(),
        );
    }

    let original_relative_path = old_state
        .note_paths
        .get(&pending.note.id)
        .ok_or_else(|| "The note must be saved before it can be archived.".to_owned())?;
    validate_markdown_relative_path(original_relative_path)?;
    let original_path = resolve_workspace_file(root, original_relative_path, false)?;
    let stored_content = fs::read_to_string(&original_path)
        .map_err(|error| format!("Could not read the note before archiving it: {error}"))?;
    let requested_content = content_with_requested_tags(&pending.note, Some(&stored_content))?;
    if requested_content.as_bytes() != stored_content.as_bytes() {
        return Err(
            "The note changed before it could be archived. Save it and try again.".to_owned(),
        );
    }
    pending.note.content = stored_content;
    let stored_folder_path = Path::new(original_relative_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    if pending.original_folder_path != stored_folder_path {
        return Err(
            "The note's original folder changed before it could be archived. Reload the vault and try again."
                .to_owned(),
        );
    }
    if !pending.original_folder_path.is_empty() {
        validate_relative_path(&pending.original_folder_path, false)?;
    }
    pending.note.relative_path = original_relative_path.clone();

    validate_recently_deleted_capacity(&old_state.recently_deleted_notes, 0)?;
    let id = new_recently_deleted_id(root, &old_state.recently_deleted_notes)?;
    let expires_at = deleted_at.saturating_add(RECENTLY_DELETED_RETENTION_MILLIS);
    let deleted_note = RecentlyDeletedNote {
        id,
        note: pending.note,
        original_folder_path: pending.original_folder_path,
        deleted_at,
        expires_at,
        editor_position: pending.editor_position,
    };
    let snapshot = RecentlyDeletedSnapshot {
        version: RECENTLY_DELETED_SNAPSHOT_VERSION,
        deleted_note: deleted_note.clone(),
    };
    let mut bytes = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| format!("Could not encode the recovery snapshot: {error}"))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
        return Err("The recovery snapshot is unexpectedly large.".to_owned());
    }
    validate_recently_deleted_capacity(&old_state.recently_deleted_notes, bytes.len() as u64)?;
    let fingerprint = fingerprint_bytes(&bytes);

    Ok(PreparedNoteArchive {
        deleted_note,
        bytes,
        fingerprint,
    })
}

pub(in crate::workspace) fn validate_recently_deleted_capacity(
    stored: &BTreeMap<String, StoredRecentlyDeletedNote>,
    additional_bytes: u64,
) -> Result<(), String> {
    if stored.len() >= MAX_RECENTLY_DELETED_NOTES && additional_bytes > 0 {
        return Err(format!(
            "Recently Deleted can contain at most {MAX_RECENTLY_DELETED_NOTES} notes."
        ));
    }

    let mut total_bytes = additional_bytes;
    for (id, entry) in stored {
        validate_recently_deleted_id(id)?;
        if entry.fingerprint.length > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
            return Err("A stored recovery snapshot is unexpectedly large.".to_owned());
        }
        total_bytes = total_bytes
            .checked_add(entry.fingerprint.length)
            .ok_or_else(|| "Recently Deleted is too large to measure safely.".to_owned())?;
    }
    if total_bytes > MAX_RECENTLY_DELETED_BYTES {
        return Err(format!(
            "Recently Deleted cannot contain more than {} MiB of note snapshots.",
            MAX_RECENTLY_DELETED_BYTES / 1024 / 1024,
        ));
    }

    Ok(())
}

pub(in crate::workspace) fn new_recently_deleted_id(
    root: &Path,
    stored: &BTreeMap<String, StoredRecentlyDeletedNote>,
) -> Result<String, String> {
    for _ in 0..100 {
        let id = format!("deleted-{}", new_transaction_id());
        if stored.contains_key(&id) {
            continue;
        }
        let path = recently_deleted_snapshot_path(root, &id)?;
        match fs::symlink_metadata(path) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(id),
            Err(error) => {
                return Err(format!(
                    "Could not inspect the recovery snapshot folder: {error}"
                ));
            }
        }
    }

    Err("Could not allocate a unique recovery snapshot ID.".to_owned())
}

pub(in crate::workspace) fn validate_recently_deleted_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 180
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("A recovery snapshot has an invalid ID.".to_owned());
    }

    Ok(())
}

pub(in crate::workspace) fn read_recovery_for_restore(
    root: &Path,
    deleted_note_id: &str,
    expected_revision: u64,
) -> Result<(WorkspaceState, RecentlyDeletedNote), String> {
    validate_recently_deleted_id(deleted_note_id)?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before restoring the note."
                .to_owned(),
        );
    }

    let state_path = workspace_state_path(root);
    let expected_state_fingerprint = fingerprint_regular_file(&state_path)?;
    let mut warnings = WarningCollector::default();
    let (state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    let state = state.ok_or_else(|| {
        if state_file_was_present {
            "Workspace metadata is unreadable or newer than this app.".to_owned()
        } else {
            "Workspace metadata is missing. Reopen the vault before restoring the note.".to_owned()
        }
    })?;
    recover_workspace_transactions(root, Some(&state), &mut warnings)?;
    if fingerprint_regular_file(&state_path)? != expected_state_fingerprint
        || revision_for_root(root)? != expected_revision
    {
        return Err(
            "The vault changed while the deleted note was being read. Reload it and try again."
                .to_owned(),
        );
    }

    inspect_recently_deleted_directory(root)?;
    let stored = state
        .recently_deleted_notes
        .get(deleted_note_id)
        .ok_or_else(|| "That deleted note is no longer available.".to_owned())?;
    if stored.expires_at <= now_millis() {
        return Err("That deleted note has expired and can no longer be restored.".to_owned());
    }
    let deleted_note = read_indexed_recently_deleted_note(root, deleted_note_id, stored)?;
    if fingerprint_regular_file(&state_path)? != expected_state_fingerprint
        || revision_for_root(root)? != expected_revision
    {
        return Err(
            "The vault changed while the deleted note was being read. Reload it and try again."
                .to_owned(),
        );
    }

    Ok((state, deleted_note))
}

pub(in crate::workspace) fn build_restored_note(
    root: &Path,
    vault: &VaultData,
    state: &WorkspaceState,
    deleted_note: &RecentlyDeletedNote,
) -> Result<(Note, String), String> {
    let folder_paths = build_folder_paths(&vault.folders)?;
    let existing_plans =
        build_note_write_plans(root, vault, state, &folder_paths, &BTreeMap::new())?;
    let original_folder_id = folder_paths
        .iter()
        .find_map(|(id, path)| (path == &deleted_note.original_folder_path).then(|| id.clone()));
    let target_folder_path = original_folder_id
        .as_ref()
        .and_then(|id| folder_paths.get(id))
        .map(String::as_str)
        .unwrap_or("");

    let original_path = Path::new(&deleted_note.note.relative_path);
    let original_stem = original_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Untitled note");
    let extension = original_path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| value.eq_ignore_ascii_case("md") || value.eq_ignore_ascii_case("markdown"))
        .unwrap_or("md");

    let mut occupied_paths = existing_plans
        .iter()
        .map(|plan| portable_path_key(&plan.new_relative_path))
        .collect::<HashSet<_>>();
    occupied_paths.extend(folder_paths.values().map(|path| portable_path_key(path)));
    occupied_paths.extend(note_file_stamps(root)?.into_keys());

    let mut preferred_relative_path = None;
    let mut restored_title = String::new();
    for suffix in 1..=MAX_NOTES {
        let title = if suffix == 1 {
            original_stem.to_owned()
        } else {
            format!("{original_stem} {suffix}")
        };
        let file_name = format!("{title}.{extension}");
        let candidate = if target_folder_path.is_empty() {
            file_name
        } else {
            format!("{target_folder_path}/{file_name}")
        };
        validate_markdown_relative_path(&candidate)?;
        if occupied_paths.contains(&portable_path_key(&candidate)) {
            continue;
        }
        let path = resolve_workspace_file(root, &candidate, true)?;
        match fs::symlink_metadata(path) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                restored_title = title;
                preferred_relative_path = Some(candidate);
                break;
            }
            Err(error) => {
                return Err(format!(
                    "Could not inspect the restore destination: {error}"
                ));
            }
        }
    }
    let preferred_relative_path = preferred_relative_path
        .ok_or_else(|| "Could not find a safe file name for the restored note.".to_owned())?;

    let mut used_ids = vault
        .notes
        .iter()
        .map(|note| note.id.clone())
        .collect::<HashSet<_>>();
    let restored_id = if used_ids.insert(deleted_note.note.id.clone()) {
        deleted_note.note.id.clone()
    } else {
        fresh_id("note", &preferred_relative_path, &mut used_ids)
    };
    let mut restored_note = deleted_note.note.clone();
    restored_note.id = restored_id;
    restored_note.title = restored_title;
    restored_note.folder_id = original_folder_id;
    restored_note.relative_path = preferred_relative_path.clone();

    Ok((restored_note, preferred_relative_path))
}

pub(in crate::workspace) fn prepare_note_restore(
    root: &Path,
    vault: &VaultData,
    old_state: &WorkspaceState,
    folder_paths: &BTreeMap<String, String>,
    pending: PendingNoteRestore,
) -> Result<PreparedNoteRestore, String> {
    validate_recently_deleted_id(&pending.deleted_note_id)?;
    inspect_recently_deleted_directory(root)?;
    let stored = old_state
        .recently_deleted_notes
        .get(&pending.deleted_note_id)
        .ok_or_else(|| "That deleted note is no longer available.".to_owned())?;
    let deleted_note = read_indexed_recently_deleted_note(root, &pending.deleted_note_id, stored)?;

    let restored = &pending.restored_note;
    if restored.content != deleted_note.note.content
        || restored.tags != deleted_note.note.tags
        || restored.pinned != deleted_note.note.pinned
        || restored.created_at != deleted_note.note.created_at
        || restored.updated_at != deleted_note.note.updated_at
    {
        return Err("The restored note changed before it could be saved.".to_owned());
    }
    if restored.relative_path != pending.preferred_relative_path {
        return Err("The restored note path changed before it could be saved.".to_owned());
    }
    let restored_folder_path = match restored.folder_id.as_deref() {
        Some(folder_id) => folder_paths
            .get(folder_id)
            .ok_or_else(|| "The restored note folder no longer exists.".to_owned())?
            .as_str(),
        None => "",
    };
    let path_folder = Path::new(&restored.relative_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    if path_folder != restored_folder_path {
        return Err("The restored note path does not match its folder.".to_owned());
    }

    let old_ids = old_state
        .note_paths
        .keys()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if old_ids
        .iter()
        .any(|id| !vault.notes.iter().any(|note| note.id == **id))
    {
        return Err("Restore the note without removing another live note.".to_owned());
    }
    let new_notes = vault
        .notes
        .iter()
        .filter(|note| !old_ids.contains(note.id.as_str()))
        .collect::<Vec<_>>();
    if new_notes.len() != 1 || new_notes[0] != restored {
        return Err("Exactly one recovery snapshot must be restored at a time.".to_owned());
    }

    Ok(PreparedNoteRestore {
        restored_note: restored.clone(),
        editor_position: deleted_note.editor_position,
        recovery_id: pending.deleted_note_id,
        fingerprint: stored.fingerprint.clone(),
    })
}

pub(in crate::workspace) fn load_recently_deleted_notes(
    root: &Path,
    stored: &BTreeMap<String, StoredRecentlyDeletedNote>,
    warnings: &mut WarningCollector,
) -> Vec<RecentlyDeletedNote> {
    if stored.is_empty() {
        return Vec::new();
    }
    if let Err(error) = inspect_recently_deleted_directory(root) {
        warnings.push(error);

        return Vec::new();
    }

    let mut entries = stored.iter().collect::<Vec<_>>();
    entries.sort_by(|(left_id, left), (right_id, right)| {
        right
            .deleted_at
            .cmp(&left.deleted_at)
            .then_with(|| right_id.cmp(left_id))
    });
    if entries.len() > MAX_RECENTLY_DELETED_NOTES {
        warnings.push(format!(
            "Only the newest {MAX_RECENTLY_DELETED_NOTES} recovery snapshots were loaded."
        ));
        entries.truncate(MAX_RECENTLY_DELETED_NOTES);
    }

    let mut deleted_notes = Vec::with_capacity(entries.len());
    let mut total_bytes = 0_u64;
    for (id, entry) in entries {
        if validate_recently_deleted_id(id).is_err() {
            warnings.push("Ignored a recovery snapshot with an invalid ID.".to_owned());
            continue;
        }
        if entry.expires_at
            != entry
                .deleted_at
                .saturating_add(RECENTLY_DELETED_RETENTION_MILLIS)
        {
            warnings.push(format!(
                "Ignored recovery snapshot {id} because its retention period is invalid."
            ));
            continue;
        }
        if entry.fingerprint.length > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
            warnings.push(format!(
                "Ignored recovery snapshot {id} because it is unexpectedly large."
            ));
            continue;
        }
        let Some(next_total) = total_bytes.checked_add(entry.fingerprint.length) else {
            warnings.push(
                "Stopped loading recovery snapshots because their size overflowed.".to_owned(),
            );
            break;
        };
        if next_total > MAX_RECENTLY_DELETED_BYTES {
            warnings.push(format!(
                "Stopped after reading {} MiB of recovery snapshots.",
                MAX_RECENTLY_DELETED_BYTES / 1024 / 1024,
            ));
            break;
        }
        total_bytes = next_total;

        match read_indexed_recently_deleted_note(root, id, entry) {
            Ok(deleted_note) => deleted_notes.push(deleted_note),
            Err(error) => warnings.push(error),
        }
    }

    deleted_notes
}

pub(in crate::workspace) fn read_indexed_recently_deleted_note(
    root: &Path,
    id: &str,
    entry: &StoredRecentlyDeletedNote,
) -> Result<RecentlyDeletedNote, String> {
    validate_recently_deleted_id(id)?;
    if entry.expires_at
        != entry
            .deleted_at
            .saturating_add(RECENTLY_DELETED_RETENTION_MILLIS)
    {
        return Err(format!(
            "Recovery snapshot {id} has an invalid retention period."
        ));
    }
    if entry.fingerprint.length > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
        return Err(format!("Recovery snapshot {id} is unexpectedly large."));
    }

    let path = recently_deleted_snapshot_path(root, id)?;
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            format!("Recovery snapshot {id} is missing.")
        } else {
            format!("Could not inspect recovery snapshot {id}: {error}")
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("Recovery snapshot {id} is not a regular file."));
    }
    if metadata.len() != entry.fingerprint.length {
        return Err(format!(
            "Recovery snapshot {id} does not match its metadata."
        ));
    }

    let file = File::open(&path)
        .map_err(|error| format!("Could not open recovery snapshot {id}: {error}"))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect recovery snapshot {id}: {error}"))?;
    if !opened_metadata.is_file() || opened_metadata.len() != entry.fingerprint.length {
        return Err(format!(
            "Recovery snapshot {id} changed while it was being opened."
        ));
    }
    let read_limit = entry
        .fingerprint
        .length
        .checked_add(1)
        .ok_or_else(|| format!("Recovery snapshot {id} is too large to read safely."))?;
    let mut bytes = Vec::with_capacity(entry.fingerprint.length as usize);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read recovery snapshot {id}: {error}"))?;
    if bytes.len() as u64 != entry.fingerprint.length
        || fingerprint_bytes(&bytes) != entry.fingerprint
    {
        return Err(format!(
            "Recovery snapshot {id} failed its integrity check."
        ));
    }

    let snapshot = serde_json::from_slice::<RecentlyDeletedSnapshot>(&bytes)
        .map_err(|error| format!("Could not parse recovery snapshot {id}: {error}"))?;
    if snapshot.version == 0 || snapshot.version > RECENTLY_DELETED_SNAPSHOT_VERSION {
        return Err(format!(
            "Recovery snapshot {id} uses unsupported version {}.",
            snapshot.version,
        ));
    }
    validate_loaded_recently_deleted_note(id, entry, &snapshot.deleted_note)?;

    Ok(snapshot.deleted_note)
}

pub(in crate::workspace) fn verify_recovery_snapshot_target(
    root: &Path,
    id: &str,
    fingerprint: &FileFingerprint,
) -> Result<(), String> {
    let path = recently_deleted_snapshot_path(root, id)?;
    if fingerprint_regular_file(&path)? != Some(fingerprint.clone()) {
        return Err(format!(
            "Recovery snapshot {id} changed while the operation was being prepared."
        ));
    }

    Ok(())
}

pub(in crate::workspace) fn remove_recovery_snapshot_if_matches(
    root: &Path,
    id: &str,
    fingerprint: &FileFingerprint,
    warnings: &mut WarningCollector,
) -> bool {
    match delete_recovery_snapshot_if_matches(root, id, fingerprint) {
        Ok(RecoverySnapshotRemoval::Removed | RecoverySnapshotRemoval::AlreadyMissing) => true,
        Ok(RecoverySnapshotRemoval::RemovedWithoutDurability(error)) => {
            warnings.push(format!(
                "Recovery snapshot {id} was removed, but its deletion could not be made fully \
                 durable: {error}"
            ));
            true
        }
        Err(error) => {
            warnings.push(format!(
                "The recovery entry was removed, but snapshot {id} could not be cleaned up: \
                 {error}"
            ));
            false
        }
    }
}

pub(in crate::workspace) fn delete_recovery_snapshot_if_matches(
    root: &Path,
    id: &str,
    fingerprint: &FileFingerprint,
) -> Result<RecoverySnapshotRemoval, String> {
    let directory = root.join(STATE_DIRECTORY).join(RECENTLY_DELETED_DIRECTORY);
    match inspect_recently_deleted_directory(root) {
        Ok(_) => {}
        Err(_)
            if fs::symlink_metadata(&directory)
                .is_err_and(|error| error.kind() == io::ErrorKind::NotFound) =>
        {
            return Ok(RecoverySnapshotRemoval::AlreadyMissing)
        }
        Err(error) => return Err(error),
    }
    let path = recently_deleted_snapshot_path(root, id)?;
    match fingerprint_regular_file(&path)? {
        Some(current) if current == *fingerprint => match remove_file_durable(&path) {
            Ok(()) => Ok(RecoverySnapshotRemoval::Removed),
            Err(error) => classify_recovery_snapshot_removal_error(&path, id, error),
        },
        Some(_) => Err(format!(
            "Recovery snapshot {id} changed during cleanup and was left untouched."
        )),
        None => Ok(RecoverySnapshotRemoval::AlreadyMissing),
    }
}

pub(in crate::workspace) fn classify_recovery_snapshot_removal_error(
    path: &Path,
    id: &str,
    removal_error: io::Error,
) -> Result<RecoverySnapshotRemoval, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(
            RecoverySnapshotRemoval::RemovedWithoutDurability(removal_error.to_string()),
        ),
        Ok(_) => Err(format!(
            "Could not remove recovery snapshot {id}: {removal_error}"
        )),
        Err(error) => Err(format!(
            "Could not remove recovery snapshot {id}: {removal_error}. Its path could not be \
             checked afterward: {error}"
        )),
    }
}

pub(in crate::workspace) fn remove_expired_recovery_snapshot(
    root: &Path,
    id: &str,
    entry: &StoredRecentlyDeletedNote,
    warnings: &mut WarningCollector,
) -> bool {
    let directory = root.join(STATE_DIRECTORY).join(RECENTLY_DELETED_DIRECTORY);
    match inspect_recently_deleted_directory(root) {
        Ok(_) => {}
        Err(_)
            if fs::symlink_metadata(&directory)
                .is_err_and(|error| error.kind() == io::ErrorKind::NotFound) =>
        {
            warnings.push(format!(
                "Finished removing expired recovery entry {id} after its snapshot was already \
                 cleaned up."
            ));

            return true;
        }
        Err(error) => {
            warnings.push(format!(
                "Expired recovery entry {id} remains recoverable because cleanup could not start: \
                 {error}"
            ));

            return false;
        }
    }

    match read_indexed_recently_deleted_note(root, id, entry) {
        Ok(_) => {}
        Err(error) => {
            let path = match recently_deleted_snapshot_path(root, id) {
                Ok(path) => path,
                Err(path_error) => {
                    warnings.push(path_error);

                    return false;
                }
            };
            if fs::symlink_metadata(path)
                .is_err_and(|metadata_error| metadata_error.kind() == io::ErrorKind::NotFound)
            {
                warnings.push(format!(
                    "Finished removing expired recovery entry {id} after its snapshot was already \
                     cleaned up."
                ));

                return true;
            }
            warnings.push(format!(
                "Expired recovery entry {id} was retained because it could not be verified: \
                 {error}"
            ));

            return false;
        }
    }

    match delete_recovery_snapshot_if_matches(root, id, &entry.fingerprint) {
        Ok(RecoverySnapshotRemoval::Removed | RecoverySnapshotRemoval::AlreadyMissing) => true,
        Ok(RecoverySnapshotRemoval::RemovedWithoutDurability(error)) => {
            warnings.push(format!(
                "Expired recovery snapshot {id} was removed, but its deletion could not be made \
                 fully durable: {error}"
            ));
            true
        }
        Err(error) => {
            warnings.push(format!(
                "Expired recovery entry {id} remains recoverable because its snapshot could not \
                 be removed: {error}"
            ));
            false
        }
    }
}

pub(in crate::workspace) fn remove_recently_deleted_notes(
    root: &Path,
    requested_ids: Vec<String>,
    expected_revision: u64,
    expired_only: bool,
) -> Result<WorkspaceRecoveryMutationResult, String> {
    if !expired_only && requested_ids.is_empty() {
        return Err("Choose at least one deleted note to remove.".to_owned());
    }
    if requested_ids.len() > MAX_RECENTLY_DELETED_NOTES {
        return Err("Too many recovery entries were requested at once.".to_owned());
    }
    let mut requested = HashSet::new();
    for id in &requested_ids {
        validate_recently_deleted_id(id)?;
        if !requested.insert(id.clone()) {
            return Err("A recovery entry was requested more than once.".to_owned());
        }
    }

    let mut warnings = WarningCollector::default();
    let state_path = workspace_state_path(root);
    let expected_state_fingerprint = fingerprint_regular_file(&state_path)?;
    let (state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    let mut state = state.ok_or_else(|| {
        if state_file_was_present {
            "Workspace metadata is unreadable or newer than this app.".to_owned()
        } else {
            "Workspace metadata is missing. Reopen the vault and try again.".to_owned()
        }
    })?;
    recover_workspace_transactions(root, Some(&state), &mut warnings)?;
    if revision_for_root(root)? != expected_revision
        || fingerprint_regular_file(&state_path)? != expected_state_fingerprint
    {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before changing Recently Deleted."
                .to_owned(),
        );
    }

    if !expired_only && !state.recently_deleted_notes.is_empty() {
        inspect_recently_deleted_directory(root)?;
    }
    let now = now_millis();
    let candidate_ids = if expired_only {
        state
            .recently_deleted_notes
            .iter()
            .filter_map(|(id, entry)| (now >= entry.expires_at).then(|| id.clone()))
            .collect::<Vec<_>>()
    } else {
        requested_ids
    };
    let mut removals = Vec::new();
    for id in candidate_ids {
        let Some(entry) = state.recently_deleted_notes.get(&id) else {
            if expired_only {
                continue;
            }

            return Err(format!("Recovery entry {id} is no longer available."));
        };
        if expired_only {
            removals.push((id, entry.clone()));
        } else {
            match read_indexed_recently_deleted_note(root, &id, entry) {
                Ok(_) => removals.push((id, entry.clone())),
                Err(error) => return Err(error),
            }
        }
    }

    if fingerprint_regular_file(&state_path)? != expected_state_fingerprint
        || revision_for_root(root)? != expected_revision
    {
        return Err(
            "The vault changed while Recently Deleted was being updated. Reload it and try again."
                .to_owned(),
        );
    }
    if expired_only {
        removals
            .retain(|(id, entry)| remove_expired_recovery_snapshot(root, id, entry, &mut warnings));
    } else {
        for (id, entry) in &removals {
            verify_recovery_snapshot_target(root, id, &entry.fingerprint)?;
        }
    }

    let saved_at = now_millis();
    let removed_ids = removals
        .iter()
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    if !removed_ids.is_empty() {
        state.version = STATE_VERSION;
        for id in &removed_ids {
            state.recently_deleted_notes.remove(id);
        }
        write_workspace_state(root, &state)?;
    }
    let mut protected_ids = HashSet::new();
    if !expired_only && !removed_ids.is_empty() {
        for (id, entry) in &removals {
            if !remove_recovery_snapshot_if_matches(root, id, &entry.fingerprint, &mut warnings) {
                protected_ids.insert(id.clone());
            }
        }
    }

    cleanup_orphaned_recovery_snapshots(
        root,
        &state.recently_deleted_notes,
        &protected_ids,
        &mut warnings,
    );
    let revision = revision_for_root(root)?;

    Ok(WorkspaceRecoveryMutationResult {
        removed_ids,
        revision,
        saved_at,
        warnings: warnings.finish(),
    })
}

pub(in crate::workspace) fn cleanup_orphaned_recovery_snapshots(
    root: &Path,
    indexed: &BTreeMap<String, StoredRecentlyDeletedNote>,
    protected_ids: &HashSet<String>,
    warnings: &mut WarningCollector,
) {
    let directory = match inspect_recently_deleted_directory(root) {
        Ok(directory) => directory,
        Err(error) => {
            if fs::symlink_metadata(root.join(STATE_DIRECTORY).join(RECENTLY_DELETED_DIRECTORY))
                .is_ok()
            {
                warnings.push(error);
            }

            return;
        }
    };
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!(
                "Could not inspect Recently Deleted cleanup: {error}"
            ));

            return;
        }
    };
    for entry in entries.take(MAX_RECENTLY_DELETED_NOTES.saturating_mul(2)) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("Could not inspect a recovery snapshot: {error}"));
                continue;
            }
        };
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(id) = file_name.strip_suffix(".snapshot") else {
            continue;
        };
        if validate_recently_deleted_id(id).is_err()
            || indexed.contains_key(id)
            || protected_ids.contains(id)
        {
            continue;
        }
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!(
                    "Could not inspect orphaned recovery snapshot {id}: {error}"
                ));
                continue;
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            warnings.push(format!(
                "Orphaned recovery snapshot {id} was left untouched because it is not a regular \
                 file."
            ));
            continue;
        }
        if let Err(error) = remove_file_durable(&path) {
            warnings.push(format!(
                "Could not clean orphaned recovery snapshot {id}: {error}"
            ));
        }
    }
}

pub(in crate::workspace) fn validate_loaded_recently_deleted_note(
    id: &str,
    stored: &StoredRecentlyDeletedNote,
    deleted_note: &RecentlyDeletedNote,
) -> Result<(), String> {
    if deleted_note.id != id
        || deleted_note.deleted_at != stored.deleted_at
        || deleted_note.expires_at != stored.expires_at
        || deleted_note.note.id.trim().is_empty()
        || deleted_note.note.content.len() as u64 > MAX_NOTE_BYTES
    {
        return Err(format!(
            "Recovery snapshot {id} contains invalid note metadata."
        ));
    }
    validate_markdown_relative_path(&deleted_note.note.relative_path)
        .map_err(|_| format!("Recovery snapshot {id} contains an unsafe original note path."))?;
    let expected_folder_path = Path::new(&deleted_note.note.relative_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    if deleted_note.original_folder_path != expected_folder_path {
        return Err(format!(
            "Recovery snapshot {id} does not match its original folder."
        ));
    }
    if deleted_note
        .editor_position
        .as_ref()
        .is_some_and(|position| !is_valid_editor_position(position))
    {
        return Err(format!(
            "Recovery snapshot {id} contains an invalid editor position."
        ));
    }

    Ok(())
}
