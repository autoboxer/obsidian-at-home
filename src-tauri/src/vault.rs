use crate::workspace::{
    copy_attachment_file_for_transfer, is_supported_image_path, validate_image_bytes,
    MAX_ATTACHMENT_BYTES, MAX_IMAGE_BYTES,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use walkdir::{DirEntry, WalkDir};

pub(crate) mod dialogs;
pub(crate) mod export;
pub(crate) mod import;

pub use dialogs::*;
pub use export::*;
pub use import::*;

#[cfg(test)]
mod tests;

const MAX_NOTE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_SNIPPET_BYTES: u64 = 5 * 1024 * 1024;
const MAX_TOTAL_IMPORT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_IMPORTED_ASSETS: usize = 100_000;
const MAX_IMPORTED_NOTES: usize = 100_000;
const MAX_WARNINGS: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedNote {
    pub title: String,
    pub content: String,
    pub folder_path: String,
    pub relative_path: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedImage {
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedAttachment {
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultSnippet {
    pub name: String,
    pub css: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub vault_name: String,
    pub images: Vec<ImportedImage>,
    pub attachments: Vec<ImportedAttachment>,
    pub notes: Vec<ImportedNote>,
    pub snippets: Vec<VaultSnippet>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportNote {
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub folder_path: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTemplate {
    pub name: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub image_count: usize,
    pub attachment_count: usize,
    pub note_count: usize,
    pub template_count: usize,
    pub snippet_count: usize,
    pub warnings: Vec<String>,
}

fn validate_import_root(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose an Obsidian vault folder to import.".to_owned());
    }
    let path = Path::new(input);
    if !path.is_absolute() {
        return Err("The vault path must be absolute.".to_owned());
    }
    let symlink_metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The selected vault could not be opened: {error}"))?;
    if symlink_metadata.file_type().is_symlink() {
        return Err("The selected vault folder cannot be a symbolic link.".to_owned());
    }
    if !symlink_metadata.is_dir() {
        return Err("The selected vault path is not a folder.".to_owned());
    }
    path.canonicalize()
        .map_err(|error| format!("The selected vault could not be resolved: {error}"))
}

fn validate_export_parent(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose a destination folder for the export.".to_owned());
    }
    let path = Path::new(input);
    if !path.is_absolute() {
        return Err("The export destination must be an absolute path.".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The export destination could not be opened: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("The export destination cannot be a symbolic link.".to_owned());
    }
    if !metadata.is_dir() {
        return Err("The export destination is not a folder.".to_owned());
    }
    path.canonicalize()
        .map_err(|error| format!("The export destination could not be resolved: {error}"))
}

fn validate_vault_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a name for the exported vault.".to_owned());
    }
    if name.len() > 180 {
        return Err("The exported vault name is too long.".to_owned());
    }
    if name == "." || name == ".." {
        return Err("The exported vault name is not valid.".to_owned());
    }
    if name.ends_with('.') || name.chars().any(is_forbidden_component_character) {
        return Err(
            "The exported vault name contains characters that are not safe in a folder name."
                .to_owned(),
        );
    }
    if is_windows_reserved_name(name) {
        return Err("The exported vault name is reserved by the operating system.".to_owned());
    }
    Ok(())
}

fn create_unique_export_dir(parent: &Path, name: &str) -> io::Result<PathBuf> {
    for index in 0..10_000_u32 {
        let child_name = if index == 0 {
            name.to_owned()
        } else {
            format!("{name} ({index})")
        };
        let candidate = parent.join(child_name);
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many exports already use this name",
    ))
}

fn should_visit_note_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if entry.file_type().is_symlink() {
        return false;
    }
    if entry.file_type().is_dir() {
        let name = entry.file_name().to_string_lossy();

        return !name.eq_ignore_ascii_case(".obsidian")
            && !name.eq_ignore_ascii_case(".git")
            && !name.eq_ignore_ascii_case(".trash")
            && !name.eq_ignore_ascii_case(".obsidian-at-home");
    }
    true
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
        .unwrap_or(false)
}

fn is_canvas_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("canvas"))
        .unwrap_or(false)
}

