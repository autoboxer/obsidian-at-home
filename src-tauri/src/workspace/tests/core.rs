#[test]
fn import_note_extensions_survive_save_and_reload() {
    let workspace = TestWorkspace::new("import-note-extensions");
    let mut vault = empty_vault("Imported notes");
    for (index, path) in ["Same.md", "Same.markdown", "Upper.MD", "Upper.MARKDOWN"]
        .into_iter()
        .enumerate()
    {
        let mut note = test_note("[Other extension](Same.markdown)\n");
        note.id = format!("import-{index}");
        note.title = Path::new(path)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        note.relative_path = path.to_owned();
        vault.notes.push(note);
    }
    save_workspace_files(
        &workspace.root,
        &vault,
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect("distinct Markdown extensions should save");
    let mut loaded = load_workspace(&workspace.root, &vault).unwrap();
    for note in &vault.notes {
        let saved = loaded
            .vault
            .notes
            .iter()
            .find(|saved| saved.id == note.id)
            .unwrap();
        assert_eq!(saved.relative_path, note.relative_path);
        assert_eq!(saved.content, note.content);
        assert_eq!(
            fs::read_to_string(workspace.root.join(&note.relative_path)).unwrap(),
            note.content
        );
    }

    let renamed = loaded
        .vault
        .notes
        .iter_mut()
        .find(|note| note.relative_path == "Upper.MD")
        .unwrap();
    renamed.title = "Renamed".to_owned();
    renamed.relative_path = "Untrusted.exe".to_owned();
    let renamed_id = renamed.id.clone();
    let saved = save_workspace_files(&workspace.root, &loaded.vault, loaded.revision).unwrap();
    assert_eq!(saved.note_paths[&renamed_id], "Renamed.MD");
    assert!(!workspace.root.join("Untrusted.exe").exists());
}

#[test]
fn import_note_extension_hint_does_not_choose_the_destination_path() {
    for (hint, expected) in [
        ("../Else/Unrelated.markdown", "Safe.markdown"),
        ("../Else/Unrelated.exe", "Safe.md"),
    ] {
        let workspace = TestWorkspace::new("import-note-extension-hint");
        let mut vault = empty_vault("Imported notes");
        let mut note = test_note("# Safe content\n");
        note.title = "Safe".to_owned();
        note.relative_path = hint.to_owned();
        vault.notes.push(note);
        let saved = save_workspace_files(
            &workspace.root,
            &vault,
            revision_for_root(&workspace.root).unwrap(),
        )
        .unwrap();
        assert_eq!(saved.note_paths["note-1"], expected);
        assert_eq!(
            fs::read_to_string(workspace.root.join(expected)).unwrap(),
            "# Safe content\n"
        );
        assert!(!workspace.root.join("Else").exists());
    }
}

#[test]
fn import_note_extension_collisions_fail_before_writing() {
    let workspace = TestWorkspace::new("import-note-extension-collision");
    let original = test_note("# Keep existing source\n");
    write_saved_note(&workspace, &original);
    let mut loaded = load_workspace(&workspace.root, &empty_vault("Imported notes")).unwrap();
    let original_state = fs::read(workspace_state_path(&workspace.root)).unwrap();
    for (index, extension) in ["md", "MD"].into_iter().enumerate() {
        let mut note = test_note("# New content\n");
        note.id = format!("new-{index}");
        note.title = "Duplicate".to_owned();
        note.relative_path = format!("Duplicate.{extension}");
        loaded.vault.notes.push(note);
    }
    let error = save_workspace_files(&workspace.root, &loaded.vault, loaded.revision).unwrap_err();
    assert!(error.contains("More than one note would be saved"));
    assert_eq!(
        fs::read_to_string(workspace.root.join(&original.relative_path)).unwrap(),
        original.content
    );
    assert_eq!(
        fs::read(workspace_state_path(&workspace.root)).unwrap(),
        original_state
    );
    assert!(!workspace.root.join("Duplicate.md").exists());
    assert!(!workspace.root.join("Duplicate.MD").exists());
}

#[test]
fn version_one_state_defaults_and_migrates_recently_deleted_notes() {
    let workspace = TestWorkspace::new("state-v1-migration");
    let state: WorkspaceState = serde_json::from_value(serde_json::json!({
        "version": 1,
        "name": "Legacy vault",
        "notePaths": {},
        "folderPaths": {},
        "noteMetadata": {},
        "templates": [],
        "snippets": [],
        "activeNoteId": null,
        "recentNoteIds": [],
        "selectedFolderId": "all",
        "lastCommittedTransactionId": null
    }))
    .expect("version one state should deserialize");
    assert!(state.recently_deleted_notes.is_empty());
    write_workspace_state(&workspace.root, &state).expect("legacy state should be written");

    let loaded = load_workspace(&workspace.root, &empty_vault("Test vault"))
        .expect("legacy workspace should load");
    let (migrated, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());

    assert!(loaded.recently_deleted_notes.is_empty());
    assert_eq!(
        migrated.expect("migrated state should exist").version,
        STATE_VERSION
    );
}

#[test]
fn version_three_image_assets_migrate_to_vault_assets() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nlegacy-image-asset";
    let workspace = TestWorkspace::new("state-v3-image-asset-migration");
    fs::write(workspace.root.join("Legacy.png"), PNG).expect("legacy image should be written");
    fs::create_dir(workspace.root.join(STATE_DIRECTORY))
        .expect("state directory should be created");
    let legacy_state = serde_json::json!({
        "version": 3,
        "name": "Legacy vault",
        "imageAssets": {
            "image-legacy": {
                "relativePath": "Legacy.png",
                "mediaType": "image/png",
                "fingerprint": fingerprint_bytes(PNG),
                "modifiedNanos": 0
            }
        }
    });
    fs::write(
        workspace.root.join(STATE_DIRECTORY).join(STATE_FILE),
        serde_json::to_vec_pretty(&legacy_state).expect("legacy state should encode"),
    )
    .expect("legacy state should be written");

    let loaded = load_workspace(&workspace.root, &empty_vault("Fallback"))
        .expect("legacy workspace should load");
    assert_eq!(
        loaded.vault.embedded_images,
        vec![EmbeddedImage {
            id: "image-legacy".to_owned(),
            relative_path: "Legacy.png".to_owned(),
            media_type: "image/png".to_owned(),
        }],
    );

    let migrated: serde_json::Value = serde_json::from_slice(
        &fs::read(workspace.root.join(STATE_DIRECTORY).join(STATE_FILE))
            .expect("migrated state should be readable"),
    )
    .expect("migrated state should decode");
    assert_eq!(migrated["version"], STATE_VERSION);
    assert!(migrated.get("imageAssets").is_none());
    assert_eq!(migrated["assets"]["image-legacy"]["kind"], "image");
}

#[test]
fn image_reconciliation_leaves_attachment_assets_untouched() {
    let workspace = TestWorkspace::new("attachment-survives-image-reconciliation");
    let attachment = StoredVaultAsset {
        kind: VaultAssetKind::Attachment,
        relative_path: "Files/Archive.zip".to_owned(),
        media_type: "application/zip".to_owned(),
        fingerprint: fingerprint_bytes(b"not-yet-managed"),
        modified_nanos: 0,
    };
    let mut assets = BTreeMap::from([("asset-archive".to_owned(), attachment.clone())]);

    assert!(reconcile_image_assets(
        &workspace.root,
        &mut assets,
        &mut WarningCollector::default(),
    )
    .is_empty());
    assert_eq!(assets.get("asset-archive"), Some(&attachment));
}

#[test]
fn version_two_transactions_default_recovery_targets() {
    let manifest: TransactionManifest = serde_json::from_value(serde_json::json!({
        "version": 2,
        "id": "save-legacy",
        "phase": "prepared",
        "originals": [],
        "targets": [],
        "folderCaseRenames": [],
        "createdDirectories": []
    }))
    .expect("version two transaction should deserialize");

    assert!(manifest.recovery_targets.is_empty());
}

#[test]
fn workspace_lock_serializes_separate_file_handles() {
    let workspace = TestWorkspace::new("workspace-lock");
    let revision_before =
        revision_for_root(&workspace.root).expect("initial revision should be calculated");
    let first = lock_workspace_files(&workspace.root).expect("first workspace handle should lock");
    let second =
        open_workspace_lock_file(&workspace.root).expect("second workspace handle should open");

    assert!(second.try_lock().is_err());
    drop(first);
    second
        .try_lock()
        .expect("second workspace handle should lock after release");

    let revision_after =
        revision_for_root(&workspace.root).expect("updated revision should be calculated");
    assert_eq!(revision_after, revision_before);
}

