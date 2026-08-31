use super::*;

pub(super) fn directory_contains_nested_vault(directory: &Path) -> bool {
    WalkDir::new(directory)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
        .filter_map(Result::ok)
        .any(|entry| is_nested_vault_directory(&entry))
}

pub(super) fn ensure_directory_path(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let relative = checked_relative_path(relative_path, false)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Refusing to use the symbolic link {}.",
                    current.display()
                ));
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(format!("{} is not a folder.", current.display()));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                create_directory_durable(&current).map_err(|error| {
                    format!("Could not create the folder {}: {error}", current.display())
                })?;
            }
            Err(error) => {
                return Err(format!("Could not inspect {}: {error}", current.display()));
            }
        }
    }
    Ok(current)
}

pub(super) fn resolve_workspace_directory(
    root: &Path,
    relative_path: &str,
) -> Result<PathBuf, String> {
    let relative = checked_relative_path(relative_path, false)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Refusing to use the symbolic link {}.",
                    current.display()
                ));
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(format!("{} is not a folder.", current.display())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!("Could not inspect {}: {error}", current.display()));
            }
        }
    }
    Ok(root.join(relative))
}

pub(super) fn ensure_existing_directory_without_symlink(
    root: &Path,
    directory: &Path,
) -> Result<(), String> {
    let relative = directory
        .strip_prefix(root)
        .map_err(|_| "A note path escaped the vault.".to_owned())?;
    let relative = path_to_slash_string(relative)
        .ok_or_else(|| "A note folder path is not valid Unicode.".to_owned())?;
    if relative.is_empty() {
        return Ok(());
    }
    ensure_directory_path(root, &relative).map(|_| ())
}

pub(super) fn ensure_state_directory(root: &Path, directory: &Path) -> Result<(), String> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("The .obsidian-at-home folder cannot be a symbolic link.".to_owned())
        }
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(".obsidian-at-home exists but is not a folder.".to_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_directory_durable(directory)
                .map_err(|error| format!("Could not create .obsidian-at-home: {error}"))
        }
        Err(error) => Err(format!("Could not inspect .obsidian-at-home: {error}")),
    }?;
    if directory.parent() != Some(root) {
        return Err("The workspace metadata path escaped the vault.".to_owned());
    }
    Ok(())
}

pub(super) fn prepare_transaction_root(
    root: &Path,
    transaction_id: &str,
) -> Result<PathBuf, String> {
    let state_directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &state_directory)?;
    validate_transaction_id(transaction_id)?;
    let transactions_directory = state_directory.join(TRANSACTIONS_DIRECTORY);
    ensure_regular_directory(&transactions_directory, "save transactions")?;
    let save_directory = transactions_directory.join(transaction_id);
    match create_directory_durable(&save_directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err("A save transaction with the same ID already exists.".to_owned());
        }
        Err(error) => return Err(format!("Could not create the save transaction: {error}")),
    }
    Ok(save_directory)
}

pub(super) fn validate_transaction_id(transaction_id: &str) -> Result<(), String> {
    if transaction_id.is_empty()
        || transaction_id.len() > 180
        || transaction_id
            .chars()
            .any(|character| !character.is_ascii_alphanumeric() && character != '-')
    {
        return Err("The save transaction ID is invalid.".to_owned());
    }

    Ok(())
}

pub(super) fn existing_transaction_root(
    root: &Path,
    transaction_id: &str,
) -> Result<PathBuf, String> {
    validate_transaction_id(transaction_id)?;
    let state_directory = root.join(STATE_DIRECTORY);
    let transactions_directory = state_directory.join(TRANSACTIONS_DIRECTORY);
    let transaction_root = transactions_directory.join(transaction_id);
    for (path, label) in [
        (&state_directory, "workspace metadata"),
        (&transactions_directory, "save transactions"),
        (&transaction_root, "asset import transaction"),
    ] {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("Could not inspect the {label} folder: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!("The {label} path is not a regular folder."));
        }
    }

    Ok(transaction_root)
}

pub(super) fn ensure_regular_directory(path: &Path, label: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("The {label} folder cannot be a symbolic link."))
        }
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(format!("The {label} path is not a folder.")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_directory_durable(path)
            .map_err(|error| format!("Could not create the {label} folder: {error}")),
        Err(error) => Err(format!("Could not inspect the {label} folder: {error}")),
    }
}