fn collect_portable_assets(
    root: &Path,
    warnings: &mut WarningCollector,
) -> (Vec<ImportedImage>, Vec<ImportedAttachment>) {
    let mut images = Vec::new();
    let mut attachments = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
        .filter_entry(should_visit_note_entry)
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("Could not inspect a vault entry: {error}"));
                continue;
            }
        };
        if entry.file_type().is_symlink() || !entry.file_type().is_file() {
            continue;
        }
        if is_markdown_path(entry.path()) || is_canvas_path(entry.path()) {
            continue;
        }
        if images.len().saturating_add(attachments.len()) >= MAX_IMPORTED_ASSETS {
            warnings.push(format!(
                "Only the first {MAX_IMPORTED_ASSETS} asset files can be transferred."
            ));
            continue;
        }
        if is_supported_image_path(entry.path()) {
            if let Some(image) = inspect_portable_image(root, &entry, warnings) {
                images.push(image);
            }
        } else if let Some(attachment) = inspect_portable_attachment(root, &entry, warnings) {
            attachments.push(attachment);
        }
    }
    images.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    attachments.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    (images, attachments)
}

fn inspect_portable_image(
    root: &Path,
    entry: &DirEntry,
    warnings: &mut WarningCollector,
) -> Option<ImportedImage> {
    let metadata = match entry.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            warnings.push(format!(
                "Skipped {} because its metadata could not be read: {error}",
                display_relative(root, entry.path())
            ));
            return None;
        }
    };
    if metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES {
        warnings.push(format!(
            "Skipped {} because images must be between 1 byte and {} MiB.",
            display_relative(root, entry.path()),
            MAX_IMAGE_BYTES / 1024 / 1024
        ));
        return None;
    }
    let Some(relative_path) = entry
        .path()
        .strip_prefix(root)
        .ok()
        .and_then(path_to_slash_string)
    else {
        warnings.push(format!(
            "Skipped an image whose path is not valid Unicode: {}",
            entry.path().display()
        ));
        return None;
    };
    if let Err(reason) = checked_relative_image_path(&relative_path) {
        warnings.push(format!(
            "Skipped {relative_path} because its image path is unsafe: {reason}"
        ));
        return None;
    }
    Some(ImportedImage { relative_path })
}

fn inspect_portable_attachment(
    root: &Path,
    entry: &DirEntry,
    warnings: &mut WarningCollector,
) -> Option<ImportedAttachment> {
    let metadata = match entry.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            warnings.push(format!(
                "Skipped {} because its metadata could not be read: {error}",
                display_relative(root, entry.path())
            ));
            return None;
        }
    };
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        warnings.push(format!(
            "Skipped {} because attachments must be no larger than {} GiB.",
            display_relative(root, entry.path()),
            MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024
        ));
        return None;
    }
    let Some(relative_path) = entry
        .path()
        .strip_prefix(root)
        .ok()
        .and_then(path_to_slash_string)
    else {
        warnings.push(format!(
            "Skipped an attachment whose path is not valid Unicode: {}",
            entry.path().display()
        ));
        return None;
    };
    if let Err(reason) = checked_relative_attachment_path(&relative_path) {
        warnings.push(format!(
            "Skipped {relative_path} because its attachment path is unsafe: {reason}"
        ));
        return None;
    }
    Some(ImportedAttachment { relative_path })
}

