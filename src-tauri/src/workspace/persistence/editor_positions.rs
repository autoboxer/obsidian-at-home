use super::*;

pub(in crate::workspace) fn load_editor_positions(
    root: &Path,
    note_ids: &HashSet<&str>,
    warnings: &mut WarningCollector,
) -> (BTreeMap<String, NoteEditorPosition>, bool, Option<String>) {
    let raw = match read_editor_positions(root) {
        Ok(EditorPositionsRead::Missing) => return (BTreeMap::new(), true, None),
        Ok(EditorPositionsRead::Invalid(error, fingerprint)) => {
            warnings.push(format!("Ignored saved editor positions: {error}"));
            let positions = BTreeMap::new();
            let revision = rewrite_editor_positions_if_unchanged(
                root,
                &positions,
                &fingerprint,
                warnings,
                "replace invalid saved editor positions",
            );

            return (positions, revision.is_some(), revision);
        }
        Ok(EditorPositionsRead::Newer(version, _)) => {
            warnings.push(format!(
                "Saved editor positions use version {version}, but this app supports up to version {EDITOR_POSITIONS_VERSION}. They were ignored and not changed."
            ));

            return (BTreeMap::new(), false, None);
        }
        Ok(EditorPositionsRead::Loaded(raw, fingerprint)) => (raw, fingerprint),
        Err(error) => {
            warnings.push(format!("Ignored saved editor positions: {error}"));

            return (BTreeMap::new(), false, None);
        }
    };
    let (raw, fingerprint) = raw;
    let decoded = decode_editor_positions(raw.positions, note_ids);
    if decoded.invalid_count > 0 {
        warnings.push(format!(
            "Ignored {} invalid saved editor position{}.",
            decoded.invalid_count,
            if decoded.invalid_count == 1 { "" } else { "s" },
        ));
    }
    if decoded.unknown_count > 0 {
        warnings.push(format!(
            "Ignored {} saved editor position{} for notes that no longer exist.",
            decoded.unknown_count,
            if decoded.unknown_count == 1 { "" } else { "s" },
        ));
    }
    let revision = if decoded.invalid_count > 0 || decoded.unknown_count > 0 {
        rewrite_editor_positions_if_unchanged(
            root,
            &decoded.positions,
            &fingerprint,
            warnings,
            "prune saved editor positions",
        )
    } else {
        Some(editor_positions_revision(&fingerprint))
    };

    (decoded.positions, revision.is_some(), revision)
}

pub(in crate::workspace) fn rewrite_editor_positions_if_unchanged(
    root: &Path,
    positions: &BTreeMap<String, NoteEditorPosition>,
    fingerprint: &FileFingerprint,
    warnings: &mut WarningCollector,
    action: &str,
) -> Option<String> {
    let _lock = match lock_editor_positions(root) {
        Ok(lock) => lock,
        Err(error) => {
            warnings.push(format!("Could not {action}: {error}"));

            return None;
        }
    };
    let unchanged = fingerprint_regular_file(&editor_positions_path(root))
        .is_ok_and(|current| current.as_ref() == Some(fingerprint));
    if !unchanged {
        warnings.push(format!(
            "Saved editor positions changed while the app tried to {action} and were left untouched."
        ));

        return None;
    }
    if let Err(error) = write_editor_positions(root, positions) {
        warnings.push(format!("Could not {action}: {error}"));

        return None;
    }

    fingerprint_regular_file(&editor_positions_path(root))
        .ok()
        .flatten()
        .as_ref()
        .map(editor_positions_revision)
}

pub(in crate::workspace) fn save_editor_positions(
    root: &Path,
    positions: BTreeMap<String, NoteEditorPosition>,
    expected_revision: Option<String>,
) -> Result<String, String> {
    let _lock = lock_editor_positions(root)?;
    validate_editor_positions(&positions)?;
    let state_path = workspace_state_path(root);
    let expected_state_fingerprint = fingerprint_regular_file(&state_path)?.ok_or_else(|| {
        "Workspace metadata is missing. Reopen the vault before saving editor positions.".to_owned()
    })?;
    let mut state_warnings = WarningCollector::default();
    let (state, state_file_was_present) = read_workspace_state(root, &mut state_warnings);
    let state = state.ok_or_else(|| {
        if state_file_was_present {
            "Workspace metadata is unreadable or newer than this app. Editor positions were not changed."
        } else {
            "Workspace metadata is missing. Reopen the vault before saving editor positions."
        }
        .to_owned()
    })?;
    if positions
        .keys()
        .any(|note_id| !state.note_paths.contains_key(note_id))
    {
        return Err(
            "Editor positions refer to notes that have not been saved yet. Try again.".to_owned(),
        );
    }
    let existing_positions = read_editor_positions(root)?;
    let expected_positions_fingerprint = existing_positions.fingerprint().cloned();
    match existing_positions {
        EditorPositionsRead::Missing => {}
        EditorPositionsRead::Newer(version, _) => {
            return Err(format!(
                "The existing editor positions use version {version}, but this app supports up to version {EDITOR_POSITIONS_VERSION}. Update the app before changing them."
            ));
        }
        EditorPositionsRead::Loaded(_, _) | EditorPositionsRead::Invalid(_, _) => {}
    }
    let current_revision = expected_positions_fingerprint
        .as_ref()
        .map(editor_positions_revision);
    if current_revision != expected_revision {
        return Err(
            "Editor positions changed in another app window. Reopen the vault before saving them."
                .to_owned(),
        );
    }
    if fingerprint_regular_file(&state_path)? != Some(expected_state_fingerprint) {
        return Err(
            "Workspace metadata changed while editor positions were being saved. Try again."
                .to_owned(),
        );
    }
    if fingerprint_regular_file(&editor_positions_path(root))? != expected_positions_fingerprint {
        return Err("Editor positions changed while they were being saved. Try again.".to_owned());
    }
    write_editor_positions(root, &positions)?;
    fingerprint_regular_file(&editor_positions_path(root))?
        .as_ref()
        .map(editor_positions_revision)
        .ok_or_else(|| "Editor positions disappeared after they were saved.".to_owned())
}

