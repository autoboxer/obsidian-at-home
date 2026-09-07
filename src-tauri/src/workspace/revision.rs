use super::*;

pub(super) type RevisionEntry = (String, Option<FileStamp>);

pub(super) fn revision_for_root(root: &Path) -> Result<u64, String> {
    revision_entries_for_root(root).map(|entries| revision_for_entries(&entries))
}

pub(super) fn revision_entries_for_root(root: &Path) -> Result<Vec<RevisionEntry>, String> {
    let mut entries = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
        .filter_entry(should_visit_revision_entry)
    {
        let entry = entry.map_err(|error| format!("Could not inspect the vault: {error}"))?;
        if entry.depth() == 0 || entry.file_type().is_symlink() {
            continue;
        }
        let Some(relative) = entry
            .path()
            .strip_prefix(root)
            .ok()
            .and_then(path_to_slash_string)
        else {
            continue;
        };
        if entry.file_type().is_dir() && relative != STATE_DIRECTORY {
            entries.push((format!("D:{relative}"), None));
        } else if entry.file_type().is_file() {
            let metadata = entry.metadata().map_err(|error| {
                format!("Could not inspect {}: {error}", entry.path().display())
            })?;
            let needs_content_hash = revision_file_needs_content_hash(entry.path(), &relative);
            let fingerprint = if needs_content_hash {
                let fingerprint = fingerprint_regular_file(entry.path())?.ok_or_else(|| {
                    format!(
                        "{} disappeared while its revision was being read.",
                        entry.path().display()
                    )
                })?;
                Some(fingerprint)
            } else {
                None
            };
            entries.push((
                format!("F:{relative}"),
                Some(revision_file_stamp(&metadata, fingerprint)),
            ));
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    Ok(entries)
}

pub(super) fn revision_for_entries(entries: &[RevisionEntry]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for (label, metadata) in entries {
        fnv_update(&mut hash, label.as_bytes());
        fnv_update(&mut hash, &[0]);
        if let Some(stamp) = metadata {
            fnv_update(&mut hash, &stamp.length.to_le_bytes());
            fnv_update(&mut hash, &stamp.modified_nanos.to_le_bytes());
            match stamp.content_hash {
                Some(content_hash) => {
                    fnv_update(&mut hash, &[1]);
                    fnv_update(&mut hash, &content_hash.to_le_bytes());
                }
                None => fnv_update(&mut hash, &[0]),
            }
        }
        fnv_update(&mut hash, &[0xff]);
    }
    let revision = hash & MAX_SAFE_JAVASCRIPT_INTEGER;
    if revision == 0 {
        1
    } else {
        revision
    }
}

fn revision_file_needs_content_hash(path: &Path, relative_path: &str) -> bool {
    is_markdown_path(path) || relative_path == format!("{STATE_DIRECTORY}/{STATE_FILE}")
}

pub(super) fn fnv_update(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

pub(super) fn revision_file_stamp(
    metadata: &fs::Metadata,
    fingerprint: Option<FileFingerprint>,
) -> FileStamp {
    FileStamp {
        length: fingerprint
            .as_ref()
            .map_or(metadata.len(), |value| value.length),
        modified_nanos: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0),
        content_hash: fingerprint.map(|value| value.hash),
    }
}

pub(super) fn workspace_load_changed() -> String {
    "The vault changed while it was being opened. Reopen it to load the latest changes.".to_owned()
}

fn revision_entry_stamp<'a>(entries: &'a [RevisionEntry], label: &str) -> Option<&'a FileStamp> {
    entries
        .binary_search_by(|entry| entry.0.as_str().cmp(label))
        .ok()
        .and_then(|index| entries[index].1.as_ref())
}

fn stamp_matches_fingerprint(stamp: Option<&FileStamp>, fingerprint: &FileFingerprint) -> bool {
    stamp.is_some_and(|stamp| {
        stamp.length == fingerprint.length && stamp.content_hash == Some(fingerprint.hash)
    })
}

pub(super) fn workspace_state_revision_fingerprint(
    entries: &[RevisionEntry],
) -> Option<FileFingerprint> {
    let stamp = revision_entry_stamp(entries, &format!("F:{STATE_DIRECTORY}/{STATE_FILE}"))?;
    Some(FileFingerprint {
        length: stamp.length,
        hash: stamp.content_hash?,
    })
}

pub(super) fn workspace_state_matches_revision(
    entries: &[RevisionEntry],
    fingerprint: &FileFingerprint,
) -> bool {
    stamp_matches_fingerprint(
        revision_entry_stamp(entries, &format!("F:{STATE_DIRECTORY}/{STATE_FILE}")),
        fingerprint,
    )
}

pub(super) fn verify_workspace_load_reads(
    baseline: &[RevisionEntry],
    scanned: &ScannedWorkspace,
    state_fingerprint: Option<&FileFingerprint>,
    state_file_was_present: bool,
) -> Result<(), String> {
    // A second filesystem read alone cannot prove that the bytes used by the
    // loader matched its baseline: a file may have changed and changed back.
    let state_stamp = revision_entry_stamp(baseline, &format!("F:{STATE_DIRECTORY}/{STATE_FILE}"));
    if state_fingerprint
        .is_some_and(|fingerprint| !stamp_matches_fingerprint(state_stamp, fingerprint))
        || (!state_file_was_present && state_stamp.is_some())
        || !baseline
            .iter()
            .filter(|(label, _)| is_workspace_content_revision_entry(label))
            .eq(scanned.revision_entries.iter())
    {
        return Err(workspace_load_changed());
    }
    Ok(())
}

fn is_workspace_content_revision_entry(label: &str) -> bool {
    let relative = &label[2..];
    let directories = if label.starts_with("D:") {
        relative
    } else {
        relative.rsplit_once('/').map_or("", |(parent, _)| parent)
    };
    !directories.split('/').any(is_reserved_workspace_directory)
}

pub(super) fn verify_workspace_load_revision(
    root: &Path,
    baseline: &[RevisionEntry],
    written_state: Option<&FileFingerprint>,
) -> Result<u64, String> {
    let current = revision_entries_for_root(root)?;
    let unchanged = if let Some(fingerprint) = written_state {
        let state_label = format!("F:{STATE_DIRECTORY}/{STATE_FILE}");
        stamp_matches_fingerprint(revision_entry_stamp(&current, &state_label), fingerprint)
            && baseline
                .iter()
                .filter(|entry| entry.0 != state_label)
                .eq(current.iter().filter(|entry| entry.0 != state_label))
    } else {
        current == baseline
    };
    if !unchanged {
        return Err(workspace_load_changed());
    }
    Ok(revision_for_entries(&current))
}