#[test]
fn content_sensitive_revisions_reject_same_metadata_note_edits() {
    let workspace = TestWorkspace::new("content-sensitive-revision");
    let note = test_note("before");
    write_saved_note(&workspace, &note);
    let note_path = workspace.root.join(&note.relative_path);
    let original_modified = fs::metadata(&note_path)
        .expect("note metadata should be readable")
        .modified()
        .expect("note should have a modified time");
    let expected_revision =
        revision_for_root(&workspace.root).expect("initial revision should be calculated");
    let baseline_stamps =
        note_file_stamps(&workspace.root).expect("initial note stamps should be calculated");

    fs::write(&note_path, "edited").expect("external note edit should be written");
    File::options()
        .write(true)
        .open(&note_path)
        .expect("external note should reopen")
        .set_times(FileTimes::new().set_modified(original_modified))
        .expect("the original modified time should be restored");
    let edited_metadata = fs::metadata(&note_path).expect("edited metadata should be readable");
    assert_eq!(edited_metadata.len(), note.content.len() as u64);
    assert_eq!(edited_metadata.modified().unwrap(), original_modified);

    let current_revision =
        revision_for_root(&workspace.root).expect("edited revision should be calculated");
    let current_stamps =
        note_file_stamps(&workspace.root).expect("edited note stamps should be calculated");
    assert_ne!(current_revision, expected_revision);
    assert_ne!(current_stamps, baseline_stamps);

    let mut stale_vault = empty_vault("Test vault");
    stale_vault.notes.push(note);
    let error = save_workspace_files(&workspace.root, &stale_vault, expected_revision)
        .expect_err("a stale save must not overwrite the external edit");
    assert!(error.contains("vault changed"));
    assert_eq!(
        fs::read_to_string(&note_path).unwrap(),
        "edited",
        "the external content must be preserved",
    );
}

#[test]
fn revision_hashes_bounded_mutable_files_but_not_assets() {
    let workspace = TestWorkspace::new("revision-content-scope");
    fs::write(workspace.root.join("Note.md"), "note").expect("note should be written");
    fs::write(workspace.root.join("Archive.zip"), "asset").expect("asset should be written");
    write_workspace_state(&workspace.root, &WorkspaceState::default())
        .expect("workspace state should be written");

    let entries =
        revision_entries_for_root(&workspace.root).expect("revision entries should be calculated");
    let content_hash = |label: &str| {
        entries
            .iter()
            .find(|entry| entry.0 == label)
            .and_then(|entry| entry.1.as_ref())
            .and_then(|stamp| stamp.content_hash)
    };

    assert!(content_hash("F:Note.md").is_some());
    assert!(content_hash(&format!("F:{STATE_DIRECTORY}/{STATE_FILE}")).is_some());
    assert_eq!(content_hash("F:Archive.zip"), None);

    let state_path = workspace_state_path(&workspace.root);
    let original_state_modified = fs::metadata(&state_path).unwrap().modified().unwrap();
    let mut changed_state = fs::read(&state_path).expect("workspace state should be readable");
    let version_digit = changed_state
        .windows(b"\"version\": 4".len())
        .position(|window| window == b"\"version\": 4")
        .map(|position| position + b"\"version\": ".len())
        .expect("workspace state should contain its version");
    changed_state[version_digit] = b'3';
    let original_revision = revision_for_entries(&entries);
    fs::write(&state_path, changed_state).expect("external state edit should be written");
    File::options()
        .write(true)
        .open(&state_path)
        .expect("workspace state should reopen")
        .set_times(FileTimes::new().set_modified(original_state_modified))
        .expect("the original state modified time should be restored");
    assert_ne!(
        revision_for_root(&workspace.root).expect("changed state revision should be calculated"),
        original_revision,
    );
}

#[test]
fn streamed_file_fingerprints_match_in_memory_fingerprints() {
    let workspace = TestWorkspace::new("streamed-fingerprint");
    let bytes = (0..64 * 1024 * 2 + 37)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let path = workspace.root.join("Large.bin");
    fs::write(&path, &bytes).expect("fingerprint fixture should be written");

    assert_eq!(
        fingerprint_regular_file(&path).expect("file should be fingerprinted"),
        Some(fingerprint_bytes(&bytes)),
    );
}

#[test]
fn recently_deleted_contract_uses_camel_case_and_fixed_retention() {
    let note = test_note("Remember me");
    let deleted_note = RecentlyDeletedNote {
        id: "deleted-contract".to_owned(),
        note,
        original_folder_path: "Projects".to_owned(),
        deleted_at: 5_000,
        expires_at: 5_000 + RECENTLY_DELETED_RETENTION_MILLIS,
        editor_position: Some(editor_position(2)),
    };
    let value = serde_json::to_value(&deleted_note).expect("deleted note should serialize");

    assert_eq!(value["id"], "deleted-contract");
    assert_eq!(value["originalFolderPath"], "Projects");
    assert_eq!(value["deletedAt"], 5_000);
    assert_eq!(
        value["expiresAt"],
        5_000 + RECENTLY_DELETED_RETENTION_MILLIS,
    );
    assert!(value.get("editorPosition").is_some());
}

fn tag_sync_note(content: &str, tags: &[&str]) -> Note {
    let mut note = test_note(content);
    note.tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
    note
}

#[test]
fn tag_sync_rejects_existing_note_mismatches_before_writes() {
    let workspace = TestWorkspace::new("tag-sync-rejected-save");
    let saved = tag_sync_note("---\ntags: [old]\n---\nBody\n", &["old"]);
    write_saved_note(&workspace, &saved);
    let state_path = workspace_state_path(&workspace.root);
    let state_before = fs::read(&state_path).unwrap();
    let revision = revision_for_root(&workspace.root).unwrap();

    for (content, tags) in [
        ("---\ntags: [new]\n---\nBody\n", vec!["old"]),
        ("---\ntags: []\n---\nBody\n", vec!["old"]),
        ("---\ncustom: keep\n---\nBody\n", vec!["old"]),
        ("Body without frontmatter\n", vec!["old"]),
        ("---\ntags: [new]\nUnfinished\n", vec!["old"]),
        ("---\ntags: [old]\n---\nBody\n", vec!["control"]),
    ] {
        let mut changed = tag_sync_note(content, &tags);
        changed.title = "Renamed note".to_owned();
        let mut new_note = test_note("A separate valid change\n");
        new_note.id = "new-note".to_owned();
        new_note.title = "New note".to_owned();
        let mut vault = empty_vault("Test vault");
        vault.notes = vec![new_note, changed];

        let error = save_workspace_files(&workspace.root, &vault, revision)
            .expect_err("inconsistent tags must reject the entire save");
        assert!(error.contains("tags do not match"), "{error}");
        assert_eq!(
            fs::read_to_string(workspace.root.join(&saved.relative_path)).unwrap(),
            saved.content
        );
        assert_eq!(fs::read(&state_path).unwrap(), state_before);
        assert_eq!(revision_for_root(&workspace.root).unwrap(), revision);
        assert!(!workspace.root.join("New note.md").exists());
        assert!(!workspace.root.join("Renamed note.md").exists());
    }
}

#[test]
fn tag_sync_missing_saved_file_does_not_allow_legacy_tag_initialization() {
    let workspace = TestWorkspace::new("tag-sync-missing-saved-file");
    let mut note = tag_sync_note("---\ntags: [old]\n---\nBody\n", &["old"]);
    write_saved_note(&workspace, &note);
    let note_path = workspace.root.join(&note.relative_path);
    fs::remove_file(&note_path).unwrap();
    note.content = "Body without frontmatter\n".to_owned();
    let revision = revision_for_root(&workspace.root).unwrap();
    let state_before = fs::read(workspace_state_path(&workspace.root)).unwrap();
    let mut vault = empty_vault("Test vault");
    vault.notes.push(note);

    let error = save_workspace_files(&workspace.root, &vault, revision)
        .expect_err("a known note must not be treated as a new legacy note");
    assert!(error.contains("tags do not match"), "{error}");
    assert!(!note_path.exists());
    assert_eq!(
        fs::read(workspace_state_path(&workspace.root)).unwrap(),
        state_before
    );
}