fn export_portable_images(
    source_root: &Path,
    export_root: &Path,
    images: &[ImportedImage],
    warnings: &mut WarningCollector,
) -> usize {
    let mut copied = 0;
    for image in images {
        let relative = match checked_relative_image_path(&image.relative_path) {
            Ok(relative) => relative,
            Err(reason) => {
                warnings.push(format!(
                    "Skipped {} because its image path is unsafe: {reason}",
                    image.relative_path
                ));
                continue;
            }
        };
        let source = match resolve_portable_source_file(source_root, &relative, "image") {
            Ok(source) => source,
            Err(error) => {
                warnings.push(format!("Skipped {}: {error}", image.relative_path));
                continue;
            }
        };
        let bytes = match fs::read(&source) {
            Ok(bytes) => bytes,
            Err(error) => {
                warnings.push(format!("Could not read {}: {error}", image.relative_path));
                continue;
            }
        };
        if let Err(error) = validate_image_bytes(&bytes, Some(&image.relative_path)) {
            warnings.push(format!("Skipped {}: {error}", image.relative_path));
            continue;
        }

        let target = export_root.join(&relative);
        if let Some(parent) = target.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                warnings.push(format!(
                    "Could not create the folder for {}: {error}",
                    image.relative_path
                ));
                continue;
            }
        }
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(file) => file,
            Err(error) => {
                warnings.push(format!("Could not export {}: {error}", image.relative_path));
                continue;
            }
        };
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&target);
            warnings.push(format!("Could not export {}: {error}", image.relative_path));
            continue;
        }
        copied += 1;
    }
    copied
}

fn export_portable_attachments(
    source_root: &Path,
    export_root: &Path,
    attachments: &[ImportedAttachment],
    warnings: &mut WarningCollector,
) -> usize {
    let mut copied = 0;
    for attachment in attachments {
        let relative = match checked_relative_attachment_path(&attachment.relative_path) {
            Ok(relative) => relative,
            Err(reason) => {
                warnings.push(format!(
                    "Skipped {} because its attachment path is unsafe: {reason}",
                    attachment.relative_path
                ));
                continue;
            }
        };
        let source = match resolve_portable_source_file(source_root, &relative, "attachment") {
            Ok(source) => source,
            Err(error) => {
                warnings.push(format!("Skipped {}: {error}", attachment.relative_path));
                continue;
            }
        };
        let target = export_root.join(&relative);
        if let Some(parent) = target.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                warnings.push(format!(
                    "Could not create the folder for {}: {error}",
                    attachment.relative_path
                ));
                continue;
            }
        }
        match copy_portable_attachment(&source, &target) {
            Ok(()) => copied += 1,
            Err(error) => warnings.push(format!(
                "Could not export {}: {error}",
                attachment.relative_path
            )),
        }
    }
    copied
}

fn copy_portable_attachment(source: &Path, target: &Path) -> Result<(), String> {
    copy_attachment_file_for_transfer(source, target)
}

fn resolve_portable_source_file(
    root: &Path,
    relative: &Path,
    label: &str,
) -> Result<PathBuf, String> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("the source {label} could not be inspected: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("symbolic links are not followed".to_owned());
        }
    }
    let metadata = fs::symlink_metadata(&current)
        .map_err(|error| format!("the source {label} could not be inspected: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("the source {label} is not a regular file"));
    }
    Ok(current)
}

fn import_snippets(root: &Path, warnings: &mut WarningCollector) -> Vec<VaultSnippet> {
    let obsidian = root.join(".obsidian");
    if is_symlink(&obsidian) {
        warnings.push("Skipped .obsidian because it is a symbolic link.".to_owned());

        return Vec::new();
    }

    let snippets_directory = obsidian.join("snippets");
    if is_symlink(&snippets_directory) {
        warnings.push("Skipped .obsidian/snippets because it is a symbolic link.".to_owned());

        return Vec::new();
    }
    if !snippets_directory.is_dir() {
        return Vec::new();
    }

    let enabled = read_enabled_snippets(&obsidian, warnings);
    let entries = match fs::read_dir(&snippets_directory) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!("Could not read CSS snippets: {error}"));

            return Vec::new();
        }
    };

    let mut snippets = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("Could not inspect a CSS snippet: {error}"));
                continue;
            }
        };
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!("Could not inspect {}: {error}", path.display()));
                continue;
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("css"))
            .unwrap_or(false)
        {
            continue;
        }
        if metadata.len() > MAX_SNIPPET_BYTES {
            warnings.push(format!(
                "Skipped CSS snippet {} because it is larger than {} MiB.",
                entry.file_name().to_string_lossy(),
                MAX_SNIPPET_BYTES / 1024 / 1024
            ));
            continue;
        }
        let name = match path.file_stem().and_then(|stem| stem.to_str()) {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => {
                warnings.push(format!(
                    "Skipped a CSS snippet with an invalid name: {}",
                    path.display()
                ));
                continue;
            }
        };
        let css = match fs::read_to_string(&path) {
            Ok(css) => css,
            Err(error) => {
                warnings.push(format!(
                    "Skipped CSS snippet {:?} because it is not readable UTF-8: {error}",
                    name
                ));
                continue;
            }
        };
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let is_enabled = enabled.contains(&name) || enabled.contains(&file_name);
        snippets.push(VaultSnippet {
            name,
            css,
            enabled: is_enabled,
        });
    }
    snippets.sort_by(|left, right| left.name.cmp(&right.name));
    snippets
}