pub(super) fn resolve_workspace_image_file(
    root: &Path,
    relative_path: &str,
    allow_missing: bool,
) -> Result<PathBuf, String> {
    validate_image_relative_path(relative_path)?;
    let relative = checked_relative_path(relative_path, false)?;
    let target = root.join(&relative);
    if let Some(parent) = target.parent() {
        let mut current = root.to_path_buf();
        for component in parent
            .strip_prefix(root)
            .map_err(|_| "An image path escaped the vault.".to_owned())?
            .components()
        {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(format!(
                        "Refusing to follow the symbolic link {}.",
                        current.display()
                    ));
                }
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => return Err(format!("{} is not a folder.", current.display())),
                Err(error) if error.kind() == io::ErrorKind::NotFound && allow_missing => break,
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => {
                    return Err(format!("Could not inspect {}: {error}", current.display()));
                }
            }
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing to use the symbolic link {}.",
                target.display()
            ));
        }
    }
    Ok(target)
}

pub(super) fn resolve_workspace_asset_file(
    root: &Path,
    relative_path: &str,
    allow_missing: bool,
) -> Result<PathBuf, String> {
    validate_attachment_relative_path(relative_path)?;
    let relative = checked_relative_path(relative_path, false)?;
    let target = root.join(&relative);
    if let Some(parent) = target.parent() {
        let mut current = root.to_path_buf();
        for component in parent
            .strip_prefix(root)
            .map_err(|_| "An attachment path escaped the vault.".to_owned())?
            .components()
        {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(format!(
                        "Refusing to follow the symbolic link {}.",
                        current.display()
                    ));
                }
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => return Err(format!("{} is not a folder.", current.display())),
                Err(error) if error.kind() == io::ErrorKind::NotFound && allow_missing => break,
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => {
                    return Err(format!("Could not inspect {}: {error}", current.display()));
                }
            }
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing to use the symbolic link {}.",
                target.display()
            ));
        }
    }
    Ok(target)
}

pub(super) fn resolve_workspace_file(
    root: &Path,
    relative_path: &str,
    allow_missing: bool,
) -> Result<PathBuf, String> {
    let relative = checked_relative_path(relative_path, true)?;
    let target = root.join(&relative);
    if let Some(parent) = target.parent() {
        let mut current = root.to_path_buf();
        for component in parent
            .strip_prefix(root)
            .map_err(|_| "A note path escaped the vault.".to_owned())?
            .components()
        {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(format!(
                        "Refusing to follow the symbolic link {}.",
                        current.display()
                    ));
                }
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => return Err(format!("{} is not a folder.", current.display())),
                Err(error) if error.kind() == io::ErrorKind::NotFound && allow_missing => break,
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => {
                    return Err(format!("Could not inspect {}: {error}", current.display()));
                }
            }
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing to use the symbolic link {}.",
                target.display()
            ));
        }
    }
    Ok(target)
}

pub(super) fn checked_relative_path(relative_path: &str, file: bool) -> Result<PathBuf, String> {
    validate_relative_path(relative_path, file)?;
    let mut result = PathBuf::new();
    for component in relative_path.split('/') {
        result.push(component);
    }
    Ok(result)
}

pub(super) fn validate_relative_path(relative_path: &str, file: bool) -> Result<(), String> {
    if relative_path.is_empty()
        || relative_path.starts_with('/')
        || relative_path.starts_with('\\')
        || relative_path.contains('\\')
    {
        return Err("Paths must be relative to the vault and use forward slashes.".to_owned());
    }
    let path = Path::new(relative_path);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("Parent, root, and drive-qualified paths are not allowed.".to_owned());
    }
    let components: Vec<&str> = relative_path.split('/').collect();
    if components.len() > MAX_PATH_COMPONENTS {
        return Err("The path contains too many nested folders.".to_owned());
    }
    for (index, component) in components.iter().enumerate() {
        if component.is_empty() || *component == "." || *component == ".." {
            return Err(
                "Empty, current-directory, and parent-directory segments are not allowed."
                    .to_owned(),
            );
        }
        if is_reserved_workspace_directory(component) {
            return Err(
                "App settings, Obsidian settings, trash, and Git folders are reserved.".to_owned(),
            );
        }
        if component.len() > 255 || component.chars().any(char::is_control) {
            return Err("A path segment is too long or contains control characters.".to_owned());
        }
        if !file || index + 1 < components.len() {
            validate_component_name(component, "folder")?;
        }
    }
    if file && !is_markdown_path(path) {
        return Err("Note paths must end in .md or .markdown.".to_owned());
    }
    Ok(())
}

