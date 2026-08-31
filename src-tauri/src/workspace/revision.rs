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
            let modified_nanos = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            let needs_content_hash = revision_file_needs_content_hash(entry.path(), &relative);
            let (length, content_hash) = if needs_content_hash {
                let fingerprint = fingerprint_regular_file(entry.path())?.ok_or_else(|| {
                    format!(
                        "{} disappeared while its revision was being read.",
                        entry.path().display()
                    )
                })?;
                (fingerprint.length, Some(fingerprint.hash))
            } else {
                (metadata.len(), None)
            };
            entries.push((
                format!("F:{relative}"),
                Some(FileStamp {
                    length,
                    modified_nanos,
                    content_hash,
                }),
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
