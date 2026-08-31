use super::*;

pub(in crate::workspace) const EXTERNAL_FILE_UPLOAD_DIRECTORY: &str = "external-file-drops";
pub(in crate::workspace) const EXTERNAL_FILE_UPLOAD_CHUNK_BYTES: usize = 512 * 1024;
pub(in crate::workspace) const MAX_EXTERNAL_FILE_UPLOADS: usize = 16;
pub(in crate::workspace) const ABANDONED_EXTERNAL_FILE_UPLOAD_MILLIS: u64 = 5 * 60 * 1000;
pub(in crate::workspace) const STALE_EXTERNAL_FILE_UPLOAD_MILLIS: u64 = 24 * 60 * 60 * 1000;

pub(in crate::workspace) static EXTERNAL_FILE_UPLOADS: LazyLock<
    Mutex<HashMap<String, ExternalFileUpload>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug)]
pub(in crate::workspace) struct ExternalFileUpload {
    pub(in crate::workspace) directory: PathBuf,
    pub(in crate::workspace) path: PathBuf,
    pub(in crate::workspace) file: Option<File>,
    pub(in crate::workspace) file_name: String,
    pub(in crate::workspace) expected_length: u64,
    pub(in crate::workspace) received_length: u64,
    pub(in crate::workspace) root: PathBuf,
    pub(in crate::workspace) note_relative_path: String,
    pub(in crate::workspace) kind: ExternalFileUploadKind,
    pub(in crate::workspace) last_activity: Instant,
    pub(in crate::workspace) cleanup_on_drop: bool,
}

impl Drop for ExternalFileUpload {
    fn drop(&mut self) {
        if !self.cleanup_on_drop {
            return;
        }
        self.file.take();
        let _ = remove_file_durable(&self.path);
        let _ = remove_directory_durable(&self.directory);
    }
}

#[derive(Debug)]
pub(in crate::workspace) struct StagedExternalFile {
    pub(in crate::workspace) directory: PathBuf,
    pub(in crate::workspace) path: PathBuf,
    pub(in crate::workspace) file_name: String,
    pub(in crate::workspace) root: PathBuf,
    pub(in crate::workspace) note_relative_path: String,
}

impl Drop for StagedExternalFile {
    fn drop(&mut self) {
        let _ = remove_file_durable(&self.path);
        let _ = remove_directory_durable(&self.directory);
    }
}

pub(in crate::workspace) fn external_file_staging_directory(
    app: &AppHandle,
) -> Result<PathBuf, String> {
    app.path()
        .app_cache_dir()
        .map(|directory| directory.join(EXTERNAL_FILE_UPLOAD_DIRECTORY))
        .map_err(|error| format!("Could not locate temporary dropped-file storage: {error}"))
}

pub(in crate::workspace) fn safe_external_file_name(file_name: &str) -> Result<String, String> {
    if file_name.trim().is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || Path::new(file_name).components().count() != 1
    {
        return Err("The dropped file name is not safe.".to_owned());
    }
    let path = Path::new(file_name);
    let stem = safe_file_stem(
        path.file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Dropped file"),
        "Dropped file",
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
    validate_component_name(&safe_name, "file")?;
    Ok(safe_name)
}

pub(in crate::workspace) fn validate_external_file_drop_note(
    root: &Path,
    note_relative_path: &str,
) -> Result<(), String> {
    validate_markdown_relative_path(note_relative_path)?;
    let note_path = resolve_workspace_file(root, note_relative_path, false)?;
    let metadata = fs::symlink_metadata(&note_path)
        .map_err(|_| "Save the active note before dropping a file into it.".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Save the active note before dropping a file into it.".to_owned());
    }
    Ok(())
}

pub(in crate::workspace) fn prepare_external_file_staging_directory(
    directory: &Path,
) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not prepare temporary dropped-file storage: {error}"))?;
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("Could not inspect temporary dropped-file storage: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Temporary dropped-file storage is not a regular folder.".to_owned());
    }
    cleanup_stale_external_file_uploads(directory);
    Ok(())
}