pub(super) fn validate_markdown_relative_path(path: &str) -> Result<(), String> {
    validate_relative_path(path, true)
}

pub(super) fn validate_component_name(name: &str, kind: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err(format!("Enter a valid {kind} name."));
    }
    if name.len() > 180
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(is_forbidden_component_character)
        || is_windows_reserved_name(name)
        || is_reserved_workspace_directory(name)
    {
        return Err(format!(
            "The {kind} name contains characters that are not safe in a path."
        ));
    }
    Ok(())
}

pub(super) fn validate_workspace_root(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose a vault folder.".to_owned());
    }
    validate_workspace_root_path(Path::new(input))
}

pub(super) fn validate_workspace_root_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("The vault path must be absolute.".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The vault folder could not be opened: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("The vault folder cannot be a symbolic link.".to_owned());
    }
    if !metadata.is_dir() {
        return Err("The vault path is not a folder.".to_owned());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("The vault folder could not be resolved: {error}"))?;
    if canonical.parent().is_none() {
        return Err("The filesystem root cannot be used as a vault.".to_owned());
    }
    if canonical
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(is_reserved_workspace_directory)
    {
        return Err(
            "An app settings, trash, or Git folder cannot be opened as a vault.".to_owned(),
        );
    }
    Ok(canonical)
}

pub(super) fn reject_home_vault(app: &AppHandle, root: &Path) -> Result<(), String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Could not resolve the home folder: {error}"))?;
    if home
        .canonicalize()
        .ok()
        .as_deref()
        .is_some_and(|home| home == root)
    {
        return Err(
            "Your home folder is too broad to use as a vault. Create or choose a dedicated folder inside it instead."
                .to_owned(),
        );
    }
    Ok(())
}

pub(super) fn reject_nested_registered_vault(
    root: &Path,
    registry: &WorkspaceRegistry,
    allow_same: bool,
) -> Result<(), String> {
    for registered in &registry.recent_vaults {
        let Ok(registered_root) = Path::new(&registered.path).canonicalize() else {
            continue;
        };
        if root == registered_root && allow_same {
            continue;
        }
        if root == registered_root
            || root.starts_with(&registered_root)
            || registered_root.starts_with(root)
        {
            return Err(format!(
                "Vaults cannot be nested. Choose a folder outside {}.",
                registered_root.display()
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_parent_directory(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose a parent folder for the new vault.".to_owned());
    }
    let path = Path::new(input);
    if !path.is_absolute() {
        return Err("The parent folder path must be absolute.".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The parent folder could not be opened: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The parent path must be a regular folder, not a symbolic link.".to_owned());
    }
    path.canonicalize()
        .map_err(|error| format!("The parent folder could not be resolved: {error}"))
}

pub(super) fn should_visit_workspace_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if entry.file_type().is_symlink() {
        return false;
    }
    if is_nested_vault_directory(entry) {
        return false;
    }
    !entry.file_type().is_dir()
        || !is_reserved_workspace_directory(&entry.file_name().to_string_lossy())
}

pub(super) fn should_visit_revision_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if entry.file_type().is_symlink() {
        return false;
    }
    if is_nested_vault_directory(entry) {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    let inside_state_directory = entry
        .path()
        .components()
        .any(|component| component.as_os_str() == STATE_DIRECTORY);
    if inside_state_directory {
        if entry.depth() == 1 && name.eq_ignore_ascii_case(STATE_DIRECTORY) {
            return true;
        }

        return entry.depth() == 2
            && !entry.file_type().is_dir()
            && name.eq_ignore_ascii_case(STATE_FILE);
    }
    if !entry.file_type().is_dir() {
        return true;
    }
    !name.eq_ignore_ascii_case(".obsidian")
        && !name.eq_ignore_ascii_case(".trash")
        && !name.eq_ignore_ascii_case(".git")
}

pub(super) fn is_nested_vault_directory(entry: &DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return false;
    }
    let state_directory = entry.path().join(STATE_DIRECTORY);
    let Ok(state_directory_metadata) = fs::symlink_metadata(&state_directory) else {
        return false;
    };
    if state_directory_metadata.file_type().is_symlink() || !state_directory_metadata.is_dir() {
        return false;
    }
    let state_file = state_directory.join(STATE_FILE);
    fs::symlink_metadata(state_file)
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

pub(super) fn is_reserved_workspace_directory(name: &str) -> bool {
    name.eq_ignore_ascii_case(STATE_DIRECTORY)
        || name.eq_ignore_ascii_case(".obsidian")
        || name.eq_ignore_ascii_case(".trash")
        || name.eq_ignore_ascii_case(".git")
}

pub(super) fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
}

pub(super) fn safe_file_stem(input: &str, fallback: &str) -> String {
    let mut result = String::new();
    let mut previous_was_replacement = false;
    for character in input.trim().chars() {
        if character.is_control()
            || matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
        {
            if !previous_was_replacement {
                result.push('-');
                previous_was_replacement = true;
            }
        } else {
            result.push(character);
            previous_was_replacement = false;
        }
        if result.len() >= 120 {
            break;
        }
    }
    let result = result.trim_matches(|character| character == ' ' || character == '.');
    let result = if result.is_empty() { fallback } else { result };
    if is_windows_reserved_name(result) {
        format!("_{result}")
    } else {
        result.to_owned()
    }
}

pub(super) fn is_forbidden_component_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
        )
}

pub(super) fn is_windows_reserved_name(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(
        base.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

pub(super) fn ensure_private_directory_tree(root: &Path, directory: &Path) -> io::Result<()> {
    let relative = directory.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "private directory escaped its transaction",
        )
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private directory contains a symbolic link",
                ));
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "private directory path is not a folder",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                create_directory_durable(&current)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(super) fn create_directory_durable(path: &Path) -> io::Result<()> {
    fs::create_dir(path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

pub(super) fn set_file_modified_millis(path: &Path, modified_at: u64) -> io::Result<()> {
    let modified = UNIX_EPOCH
        .checked_add(Duration::from_millis(modified_at))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "modified time is too large"))?;
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "modified-time target is not a regular file",
        ));
    }
    file.set_times(FileTimes::new().set_modified(modified))?;
    file.sync_all()
}

