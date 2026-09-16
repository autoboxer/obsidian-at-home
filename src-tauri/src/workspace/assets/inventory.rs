use super::*;

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