fn read_enabled_snippets(obsidian: &Path, warnings: &mut WarningCollector) -> HashSet<String> {
    let path = obsidian.join("appearance.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return HashSet::new(),
        Err(error) => {
            warnings.push(format!(
                "Could not inspect .obsidian/appearance.json: {error}"
            ));

            return HashSet::new();
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        warnings.push("Skipped .obsidian/appearance.json because it is not a regular file.".into());

        return HashSet::new();
    }
    if metadata.len() > 1024 * 1024 {
        warnings.push("Skipped .obsidian/appearance.json because it is unexpectedly large.".into());

        return HashSet::new();
    }
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            warnings.push(format!("Could not read .obsidian/appearance.json: {error}"));

            return HashSet::new();
        }
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!(
                "Could not parse .obsidian/appearance.json; snippets were imported as disabled: {error}"
            ));

            return HashSet::new();
        }
    };
    value
        .get("enabledCssSnippets")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct BasicFrontmatter {
    title: Option<String>,
    tags: Vec<String>,
}

fn parse_basic_frontmatter(content: &str) -> BasicFrontmatter {
    let Some(body) = frontmatter_body(content) else {
        return BasicFrontmatter::default();
    };

    let mut parsed = BasicFrontmatter::default();
    let mut reading_tag_list = false;

    for raw_line in body.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let is_indented = raw_line.starts_with(' ') || raw_line.starts_with('\t');
        if reading_tag_list && is_indented {
            if let Some(value) = trimmed.strip_prefix('-') {
                push_tag(&mut parsed.tags, parse_yaml_scalar(value.trim()));
            }
            continue;
        }
        reading_tag_list = false;

        if is_indented {
            continue;
        }
        let Some((key, value)) = raw_line.split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("title") {
            let title = parse_yaml_scalar(value.trim());
            if !title.is_empty() {
                parsed.title = Some(title);
            }
        } else if key.trim().eq_ignore_ascii_case("tags") {
            let value = value.trim();
            if value.is_empty() {
                reading_tag_list = true;
            } else if value.starts_with('[') {
                for tag in parse_inline_yaml_list(value) {
                    push_tag(&mut parsed.tags, tag);
                }
            } else {
                push_tag(&mut parsed.tags, parse_yaml_scalar(value));
            }
        }
    }
    parsed
}

fn frontmatter_body(content: &str) -> Option<&str> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = content.split_inclusive('\n');
    let first = lines.next()?;
    if trim_line_ending(first).trim() != "---" {
        return None;
    }

    let body_start = first.len();
    let mut cursor = body_start;
    for line in lines {
        let trimmed = trim_line_ending(line).trim();
        if trimmed == "---" || trimmed == "..." {
            return Some(&content[body_start..cursor]);
        }
        cursor += line.len();
    }
    None
}

fn trim_line_ending(line: &str) -> &str {
    line.strip_suffix('\n')
        .unwrap_or(line)
        .strip_suffix('\r')
        .unwrap_or_else(|| line.strip_suffix('\n').unwrap_or(line))
}