pub(super) fn remove_file_durable(path: &Path) -> io::Result<()> {
    fs::remove_file(path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

pub(super) fn remove_directory_durable(path: &Path) -> io::Result<()> {
    fs::remove_dir(path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

pub(super) fn rename_durable(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)?;
    if let Some(source_parent) = source.parent() {
        sync_directory(source_parent)?;
    }
    if target.parent() != source.parent() {
        if let Some(target_parent) = target.parent() {
            sync_directory(target_parent)?;
        }
    }
    Ok(())
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file has no parent"))?;
    fs::create_dir_all(parent)?;
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("workspace");
    let temporary = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        counter
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);

        #[cfg(windows)]
        {
            if path.exists() {
                let backup = parent.join(format!(
                    ".{file_name}.{}.{}.bak",
                    std::process::id(),
                    counter
                ));
                fs::rename(path, &backup)?;
                if let Err(error) = fs::rename(&temporary, path) {
                    let _ = fs::rename(&backup, path);

                    return Err(error);
                }
                let _ = fs::remove_file(backup);
                sync_directory(parent)?;

                return Ok(());
            }
        }

        fs::rename(&temporary, path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
pub(super) fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub(super) fn registry_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(REGISTRY_FILE))
        .map_err(|error| format!("Could not locate the app configuration folder: {error}"))
}

pub(super) fn workspace_state_path(root: &Path) -> PathBuf {
    root.join(STATE_DIRECTORY).join(STATE_FILE)
}

pub(super) fn recently_deleted_snapshot_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    validate_recently_deleted_id(id)?;
    Ok(root
        .join(STATE_DIRECTORY)
        .join(RECENTLY_DELETED_DIRECTORY)
        .join(format!("{id}.snapshot")))
}

pub(super) fn transaction_recovery_snapshot_path(
    transaction_root: &Path,
    id: &str,
) -> Result<PathBuf, String> {
    validate_recently_deleted_id(id)?;
    let relative_path = format!("recoveries/{id}.snapshot");
    Ok(transaction_root.join(checked_internal_transaction_path(&relative_path, true)?))
}

pub(super) fn ensure_recently_deleted_directory(root: &Path) -> Result<PathBuf, String> {
    let state_directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &state_directory)?;
    let directory = state_directory.join(RECENTLY_DELETED_DIRECTORY);
    ensure_regular_directory(&directory, "Recently Deleted")?;

    Ok(directory)
}