#[test]
fn tag_sync_saves_source_and_control_edits_verbatim() {
    let workspace = TestWorkspace::new("tag-sync-save-round-trip");
    let note = tag_sync_note("---\ntags: [old]\n---\nBody\n", &["old"]);
    write_saved_note(&workspace, &note);
    let mut loaded = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();

    for (content, tags) in [
        (
            "\u{feff}---\r\n# Keep\r\ntags: [new]\r\ncustom: keep\r\n...\r\nBody\r\n",
            vec!["new"],
        ),
        (
            "---\ntags:\n  - \"new\"\n  - \"control\"\ncustom: keep\n---\nBody\n",
            vec!["new", "control"],
        ),
        (
            "---\ntags:\n  - \"control\"\ncustom: keep\n---\nEdited body\n",
            vec!["control"],
        ),
        (
            "---\ntags: [source] # preserve this comment\n---\nBody\n",
            vec!["source"],
        ),
        ("---\ntags: []\ncustom: keep\n---\nBody\n", vec![]),
        ("Body without frontmatter\n", vec![]),
        ("---\ntags: [draft]\nUnfinished frontmatter\n", vec![]),
        ("---\ntags: [finished]\n---\nBody\n", vec!["finished"]),
    ] {
        loaded.vault.notes[0].content = content.to_owned();
        loaded.vault.notes[0].tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
        save_workspace_files(&workspace.root, &loaded.vault, loaded.revision)
            .expect("synchronized source and tags should save");
        assert_eq!(
            fs::read(workspace.root.join(&note.relative_path)).unwrap(),
            content.as_bytes()
        );
        loaded = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
        assert_eq!(loaded.vault.notes[0].content, content);
        assert_eq!(loaded.vault.notes[0].tags, tags);
    }
}

#[test]
fn tag_sync_initializes_legacy_tags_only_when_new_source_has_no_tag_field() {
    for (content, expected) in [
        ("# Legacy note\n", "---\ntags:\n  - \"legacy\"\n  - \"second\"\n---\n\n# Legacy note\n"),
        ("\u{feff}# Legacy note\r\n", "\u{feff}---\r\ntags:\r\n  - \"legacy\"\r\n  - \"second\"\r\n---\r\n\r\n# Legacy note\r\n"),
        ("---\ncustom: keep\n---\nBody\n", "---\ncustom: keep\ntags:\n  - \"legacy\"\n  - \"second\"\n---\nBody\n"),
        ("---\ncustom:\n  tags: [nested]\n---\nBody\n", "---\ncustom:\n  tags: [nested]\ntags:\n  - \"legacy\"\n  - \"second\"\n---\nBody\n"),
    ] {
        let workspace = TestWorkspace::new("tag-sync-legacy-create");
        let note = tag_sync_note(content, &["#legacy", "legacy", "second"]);
        let mut vault = empty_vault("Legacy vault");
        vault.notes.push(note);
        save_workspace_files(&workspace.root, &vault, revision_for_root(&workspace.root).unwrap())
            .expect("new legacy notes should retain metadata-only tags");
        let loaded = load_workspace(&workspace.root, &vault).unwrap();
        assert_eq!(loaded.vault.notes[0].content, expected);
        assert_eq!(loaded.vault.notes[0].tags, vec!["legacy", "second"]);
        save_workspace_files(&workspace.root, &loaded.vault, loaded.revision)
            .expect("the initialized note should save again without rewriting");
        assert_eq!(fs::read_to_string(workspace.root.join("First note.md")).unwrap(), expected);
    }

    for (content, tags) in [
        ("---\ntags: [source]\n---\nBody\n", vec!["stale"]),
        ("---\ntags: [source]\n---\nBody\n", vec![]),
        ("---\ntags: []\n---\nBody\n", vec!["stale"]),
        ("---\ntags:\n---\nBody\n", vec!["stale"]),
        ("---\ntags: []\ntags: []\n---\nBody\n", vec!["stale"]),
        ("---\nTags: [] # preserve\n---\nBody\n", vec!["stale"]),
    ] {
        let workspace = TestWorkspace::new("tag-sync-rejected-create");
        let mut vault = empty_vault("Test vault");
        vault.notes.push(tag_sync_note(content, &tags));
        let error = save_workspace_files(
            &workspace.root,
            &vault,
            revision_for_root(&workspace.root).unwrap(),
        )
        .expect_err("an explicit source field must not be replaced during creation");
        assert!(error.contains("tags do not match"), "{error}");
        assert!(!workspace.root.join("First note.md").exists());
        assert!(!workspace_state_path(&workspace.root).exists());
    }
}

#[test]
fn tag_sync_native_frontmatter_contract_preserves_source() {
    let fixtures: &[(&str, &str, &[&str])] = &[
        ("body is not metadata", "# Note\ntags: [body]\n", &[]),
        (
            "inline tags normalize and deduplicate",
            "---\ntags: [old, '#work', old, '']\n---\nBody\n",
            &["old", "work"],
        ),
        (
            "block list",
            "---\ntags:\n  - old\n  - 'isn''t'\nother: keep\n---\nBody\n",
            &["old", "isn't"],
        ),
        (
            "unindented list",
            "---\ntags:\n- one\n- two\n---\n",
            &["one", "two"],
        ),
        (
            "scalar",
            "---\ntags: old # keep this comment\n---\n",
            &["old"],
        ),
        (
            "quoted punctuation",
            "---\ntags: ['a,b', 'isn''t', \"say \\\"hi\\\"\", '#work']\n---\n",
            &["a,b", "isn't", "say \"hi\"", "work"],
        ),
        (
            "BOM and CRLF",
            "\u{feff}---\r\nTags: [old]\r\n...\r\nBody\r\n",
            &["old"],
        ),
        ("empty list", "---\ntags: []\n---\nBody\n", &[]),
        ("empty scalar", "---\ntags:\nother: keep\n---\n", &[]),
        ("unfinished frontmatter", "---\ntags: [new]\nBody\n", &[]),
        (
            "nested tags are unrelated",
            "---\nother:\n  tags: [nested]\ntags: [top]\n---\n",
            &["top"],
        ),
        (
            "comments in block list",
            "---\ntags:\n  - one\n# keep\n  - two\n---\n",
            &["one", "two"],
        ),
        (
            "delimiter at end of file",
            "---\ntags: [old]\n---",
            &["old"],
        ),
        ("empty frontmatter", "---\n---\nBody\n", &[]),
        (
            "duplicate tag fields retain observed values",
            "---\ntags: [one]\ntags: [two]\n---\n",
            &["one", "two"],
        ),
    ];
    for (label, content, tags) in fixtures {
        assert_eq!(parse_frontmatter_tags(content), *tags, "{label}");
        let note = tag_sync_note(content, tags);
        for allow_initialization in [false, true] {
            assert_eq!(
                content_with_requested_tags(&note, allow_initialization).unwrap(),
                *content,
                "{label}"
            );
        }
    }
}

#[test]
fn load_snapshot_rejects_note_changes_before_metadata_writes() {
    for change in ["edit", "add", "remove", "rename", "folder", "attachment"] {
        let workspace = TestWorkspace::new("load-snapshot-note-change");
        let note = test_note("before");
        let state = write_saved_note(&workspace, &note);
        let state_path = workspace_state_path(&workspace.root);
        let original_state = fs::read(&state_path).unwrap();
        let original_state_modified = fs::metadata(&state_path).unwrap().modified().unwrap();
        let baseline = revision_entries_for_root(&workspace.root).unwrap();
        let scanned =
            scan_workspace_files(&workspace.root, &mut WarningCollector::default()).unwrap();
        assert_eq!(scanned.notes[0].content, "before");
        let path = workspace.root.join(&note.relative_path);
        match change {
            "edit" => {
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                fs::write(&path, "edited").unwrap();
                File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_times(FileTimes::new().set_modified(modified))
                    .unwrap();
            }
            "add" => fs::write(workspace.root.join("External.md"), "external").unwrap(),
            "remove" => fs::remove_file(&path).unwrap(),
            "rename" => fs::rename(&path, workspace.root.join("Renamed.md")).unwrap(),
            "folder" => fs::create_dir(workspace.root.join("External folder")).unwrap(),
            "attachment" => fs::write(workspace.root.join("External.txt"), "attachment").unwrap(),
            _ => unreachable!(),
        }
        let external_revision = revision_for_root(&workspace.root).unwrap();
        let error = persist_loaded_workspace(
            &workspace.root,
            &state,
            true,
            &baseline,
            &mut WarningCollector::default(),
        )
        .expect_err("changed notes must not receive a revision accepting stale content");
        assert!(error.contains("vault changed"), "{change}: {error}");
        assert_eq!(
            revision_for_root(&workspace.root).unwrap(),
            external_revision
        );
        assert_eq!(fs::read(&state_path).unwrap(), original_state);
        assert_eq!(
            fs::metadata(&state_path).unwrap().modified().unwrap(),
            original_state_modified
        );
    }
}

