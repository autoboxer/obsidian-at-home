use super::*;

pub(in crate::workspace) const ATTACHMENT_COPY_BUFFER_BYTES: usize = 256 * 1024;

pub(in crate::workspace) fn validate_attachment_source_file(
    input: &str,
) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose a file to embed.".to_owned());
    }
    validate_attachment_source_path(Path::new(input))
}

pub(in crate::workspace) fn validate_attachment_source_path(
    path: &Path,
) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("The selected attachment path must be absolute.".to_owned());
    }
    validate_attachment_relative_path(
        path.file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "The selected attachment name is not valid Unicode.".to_owned())?,
    )?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The selected attachment could not be opened: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(
            "Choose a regular file, not a folder, symbolic link, or special file.".to_owned(),
        );
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "The selected attachment is larger than {} GiB.",
            MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024,
        ));
    }
    path.canonicalize()
        .map_err(|error| format!("The selected attachment could not be resolved: {error}"))
}

pub(in crate::workspace) fn fingerprint_attachment_file(
    path: &Path,
) -> Result<FileFingerprint, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "{} is not a regular attachment file.",
            path.display()
        ));
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "The attachment is larger than {} GiB.",
            MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024,
        ));
    }
    let mut file =
        File::open(path).map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if !opened_metadata.is_file() || opened_metadata.len() != metadata.len() {
        return Err("The attachment changed while it was being opened.".to_owned());
    }

    let mut buffer = vec![0_u8; ATTACHMENT_COPY_BUFFER_BYTES];
    let mut hash = 0xcbf29ce484222325_u64;
    let mut length = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(read as u64)
            .filter(|value| *value <= MAX_ATTACHMENT_BYTES)
            .ok_or_else(|| "The attachment became too large while it was being read.".to_owned())?;
        fnv_update(&mut hash, &buffer[..read]);
    }
    if length != metadata.len() {
        return Err("The attachment changed while it was being read.".to_owned());
    }

    Ok(FileFingerprint { length, hash })
}

pub(in crate::workspace) fn copy_attachment_file_durable(
    source: &Path,
    destination: &Path,
) -> Result<FileFingerprint, String> {
    let source_metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("Could not inspect the selected attachment: {error}"))?;
    if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
        return Err("The selected attachment is not a regular file.".to_owned());
    }
    if source_metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "The selected attachment is larger than {} GiB.",
            MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024,
        ));
    }
    let source_modified_nanos = image_modified_nanos(&source_metadata);
    let mut source_file = File::open(source)
        .map_err(|error| format!("Could not open the selected attachment: {error}"))?;
    let opened_metadata = source_file
        .metadata()
        .map_err(|error| format!("Could not inspect the selected attachment: {error}"))?;
    if !opened_metadata.is_file() || opened_metadata.len() != source_metadata.len() {
        return Err("The selected attachment changed while it was being opened.".to_owned());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "The attachment destination has no parent folder.".to_owned())?;
    let mut destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("Could not create the embedded attachment: {error}"))?;

    let copy_result = (|| -> Result<FileFingerprint, String> {
        let mut buffer = vec![0_u8; ATTACHMENT_COPY_BUFFER_BYTES];
        let mut hash = 0xcbf29ce484222325_u64;
        let mut length = 0_u64;
        loop {
            let read = source_file
                .read(&mut buffer)
                .map_err(|error| format!("Could not read the selected attachment: {error}"))?;
            if read == 0 {
                break;
            }
            length = length
                .checked_add(read as u64)
                .filter(|value| *value <= MAX_ATTACHMENT_BYTES)
                .ok_or_else(|| {
                    "The selected attachment became too large while it was copied.".to_owned()
                })?;
            destination_file
                .write_all(&buffer[..read])
                .map_err(|error| format!("Could not copy the embedded attachment: {error}"))?;
            fnv_update(&mut hash, &buffer[..read]);
        }
        let final_source_metadata = source_file
            .metadata()
            .map_err(|error| format!("Could not recheck the selected attachment: {error}"))?;
        if length != source_metadata.len()
            || final_source_metadata.len() != source_metadata.len()
            || image_modified_nanos(&final_source_metadata) != source_modified_nanos
        {
            return Err("The selected attachment changed while it was being copied.".to_owned());
        }
        destination_file
            .flush()
            .and_then(|_| destination_file.sync_all())
            .map_err(|error| format!("Could not finish the embedded attachment: {error}"))?;

        Ok(FileFingerprint { length, hash })
    })();
    drop(destination_file);
    match copy_result {
        Ok(fingerprint) => {
            sync_directory(parent)
                .map_err(|error| format!("Could not finish the attachment folder: {error}"))?;
            Ok(fingerprint)
        }
        Err(error) => {
            let _ = remove_file_durable(destination);
            Err(error)
        }
    }
}

