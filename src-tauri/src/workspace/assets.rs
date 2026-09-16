use super::*;

mod deletion;
mod embedding;
mod external_upload;
pub(crate) mod files;
mod imports;
mod relocation;

pub(in crate::workspace) use deletion::*;
pub(in crate::workspace) use embedding::*;
pub(in crate::workspace) use external_upload::*;
pub(in crate::workspace) use files::*;
pub(in crate::workspace) use imports::*;
pub(in crate::workspace) use relocation::*;

pub(in crate::workspace) const MAX_VAULT_ASSETS: usize = 100_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(in crate::workspace) enum VaultAssetKind {
    Image,
    Attachment,
}

impl Default for VaultAssetKind {
    fn default() -> Self {
        Self::Image
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(in crate::workspace) struct StoredVaultAsset {
    #[serde(default)]
    pub(in crate::workspace) kind: VaultAssetKind,
    pub(in crate::workspace) relative_path: String,
    pub(in crate::workspace) media_type: String,
    pub(in crate::workspace) fingerprint: FileFingerprint,
    #[serde(default)]
    pub(in crate::workspace) modified_nanos: u64,
}

#[derive(Debug)]
pub(in crate::workspace) struct PreparedAssetNoteUpdate {
    path: PathBuf,
    expected_content: Vec<u8>,
    content: Vec<u8>,
}

pub(in crate::workspace) fn prepare_asset_note_updates(
    root: &Path,
    state: &WorkspaceState,
    note_updates: &[WorkspaceImageNoteUpdate],
    asset_label: &str,
) -> Result<Vec<PreparedAssetNoteUpdate>, String> {
    if note_updates.len() > MAX_NOTES {
        return Err(format!("Only {MAX_NOTES} notes can be updated at once."));
    }
    let mut seen_note_ids = HashSet::new();
    let mut seen_paths = HashSet::new();
    let mut total_bytes = 0_u64;
    let mut prepared = Vec::with_capacity(note_updates.len());
    for update in note_updates {
        validate_markdown_relative_path(&update.relative_path)?;
        if state.note_paths.get(&update.note_id).map(String::as_str)
            != Some(update.relative_path.as_str())
        {
            return Err(format!(
                "A note path changed before its {asset_label} reference could be updated."
            ));
        }
        if !seen_note_ids.insert(update.note_id.as_str())
            || !seen_paths.insert(portable_path_key(&update.relative_path))
        {
            return Err(format!(
                "The {asset_label} move contains a duplicate note update."
            ));
        }
        if update.content.len() as u64 > MAX_NOTE_BYTES
            || update.expected_content.len() as u64 > MAX_NOTE_BYTES
        {
            return Err(format!(
                "{} is larger than {} MiB and cannot be updated.",
                update.relative_path,
                MAX_NOTE_BYTES / 1024 / 1024,
            ));
        }
        total_bytes = total_bytes.saturating_add(update.content.len() as u64);
        if total_bytes > MAX_TOTAL_NOTE_BYTES {
            return Err(format!(
                "The {asset_label} move would update too much note content at once."
            ));
        }
        let path = resolve_workspace_file(root, &update.relative_path, false)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Could not inspect {}: {error}", update.relative_path))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "{} is not a regular Markdown file.",
                update.relative_path
            ));
        }
        let current = fs::read(&path)
            .map_err(|error| format!("Could not read {}: {error}", update.relative_path))?;
        if current != update.expected_content.as_bytes() {
            return Err(format!(
                "{} changed before its {asset_label} reference could be updated.",
                update.relative_path,
            ));
        }
        prepared.push(PreparedAssetNoteUpdate {
            path,
            expected_content: current,
            content: update.content.as_bytes().to_vec(),
        });
    }
    Ok(prepared)
}

pub(in crate::workspace) fn read_workspace_image(
    root: &Path,
    asset_id: Option<&str>,
    note_relative_path: &str,
    destination: &str,
) -> Result<Vec<u8>, String> {
    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present && asset_id.is_some() {
        return Err("Workspace metadata is unreadable or newer than this app.".to_owned());
    }
    // Reading an explicit Markdown path needs no metadata write or stable-ID
    // lookup. Keep those images visible in a read-only vault as well.
    let mut state = stored_state.unwrap_or_default();
    let valid_asset_id = asset_id.filter(|id| is_valid_asset_id(id));
    let tracked_asset_id = valid_asset_id.filter(|id| {
        state
            .assets
            .get(*id)
            .is_some_and(|asset| asset.kind == VaultAssetKind::Image)
    });
    if let Some(relative_path) = tracked_asset_id
        .and_then(|id| state.assets.get(id))
        .map(|asset| asset.relative_path.as_str())
    {
        if let Ok(bytes) = read_relative_workspace_image(root, relative_path) {
            return Ok(bytes);
        }
    }
    if tracked_asset_id.is_some() {
        let _ = reconcile_image_assets(root, &mut state.assets, &mut warnings);
        if let Some(relative_path) = tracked_asset_id
            .and_then(|id| state.assets.get(id))
            .map(|asset| asset.relative_path.as_str())
        {
            if let Ok(bytes) = read_relative_workspace_image(root, relative_path) {
                return Ok(bytes);
            }
        }
    }

    let relative_path = resolve_markdown_image_path(note_relative_path, destination)?;
    read_relative_workspace_image(root, &relative_path)
}