#[test]
fn load_snapshot_preserves_external_metadata_edits() {
    let workspace = TestWorkspace::new("load-snapshot-state-change");
    let state = write_saved_note(&workspace, &test_note("before"));
    let baseline = revision_entries_for_root(&workspace.root).unwrap();
    let state_path = workspace_state_path(&workspace.root);
    let modified = fs::metadata(&state_path).unwrap().modified().unwrap();
    let external_state = fs::read_to_string(&state_path)
        .unwrap()
        .replace("Test vault", "User vault");
    fs::write(&state_path, &external_state).unwrap();
    File::options()
        .write(true)
        .open(&state_path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();

    let error = persist_loaded_workspace(
        &workspace.root,
        &state,
        true,
        &baseline,
        &mut WarningCollector::default(),
    )
    .expect_err("loading must not overwrite external metadata edits");
    assert!(error.contains("vault changed"));
    assert_eq!(fs::read_to_string(&state_path).unwrap(), external_state);
}

#[test]
fn load_snapshot_checks_the_note_bytes_actually_read() {
    let workspace = TestWorkspace::new("load-snapshot-transient-note-edit");
    let note = test_note("before");
    write_saved_note(&workspace, &note);
    let baseline = revision_entries_for_root(&workspace.root).unwrap();
    let path = workspace.root.join(&note.relative_path);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "edited").unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    let scanned = scan_workspace_files(&workspace.root, &mut WarningCollector::default()).unwrap();
    assert_eq!(scanned.notes[0].content, "edited");
    fs::write(&path, "before").unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(
        revision_entries_for_root(&workspace.root).unwrap(),
        baseline
    );

    let error = verify_workspace_load_reads(&baseline, &scanned, None, true)
        .expect_err("matching filesystem snapshots must not accept different loaded bytes");
    assert!(error.contains("vault changed"));
}

#[test]
fn load_snapshot_checks_the_metadata_bytes_actually_read() {
    let workspace = TestWorkspace::new("load-snapshot-state-read");
    write_saved_note(&workspace, &test_note("before"));
    let state_path = workspace_state_path(&workspace.root);
    let (state, present, fingerprint) =
        read_workspace_state_with_fingerprint(&workspace.root, &mut WarningCollector::default());
    assert_eq!(state.unwrap().name, "Test vault");
    let modified = fs::metadata(&state_path).unwrap().modified().unwrap();
    let external_state = fs::read_to_string(&state_path)
        .unwrap()
        .replace("Test vault", "User vault");
    fs::write(&state_path, &external_state).unwrap();
    File::options()
        .write(true)
        .open(&state_path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    let baseline = revision_entries_for_root(&workspace.root).unwrap();
    let scanned = scan_workspace_files(&workspace.root, &mut WarningCollector::default()).unwrap();
    let error = verify_workspace_load_reads(&baseline, &scanned, fingerprint.as_ref(), present)
        .expect_err("metadata read before the snapshot must still match it");
    assert!(error.contains("vault changed"));
    assert_eq!(fs::read_to_string(&state_path).unwrap(), external_state);
}

#[test]
fn load_snapshot_rejects_metadata_created_during_loading() {
    let workspace = TestWorkspace::new("load-snapshot-new-state");
    let (state, present, fingerprint) =
        read_workspace_state_with_fingerprint(&workspace.root, &mut WarningCollector::default());
    assert!(state.is_none());
    assert!(!present);
    assert!(fingerprint.is_none());
    let baseline = revision_entries_for_root(&workspace.root).unwrap();
    let external_state = WorkspaceState {
        name: "External vault".to_owned(),
        ..WorkspaceState::default()
    };
    write_workspace_state(&workspace.root, &external_state).unwrap();
    let current = revision_entries_for_root(&workspace.root).unwrap();

    assert!(
        verify_workspace_load_reads(&current, &ScannedWorkspace::default(), None, false).is_err()
    );
    assert!(persist_loaded_workspace(
        &workspace.root,
        &WorkspaceState::default(),
        true,
        &baseline,
        &mut WarningCollector::default(),
    )
    .is_err());
    assert_eq!(
        read_workspace_state(&workspace.root, &mut WarningCollector::default()).0,
        Some(external_state)
    );
}

#[test]
fn load_snapshot_rechecks_changes_after_its_own_metadata_write() {
    for change in ["note", "metadata", "new note"] {
        let workspace = TestWorkspace::new("load-snapshot-final-revision");
        let note = test_note("before");
        let mut state = write_saved_note(&workspace, &note);
        let baseline = revision_entries_for_root(&workspace.root).unwrap();
        state.name = "Loaded vault".to_owned();
        write_workspace_state(&workspace.root, &state).unwrap();
        let expected_state =
            fingerprint_regular_file(&workspace_state_path(&workspace.root)).unwrap();
        assert_eq!(
            verify_workspace_load_revision(&workspace.root, &baseline, expected_state.as_ref())
                .unwrap(),
            revision_for_root(&workspace.root).unwrap(),
        );
        match change {
            "note" => fs::write(workspace.root.join(&note.relative_path), "edited").unwrap(),
            "metadata" => {
                state.name = "External vault".to_owned();
                write_workspace_state(&workspace.root, &state).unwrap();
            }
            "new note" => fs::write(workspace.root.join("External.md"), "external").unwrap(),
            _ => unreachable!(),
        }
        let error =
            verify_workspace_load_revision(&workspace.root, &baseline, expected_state.as_ref())
                .expect_err("the loader's metadata write must not mask another change");
        assert!(error.contains("vault changed"), "{change}: {error}");
    }
}

#[test]
fn load_snapshot_initializes_metadata_and_protects_subsequent_saves() {
    let workspace = TestWorkspace::new("load-snapshot-save");
    let note_path = workspace.root.join("External.md");
    fs::write(&note_path, "original").unwrap();
    let loaded = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert_eq!(loaded.vault.notes[0].content, "original");
    assert_eq!(loaded.revision, revision_for_root(&workspace.root).unwrap());
    let mut edited = loaded.vault;
    edited.notes[0].content = "app edit".to_owned();
    save_workspace_files(&workspace.root, &edited, loaded.revision).unwrap();
    assert_eq!(fs::read_to_string(&note_path).unwrap(), "app edit");
    let reopened = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    assert_eq!(reopened.vault.notes[0].content, "app edit");
    let modified = fs::metadata(&note_path).unwrap().modified().unwrap();
    fs::write(&note_path, "external").unwrap();
    File::options()
        .write(true)
        .open(&note_path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    assert!(save_workspace_files(&workspace.root, &reopened.vault, reopened.revision).is_err());
    assert_eq!(fs::read_to_string(&note_path).unwrap(), "external");
}

#[test]
fn load_snapshot_preserves_unsupported_metadata() {
    for bytes in [
        b"not JSON".as_slice(),
        b"{\"version\":999,\"name\":\"Future vault\"}",
    ] {
        let workspace = TestWorkspace::new("load-snapshot-unsupported-state");
        fs::write(workspace.root.join("Note.md"), "keep me").unwrap();
        fs::create_dir(workspace.root.join(STATE_DIRECTORY)).unwrap();
        let path = workspace_state_path(&workspace.root);
        fs::write(&path, bytes).unwrap();
        let before = revision_for_root(&workspace.root).unwrap();
        let loaded = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
        assert_eq!(loaded.vault.notes[0].content, "keep me");
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(loaded.revision, before);
        assert!(!loaded.warnings.is_empty());
    }
}

const READ_ONLY_METADATA: [&[u8]; 2] =
    [b"{\"version\":999,\"name\":\"Future vault\"}", b"not JSON"];

fn read_only_file_snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .map(|entry| entry.unwrap())
        .filter(|entry| {
            // Commands may acquire advisory locks even when rejecting a write.
            entry.path() != root.join(STATE_DIRECTORY).join(WORKSPACE_LOCK_FILE)
                && entry.path() != root.join(STATE_DIRECTORY).join(EDITOR_POSITIONS_LOCK_FILE)
        })
        .map(|entry| {
            let bytes = entry
                .file_type()
                .is_file()
                .then(|| fs::read(entry.path()).unwrap());
            (
                entry.path().strip_prefix(root).unwrap().to_path_buf(),
                bytes,
            )
        })
        .collect()
}

#[test]
fn read_only_access_is_reported_without_rewriting_metadata() {
    for bytes in READ_ONLY_METADATA {
        let workspace = TestWorkspace::new("read-only-access");
        let note = test_note("# Still readable\n");
        write_saved_note(&workspace, &note);
        write_editor_positions(
            &workspace.root,
            &BTreeMap::from([(note.id.clone(), editor_position(7))]),
        )
        .unwrap();
        let positions_path = editor_positions_path(&workspace.root);
        let positions_before = fs::read(&positions_path).unwrap();
        let state_path = workspace_state_path(&workspace.root);
        fs::write(&state_path, bytes).unwrap();
        let recoveries = ensure_recently_deleted_directory(&workspace.root).unwrap();
        fs::write(
            recoveries.join("deleted-orphan.snapshot"),
            b"preserve orphan",
        )
        .unwrap();
        let transactions = workspace
            .root
            .join(STATE_DIRECTORY)
            .join(TRANSACTIONS_DIRECTORY);
        fs::create_dir_all(transactions.join("pending")).unwrap();
        fs::write(
            transactions.join("pending/manifest.json"),
            b"preserve transaction",
        )
        .unwrap();
        let files_before = read_only_file_snapshot(&workspace.root);
        let before = revision_for_root(&workspace.root).unwrap();
        let loaded = load_workspace(&workspace.root, &empty_vault("Defaults")).unwrap();
        let json = serde_json::to_value(&loaded).unwrap();
        assert_eq!(json["access"]["mode"], "read-only");
        assert!(!json["access"]["reason"].as_str().unwrap().is_empty());
        assert_eq!(loaded.vault.notes[0].content, note.content);
        assert_eq!(loaded.revision, before);
        assert_eq!(fs::read(state_path).unwrap(), bytes);
        assert_eq!(fs::read(positions_path).unwrap(), positions_before);
        assert!(!loaded.editor_positions_writable);
        assert_eq!(read_only_file_snapshot(&workspace.root), files_before);
    }
}

#[test]
fn read_only_access_returns_to_read_write_when_supported_metadata_opens() {
    let workspace = TestWorkspace::new("read-only-reopen");
    let loaded = load_workspace(&workspace.root, &empty_vault("New vault")).unwrap();
    assert_eq!(loaded.access, WorkspaceAccess::ReadWrite);
    assert_eq!(
        serde_json::to_value(&loaded).unwrap()["access"],
        serde_json::json!({"mode": "read-write"})
    );
    let note = test_note("before");
    let mut state = write_saved_note(&workspace, &note);
    for version in [1, STATE_VERSION] {
        fs::write(workspace_state_path(&workspace.root), READ_ONLY_METADATA[0]).unwrap();
        let blocked = load_workspace(&workspace.root, &empty_vault("Defaults")).unwrap();
        assert!(matches!(blocked.access, WorkspaceAccess::ReadOnly { .. }));
        state.version = version;
        write_workspace_state(&workspace.root, &state).unwrap();
        let mut reopened = load_workspace(&workspace.root, &empty_vault("Defaults")).unwrap();
        assert_eq!(reopened.access, WorkspaceAccess::ReadWrite);
        reopened.vault.notes[0].content = format!("after version {version}");
        save_workspace_files(&workspace.root, &reopened.vault, reopened.revision).unwrap();
        assert_eq!(
            fs::read_to_string(workspace.root.join(&note.relative_path)).unwrap(),
            reopened.vault.notes[0].content
        );
    }
}

#[test]
fn read_only_access_keeps_editor_position_compatibility_separate() {
    let workspace = TestWorkspace::new("read-only-positions-only");
    write_saved_note(&workspace, &test_note("before"));
    let path = editor_positions_path(&workspace.root);
    let bytes = format!(
        "{{\"version\":{},\"positions\":{{}}}}",
        EDITOR_POSITIONS_VERSION + 1
    );
    fs::write(&path, &bytes).unwrap();
    let mut loaded = load_workspace(&workspace.root, &empty_vault("Defaults")).unwrap();
    assert_eq!(loaded.access, WorkspaceAccess::ReadWrite);
    assert!(!loaded.editor_positions_writable);
    loaded.vault.notes[0].content = "can still edit notes".to_owned();
    save_workspace_files(&workspace.root, &loaded.vault, loaded.revision).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), bytes);
}