pub(in crate::workspace) fn cleanup_stale_external_file_uploads(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten().take(256) {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with("drop-") {
            continue;
        }
        let upload_directory = entry.path();
        let Ok(directory_metadata) = fs::symlink_metadata(&upload_directory) else {
            continue;
        };
        if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
            continue;
        }
        let Ok(children) = fs::read_dir(&upload_directory) else {
            continue;
        };
        let children = children.flatten().take(2).collect::<Vec<_>>();
        if children.len() > 1 {
            continue;
        }
        let child = children.first();
        let modified = child
            .and_then(|child| child.metadata().ok())
            .and_then(|metadata| metadata.modified().ok())
            .or_else(|| directory_metadata.modified().ok());
        let is_stale = modified
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= Duration::from_millis(STALE_EXTERNAL_FILE_UPLOAD_MILLIS));
        if !is_stale {
            continue;
        }
        if let Some(child) = child {
            let Ok(metadata) = fs::symlink_metadata(child.path()) else {
                continue;
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            if remove_file_durable(&child.path()).is_err() {
                continue;
            }
        }
        let _ = remove_directory_durable(&upload_directory);
    }
}

pub(in crate::workspace) fn remove_abandoned_external_file_uploads(
    uploads: &mut HashMap<String, ExternalFileUpload>,
    now: Instant,
) {
    let timeout = Duration::from_millis(ABANDONED_EXTERNAL_FILE_UPLOAD_MILLIS);
    uploads.retain(|_, upload| {
        now.checked_duration_since(upload.last_activity)
            .map_or(true, |inactive| inactive < timeout)
    });
}

pub(in crate::workspace) fn begin_external_file_upload(
    staging_directory: &Path,
    file_name: String,
    expected_length: u64,
    kind: ExternalFileUploadKind,
    root: PathBuf,
    note_relative_path: String,
) -> Result<WorkspaceExternalFileUpload, String> {
    match kind {
        ExternalFileUploadKind::Image
            if expected_length == 0 || expected_length > MAX_IMAGE_BYTES =>
        {
            return Err(format!(
                "The dropped image must be between 1 byte and {} MiB.",
                MAX_IMAGE_BYTES / 1024 / 1024,
            ));
        }
        ExternalFileUploadKind::Attachment if expected_length > MAX_ATTACHMENT_BYTES => {
            return Err(format!(
                "The dropped file is larger than {} GiB.",
                MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024,
            ));
        }
        _ => {}
    }
    let file_name = safe_external_file_name(&file_name)?;
    match kind {
        ExternalFileUploadKind::Image => validate_image_relative_path(&file_name)?,
        ExternalFileUploadKind::Attachment => validate_attachment_relative_path(&file_name)?,
    }
    prepare_external_file_staging_directory(staging_directory)?;
    let mut uploads = EXTERNAL_FILE_UPLOADS.lock().map_err(|_| {
        "Dropped-file transfers are unavailable because an earlier transfer failed.".to_owned()
    })?;
    remove_abandoned_external_file_uploads(&mut uploads, Instant::now());
    if uploads.len() >= MAX_EXTERNAL_FILE_UPLOADS {
        return Err("Wait for the current dropped files to finish before adding more.".to_owned());
    }

    for _ in 0..10_000 {
        let id = format!(
            "drop-{}-{}-{}",
            std::process::id(),
            now_millis(),
            TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed),
        );
        if uploads.contains_key(&id) {
            continue;
        }
        let upload_directory = staging_directory.join(&id);
        match fs::create_dir(&upload_directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Could not create temporary dropped-file storage: {error}"
                ));
            }
        }
        let upload_path = upload_directory.join(&file_name);
        let file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&upload_path)
        {
            Ok(file) => file,
            Err(error) => {
                let _ = remove_directory_durable(&upload_directory);
                return Err(format!("Could not stage the dropped file: {error}"));
            }
        };
        uploads.insert(
            id.clone(),
            ExternalFileUpload {
                directory: upload_directory,
                path: upload_path,
                file: Some(file),
                file_name,
                expected_length,
                received_length: 0,
                root,
                note_relative_path,
                kind,
                last_activity: Instant::now(),
                cleanup_on_drop: true,
            },
        );
        return Ok(WorkspaceExternalFileUpload {
            id,
            chunk_bytes: EXTERNAL_FILE_UPLOAD_CHUNK_BYTES,
        });
    }
    Err("Could not reserve temporary storage for the dropped file.".to_owned())
}