pub(in crate::workspace) fn read_relative_workspace_image(
    root: &Path,
    relative_path: &str,
) -> Result<Vec<u8>, String> {
    let path = resolve_workspace_image_file(root, relative_path, false)?;
    let bytes = read_image_file(&path)?;
    validate_image_bytes_impl(&bytes, Some(relative_path))?;
    Ok(bytes)
}

pub(in crate::workspace) fn reconcile_image_assets(
    root: &Path,
    assets: &mut BTreeMap<String, StoredVaultAsset>,
    warnings: &mut WarningCollector,
) -> Vec<EmbeddedImage> {
    assets.retain(|_, asset| !is_finder_metadata_path(Path::new(&asset.relative_path)));
    if assets.len() > MAX_VAULT_ASSETS {
        warnings.push(format!(
            "Only the first {MAX_VAULT_ASSETS} embedded image records were loaded."
        ));
        let retained = assets
            .keys()
            .take(MAX_VAULT_ASSETS)
            .cloned()
            .collect::<HashSet<_>>();
        assets.retain(|id, _| retained.contains(id));
    }
    let invalid_ids = assets
        .iter()
        .filter_map(|(id, asset)| {
            let path_is_invalid = match asset.kind {
                VaultAssetKind::Image => {
                    validate_image_relative_path(&asset.relative_path).is_err()
                }
                VaultAssetKind::Attachment => {
                    validate_attachment_relative_path(&asset.relative_path).is_err()
                }
            };
            (!is_valid_asset_id(id) || path_is_invalid).then(|| id.clone())
        })
        .collect::<Vec<_>>();
    for id in invalid_ids {
        assets.remove(&id);
        warnings.push("Ignored an invalid embedded image record.".to_owned());
    }

    let mut assigned_paths = HashSet::new();
    let mut missing_ids = Vec::new();
    for (id, asset) in assets
        .iter_mut()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Image)
    {
        let path = match resolve_workspace_image_file(root, &asset.relative_path, false) {
            Ok(path) => path,
            Err(_) => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => metadata,
            _ => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let expected_media_type = Path::new(&asset.relative_path)
            .extension()
            .and_then(|value| value.to_str())
            .and_then(image_media_type_for_extension);
        let modified_nanos = image_modified_nanos(&metadata);
        if asset.modified_nanos != 0
            && asset.modified_nanos == modified_nanos
            && asset.fingerprint.length == metadata.len()
            && expected_media_type == Some(asset.media_type.as_str())
        {
            assigned_paths.insert(portable_path_key(&asset.relative_path));
            continue;
        }
        match read_image_file(&path) {
            Ok(bytes) => match validate_image_bytes_impl(&bytes, Some(&asset.relative_path)) {
                Ok((media_type, _)) => {
                    asset.media_type = media_type.to_owned();
                    asset.fingerprint = fingerprint_bytes(&bytes);
                    asset.modified_nanos = modified_nanos;
                    assigned_paths.insert(portable_path_key(&asset.relative_path));
                }
                Err(_) => missing_ids.push(id.clone()),
            },
            Err(_) => missing_ids.push(id.clone()),
        }
    }

    if !missing_ids.is_empty() {
        let missing_lengths = missing_ids
            .iter()
            .filter_map(|id| assets.get(id).map(|asset| asset.fingerprint.length))
            .collect::<HashSet<_>>();
        let mut candidates: HashMap<FileFingerprint, Vec<(String, String, u64)>> = HashMap::new();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(128)
            .into_iter()
            .filter_entry(should_visit_workspace_entry)
            .filter_map(Result::ok)
        {
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
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
            if assigned_paths.contains(&portable_path_key(&relative_path))
                || validate_image_relative_path(&relative_path).is_err()
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !missing_lengths.contains(&metadata.len()) {
                continue;
            }
            let Ok(bytes) = read_image_file(entry.path()) else {
                continue;
            };
            let Ok((media_type, _)) = validate_image_bytes_impl(&bytes, Some(&relative_path))
            else {
                continue;
            };
            let modified_nanos = image_modified_nanos(&metadata);
            candidates
                .entry(fingerprint_bytes(&bytes))
                .or_default()
                .push((relative_path, media_type.to_owned(), modified_nanos));
        }

        for id in missing_ids {
            let Some(asset) = assets.get_mut(&id) else {
                continue;
            };
            let Some(matches) = candidates.get(&asset.fingerprint) else {
                warnings.push(format!(
                    "Could not find the embedded image {}.",
                    asset.relative_path
                ));
                continue;
            };
            if matches.len() != 1 {
                warnings.push(format!(
                    "Could not uniquely locate the moved embedded image {}.",
                    asset.relative_path,
                ));
                continue;
            }
            let (relative_path, media_type, modified_nanos) = matches[0].clone();
            asset.relative_path = relative_path.clone();
            asset.media_type = media_type;
            asset.modified_nanos = modified_nanos;
            assigned_paths.insert(portable_path_key(&relative_path));
            candidates.remove(&asset.fingerprint);
        }
    }

    assets
        .iter()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Image)
        .map(|(id, asset)| EmbeddedImage {
            id: id.clone(),
            relative_path: asset.relative_path.clone(),
            media_type: asset.media_type.clone(),
        })
        .collect()
}