#[test]
fn read_only_access_rejects_vault_mutations_after_open() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nimage";
    for bytes in READ_ONLY_METADATA {
        let workspace = TestWorkspace::new("read-only-mutations");
        let note = test_note("keep this note");
        write_saved_note(&workspace, &note);
        fs::write(workspace.root.join("Photo.png"), PNG).unwrap();
        let attachment = workspace.root.join("Report.pdf");
        fs::write(&attachment, b"keep this attachment").unwrap();
        let mut loaded = load_workspace(&workspace.root, &empty_vault("Defaults")).unwrap();
        assert_eq!(loaded.access, WorkspaceAccess::ReadWrite);
        // The previously loaded access mode must not authorize later native writes.
        fs::write(workspace_state_path(&workspace.root), bytes).unwrap();
        let revision = revision_for_root(&workspace.root).unwrap();
        let files_before = read_only_file_snapshot(&workspace.root);
        let assert_blocked = |result: Result<(), String>| {
            let error = result.expect_err("unreadable metadata must block mutation");
            assert!(
                error.contains("metadata") || error.contains("state.json"),
                "{error}"
            );
            assert_eq!(read_only_file_snapshot(&workspace.root), files_before);
            assert_eq!(revision_for_root(&workspace.root).unwrap(), revision);
        };
        loaded.vault.notes[0].content = "unsavable edit".to_owned();
        assert_blocked(save_workspace_files(&workspace.root, &loaded.vault, revision).map(|_| ()));
        assert_blocked(
            save_workspace_files_with_archive(
                &workspace.root,
                &loaded.vault,
                revision,
                Some(PendingNoteArchive {
                    note: note.clone(),
                    original_folder_path: String::new(),
                    editor_position: None,
                }),
            )
            .map(|_| ()),
        );
        assert_blocked(
            read_recovery_for_restore(&workspace.root, "deleted-note", revision).map(|_| ()),
        );
        assert_blocked(
            remove_recently_deleted_notes(
                &workspace.root,
                vec!["deleted-note".to_owned()],
                revision,
                false,
            )
            .map(|_| ()),
        );
        assert_blocked(
            remove_recently_deleted_notes(&workspace.root, Vec::new(), revision, true).map(|_| ()),
        );
        assert_blocked(
            save_editor_positions(
                &workspace.root,
                BTreeMap::from([(note.id.clone(), editor_position(2))]),
                None,
            )
            .map(|_| ()),
        );
        assert_blocked(
            embed_workspace_image(
                &workspace.root,
                &note.relative_path,
                ImageEmbedSettings::default(),
                "New.png",
                PNG,
                None,
                revision,
            )
            .map(|_| ()),
        );
        assert_blocked(
            embed_workspace_attachment(
                &workspace.root,
                &note.relative_path,
                AttachmentEmbedSettings::default(),
                &attachment,
                None,
                revision,
            )
            .map(|_| ()),
        );
        assert_blocked(
            relocate_workspace_image(
                &workspace.root,
                "Photo.png",
                "Moved.png",
                "image-id",
                &[],
                revision,
            )
            .map(|_| ()),
        );
        assert_blocked(
            relocate_workspace_attachment(
                &workspace.root,
                "Report.pdf",
                "Moved.pdf",
                "attachment-id",
                &[],
                revision,
            )
            .map(|_| ()),
        );
        assert_blocked(
            discard_workspace_external_asset(
                &workspace.root,
                "attachment-id",
                "Report.pdf",
                revision,
            )
            .map(|_| ()),
        );
    }
}