pub(in crate::workspace) fn append_external_file_upload(
    upload_id: &str,
    offset: u64,
    bytes: &[u8],
) -> Result<u64, String> {
    let mut uploads = EXTERNAL_FILE_UPLOADS.lock().map_err(|_| {
        "Dropped-file transfers are unavailable because an earlier transfer failed.".to_owned()
    })?;
    let result = (|| {
        let upload = uploads
            .get_mut(upload_id)
            .ok_or_else(|| "The dropped-file transfer is no longer available.".to_owned())?;
        if bytes.is_empty() || bytes.len() > EXTERNAL_FILE_UPLOAD_CHUNK_BYTES {
            return Err("A dropped-file transfer chunk has an invalid size.".to_owned());
        }
        if offset != upload.received_length {
            return Err("Dropped-file transfer chunks arrived out of order.".to_owned());
        }
        let received_length = upload
            .received_length
            .checked_add(bytes.len() as u64)
            .filter(|length| *length <= upload.expected_length)
            .ok_or_else(|| "The dropped file contains more data than expected.".to_owned())?;
        upload
            .file
            .as_mut()
            .ok_or_else(|| "The dropped-file transfer is already closed.".to_owned())?
            .write_all(bytes)
            .map_err(|error| format!("Could not stage the dropped file: {error}"))?;
        upload.received_length = received_length;
        upload.last_activity = Instant::now();
        Ok(received_length)
    })();
    if result.is_err() {
        uploads.remove(upload_id);
    }
    result
}

pub(in crate::workspace) fn cancel_external_file_upload(upload_id: &str) -> Result<bool, String> {
    let mut uploads = EXTERNAL_FILE_UPLOADS.lock().map_err(|_| {
        "Dropped-file transfers are unavailable because an earlier transfer failed.".to_owned()
    })?;
    Ok(uploads.remove(upload_id).is_some())
}

pub(in crate::workspace) fn finish_external_file_upload(
    upload_id: &str,
    expected_kind: ExternalFileUploadKind,
) -> Result<StagedExternalFile, String> {
    let mut upload = EXTERNAL_FILE_UPLOADS
        .lock()
        .map_err(|_| {
            "Dropped-file transfers are unavailable because an earlier transfer failed.".to_owned()
        })?
        .remove(upload_id)
        .ok_or_else(|| "The dropped-file transfer is no longer available.".to_owned())?;
    if upload.kind != expected_kind {
        return Err(
            "The dropped-file transfer does not match the requested asset type.".to_owned(),
        );
    }
    if upload.received_length != upload.expected_length {
        return Err("The dropped file did not finish transferring.".to_owned());
    }
    let mut file = upload
        .file
        .take()
        .ok_or_else(|| "The dropped-file transfer is already closed.".to_owned())?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Could not finish staging the dropped file: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect the staged dropped file: {error}"))?;
    if !metadata.is_file() || metadata.len() != upload.expected_length {
        return Err("The staged dropped file is incomplete or unsafe.".to_owned());
    }
    drop(file);
    let staged = StagedExternalFile {
        directory: upload.directory.clone(),
        path: upload.path.clone(),
        file_name: upload.file_name.clone(),
        root: upload.root.clone(),
        note_relative_path: upload.note_relative_path.clone(),
    };
    upload.cleanup_on_drop = false;
    Ok(staged)
}