pub(in crate::workspace) fn reconcile_attachment_assets(
    root: &Path,
    assets: &mut BTreeMap<String, StoredVaultAsset>,
    warnings: &mut WarningCollector,
) -> Vec<EmbeddedAttachment> {
    let mut assigned_paths = HashSet::new();
    let mut missing_ids = Vec::new();
    for (id, asset) in assets
        .iter_mut()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Attachment)
    {
        let path = match resolve_workspace_asset_file(root, &asset.relative_path, false) {
            Ok(path) => path,
            Err(_) => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata)
                if !metadata.file_type().is_symlink()
                    && metadata.is_file()
                    && metadata.len() <= MAX_ATTACHMENT_BYTES =>
            {
                metadata
            }
            _ => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let media_type = attachment_media_type_for_path(Path::new(&asset.relative_path));
        let modified_nanos = image_modified_nanos(&metadata);
        if asset.modified_nanos != 0
            && asset.modified_nanos == modified_nanos
            && asset.fingerprint.length == metadata.len()
            && asset.media_type == media_type
        {
            assigned_paths.insert(portable_path_key(&asset.relative_path));
            continue;
        }
        match fingerprint_attachment_file(&path) {
            Ok(fingerprint) => {
                asset.media_type = media_type.to_owned();
                asset.fingerprint = fingerprint;
                asset.modified_nanos = modified_nanos;
                assigned_paths.insert(portable_path_key(&asset.relative_path));
            }
            Err(_) => missing_ids.push(id.clone()),
        }
    }

    if !missing_ids.is_empty() {
        let missing_lengths = missing_ids
            .iter()
            .filter_map(|id| assets.get(id).map(|asset| asset.fingerprint.length))
            .collect::<HashSet<_>>();
        let mut candidates: HashMap<FileFingerprint, Vec<(String, String, u64)>> = HashMap::new();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(128)
            .into_iter()
            .filter_entry(should_visit_workspace_entry)
            .filter_map(Result::ok)
        {
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
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
            if assigned_paths.contains(&portable_path_key(&relative_path))
                || validate_attachment_relative_path(&relative_path).is_err()
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !missing_lengths.contains(&metadata.len()) {
                continue;
            }
            let Ok(fingerprint) = fingerprint_attachment_file(entry.path()) else {
                continue;
            };
            let modified_nanos = image_modified_nanos(&metadata);
            candidates.entry(fingerprint).or_default().push((
                relative_path.clone(),
                attachment_media_type_for_path(Path::new(&relative_path)).to_owned(),
                modified_nanos,
            ));
        }

        for id in missing_ids {
            let Some(asset) = assets.get_mut(&id) else {
                continue;
            };
            let Some(matches) = candidates.get(&asset.fingerprint) else {
                warnings.push(format!(
                    "Could not find the embedded attachment {}.",
                    asset.relative_path,
                ));
                continue;
            };
            if matches.len() != 1 {
                warnings.push(format!(
                    "Could not uniquely locate the moved embedded attachment {}.",
                    asset.relative_path,
                ));
                continue;
            }
            let (relative_path, media_type, modified_nanos) = matches[0].clone();
            asset.relative_path = relative_path.clone();
            asset.media_type = media_type;
            asset.modified_nanos = modified_nanos;
            assigned_paths.insert(portable_path_key(&relative_path));
            candidates.remove(&asset.fingerprint);
        }
    }

    assets
        .iter()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Attachment)
        .map(|(id, asset)| EmbeddedAttachment {
            id: id.clone(),
            relative_path: asset.relative_path.clone(),
            media_type: asset.media_type.clone(),
            byte_length: asset.fingerprint.length,
            opening_disabled: attachment_opening_is_disabled(
                &root.join(Path::new(&asset.relative_path)),
            )
            .unwrap_or(true),
        })
        .collect()
}

pub(in crate::workspace) fn workspace_asset_limit_reached(
    image_count: usize,
    attachment_count: usize,
    limit: usize,
) -> bool {
    image_count.saturating_add(attachment_count) >= limit
}