pub(in crate::workspace) fn read_editor_positions(
    root: &Path,
) -> Result<EditorPositionsRead, String> {
    let directory = root.join(STATE_DIRECTORY);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("the .obsidian-at-home folder is a symbolic link".to_owned());
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(".obsidian-at-home is not a folder".to_owned());
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(EditorPositionsRead::Missing);
        }
        Err(error) => {
            return Err(format!(
                "the .obsidian-at-home folder could not be inspected: {error}"
            ));
        }
    }

    let path = editor_positions_path(root);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(EditorPositionsRead::Missing);
        }
        Err(error) => {
            return Err(format!(
                "editor-positions.json could not be inspected: {error}"
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("editor-positions.json is not a regular file".to_owned());
    }
    if metadata.len() > MAX_EDITOR_POSITIONS_BYTES {
        return Err("editor-positions.json is unexpectedly large".to_owned());
    }

    let bytes = fs::read(&path)
        .map_err(|error| format!("editor-positions.json could not be read: {error}"))?;
    let fingerprint = fingerprint_bytes(&bytes);
    let raw = match serde_json::from_slice::<RawEditorPositions>(&bytes) {
        Ok(raw) => raw,
        Err(error) => {
            return Ok(EditorPositionsRead::Invalid(
                format!("editor-positions.json is invalid: {error}"),
                fingerprint,
            ));
        }
    };
    if raw.version == 0 {
        return Ok(EditorPositionsRead::Invalid(
            "editor-positions.json has an invalid version".to_owned(),
            fingerprint,
        ));
    }
    if raw.version > EDITOR_POSITIONS_VERSION {
        return Ok(EditorPositionsRead::Newer(raw.version, fingerprint));
    }
    Ok(EditorPositionsRead::Loaded(raw, fingerprint))
}

pub(in crate::workspace) fn decode_editor_positions(
    positions: BTreeMap<String, serde_json::Value>,
    note_ids: &HashSet<&str>,
) -> DecodedEditorPositions {
    let mut decoded = BTreeMap::new();
    let mut invalid_count = 0;
    let mut unknown_count = 0;

    for (note_id, value) in positions {
        if !note_ids.contains(note_id.as_str()) {
            unknown_count += 1;
            continue;
        }
        match serde_json::from_value::<NoteEditorPosition>(value) {
            Ok(position) if is_valid_editor_position(&position) => {
                decoded.insert(note_id, position);
            }
            _ => invalid_count += 1,
        }
    }

    DecodedEditorPositions {
        positions: decoded,
        invalid_count,
        unknown_count,
    }
}

pub(in crate::workspace) fn validate_editor_positions(
    positions: &BTreeMap<String, NoteEditorPosition>,
) -> Result<(), String> {
    if positions.len() > MAX_NOTES {
        return Err(format!(
            "A vault can store positions for at most {MAX_NOTES} notes."
        ));
    }
    if positions
        .iter()
        .any(|(note_id, position)| note_id.trim().is_empty() || !is_valid_editor_position(position))
    {
        return Err("Editor positions contain an invalid entry.".to_owned());
    }

    Ok(())
}

pub(in crate::workspace) fn is_valid_editor_position(position: &NoteEditorPosition) -> bool {
    let maximum = MAX_SAFE_JAVASCRIPT_INTEGER as f64;

    position.selection.anchor <= MAX_SAFE_JAVASCRIPT_INTEGER
        && position.selection.head <= MAX_SAFE_JAVASCRIPT_INTEGER
        && position.viewport.anchor <= MAX_SAFE_JAVASCRIPT_INTEGER
        && position.viewport.offset.is_finite()
        && position.viewport.offset.abs() <= maximum
        && position.viewport.left.is_finite()
        && (0.0..=maximum).contains(&position.viewport.left)
}

pub(in crate::workspace) fn write_editor_positions(
    root: &Path,
    positions: &BTreeMap<String, NoteEditorPosition>,
) -> Result<(), String> {
    let directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &directory)?;
    let stored = StoredEditorPositions {
        version: EDITOR_POSITIONS_VERSION,
        positions,
    };
    let mut bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|error| format!("Could not encode editor positions: {error}"))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_EDITOR_POSITIONS_BYTES {
        return Err("There are too many editor positions to save safely.".to_owned());
    }
    atomic_write(&editor_positions_path(root), &bytes)
        .map_err(|error| format!("Could not write editor positions: {error}"))
}