fn parse_inline_yaml_list(value: &str) -> Vec<String> {
    let Some(end) = value.rfind(']') else {
        return vec![parse_yaml_scalar(value)];
    };
    let inner = &value[1..end];
    let mut values = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;

    for (index, character) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('"') {
            escaped = true;
            continue;
        }
        if character == '\'' || character == '"' {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        } else if character == ',' && quote.is_none() {
            values.push(parse_yaml_scalar(inner[start..index].trim()));
            start = index + character.len_utf8();
        }
    }
    values.push(parse_yaml_scalar(inner[start..].trim()));
    values
}

fn parse_yaml_scalar(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].replace("''", "'");
    }
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let mut output = String::new();
        let mut escaped = false;
        for character in value[1..value.len() - 1].chars() {
            if escaped {
                output.push(match character {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                });
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else {
                output.push(character);
            }
        }
        if escaped {
            output.push('\\');
        }

        return output;
    }

    // In an unquoted YAML scalar, a hash preceded by whitespace starts a comment.
    let without_comment = value
        .find(" #")
        .map(|index| &value[..index])
        .unwrap_or(value);
    without_comment.trim().to_owned()
}

fn push_tag(tags: &mut Vec<String>, tag: String) {
    let tag = tag.trim().trim_start_matches('#').trim();
    if tag.is_empty() || tags.iter().any(|existing| existing == tag) {
        return;
    }
    tags.push(tag.to_owned());
}

fn checked_relative_folder(folder: &str) -> Result<PathBuf, String> {
    if folder.is_empty() {
        return Ok(PathBuf::new());
    }
    if folder.starts_with('/') || folder.starts_with('\\') || folder.contains('\\') {
        return Err("absolute paths and backslashes are not allowed".to_owned());
    }
    let bytes = folder.as_bytes();
    if bytes.get(1) == Some(&b':') && bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return Err("drive-qualified paths are not allowed".to_owned());
    }

    let mut result = PathBuf::new();
    for component in folder.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(
                "empty, current-directory, and parent-directory segments are not allowed"
                    .to_owned(),
            );
        }
        if component.eq_ignore_ascii_case(".obsidian")
            || component.eq_ignore_ascii_case(".trash")
            || component.eq_ignore_ascii_case(".obsidian-at-home")
        {
            return Err(
                "App settings, Obsidian settings, and trash folders are reserved".to_owned(),
            );
        }
        if component.len() > 255 || component.chars().any(char::is_control) {
            return Err("a folder name is too long or contains control characters".to_owned());
        }
        result.push(component);
    }
    Ok(result)
}

fn checked_relative_image_path(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains('\\') {
        return Err("absolute paths and backslashes are not allowed".to_owned());
    }
    let candidate = Path::new(path);
    if !is_supported_image_path(candidate) {
        return Err("the file extension is not a supported image type".to_owned());
    }
    let file_name = candidate
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && *value != "." && *value != "..")
        .ok_or_else(|| "the image file name is invalid".to_owned())?;
    if file_name.len() > 255
        || file_name.ends_with('.')
        || file_name.ends_with(' ')
        || file_name.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
        })
        || is_windows_reserved_name(file_name)
    {
        return Err("the image file name is not portable".to_owned());
    }
    let folder = candidate
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    let mut relative = checked_relative_folder(&folder)?;
    relative.push(file_name);
    Ok(relative)
}

fn checked_relative_attachment_path(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains('\\') {
        return Err("absolute paths and backslashes are not allowed".to_owned());
    }
    let candidate = Path::new(path);
    if is_markdown_path(candidate) {
        return Err("Markdown files are notes, not attachments".to_owned());
    }
    if is_canvas_path(candidate) {
        return Err("Canvas files are not attachments".to_owned());
    }
    if is_supported_image_path(candidate) {
        return Err("supported image files must be transferred as images".to_owned());
    }
    let file_name = candidate
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && *value != "." && *value != "..")
        .ok_or_else(|| "the attachment file name is invalid".to_owned())?;
    if file_name.len() > 255
        || file_name.ends_with('.')
        || file_name.ends_with(' ')
        || file_name.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
        })
        || is_windows_reserved_name(file_name)
    {
        return Err("the attachment file name is not portable".to_owned());
    }
    let folder = candidate
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    let mut relative = checked_relative_folder(&folder)?;
    relative.push(file_name);
    Ok(relative)
}

