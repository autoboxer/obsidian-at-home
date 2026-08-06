use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use walkdir::{DirEntry, WalkDir};

const MAX_NOTE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_SNIPPET_BYTES: u64 = 5 * 1024 * 1024;
const MAX_TOTAL_IMPORT_BYTES: u64 = 512 * 1024 * 1024;
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
    pub note_count: usize,
    pub template_count: usize,
    pub snippet_count: usize,
    pub warnings: Vec<String>,
}

/// Opens the system's native directory picker and returns a normal filesystem path.
#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let selected = app.dialog().file().blocking_pick_folder();
    selected
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| format!("The selected folder is not a local filesystem path: {error}"))
        })
        .transpose()
}

/// Reads an Obsidian vault without modifying it.
///
/// Markdown is returned byte-for-byte as UTF-8 text. Frontmatter is only inspected to
/// infer a display title and tags; the caller receives the original content unchanged.
#[tauri::command]
pub fn import_obsidian_vault(path: String) -> Result<ImportResult, String> {
    let root = validate_import_root(&path)?;
    let vault_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Imported vault")
        .to_owned();

    let mut notes = Vec::new();
    let mut warnings = WarningCollector::default();
    let mut total_bytes = 0_u64;

    let walker = WalkDir::new(&root)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
        .filter_entry(should_visit_note_entry);

    for entry in walker {
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
        if !is_markdown_path(entry.path()) {
            continue;
        }
        if notes.len() >= MAX_IMPORTED_NOTES {
            warnings.push(format!(
                "Stopped after {MAX_IMPORTED_NOTES} notes to stay within the import limit."
            ));
            break;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!(
                    "Skipped {} because its metadata could not be read: {error}",
                    display_relative(&root, entry.path())
                ));
                continue;
            }
        };
        if metadata.len() > MAX_NOTE_BYTES {
            warnings.push(format!(
                "Skipped {} because it is larger than {} MiB.",
                display_relative(&root, entry.path()),
                MAX_NOTE_BYTES / 1024 / 1024
            ));
            continue;
        }
        if total_bytes.saturating_add(metadata.len()) > MAX_TOTAL_IMPORT_BYTES {
            warnings.push(format!(
                "Stopped after reading {} MiB of notes to stay within the import limit.",
                MAX_TOTAL_IMPORT_BYTES / 1024 / 1024
            ));
            break;
        }

        let relative = match entry.path().strip_prefix(&root) {
            Ok(relative) => relative,
            Err(_) => {
                warnings.push(format!(
                    "Skipped an entry outside the selected vault: {}",
                    entry.path().display()
                ));
                continue;
            }
        };
        let relative_path = match path_to_slash_string(relative) {
            Some(path) => path,
            None => {
                warnings.push(format!(
                    "Skipped a note whose path is not valid Unicode: {}",
                    entry.path().display()
                ));
                continue;
            }
        };
        let folder_path = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .and_then(path_to_slash_string)
            .unwrap_or_default();

        let content = match fs::read_to_string(entry.path()) {
            Ok(content) => content,
            Err(error) => {
                warnings.push(format!(
                    "Skipped {relative_path} because it is not readable UTF-8: {error}"
                ));
                continue;
            }
        };
        total_bytes += metadata.len();

        let frontmatter = parse_basic_frontmatter(&content);
        let fallback_title = relative
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.trim().is_empty())
            .unwrap_or("Untitled");

        notes.push(ImportedNote {
            title: frontmatter
                .title
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| fallback_title.to_owned()),
            content,
            folder_path,
            relative_path,
            tags: frontmatter.tags,
        });
    }

    notes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let snippets = import_snippets(&root, &mut warnings);

    Ok(ImportResult {
        vault_name,
        notes,
        snippets,
        warnings: warnings.finish(),
    })
}

