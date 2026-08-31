use super::*;

pub(in crate::workspace) fn validate_image_source_file(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose an image file.".to_owned());
    }
    validate_image_source_path(Path::new(input))
}

pub(in crate::workspace) fn validate_image_source_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("The selected image path must be absolute.".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The selected image could not be opened: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Choose a regular image file, not a symbolic link.".to_owned());
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "The selected image is larger than {} MiB.",
            MAX_IMAGE_BYTES / 1024 / 1024,
        ));
    }
    path.canonicalize()
        .map_err(|error| format!("The selected image could not be resolved: {error}"))
}

pub(in crate::workspace) fn read_image_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{} is not a regular image file.", path.display()));
    }
    if metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "The image must be between 1 byte and {} MiB.",
            MAX_IMAGE_BYTES / 1024 / 1024,
        ));
    }
    fs::read(path).map_err(|error| format!("Could not read {}: {error}", path.display()))
}

pub(in crate::workspace) fn image_modified_nanos_for_path(
    root: &Path,
    relative_path: &str,
) -> Result<u64, String> {
    let path = resolve_workspace_image_file(root, relative_path, false)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Could not inspect {relative_path}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{relative_path} is not a regular image file."));
    }
    Ok(image_modified_nanos(&metadata))
}

pub(in crate::workspace) fn image_modified_nanos(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

pub(crate) fn validate_image_bytes_impl(
    bytes: &[u8],
    file_name: Option<&str>,
) -> Result<(&'static str, &'static str), String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(format!(
            "The image must be between 1 byte and {} MiB.",
            MAX_IMAGE_BYTES / 1024 / 1024,
        ));
    }
    let detected = detect_image_type(bytes)
        .ok_or_else(|| "Use a PNG, JPEG, GIF, WebP, BMP, or AVIF image.".to_owned())?;
    if let Some(extension) = file_name
        .and_then(|name| Path::new(name).extension())
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
    {
        if let Some(expected_media_type) = image_media_type_for_extension(&extension) {
            if expected_media_type != detected.0 {
                return Err("The image contents do not match the file extension.".to_owned());
            }
        }
    }
    Ok(detected)
}

pub(in crate::workspace) fn detect_image_type(
    bytes: &[u8],
) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(("image/png", "png"));
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(("image/jpeg", "jpg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(("image/gif", "gif"));
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(("image/webp", "webp"));
    }
    if bytes.starts_with(b"BM") {
        return Some(("image/bmp", "bmp"));
    }
    if bytes.len() >= 16
        && &bytes[4..8] == b"ftyp"
        && bytes[8..]
            .chunks_exact(4)
            .any(|brand| brand == b"avif" || brand == b"avis")
    {
        return Some(("image/avif", "avif"));
    }
    None
}

pub(in crate::workspace) fn image_media_type_for_extension(
    extension: &str,
) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "avif" => Some("image/avif"),
        _ => None,
    }
}

pub(crate) fn is_supported_image_path_impl(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .and_then(image_media_type_for_extension)
        .is_some()
}

pub(in crate::workspace) fn safe_image_file_name(file_name: &str, extension: &str) -> String {
    let source_stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Image")
        .trim();
    let normalized = source_stem
        .chars()
        .map(|character| {
            if character.is_control() || is_forbidden_component_character(character) {
                '-'
            } else {
                character
            }
        })
        .collect::<String>();
    let mut stem = String::new();
    for character in normalized.trim_matches([' ', '.', '-']).chars() {
        if stem.len() + character.len_utf8() > 140 {
            break;
        }
        stem.push(character);
    }
    if stem.is_empty() || is_windows_reserved_name(&stem) {
        stem = "Image".to_owned();
    }
    format!("{stem}.{extension}")
}

pub(in crate::workspace) fn unique_image_relative_path(
    root: &Path,
    folder: &str,
    file_name: &str,
) -> Result<String, String> {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Image");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png");
    for index in 1..=10_000_u32 {
        let candidate_name = if index == 1 {
            file_name.to_owned()
        } else {
            format!("{stem} {index}.{extension}")
        };
        let relative_path = if folder.is_empty() {
            candidate_name
        } else {
            format!("{folder}/{candidate_name}")
        };
        validate_image_relative_path(&relative_path)?;
        let (portable_path, exists) = portable_image_path(root, &relative_path)?;
        if !exists {
            return Ok(portable_path);
        }
    }
    Err("Could not choose a unique file name for the embedded image.".to_owned())
}

pub(in crate::workspace) fn validate_image_relative_path(
    relative_path: &str,
) -> Result<(), String> {
    validate_relative_path(relative_path, false)?;
    let extension = Path::new(relative_path)
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Embedded image paths must include a supported extension.".to_owned())?;
    if image_media_type_for_extension(extension).is_none() {
        return Err("Use a PNG, JPEG, GIF, WebP, BMP, or AVIF image path.".to_owned());
    }
    Ok(())
}

pub(in crate::workspace) fn resolve_markdown_image_path(
    note_relative_path: &str,
    destination: &str,
) -> Result<String, String> {
    validate_markdown_relative_path(note_relative_path)?;
    if destination.is_empty()
        || destination.contains('\\')
        || destination.contains('#')
        || destination.contains('?')
        || destination.chars().any(char::is_control)
    {
        return Err("The Markdown image path is not a safe local vault path.".to_owned());
    }
    let mut components = if destination.starts_with('/') {
        Vec::new()
    } else {
        Path::new(note_relative_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .and_then(path_to_slash_string)
            .unwrap_or_default()
            .split('/')
            .filter(|component| !component.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    for component in destination.trim_start_matches('/').split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err("The Markdown image path escapes the vault.".to_owned());
                }
            }
            value => components.push(value.to_owned()),
        }
    }
    let relative_path = components.join("/");
    validate_image_relative_path(&relative_path)?;
    Ok(relative_path)
}