pub(crate) fn copy_attachment_file_for_transfer_impl(
    source: &Path,
    destination: &Path,
) -> Result<(), String> {
    let fingerprint = copy_attachment_file_durable(source, destination)?;
    match fingerprint_attachment_file(destination) {
        Ok(copied) if copied == fingerprint => Ok(()),
        Ok(_) => {
            let _ = remove_file_durable(destination);
            Err("The transferred attachment failed its integrity check.".to_owned())
        }
        Err(error) => {
            let _ = remove_file_durable(destination);
            Err(format!(
                "The transferred attachment could not be verified: {error}"
            ))
        }
    }
}

pub(in crate::workspace) fn resolve_attachment_action_source(
    root: &Path,
    attachment_relative_path: &str,
    asset_id: Option<&str>,
) -> Result<(String, PathBuf), String> {
    let relative_path = if let Some(asset_id) = asset_id.filter(|id| !id.is_empty()) {
        if !is_valid_asset_id(asset_id) {
            return Err("The attachment has an invalid stable ID.".to_owned());
        }
        let mut warnings = WarningCollector::default();
        let (state, state_file_was_present) = read_workspace_state(root, &mut warnings);
        if state.is_none() && state_file_was_present {
            return Err(
                "The attachment cannot be opened while workspace metadata is unreadable or newer than this app."
                    .to_owned(),
            );
        }
        if let Some(stored) = state.as_ref().and_then(|state| state.assets.get(asset_id)) {
            if stored.kind != VaultAssetKind::Attachment {
                return Err(
                    "The stable attachment record refers to a different file type.".to_owned(),
                );
            }
            stored.relative_path.clone()
        } else {
            attachment_relative_path.to_owned()
        }
    } else {
        attachment_relative_path.to_owned()
    };
    validate_attachment_relative_path(&relative_path)?;
    let source = resolve_workspace_asset_file(root, &relative_path, false)?;
    let metadata = fs::symlink_metadata(&source)
        .map_err(|error| format!("Could not inspect the attachment: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("The attachment is not a regular vault file.".to_owned());
    }

    Ok((relative_path, source))
}

pub(in crate::workspace) fn is_archive_attachment_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "7z" | "bz2" | "gz" | "rar" | "tar" | "tgz" | "xz" | "zip"
    )
}

pub(in crate::workspace) fn is_executable_attachment_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "app"
            | "appimage"
            | "appx"
            | "appxbundle"
            | "bat"
            | "bin"
            | "cmd"
            | "command"
            | "com"
            | "cpl"
            | "deb"
            | "desktop"
            | "dmg"
            | "exe"
            | "fish"
            | "hta"
            | "jar"
            | "js"
            | "jse"
            | "lnk"
            | "msc"
            | "msi"
            | "msix"
            | "msixbundle"
            | "msp"
            | "mst"
            | "pif"
            | "pkg"
            | "ps1"
            | "psm1"
            | "py"
            | "pyw"
            | "reg"
            | "rpm"
            | "run"
            | "scr"
            | "sh"
            | "tool"
            | "vbe"
            | "vbs"
            | "wsf"
            | "wsh"
            | "zsh"
    )
}