/// Writes a portable Obsidian vault into a newly-created child of `parent_path`.
/// Existing files and directories are never reused or overwritten.
#[tauri::command(rename_all = "camelCase")]
pub fn export_obsidian_vault(
    parent_path: String,
    vault_name: String,
    notes: Vec<ExportNote>,
    templates: Vec<ExportTemplate>,
    snippets: Vec<VaultSnippet>,
) -> Result<ExportResult, String> {
    let parent = validate_export_parent(&parent_path)?;
    validate_vault_name(&vault_name)?;

    let root = create_unique_export_dir(&parent, vault_name.trim()).map_err(|error| {
        format!(
            "Could not create a new export folder in {}: {error}",
            parent.display()
        )
    })?;

    let mut warnings = WarningCollector::default();
    let mut note_count = 0;
    let mut template_count = 0;
    let mut snippet_count = 0;

    for note in notes {
        if note.content.len() as u64 > MAX_NOTE_BYTES {
            warnings.push(format!(
                "Skipped note {:?} because it is larger than {} MiB.",
                note.title,
                MAX_NOTE_BYTES / 1024 / 1024
            ));
            continue;
        }

        let relative_folder = match checked_relative_folder(&note.folder_path) {
            Ok(folder) => folder,
            Err(reason) => {
                warnings.push(format!(
                    "Skipped note {:?} because its folder path is unsafe: {reason}",
                    note.title
                ));
                continue;
            }
        };
        let directory = root.join(relative_folder);
        if let Err(error) = fs::create_dir_all(&directory) {
            warnings.push(format!(
                "Skipped note {:?} because its folder could not be created: {error}",
                note.title
            ));
            continue;
        }

        let stem = safe_file_stem(&note.title, "Untitled");
        let markdown = render_note_markdown(&note.title, &note.tags, &note.content);
        match write_unique_text_file(&directory, &stem, "md", &markdown) {
            Ok(_) => note_count += 1,
            Err(error) => warnings.push(format!(
                "Could not export note {:?}: {error}",
                note.title
            )),
        }
    }

    if !templates.is_empty() {
        let directory = root.join("Templates");
        match fs::create_dir(&directory) {
            Ok(()) => {
                for template in templates {
                    if template.content.len() as u64 > MAX_NOTE_BYTES {
                        warnings.push(format!(
                            "Skipped template {:?} because it is larger than {} MiB.",
                            template.name,
                            MAX_NOTE_BYTES / 1024 / 1024
                        ));
                        continue;
                    }
                    let stem = safe_file_stem(&template.name, "Untitled template");
                    match write_unique_text_file(&directory, &stem, "md", &template.content) {
                        Ok(_) => template_count += 1,
                        Err(error) => warnings.push(format!(
                            "Could not export template {:?}: {error}",
                            template.name
                        )),
                    }
                }
            }
            Err(error) => warnings.push(format!("Could not create Templates: {error}")),
        }
    }

    if !snippets.is_empty() || template_count > 0 {
        let obsidian_directory = root.join(".obsidian");
        if let Err(error) = fs::create_dir(&obsidian_directory) {
            warnings.push(format!("Could not create .obsidian settings: {error}"));
        } else {
            if template_count > 0 {
                let settings = json!({ "folder": "Templates" });
                if let Err(error) = write_json_file(
                    &obsidian_directory.join("templates.json"),
                    &settings,
                ) {
                    warnings.push(format!("Could not write template settings: {error}"));
                }
            }

            if !snippets.is_empty() {
                let snippet_directory = obsidian_directory.join("snippets");
                if let Err(error) = fs::create_dir(&snippet_directory) {
                    warnings.push(format!("Could not create the CSS snippets folder: {error}"));
                } else {
                    let mut enabled_names = Vec::new();
                    for snippet in snippets {
                        if snippet.css.len() as u64 > MAX_SNIPPET_BYTES {
                            warnings.push(format!(
                                "Skipped CSS snippet {:?} because it is larger than {} MiB.",
                                snippet.name,
                                MAX_SNIPPET_BYTES / 1024 / 1024
                            ));
                            continue;
                        }
                        let stem = safe_file_stem(
                            strip_extension_case_insensitive(&snippet.name, "css"),
                            "snippet",
                        );
                        match write_unique_text_file(&snippet_directory, &stem, "css", &snippet.css)
                        {
                            Ok(path) => {
                                snippet_count += 1;
                                if snippet.enabled {
                                    if let Some(exported_stem) =
                                        path.file_stem().and_then(|value| value.to_str())
                                    {
                                        enabled_names.push(exported_stem.to_owned());
                                    }
                                }
                            }
                            Err(error) => warnings.push(format!(
                                "Could not export CSS snippet {:?}: {error}",
                                snippet.name
                            )),
                        }
                    }

                    let appearance = json!({ "enabledCssSnippets": enabled_names });
                    if let Err(error) = write_json_file(
                        &obsidian_directory.join("appearance.json"),
                        &appearance,
                    ) {
                        warnings.push(format!("Could not write CSS snippet settings: {error}"));
                    }
                }
            }
        }
    }

    Ok(ExportResult {
        path: root.to_string_lossy().into_owned(),
        note_count,
        template_count,
        snippet_count,
        warnings: warnings.finish(),
    })
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
        return Err("The exported vault name contains characters that are not safe in a folder name."
            .to_owned());
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
            && !name.eq_ignore_ascii_case(".trash");
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
            warnings.push(format!("Could not inspect .obsidian/appearance.json: {error}"));
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
            return Err("empty, current-directory, and parent-directory segments are not allowed"
                .to_owned());
        }
        if component.eq_ignore_ascii_case(".obsidian")
            || component.eq_ignore_ascii_case(".trash")
        {
            return Err("Obsidian settings and trash folders are reserved".to_owned());
        }
        if component.len() > 255 || component.chars().any(char::is_control) {
            return Err("a folder name is too long or contains control characters".to_owned());
        }
        result.push(component);
    }
    Ok(result)
}