pub(super) fn inspect_recently_deleted_directory(root: &Path) -> Result<PathBuf, String> {
    let state_directory = root.join(STATE_DIRECTORY);
    let state_metadata = fs::symlink_metadata(&state_directory)
        .map_err(|error| format!("Could not inspect workspace metadata: {error}"))?;
    if state_metadata.file_type().is_symlink() || !state_metadata.is_dir() {
        return Err("The workspace metadata path is not a regular folder.".to_owned());
    }
    let directory = state_directory.join(RECENTLY_DELETED_DIRECTORY);
    let metadata = fs::symlink_metadata(&directory)
        .map_err(|error| format!("Could not inspect Recently Deleted: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The Recently Deleted path is not a regular folder.".to_owned());
    }

    Ok(directory)
}

pub(super) fn editor_positions_path(root: &Path) -> PathBuf {
    root.join(STATE_DIRECTORY).join(EDITOR_POSITIONS_FILE)
}

pub(super) fn path_string(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "The selected path is not valid Unicode.".to_owned())
}

pub(super) fn path_to_slash_string(path: &Path) -> Option<String> {
    let mut result = String::new();
    for component in path.components() {
        let component = component.as_os_str().to_str()?;
        if !result.is_empty() {
            result.push('/');
        }
        result.push_str(component);
    }
    Some(result)
}

pub(super) fn canonical_path_if_available(path: &str) -> String {
    Path::new(path)
        .canonicalize()
        .ok()
        .and_then(|path| path_to_slash_string(&path))
        .unwrap_or_else(|| path.replace('\\', "/"))
}

pub(super) fn metadata_time_millis(metadata: &fs::Metadata, prefer_created: bool) -> u64 {
    let time = if prefer_created {
        metadata.created().or_else(|_| metadata.modified())
    } else {
        metadata.modified().or_else(|_| metadata.created())
    };
    time.ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_else(now_millis)
}

pub(super) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

pub(super) fn fresh_id(prefix: &str, value: &str, used: &mut HashSet<String>) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    fnv_update(&mut hash, value.as_bytes());
    let base = format!("{prefix}-{hash:016x}");
    if used.insert(base.clone()) {
        return base;
    }
    for index in 2..10_000 {
        let candidate = format!("{base}-{index}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    format!("{base}-{}", now_millis())
}

pub(super) fn display_vault_name(requested: &str, root: &Path) -> String {
    if !requested.trim().is_empty() {
        return requested.trim().to_owned();
    }
    root.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Vault")
        .to_owned()
}

pub(super) fn normalize_recent_note_ids(
    recent_note_ids: &[String],
    active_note_id: Option<&str>,
    note_ids: &HashSet<&str>,
) -> Vec<String> {
    let candidates = active_note_id
        .into_iter()
        .chain(recent_note_ids.iter().map(String::as_str));
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();

    for id in candidates {
        if normalized.len() >= MAX_RECENT_NOTES {
            break;
        }
        if note_ids.contains(id) && seen.insert(id.to_owned()) {
            normalized.push(id.to_owned());
        }
    }

    normalized
}

pub(super) fn is_virtual_folder_selection(value: &str) -> bool {
    matches!(value, "all" | "favorites" | "recent")
}

pub(super) fn trim_line_ending(line: &str) -> &str {
    line.strip_suffix('\n')
        .unwrap_or(line)
        .strip_suffix('\r')
        .unwrap_or_else(|| line.strip_suffix('\n').unwrap_or(line))
}

pub(super) fn state_version() -> u32 {
    STATE_VERSION
}

pub(super) fn editor_positions_version() -> u32 {
    EDITOR_POSITIONS_VERSION
}

pub(super) fn registry_version() -> u32 {
    REGISTRY_VERSION
}

pub(super) fn default_folder_selection() -> String {
    "all".to_owned()
}

pub(super) fn lock_workspace_io() -> Result<MutexGuard<'static, ()>, String> {
    WORKSPACE_IO_LOCK.lock().map_err(|_| {
        "Workspace storage is unavailable because an earlier operation failed.".to_owned()
    })
}

#[derive(Default)]
pub(super) struct WarningCollector {
    pub(super) warnings: Vec<String>,
    pub(super) truncated: bool,
}

impl WarningCollector {
    pub(super) fn push(&mut self, warning: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(warning);
        } else {
            self.truncated = true;
        }
    }

    pub(super) fn finish(mut self) -> Vec<String> {
        if self.truncated {
            self.warnings
                .push("Additional warnings were omitted.".to_owned());
        }
        self.warnings
    }
}
