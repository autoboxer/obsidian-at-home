use super::*;
use std::io::Read;

/// Reads an Obsidian vault without modifying it.
///
/// Markdown is returned byte-for-byte as UTF-8 text, and safe asset paths are
/// inventoried for a later confirmed copy. Frontmatter is only inspected to infer a
/// display title and tags; the caller receives the original content unchanged.
#[tauri::command]
pub fn import_obsidian_vault(path: String) -> Result<ImportResult, String> {
    let root = validate_import_root(&path)?;
    let vault_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Imported vault")
        .to_owned();

    let mut images = Vec::new();
    let mut attachments = Vec::new();
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
        if is_canvas_path(entry.path()) {
            continue;
        }
        if is_supported_image_path(entry.path()) {
            if images.len().saturating_add(attachments.len()) >= MAX_IMPORTED_ASSETS {
                warnings.push(format!(
                    "Only the first {MAX_IMPORTED_ASSETS} asset files can be imported."
                ));
                continue;
            }
            if let Some(image) = inspect_portable_image(&root, &entry, &mut warnings) {
                images.push(image);
            }
            continue;
        }
        if !is_markdown_path(entry.path()) {
            if images.len().saturating_add(attachments.len()) >= MAX_IMPORTED_ASSETS {
                warnings.push(format!(
                    "Only the first {MAX_IMPORTED_ASSETS} asset files can be imported."
                ));
                continue;
            }
            if let Some(attachment) = inspect_portable_attachment(&root, &entry, &mut warnings) {
                attachments.push(attachment);
            }
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

        let content = match read_utf8_file_bounded(entry.path(), MAX_NOTE_BYTES) {
            Ok(Some(content)) => content,
            Ok(None) => {
                warnings.push(format!(
                    "Skipped {relative_path} because it is larger than {} MiB.",
                    MAX_NOTE_BYTES / 1024 / 1024
                ));
                continue;
            }
            Err(error) => {
                warnings.push(format!(
                    "Skipped {relative_path} because it is not readable UTF-8: {error}"
                ));
                continue;
            }
        };
        let content_bytes = content.len() as u64;
        if total_bytes.saturating_add(content_bytes) > MAX_TOTAL_IMPORT_BYTES {
            warnings.push(format!(
                "Stopped after reading {} MiB of notes to stay within the import limit.",
                MAX_TOTAL_IMPORT_BYTES / 1024 / 1024
            ));
            break;
        }
        total_bytes += content_bytes;

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

    images.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    attachments.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    notes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let snippets = import_snippets(&root, &mut warnings);

    Ok(ImportResult {
        vault_name,
        images,
        attachments,
        notes,
        snippets,
        warnings: warnings.finish(),
    })
}

pub(super) fn read_utf8_file_bounded(path: &Path, max_bytes: u64) -> io::Result<Option<String>> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Ok(None);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
