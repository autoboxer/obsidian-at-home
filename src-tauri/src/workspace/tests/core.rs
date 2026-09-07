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
