use super::*;

pub(in crate::workspace) fn workspace_image_import_path(
    root: &Path,
    source_relative_path: &str,
    bytes: Option<&[u8]>,
    reserved_paths: &HashSet<String>,
) -> Result<(String, bool), String> {
    let (portable_source_path, source_exists) = portable_image_path(root, source_relative_path)?;
    let source_key = portable_path_key(&portable_source_path);
    if !reserved_paths.contains(&source_key) {
        if !source_exists {
            return Ok((portable_source_path, false));
        }
        let target = resolve_workspace_image_file(root, &portable_source_path, true)?;
        match fs::symlink_metadata(&target) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => {
                if bytes.is_some_and(|bytes| {
                    fs::read(&target).is_ok_and(|existing| existing.as_slice() == bytes)
                }) {
                    return Ok((portable_source_path, true));
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }

    let path = Path::new(source_relative_path);
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Image");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png");
    for index in 2..=10_001_u32 {
        let file_name = format!("{stem} {index}.{extension}");
        let candidate = if folder.is_empty() {
            file_name
        } else {
            format!("{folder}/{file_name}")
        };
        validate_image_relative_path(&candidate)?;
        let (portable_candidate, exists) = portable_image_path(root, &candidate)?;
        if !reserved_paths.contains(&portable_path_key(&portable_candidate)) && !exists {
            return Ok((portable_candidate, false));
        }
    }

    Err(format!(
        "Could not choose a collision-free vault path for {source_relative_path}."
    ))
}

pub(in crate::workspace) fn workspace_attachment_import_path(
    root: &Path,
    source_relative_path: &str,
    fingerprint: Option<&FileFingerprint>,
    reserved_paths: &HashSet<String>,
) -> Result<(String, bool), String> {
    let (portable_source_path, source_exists) =
        portable_attachment_path(root, source_relative_path)?;
    let source_key = portable_path_key(&portable_source_path);
    if !reserved_paths.contains(&source_key) {
        if !source_exists {
            return Ok((portable_source_path, false));
        }
        let target = resolve_workspace_asset_file(root, &portable_source_path, true)?;
        match fs::symlink_metadata(&target) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => {
                if fingerprint.is_some_and(|expected| {
                    fingerprint_attachment_file(&target).as_ref() == Ok(expected)
                }) {
                    return Ok((portable_source_path, true));
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }

    let path = Path::new(source_relative_path);
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Attachment");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 2..=10_001_u32 {
        let file_name = match extension {
            Some(extension) => format!("{stem} {index}.{extension}"),
            None => format!("{stem} {index}"),
        };
        let candidate = if folder.is_empty() {
            file_name
        } else {
            format!("{folder}/{file_name}")
        };
        validate_attachment_relative_path(&candidate)?;
        let (portable_candidate, exists) = portable_attachment_path(root, &candidate)?;
        if !reserved_paths.contains(&portable_path_key(&portable_candidate)) && !exists {
            return Ok((portable_candidate, false));
        }
    }

    Err(format!(
        "Could not choose a collision-free vault path for {source_relative_path}."
    ))
}

pub(in crate::workspace) fn resolve_attachment_import_source(
    root: &Path,
    relative_path: &str,
) -> Result<PathBuf, String> {
    validate_attachment_relative_path(relative_path)?;
    let relative = checked_relative_path(relative_path, false)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("the source attachment could not be inspected: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("symbolic links are not followed".to_owned());
        }
    }
    let metadata = fs::symlink_metadata(&current)
        .map_err(|error| format!("the source attachment could not be inspected: {error}"))?;
    if !metadata.is_file() {
        return Err("the source attachment is not a regular file".to_owned());
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "the source attachment is larger than {} GiB",
            MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024
        ));
    }
    Ok(current)
}

pub(in crate::workspace) fn validate_image_import_root(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose a vault folder to import assets from.".to_owned());
    }
    let path = Path::new(input);
    if !path.is_absolute() {
        return Err("The asset import path must be absolute.".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The asset import folder could not be opened: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("The asset import folder cannot be a symbolic link.".to_owned());
    }
    if !metadata.is_dir() {
        return Err("The asset import path is not a folder.".to_owned());
    }
    path.canonicalize()
        .map_err(|error| format!("The asset import folder could not be resolved: {error}"))
}

pub(in crate::workspace) fn resolve_image_import_source(
    root: &Path,
    relative_path: &str,
) -> Result<PathBuf, String> {
    validate_image_relative_path(relative_path)?;
    let relative = checked_relative_path(relative_path, false)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("the source image could not be inspected: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("symbolic links are not followed".to_owned());
        }
    }
    let metadata = fs::symlink_metadata(&current)
        .map_err(|error| format!("the source image could not be inspected: {error}"))?;
    if !metadata.is_file() {
        return Err("the source image is not a regular file".to_owned());
    }
    Ok(current)
}