fn safe_file_stem(input: &str, fallback: &str) -> String {
    let input = strip_extension_case_insensitive(input.trim(), "md");
    let mut result = String::with_capacity(input.len().min(120));
    let mut last_was_replacement = false;
    for character in input.chars() {
        let forbidden = character.is_control()
            || matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            );
        if forbidden {
            if !last_was_replacement {
                result.push('-');
                last_was_replacement = true;
            }
        } else {
            result.push(character);
            last_was_replacement = false;
        }
        if result.len() >= 120 {
            break;
        }
    }
    while !result.is_char_boundary(result.len()) {
        result.pop();
    }
    let result = result.trim_matches(|character| character == ' ' || character == '.');
    let result = if result.is_empty() { fallback } else { result };
    if is_windows_reserved_name(result) {
        format!("_{result}")
    } else {
        result.to_owned()
    }
}

fn strip_extension_case_insensitive<'a>(value: &'a str, extension: &str) -> &'a str {
    let suffix_length = extension.len() + 1;
    if value.len() > suffix_length {
        let suffix = &value[value.len() - suffix_length..];
        if suffix.starts_with('.') && suffix[1..].eq_ignore_ascii_case(extension) {
            return &value[..value.len() - suffix_length];
        }
    }
    value
}

fn is_windows_reserved_name(name: &str) -> bool {
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

fn is_forbidden_component_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
        )
}

fn render_note_markdown(title: &str, tags: &[String], content: &str) -> String {
    if frontmatter_body(content).is_some() {
        return content.to_owned();
    }

    let title = if title.trim().is_empty() {
        "Untitled"
    } else {
        title.trim()
    };
    let mut output = String::new();
    output.push_str("---\ntitle: \"");
    output.push_str(&escape_yaml_double_quoted(title));
    output.push_str("\"\n");

    let mut normalized_tags = Vec::new();
    for tag in tags {
        push_tag(&mut normalized_tags, tag.clone());
    }
    if !normalized_tags.is_empty() {
        output.push_str("tags:\n");
        for tag in normalized_tags {
            output.push_str("  - \"");
            output.push_str(&escape_yaml_double_quoted(&tag));
            output.push_str("\"\n");
        }
    }
    output.push_str("---\n");
    if !content.is_empty() {
        output.push('\n');
        output.push_str(content);
    }
    output
}

fn escape_yaml_double_quoted(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other if other.is_control() => escaped.push(' '),
            other => escaped.push(other),
        }
    }
    escaped
}

fn write_unique_text_file(
    directory: &Path,
    stem: &str,
    extension: &str,
    content: &str,
) -> io::Result<PathBuf> {
    for index in 0..10_000_u32 {
        let filename = if index == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem} {index}.{extension}")
        };
        let path = directory.join(filename);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(content.as_bytes()) {
                    drop(file);
                    let _ = fs::remove_file(&path);

                    return Err(error);
                }
                if let Err(error) = file.flush() {
                    drop(file);
                    let _ = fs::remove_file(&path);

                    return Err(error);
                }

                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many files already use this name",
    ))
}

fn write_json_file(path: &Path, value: &serde_json::Value) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.flush()
}

fn path_to_slash_string(path: &Path) -> Option<String> {
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

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(path_to_slash_string)
        .unwrap_or_else(|| path.display().to_string())
}

#[derive(Default)]
struct WarningCollector {
    warnings: Vec<String>,
    truncated: bool,
}

impl WarningCollector {
    fn push(&mut self, warning: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(warning);
        } else {
            self.truncated = true;
        }
    }

    fn finish(mut self) -> Vec<String> {
        if self.truncated {
            self.warnings
                .push("Additional warnings were omitted.".to_owned());
        }
        self.warnings
    }
}