pub(in crate::workspace) fn attachment_opening_is_disabled(path: &Path) -> Result<bool, String> {
    if is_archive_attachment_path(path) {
        return Ok(false);
    }
    if is_executable_attachment_path(path) {
        return Ok(true);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(path)
            .map_err(|error| format!("Could not inspect the attachment before opening: {error}"))?;
        if metadata.permissions().mode() & 0o111 != 0 {
            return Ok(true);
        }
    }

    let mut file = File::open(path)
        .map_err(|error| format!("Could not inspect the attachment before opening: {error}"))?;
    let mut prefix = [0_u8; 8];
    let length = file
        .read(&mut prefix)
        .map_err(|error| format!("Could not inspect the attachment before opening: {error}"))?;

    Ok(attachment_prefix_is_executable(&prefix[..length]))
}

pub(in crate::workspace) fn attachment_prefix_is_executable(prefix: &[u8]) -> bool {
    prefix.starts_with(b"#!")
        || prefix.starts_with(b"MZ")
        || prefix.starts_with(b"\x7fELF")
        || matches!(
            prefix.get(..4),
            Some(
                [0xfe, 0xed, 0xfa, 0xce]
                    | [0xce, 0xfa, 0xed, 0xfe]
                    | [0xfe, 0xed, 0xfa, 0xcf]
                    | [0xcf, 0xfa, 0xed, 0xfe]
                    | [0xca, 0xfe, 0xba, 0xbe]
                    | [0xbe, 0xba, 0xfe, 0xca]
                    | [0xca, 0xfe, 0xba, 0xbf]
                    | [0xbf, 0xba, 0xfe, 0xca]
            )
        )
}

pub(in crate::workspace) fn safe_external_copy_directory(
    root: &Path,
    input: &Path,
) -> Option<PathBuf> {
    if !input.is_absolute() {
        return None;
    }
    let metadata = fs::symlink_metadata(input).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return None;
    }
    let directory = input.canonicalize().ok()?;
    (!directory.starts_with(root)).then_some(directory)
}

pub(in crate::workspace) fn validate_external_attachment_copy_target(
    root: &Path,
    target: &Path,
) -> Result<PathBuf, String> {
    if !target.is_absolute() {
        return Err("Choose an absolute location outside the vault.".to_owned());
    }
    let file_name = target
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Choose a file name for the attachment copy.".to_owned())?;
    let parent = target
        .parent()
        .ok_or_else(|| "The attachment copy location has no parent folder.".to_owned())?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| format!("Could not inspect the attachment copy folder: {error}"))?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err("Choose a regular folder, not a symbolic link.".to_owned());
    }
    let parent = parent
        .canonicalize()
        .map_err(|error| format!("Could not resolve the attachment copy folder: {error}"))?;
    if parent.starts_with(root) {
        return Err(
            "Archive copies must be saved outside the active vault to avoid untracked extracted files."
                .to_owned(),
        );
    }
    let target = parent.join(file_name);
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            return Err(
                "A file already exists at that location. Choose a new name for the copy."
                    .to_owned(),
            );
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Could not inspect the attachment copy location: {error}"
            ));
        }
    }

    Ok(target)
}