fn safe_file_stem(input: &str, fallback: &str) -> String {
    let input = strip_extension_case_insensitive(input.trim(), "md");
    let mut result = String::with_capacity(input.len().min(120));
    let mut last_was_replacement = false;
    for character in input.chars() {
        let forbidden = character.is_control()
            || matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
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
        || matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "obsidian-at-home-{label}-{}-{nonce}-{count}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("test directory should be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_basic_frontmatter_without_touching_content() {
        let content = "---\r\ntitle: \"A linked idea\"\r\ntags: [garden, 'in progress', '#garden']\r\nextra: keep me\r\n---\r\n# Body\r\n";
        let parsed = parse_basic_frontmatter(content);

        assert_eq!(parsed.title.as_deref(), Some("A linked idea"));
        assert_eq!(parsed.tags, vec!["garden", "in progress"]);
        assert!(content.contains("extra: keep me"));
    }

    #[test]
    fn parses_block_tags_and_yaml_comments() {
        let parsed = parse_basic_frontmatter(
            "---\ntitle: The title # a comment\ntags:\n  - one\n  - \"two words\"\n  - #three\n---\nbody",
        );

        assert_eq!(parsed.title.as_deref(), Some("The title"));
        assert_eq!(parsed.tags, vec!["one", "two words", "three"]);
    }

    #[test]
    fn rendering_adds_frontmatter_but_preserves_existing_frontmatter() {
        let rendered = render_note_markdown(
            "A \"quoted\" title",
            &["garden".into(), "#ideas".into()],
            "A [[linked note]].",
        );
        assert!(rendered.starts_with("---\ntitle: \"A \\\"quoted\\\" title\"\n"));
        assert!(rendered.contains("  - \"garden\"\n  - \"ideas\"\n"));
        assert!(rendered.ends_with("A [[linked note]]."));

        let existing = "---\ncustom: true\n---\nBody";
        assert_eq!(render_note_markdown("Changed", &[], existing), existing);
    }

    #[test]
    fn rejects_traversal_and_reserved_export_paths() {
        assert!(checked_relative_folder("../outside").is_err());
        assert!(checked_relative_folder("Notes/.obsidian").is_err());
        assert!(checked_relative_folder("C:/Users/person").is_err());
        assert!(checked_relative_folder("Projects/Ideas").is_ok());
        assert!(validate_vault_name("../vault").is_err());
        assert!(validate_vault_name("Obsidian At Home export").is_ok());
    }

    #[test]
    fn creates_new_export_directories_and_never_reuses_one() {
        let parent = TempDirectory::new("unique-export");
        let first = create_unique_export_dir(parent.path(), "My Vault").unwrap();
        fs::write(first.join("sentinel.txt"), "keep").unwrap();
        let second = create_unique_export_dir(parent.path(), "My Vault").unwrap();

        assert_eq!(first.file_name().unwrap(), "My Vault");
        assert_eq!(second.file_name().unwrap(), "My Vault (1)");
        assert_eq!(fs::read_to_string(first.join("sentinel.txt")).unwrap(), "keep");
    }

    #[test]
    fn imports_nested_notes_and_snippet_state() {
        let vault = TempDirectory::new("import");
        fs::create_dir_all(vault.path().join("Projects/Alpha")).unwrap();
        fs::create_dir_all(vault.path().join(".trash")).unwrap();
        fs::create_dir_all(vault.path().join(".obsidian/snippets")).unwrap();
        fs::write(
            vault.path().join("Projects/Alpha/Plan.md"),
            "---\ntitle: Alpha plan\ntags: [work]\n---\nSee [[Home]].",
        )
        .unwrap();
        fs::write(vault.path().join(".trash/Deleted.md"), "deleted").unwrap();
        fs::write(
            vault.path().join(".obsidian/snippets/pretty.css"),
            ".note { color: plum; }",
        )
        .unwrap();
        fs::write(
            vault.path().join(".obsidian/appearance.json"),
            r#"{"enabledCssSnippets":["pretty"]}"#,
        )
        .unwrap();

        let result = import_obsidian_vault(vault.path().to_string_lossy().into_owned()).unwrap();

        assert_eq!(result.notes.len(), 1);
        assert_eq!(result.notes[0].folder_path, "Projects/Alpha");
        assert_eq!(result.notes[0].relative_path, "Projects/Alpha/Plan.md");
        assert!(result.notes[0].content.ends_with("See [[Home]]."));
        assert_eq!(result.snippets.len(), 1);
        assert!(result.snippets[0].enabled);
    }

    #[test]
    fn exports_obsidian_compatible_structure_and_avoids_note_collisions() {
        let parent = TempDirectory::new("export");
        let result = export_obsidian_vault(
            parent.path().to_string_lossy().into_owned(),
            "Ideas".into(),
            vec![
                ExportNote {
                    title: "First note".into(),
                    content: "Link to [[Second note]].".into(),
                    folder_path: "Projects/Alpha".into(),
                    tags: vec!["work".into()],
                },
                ExportNote {
                    title: "First note".into(),
                    content: "A distinct note.".into(),
                    folder_path: "Projects/Alpha".into(),
                    tags: vec![],
                },
            ],
            vec![ExportTemplate {
                name: "Daily".into(),
                content: "# {{date}}".into(),
            }],
            vec![VaultSnippet {
                name: "focus.css".into(),
                css: ".workspace { color: plum; }".into(),
                enabled: true,
            }],
        )
        .unwrap();

        let root = PathBuf::from(&result.path);
        assert_eq!(result.note_count, 2);
        assert_eq!(result.template_count, 1);
        assert_eq!(result.snippet_count, 1);
        assert!(root.join("Projects/Alpha/First note.md").is_file());
        assert!(root.join("Projects/Alpha/First note 1.md").is_file());
        assert_eq!(
            fs::read_to_string(root.join("Templates/Daily.md")).unwrap(),
            "# {{date}}"
        );
        assert!(root.join(".obsidian/snippets/focus.css").is_file());
        let appearance = fs::read_to_string(root.join(".obsidian/appearance.json")).unwrap();
        assert!(appearance.contains("focus"));
    }

    #[cfg(unix)]
    #[test]
    fn import_does_not_follow_symbolic_links() {
        use std::os::unix::fs::symlink;

        let vault = TempDirectory::new("symlink-vault");
        let outside = TempDirectory::new("symlink-outside");
        fs::write(outside.path().join("Private.md"), "do not import").unwrap();
        symlink(outside.path(), vault.path().join("linked")).unwrap();

        let result = import_obsidian_vault(vault.path().to_string_lossy().into_owned()).unwrap();
        assert!(result.notes.is_empty());
    }
}
