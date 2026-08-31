use super::*;

/// Writes a portable Obsidian vault, including supported asset files, into a
/// newly-created child of `parent_path`. Existing files and directories are
/// never reused or overwritten.
#[tauri::command(rename_all = "camelCase")]
pub fn export_obsidian_vault(
    parent_path: String,
    source_path: String,
    vault_name: String,
    notes: Vec<ExportNote>,
    templates: Vec<ExportTemplate>,
    snippets: Vec<VaultSnippet>,
) -> Result<ExportResult, String> {
    let parent = validate_export_parent(&parent_path)?;
    let source_root = validate_import_root(&source_path)?;
    validate_vault_name(&vault_name)?;
    let mut warnings = WarningCollector::default();
    let (images, attachments) = collect_portable_assets(&source_root, &mut warnings);

    let root = create_unique_export_dir(&parent, vault_name.trim()).map_err(|error| {
        format!(
            "Could not create a new export folder in {}: {error}",
            parent.display()
        )
    })?;

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
            Err(error) => warnings.push(format!("Could not export note {:?}: {error}", note.title)),
        }
    }

    if !templates.is_empty() {
        let directory = root.join("Templates");
        match fs::create_dir_all(&directory) {
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
                if let Err(error) =
                    write_json_file(&obsidian_directory.join("templates.json"), &settings)
                {
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
                    if let Err(error) =
                        write_json_file(&obsidian_directory.join("appearance.json"), &appearance)
                    {
                        warnings.push(format!("Could not write CSS snippet settings: {error}"));
                    }
                }
            }
        }
    }

    let image_count = export_portable_images(&source_root, &root, &images, &mut warnings);
    let attachment_count =
        export_portable_attachments(&source_root, &root, &attachments, &mut warnings);

    Ok(ExportResult {
        path: root.to_string_lossy().into_owned(),
        image_count,
        attachment_count,
        note_count,
        template_count,
        snippet_count,
        warnings: warnings.finish(),
    })
}
