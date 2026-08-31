use super::*;

pub(crate) mod attachments;
pub(crate) mod images;

pub(in crate::workspace) use attachments::*;
pub(in crate::workspace) use images::*;

pub(in crate::workspace) fn locate_workspace_vault_item(
    root: &Path,
    kind: WorkspaceVaultItemKind,
    relative_path: &str,
    asset_id: Option<&str>,
) -> Result<(String, PathBuf), String> {
    let resolved_relative_path = match kind {
        WorkspaceVaultItemKind::Image | WorkspaceVaultItemKind::Attachment => {
            resolve_vault_asset_relative_path(root, kind, relative_path, asset_id)?
        }
        WorkspaceVaultItemKind::Note | WorkspaceVaultItemKind::Folder => {
            if asset_id.is_some() {
                return Err("Notes and folders do not use stable asset IDs.".to_owned());
            }
            relative_path.to_owned()
        }
    };
    let target = match kind {
        WorkspaceVaultItemKind::Note => {
            validate_markdown_relative_path(&resolved_relative_path)?;
            resolve_workspace_file(root, &resolved_relative_path, false)?
        }
        WorkspaceVaultItemKind::Folder => {
            resolve_workspace_directory(root, &resolved_relative_path)?
        }
        WorkspaceVaultItemKind::Image => {
            resolve_workspace_image_file(root, &resolved_relative_path, false)?
        }
        WorkspaceVaultItemKind::Attachment => {
            resolve_workspace_asset_file(root, &resolved_relative_path, false)?
        }
    };
    let metadata = fs::symlink_metadata(&target)
        .map_err(|error| format!("Could not inspect the vault item: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("The vault item is a symbolic link and cannot be revealed.".to_owned());
    }
    if kind == WorkspaceVaultItemKind::Folder {
        if !metadata.is_dir() {
            return Err("The vault item is not a folder.".to_owned());
        }
    } else if !metadata.is_file() {
        return Err("The vault item is not a regular file.".to_owned());
    }
    let canonical = target
        .canonicalize()
        .map_err(|error| format!("Could not resolve the vault item: {error}"))?;
    if !canonical.starts_with(root) {
        return Err("The vault item resolved outside the active vault.".to_owned());
    }

    Ok((resolved_relative_path, canonical))
}

pub(in crate::workspace) fn resolve_vault_asset_relative_path(
    root: &Path,
    kind: WorkspaceVaultItemKind,
    relative_path: &str,
    asset_id: Option<&str>,
) -> Result<String, String> {
    let Some(asset_id) = asset_id else {
        return Ok(relative_path.to_owned());
    };
    if !is_valid_asset_id(asset_id) {
        return Err("The vault item has an invalid stable ID.".to_owned());
    }
    let expected_kind = match kind {
        WorkspaceVaultItemKind::Image => VaultAssetKind::Image,
        WorkspaceVaultItemKind::Attachment => VaultAssetKind::Attachment,
        _ => return Err("Only images and attachments use stable asset IDs.".to_owned()),
    };
    let mut warnings = WarningCollector::default();
    let (state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    let state = state.ok_or_else(|| {
        if state_file_was_present {
            "The vault item cannot be located while workspace metadata is unreadable or newer than this app."
                .to_owned()
        } else {
            "The vault item no longer has a stable record.".to_owned()
        }
    })?;
    let stored = state
        .assets
        .get(asset_id)
        .ok_or_else(|| "The vault item no longer has a stable record.".to_owned())?;
    if stored.kind != expected_kind {
        return Err("The stable record refers to a different vault item type.".to_owned());
    }

    Ok(stored.relative_path.clone())
}

pub(in crate::workspace) fn file_modified_nanos_for_path(path: &Path) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{} is not a regular file.", path.display()));
    }
    Ok(image_modified_nanos(&metadata))
}

pub(in crate::workspace) fn asset_path_exists_portably(
    root: &Path,
    relative_path: &str,
) -> Result<bool, String> {
    portable_attachment_path(root, relative_path).map(|(_, exists)| exists)
}

pub(in crate::workspace) fn image_path_exists_portably(
    root: &Path,
    relative_path: &str,
) -> Result<bool, String> {
    portable_image_path(root, relative_path).map(|(_, exists)| exists)
}

pub(in crate::workspace) fn portable_attachment_path(
    root: &Path,
    relative_path: &str,
) -> Result<(String, bool), String> {
    resolve_workspace_asset_file(root, relative_path, true)?;
    portable_vault_path(root, relative_path)
}

pub(in crate::workspace) fn portable_image_path(
    root: &Path,
    relative_path: &str,
) -> Result<(String, bool), String> {
    resolve_workspace_image_file(root, relative_path, true)?;
    portable_vault_path(root, relative_path)
}

fn portable_vault_path(root: &Path, relative_path: &str) -> Result<(String, bool), String> {
    validate_relative_path(relative_path, false)?;
    let components: Vec<&str> = relative_path.split('/').collect();
    let mut current = root.to_path_buf();
    let mut resolved = Vec::with_capacity(components.len());

    for (index, component) in components.iter().enumerate() {
        let entries = fs::read_dir(&current)
            .map_err(|error| format!("Could not inspect {}: {error}", current.display()))?;
        let component_key = portable_path_key(component);
        let mut matching_entry = None;

        for entry in entries {
            let entry = entry
                .map_err(|error| format!("Could not inspect {}: {error}", current.display()))?;
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            if portable_path_key(file_name) != component_key {
                continue;
            }
            if matching_entry.is_some() {
                return Err(format!(
                    "The vault contains paths that differ only by letter case near {}.",
                    current.join(component).display()
                ));
            }
            matching_entry = Some((file_name.to_owned(), entry.path()));
        }

        let Some((file_name, path)) = matching_entry else {
            resolved.extend(
                components[index..]
                    .iter()
                    .map(|component| (*component).to_owned()),
            );
            return Ok((resolved.join("/"), false));
        };
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing to follow the symbolic link {}.",
                path.display()
            ));
        }

        resolved.push(file_name);
        if index + 1 == components.len() {
            return Ok((resolved.join("/"), true));
        }
        if !metadata.is_dir() {
            return Err(format!("{} is not a folder.", path.display()));
        }
        current = path;
    }

    Ok((resolved.join("/"), false))
}

pub(in crate::workspace) fn is_valid_asset_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 180
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub(in crate::workspace) fn percent_decode_utf8(value: &str) -> Result<String, String> {
    let source = value.as_bytes();
    let mut decoded = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            if index + 2 >= source.len() {
                return Err("Image metadata contains invalid percent encoding.".to_owned());
            }
            let high = hex_value(source[index + 1])
                .ok_or_else(|| "Image metadata contains invalid percent encoding.".to_owned())?;
            let low = hex_value(source[index + 2])
                .ok_or_else(|| "Image metadata contains invalid percent encoding.".to_owned())?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(source[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| "Image metadata is not valid UTF-8.".to_owned())
}

pub(in crate::workspace) fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