pub(in crate::workspace) fn save_workspace_attachment_copy(
    app: &AppHandle,
    root: &Path,
    attachment_relative_path: &str,
    asset_id: Option<&str>,
    preferred_directory: Option<&str>,
) -> Result<Option<WorkspaceAttachmentCopyResult>, String> {
    let (source_name, baseline_fingerprint) = {
        let _guard = lock_workspace_io()?;
        let _workspace_guard = lock_workspace_files(root)?;
        let (_, source) =
            resolve_attachment_action_source(root, attachment_relative_path, asset_id)?;
        if !is_archive_attachment_path(&source) {
            return Err("Only archive attachments use the Save archive as flow.".to_owned());
        }
        let file_name = source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "The archive name is not valid Unicode.".to_owned())?
            .to_owned();
        (file_name, fingerprint_attachment_file(&source)?)
    };

    let preferred = preferred_directory
        .map(Path::new)
        .and_then(|directory| safe_external_copy_directory(root, directory));
    let downloads = app
        .path()
        .download_dir()
        .ok()
        .and_then(|directory| safe_external_copy_directory(root, &directory));
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Save archive as")
        .set_file_name(&source_name);
    if let Some(directory) = preferred.or(downloads) {
        dialog = dialog.set_directory(directory);
    }
    let Some(selected) = dialog.blocking_save_file() else {
        return Ok(None);
    };
    let selected = selected
        .into_path()
        .map_err(|error| format!("The selected copy location is not a local path: {error}"))?;
    let target = validate_external_attachment_copy_target(root, &selected)?;

    let copied_fingerprint = {
        let _guard = lock_workspace_io()?;
        let _workspace_guard = lock_workspace_files(root)?;
        let (_, source) =
            resolve_attachment_action_source(root, attachment_relative_path, asset_id)?;
        if !is_archive_attachment_path(&source) {
            return Err("The attachment is no longer an archive.".to_owned());
        }
        if fingerprint_attachment_file(&source)? != baseline_fingerprint {
            return Err(
                "The archive changed while the copy location was being chosen. Try again."
                    .to_owned(),
            );
        }
        copy_attachment_file_durable(&source, &target)?
    };
    if copied_fingerprint != baseline_fingerprint
        || fingerprint_attachment_file(&target)? != baseline_fingerprint
    {
        let _ = remove_file_durable(&target);
        return Err("The saved archive copy failed its integrity check.".to_owned());
    }

    Ok(Some(WorkspaceAttachmentCopyResult {
        path: path_string(&target)?,
    }))
}

pub(in crate::workspace) fn safe_attachment_file_name(file_name: &str) -> Result<String, String> {
    let path = Path::new(file_name);
    let stem = safe_file_stem(
        path.file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Attachment"),
        "Attachment",
    );
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|character| {
            !character.is_control() && !is_forbidden_component_character(*character)
        })
        .take(40)
        .collect::<String>();
    let safe_name = if extension.is_empty() {
        stem
    } else {
        format!("{stem}.{extension}")
    };
    validate_attachment_relative_path(&safe_name)?;
    Ok(safe_name)
}

pub(in crate::workspace) fn unique_attachment_relative_path(
    root: &Path,
    folder: &str,
    file_name: &str,
) -> Result<String, String> {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Attachment");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 1..=10_000_u32 {
        let candidate_name = if index == 1 {
            file_name.to_owned()
        } else if let Some(extension) = extension {
            format!("{stem} {index}.{extension}")
        } else {
            format!("{stem} {index}")
        };
        let relative_path = if folder.is_empty() {
            candidate_name
        } else {
            format!("{folder}/{candidate_name}")
        };
        validate_attachment_relative_path(&relative_path)?;
        let (portable_path, exists) = portable_attachment_path(root, &relative_path)?;
        if !exists {
            return Ok(portable_path);
        }
    }
    Err("Could not choose a unique file name for the embedded attachment.".to_owned())
}

pub(in crate::workspace) fn attachment_media_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "json" => "application/json",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "rtf" => "application/rtf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

pub(in crate::workspace) fn validate_attachment_relative_path(
    relative_path: &str,
) -> Result<(), String> {
    validate_relative_path(relative_path, false)?;
    let path = Path::new(relative_path);
    if is_markdown_path(path) {
        return Err(
            "Markdown notes should be linked as notes, not embedded as attachments.".to_owned(),
        );
    }
    if is_supported_image_path_impl(path) {
        return Err(
            "Use image embedding for PNG, JPEG, GIF, WebP, BMP, and AVIF files.".to_owned(),
        );
    }
    Ok(())
}