#[test]
fn read_only_access_preserves_attachment_copy_and_path_navigation() {
    for bytes in READ_ONLY_METADATA {
        let workspace = TestWorkspace::new("read-only-attachment");
        let destination = TestWorkspace::new("read-only-copy-destination");
        let note = test_note("# Read and copy\n");
        write_saved_note(&workspace, &note);
        fs::write(workspace.root.join("Report.pdf"), b"copy this report").unwrap();
        fs::write(workspace_state_path(&workspace.root), bytes).unwrap();
        let before = read_only_file_snapshot(&workspace.root);
        let (_, source) =
            resolve_attachment_action_source(&workspace.root, "Report.pdf", None).unwrap();
        copy_attachment_file_for_transfer_impl(&source, &destination.root.join("Copy.pdf"))
            .unwrap();
        assert_eq!(
            fs::read(destination.root.join("Copy.pdf")).unwrap(),
            b"copy this report"
        );
        for (kind, path) in [
            (WorkspaceVaultItemKind::Note, note.relative_path.as_str()),
            (WorkspaceVaultItemKind::Attachment, "Report.pdf"),
        ] {
            let (relative, resolved) =
                locate_workspace_vault_item(&workspace.root, kind, path, None).unwrap();
            assert_eq!(relative, path);
            assert_eq!(resolved, workspace.root.join(path).canonicalize().unwrap());
        }
        // A supplied stable ID still requires readable metadata; do not silently
        // open a potentially different file using its old path.
        assert!(resolve_attachment_action_source(
            &workspace.root,
            "Report.pdf",
            Some("attachment-id")
        )
        .is_err());
        assert_eq!(read_only_file_snapshot(&workspace.root), before);
    }
}

#[test]
fn read_only_access_rejects_asset_import_before_staging() {
    let source = TestWorkspace::new("read-only-import-source");
    fs::write(source.root.join("Photo.png"), b"\x89PNG\r\n\x1a\nimage").unwrap();
    fs::write(source.root.join("Report.pdf"), b"report").unwrap();
    for bytes in READ_ONLY_METADATA {
        let workspace = TestWorkspace::new("read-only-import");
        write_saved_note(&workspace, &test_note("keep"));
        fs::write(workspace_state_path(&workspace.root), bytes).unwrap();
        let before = revision_for_root(&workspace.root).unwrap();
        let result = begin_workspace_asset_import(
            &workspace.root,
            &source.root,
            &["Photo.png".to_owned()],
            &["Report.pdf".to_owned()],
            before,
        );
        assert!(
            result.is_err(),
            "read-only vaults must reject asset imports"
        );
        assert_eq!(revision_for_root(&workspace.root).unwrap(), before);
        assert!(!workspace.root.join("Photo.png").exists());
        assert!(!workspace.root.join("Report.pdf").exists());
        assert!(!workspace
            .root
            .join(STATE_DIRECTORY)
            .join(TRANSACTIONS_DIRECTORY)
            .exists());
    }
}

#[test]
fn read_only_access_rejects_external_upload_before_staging() {
    for bytes in READ_ONLY_METADATA {
        let workspace = TestWorkspace::new("read-only-upload");
        let staging = TestWorkspace::new("read-only-upload-cache");
        write_saved_note(&workspace, &test_note("keep"));
        fs::write(workspace_state_path(&workspace.root), bytes).unwrap();
        let result = begin_external_file_upload(
            &staging.root,
            "Report.pdf".to_owned(),
            4,
            ExternalFileUploadKind::Attachment,
            workspace.root.clone(),
            "First note.md".to_owned(),
        );
        if let Ok(upload) = &result {
            cancel_external_file_upload(&upload.id).unwrap();
        }
        assert!(result.is_err(), "read-only vaults must reject new uploads");
        assert_eq!(fs::read_dir(&staging.root).unwrap().count(), 0);
    }
}

#[test]
fn read_only_access_preserves_path_based_image_reading() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nreadable-image";
    for bytes in READ_ONLY_METADATA {
        let workspace = TestWorkspace::new("read-only-image");
        write_saved_note(&workspace, &test_note("![Photo](Photo.png)"));
        fs::write(workspace.root.join("Photo.png"), PNG).unwrap();
        fs::write(workspace_state_path(&workspace.root), bytes).unwrap();
        let before = revision_for_root(&workspace.root).unwrap();
        assert_eq!(
            read_workspace_image(&workspace.root, None, "First note.md", "Photo.png").unwrap(),
            PNG
        );
        assert!(
            read_workspace_image(&workspace.root, None, "First note.md", "../Photo.png").is_err()
        );
        assert_eq!(revision_for_root(&workspace.root).unwrap(), before);
    }
}

#[test]
fn unchanged_load_preserves_metadata_bytes_mtime_and_revision() {
    for content in [None, Some("before")] {
        let workspace = TestWorkspace::new("unchanged-load");
        if let Some(content) = content {
            write_saved_note(&workspace, &test_note(content));
        }
        load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
        let path = workspace_state_path(&workspace.root);
        // A fixed old timestamp makes an accidental rewrite observable even
        // on filesystems whose modification times have coarse precision.
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1_600_000_000)),
            )
            .unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let revision = revision_for_root(&workspace.root).unwrap();
        for _ in 0..3 {
            let loaded = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
            assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(
                fs::metadata(&path).unwrap().modified().unwrap(),
                metadata.modified().unwrap()
            );
            assert_eq!(loaded.revision, revision);
        }
    }
}

#[test]
fn unchanged_load_in_another_window_does_not_conflict_with_saving() {
    let workspace = TestWorkspace::new("unchanged-load-two-windows");
    let note = test_note("before");
    write_saved_note(&workspace, &note);
    let mut first = {
        let _lock = lock_workspace_files(&workspace.root).unwrap();
        load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap()
    };
    first.vault.notes[0].content = "edited in first window".to_owned();
    std::thread::sleep(Duration::from_millis(20));
    let second = {
        let _lock = lock_workspace_files(&workspace.root).unwrap();
        load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap()
    };
    assert_eq!(second.vault.notes[0].content, "before");
    {
        let _lock = lock_workspace_files(&workspace.root).unwrap();
        save_workspace_files(&workspace.root, &first.vault, first.revision)
            .expect("opening another window must not invalidate unsaved edits");
    }
    assert_eq!(second.revision, first.revision);
    let error = save_workspace_files(&workspace.root, &second.vault, second.revision)
        .expect_err("a real edit must still invalidate the other window's stale save");
    assert!(error.contains("vault changed"));
    assert_eq!(
        fs::read_to_string(workspace.root.join(&note.relative_path)).unwrap(),
        "edited in first window"
    );
}

#[test]
fn unchanged_load_migrates_legacy_state_bytes_once() {
    let workspace = TestWorkspace::new("unchanged-load-legacy-state");
    load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    let (state, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    let mut state = state.unwrap();
    state.image_embed_settings = ImageEmbedSettings {
        location: ImageEmbedLocation::SpecifiedFolder,
        folder_path: "Images".to_owned(),
    };
    write_legacy_mirrored_workspace_state(&workspace.root, &state);
    let path = workspace_state_path(&workspace.root);
    let legacy_bytes = fs::read(&path).unwrap();
    // The parser already normalizes this old enum value. Comparing only the
    // decoded structs would miss the migration that must be persisted.
    assert_eq!(
        read_workspace_state(&workspace.root, &mut WarningCollector::default()).0,
        Some(state.clone())
    );
    let before = revision_for_root(&workspace.root).unwrap();
    let migrated = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    let migrated_bytes = fs::read(&path).unwrap();
    assert_ne!(migrated_bytes, legacy_bytes);
    assert!(!String::from_utf8(migrated_bytes.clone())
        .unwrap()
        .contains("specified-folder-mirrored"));
    assert_ne!(migrated.revision, before);
    assert_eq!(
        migrated.vault.image_embed_settings,
        state.image_embed_settings
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let reopened = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    assert_eq!(reopened.revision, migrated.revision);
    assert_eq!(fs::read(&path).unwrap(), migrated_bytes);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn changed_load_updates_note_and_folder_mappings_once() {
    let workspace = TestWorkspace::new("changed-load-inventory");
    let initial = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    fs::create_dir(workspace.root.join("Projects")).unwrap();
    fs::write(workspace.root.join("Projects/External.md"), "external").unwrap();
    let discovered = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    assert_ne!(discovered.revision, initial.revision);
    let (state, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    let state = state.unwrap();
    assert_eq!(
        state.note_paths.get(&discovered.vault.notes[0].id).unwrap(),
        "Projects/External.md"
    );
    assert_eq!(
        state
            .folder_paths
            .get(&discovered.vault.folders[0].id)
            .unwrap(),
        "Projects"
    );
    assert_eq!(
        load_workspace(&workspace.root, &empty_vault("Test vault"))
            .unwrap()
            .revision,
        discovered.revision
    );

    fs::remove_file(workspace.root.join("Projects/External.md")).unwrap();
    fs::rename(
        workspace.root.join("Projects"),
        workspace.root.join("Renamed"),
    )
    .unwrap();
    let reconciled = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    assert_ne!(reconciled.revision, discovered.revision);
    assert!(reconciled.vault.notes.is_empty());
    let (state, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    let state = state.unwrap();
    assert!(state.note_paths.is_empty());
    assert!(state.note_metadata.is_empty());
    assert_eq!(
        state.folder_paths.values().collect::<Vec<_>>(),
        vec!["Renamed"]
    );
    assert_eq!(
        load_workspace(&workspace.root, &empty_vault("Test vault"))
            .unwrap()
            .revision,
        reconciled.revision
    );
}

#[cfg(unix)]
#[test]
fn unchanged_load_needs_no_metadata_write_permission() {
    use std::os::unix::fs::PermissionsExt;

    let workspace = TestWorkspace::new("unchanged-load-readonly-metadata");
    let initial = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    let directory = workspace.root.join(STATE_DIRECTORY);
    let permissions = fs::metadata(&directory).unwrap().permissions();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
    let result = load_workspace(&workspace.root, &empty_vault("Test vault"));
    fs::set_permissions(&directory, permissions).unwrap();
    let loaded = result.unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert_eq!(loaded.revision, initial.revision);
}

#[cfg(unix)]
#[test]
fn changed_load_warns_and_retries_failed_metadata_writes() {
    use std::os::unix::fs::PermissionsExt;

    let workspace = TestWorkspace::new("changed-load-metadata-write-failure");
    load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    ensure_recently_deleted_directory(&workspace.root).unwrap();
    let orphan_path = recently_deleted_snapshot_path(&workspace.root, "deleted-orphan").unwrap();
    fs::write(&orphan_path, "keep until metadata is current").unwrap();
    fs::write(workspace.root.join("External.md"), "external").unwrap();
    let state_path = workspace_state_path(&workspace.root);
    let bytes = fs::read(&state_path).unwrap();
    let before = revision_for_root(&workspace.root).unwrap();
    let directory = workspace.root.join(STATE_DIRECTORY);
    let permissions = fs::metadata(&directory).unwrap().permissions();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
    let result = load_workspace(&workspace.root, &empty_vault("Test vault"));
    fs::set_permissions(&directory, permissions).unwrap();

    let loaded = result.unwrap();
    assert_eq!(loaded.vault.notes[0].content, "external");
    assert!(loaded
        .warnings
        .iter()
        .any(|warning| warning.contains("Could not save workspace metadata")));
    assert_eq!(fs::read(&state_path).unwrap(), bytes);
    assert_eq!(loaded.revision, before);
    assert!(
        orphan_path.is_file(),
        "cleanup must wait for the metadata write to succeed"
    );
    let retried = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    assert!(retried.warnings.is_empty(), "{:?}", retried.warnings);
    assert_ne!(retried.revision, before);
    assert!(!orphan_path.exists());
    let (state, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    assert_eq!(state.unwrap().note_paths.len(), 1);
    assert_eq!(
        load_workspace(&workspace.root, &empty_vault("Test vault"))
            .unwrap()
            .revision,
        retried.revision
    );
}

#[test]
fn load_snapshot_rejects_inventory_temporarily_missing_from_the_scan() {
    for relative in ["Note.md", "Folder", "Image.png", "Attachment.pdf"] {
        let workspace = TestWorkspace::new("load-snapshot-missing-inventory");
        let path = workspace.root.join(relative);
        if relative == "Folder" {
            fs::create_dir(&path).unwrap();
        } else {
            fs::write(&path, "original").unwrap();
        }
        let baseline = revision_entries_for_root(&workspace.root).unwrap();
        let outside = workspace.root.join(STATE_DIRECTORY);
        fs::create_dir(&outside).unwrap();
        let hidden = outside.join("temporarily-missing");
        fs::rename(&path, &hidden).unwrap();
        let scanned =
            scan_workspace_files(&workspace.root, &mut WarningCollector::default()).unwrap();
        fs::rename(&hidden, &path).unwrap();
        assert_eq!(
            revision_entries_for_root(&workspace.root).unwrap(),
            baseline
        );
        let error = verify_workspace_load_reads(&baseline, &scanned, None, false)
            .expect_err("restored inventory must not be omitted from a successful load");
        assert!(error.contains("vault changed"), "{relative}: {error}");
    }
}

#[test]
fn load_snapshot_rejects_inventory_present_only_during_the_scan() {
    for relative in ["Note.md", "Folder", "Image.png", "Attachment.pdf"] {
        let workspace = TestWorkspace::new("load-snapshot-transient-inventory");
        let baseline = revision_entries_for_root(&workspace.root).unwrap();
        let path = workspace.root.join(relative);
        if relative == "Folder" {
            fs::create_dir(&path).unwrap();
        } else {
            fs::write(&path, "temporary").unwrap();
        }
        let scanned =
            scan_workspace_files(&workspace.root, &mut WarningCollector::default()).unwrap();
        if relative == "Folder" {
            fs::remove_dir(&path).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
        assert_eq!(
            revision_entries_for_root(&workspace.root).unwrap(),
            baseline
        );
        let error = verify_workspace_load_reads(&baseline, &scanned, None, false)
            .expect_err("transient inventory must not leak into a successful load");
        assert!(error.contains("vault changed"), "{relative}: {error}");
    }
}

#[test]
fn load_snapshot_rejects_a_note_skipped_during_a_transient_edit() {
    let workspace = TestWorkspace::new("load-snapshot-transient-invalid-note");
    let path = workspace.root.join("Note.md");
    fs::write(&path, "before").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let baseline = revision_entries_for_root(&workspace.root).unwrap();
    fs::write(&path, [0xff; 6]).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    let scanned = scan_workspace_files(&workspace.root, &mut WarningCollector::default()).unwrap();
    assert!(scanned.notes.is_empty());
    fs::write(&path, "before").unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(
        revision_entries_for_root(&workspace.root).unwrap(),
        baseline
    );
    assert!(verify_workspace_load_reads(&baseline, &scanned, None, false).is_err());
}

#[test]
fn load_snapshot_keeps_the_complete_inventory_after_loading_limits() {
    let workspace = TestWorkspace::new("load-snapshot-limited-inventory");
    fs::create_dir(workspace.root.join("Folder")).unwrap();
    for relative in [
        "One.md",
        "Two.md",
        "Folder/Three.md",
        "Image.png",
        "Attachment.pdf",
    ] {
        fs::write(workspace.root.join(relative), "ok").unwrap();
    }
    let baseline = revision_entries_for_root(&workspace.root).unwrap();
    for (max_notes, max_bytes) in [(1, 100), (100, 3)] {
        let scanned = scan_workspace_files_with_limits(
            &workspace.root,
            &mut WarningCollector::default(),
            max_notes,
            max_bytes,
            1,
        )
        .unwrap();
        assert_eq!(scanned.notes.len(), 1);
        assert_eq!(scanned.folders.len(), 1);
        assert_eq!(scanned.images.len() + scanned.attachments.len(), 1);
        verify_workspace_load_reads(&baseline, &scanned, None, false).unwrap();
        assert_eq!(scanned.revision_entries, baseline);
    }
}

#[test]
fn load_snapshot_tracks_stable_skipped_files_and_ignores_private_directories() {
    let workspace = TestWorkspace::new("load-snapshot-skipped-inventory");
    fs::write(workspace.root.join("Valid.md"), "keep me").unwrap();
    fs::write(workspace.root.join("Invalid.md"), [0xff]).unwrap();
    File::create(workspace.root.join("Oversized.md"))
        .unwrap()
        .set_len(MAX_NOTE_BYTES + 1)
        .unwrap();
    File::create(workspace.root.join("Oversized.bin"))
        .unwrap()
        .set_len(MAX_ATTACHMENT_BYTES + 1)
        .unwrap();
    for directory in [
        STATE_DIRECTORY,
        ".obsidian",
        ".trash",
        ".git",
        "Nested/.obsidian-at-home",
    ] {
        let path = workspace.root.join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("state.json"), "{}").unwrap();
        fs::write(path.join("Private.md"), "hidden").unwrap();
    }
    fs::write(workspace.root.join("Nested/Note.md"), "another vault").unwrap();
    let baseline = revision_entries_for_root(&workspace.root).unwrap();
    let mut warnings = WarningCollector::default();
    let scanned = scan_workspace_files(&workspace.root, &mut warnings).unwrap();
    assert_eq!(scanned.notes.len(), 1);
    assert_eq!(scanned.notes[0].relative_path, "Valid.md");
    assert!(scanned.folders.is_empty());
    assert!(scanned.attachments.is_empty());
    assert_eq!(scanned.revision_entries.len(), 4);
    verify_workspace_load_reads(&baseline, &scanned, None, true).unwrap();
    assert_eq!(warnings.finish().len(), 2);
}

#[test]
fn load_snapshot_rechecks_metadata_at_the_replacement_boundary() {
    for initially_present in [false, true] {
        let workspace = TestWorkspace::new("load-snapshot-metadata-replacement");
        let mut state = WorkspaceState::default();
        if initially_present {
            state = write_saved_note(&workspace, &test_note("before"));
        }
        let baseline = revision_entries_for_root(&workspace.root).unwrap();
        verify_workspace_load_revision(&workspace.root, &baseline, None).unwrap();
        let expected = workspace_state_revision_fingerprint(&baseline);
        let path = workspace_state_path(&workspace.root);
        let modified = fs::metadata(&path)
            .ok()
            .and_then(|value| value.modified().ok());
        state.name = "User vault".to_owned();
        write_workspace_state(&workspace.root, &state).unwrap();
        if let Some(modified) = modified {
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(FileTimes::new().set_modified(modified))
                .unwrap();
        }
        let external_bytes = fs::read(&path).unwrap();
        let external_revision = revision_for_root(&workspace.root).unwrap();
        state.name = "Stale loaded vault".to_owned();
        let error = write_loaded_workspace_state_bytes(
            &workspace.root,
            &workspace_state_bytes(&state).unwrap(),
            expected.as_ref(),
        )
        .expect_err("metadata changed since validation must survive replacement preparation");
        assert!(error.contains("vault changed"));
        assert_eq!(fs::read(&path).unwrap(), external_bytes);
        assert_eq!(
            revision_for_root(&workspace.root).unwrap(),
            external_revision
        );
        assert_eq!(
            fs::read_dir(workspace.root.join(STATE_DIRECTORY))
                .unwrap()
                .count(),
            1,
            "a failed precondition must clean up its temporary file"
        );
    }
}

#[test]
fn metadata_replacement_precondition_runs_after_staging() {
    let workspace = TestWorkspace::new("metadata-replacement-precondition");
    let path = workspace.root.join("state.json");
    fs::write(&path, "before").unwrap();
    let result = atomic_write_with_precondition(&path, b"replacement", || {
        assert_eq!(
            fs::read_dir(&workspace.root)?.count(),
            2,
            "the temporary replacement should be prepared before checking"
        );
        fs::write(&path, "external")?;
        Err(io::Error::other("concurrent edit"))
    });
    assert!(result.is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "external");
    assert_eq!(fs::read_dir(&workspace.root).unwrap().count(), 1);
}

#[test]
fn metadata_replacement_creates_and_replaces_complete_files() {
    for name in [STATE_FILE, REGISTRY_FILE, EDITOR_POSITIONS_FILE] {
        let workspace = TestWorkspace::new("metadata-replacement");
        let directory = workspace.root.join("Vault with spaces and é");
        let path = directory.join(name);
        for bytes in [b"original metadata".as_slice(), b"short", b""] {
            atomic_write(&path, bytes).expect("metadata should be created or replaced");
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(
                fs::read_dir(&directory).unwrap().count(),
                1,
                "successful replacement must not leave temporary or backup files"
            );
        }
    }
}

#[test]
fn metadata_replacement_failure_keeps_the_existing_file() {
    for name in [STATE_FILE, REGISTRY_FILE, EDITOR_POSITIONS_FILE] {
        let workspace = TestWorkspace::new("metadata-replacement-failure");
        let path = workspace.root.join(name);
        atomic_write(&path, b"original").unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        let result = atomic_write_with_precondition(&path, b"replacement", || {
            // A vanished staging file forces the installation itself to fail,
            // after the precondition passed. The original must stay in place.
            let staged = fs::read_dir(&workspace.root)?
                .map(|entry| entry.unwrap().path())
                .find(|entry| entry != &path)
                .expect("replacement should be staged");
            fs::remove_file(staged)
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
        assert_eq!(fs::read_dir(&workspace.root).unwrap().count(), 1);
        atomic_write(&path, b"retry").expect("a failed installation must allow retry");
        assert_eq!(fs::read(&path).unwrap(), b"retry");
    }
}

#[test]
fn metadata_replacement_preserves_an_occupied_directory() {
    let workspace = TestWorkspace::new("metadata-replacement-directory");
    let path = workspace.root.join(STATE_FILE);
    fs::create_dir(&path).unwrap();
    fs::write(path.join("keep"), b"original").unwrap();
    atomic_write(&path, b"replacement").expect_err("a directory must not be replaced");
    assert_eq!(fs::read(path.join("keep")).unwrap(), b"original");
    assert_eq!(fs::read_dir(&workspace.root).unwrap().count(), 1);
}

#[test]
fn metadata_replacement_reloads_workspace_registry_and_editor_positions() {
    let workspace = TestWorkspace::new("metadata-replacement-reload");
    let note = test_note("# Saved note\n");
    let mut state = write_saved_note(&workspace, &note);
    for anchor in [4, 14] {
        state.name = format!("Vault {anchor}");
        write_workspace_state(&workspace.root, &state).unwrap();
        write_editor_positions(
            &workspace.root,
            &BTreeMap::from([(note.id.clone(), editor_position(anchor))]),
        )
        .unwrap();
        let registry = WorkspaceRegistry {
            active_path: Some(workspace.root.to_string_lossy().into_owned()),
            recent_vaults: vec![VaultDescriptor {
                path: workspace.root.to_string_lossy().into_owned(),
                name: state.name.clone(),
                last_opened_at: anchor,
            }],
            ..WorkspaceRegistry::default()
        };
        // Registry I/O uses an AppHandle only to locate this configuration file.
        // Exercise its shared writer and decoder without launching a desktop app.
        let registry_path = workspace.root.join("config").join(REGISTRY_FILE);
        atomic_write(
            &registry_path,
            &serde_json::to_vec_pretty(&registry).unwrap(),
        )
        .unwrap();
        let reopened: WorkspaceRegistry =
            serde_json::from_slice(&fs::read(registry_path).unwrap()).unwrap();
        assert_eq!(reopened, registry);

        let loaded = load_workspace(&workspace.root, &empty_vault("Defaults")).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.vault.name, state.name);
        assert_eq!(loaded.vault.notes.len(), 1);
        let reopened_note = &loaded.vault.notes[0];
        assert_eq!(reopened_note.id, note.id);
        assert_eq!(reopened_note.content, note.content);
        assert_eq!(reopened_note.pinned, note.pinned);
        assert_eq!(reopened_note.created_at, note.created_at);
        assert_eq!(loaded.editor_positions[&note.id], editor_position(anchor));
    }
}

#[cfg(windows)]
#[test]
fn metadata_replacement_sharing_failure_preserves_the_existing_file() {
    use std::os::windows::fs::OpenOptionsExt;

    for name in [STATE_FILE, REGISTRY_FILE, EDITOR_POSITIONS_FILE] {
        let workspace = TestWorkspace::new("metadata-replacement-sharing");
        let path = workspace.root.join(name);
        atomic_write(&path, b"original").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(1) // FILE_SHARE_READ: deny deletion/replacement while held.
            .open(&path)
            .unwrap();
        atomic_write(&path, b"replacement").expect_err("an unshared file must stay intact");
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::read_dir(&workspace.root).unwrap().count(), 1);
        drop(held);
        atomic_write(&path, b"retry").expect("replacement should succeed once released");
        assert_eq!(fs::read(&path).unwrap(), b"retry");
    }
}
