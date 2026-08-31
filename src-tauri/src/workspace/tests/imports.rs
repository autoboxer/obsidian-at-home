#[test]
fn imports_images_without_overwriting_vault_collisions() {
    const IMPORTED: &[u8] = b"\x89PNG\r\n\x1a\nimported-image";
    const EXISTING: &[u8] = b"\x89PNG\r\n\x1a\nexisting-image";
    let source = TestWorkspace::new("portable-image-source");
    let workspace = TestWorkspace::new("portable-image-target");
    fs::create_dir(source.root.join("Assets")).expect("source folder should be created");
    fs::write(source.root.join("Assets/Diagram.png"), IMPORTED)
        .expect("source image should be written");
    fs::write(source.root.join("Collision.png"), IMPORTED)
        .expect("colliding source should be written");
    fs::write(workspace.root.join("Collision.png"), EXISTING)
        .expect("existing target should be written");

    let revision = revision_for_root(&workspace.root).expect("revision should be available");
    let result = import_workspace_images(
        &workspace.root,
        &source.root,
        &["Assets/Diagram.png".into(), "Collision.png".into()],
        revision,
    )
    .expect("valid images should be imported");

    assert_eq!(result.image_count, 2);
    assert_eq!(
        result.image_files,
        vec![
            VaultImageFile {
                asset_id: None,
                relative_path: "Assets/Diagram.png".to_owned(),
                media_type: "image/png".to_owned(),
            },
            VaultImageFile {
                asset_id: None,
                relative_path: "Collision 2.png".to_owned(),
                media_type: "image/png".to_owned(),
            },
        ],
    );
    assert_eq!(
        result.path_mappings,
        BTreeMap::from([
            (
                "Assets/Diagram.png".to_owned(),
                "Assets/Diagram.png".to_owned(),
            ),
            ("Collision.png".to_owned(), "Collision 2.png".to_owned()),
        ]),
    );
    assert_eq!(result.revision, revision_for_root(&workspace.root).unwrap());
    assert_ne!(result.revision, revision);
    assert_eq!(
        fs::read(workspace.root.join("Assets/Diagram.png")).unwrap(),
        IMPORTED,
    );
    assert_eq!(
        fs::read(workspace.root.join("Collision.png")).unwrap(),
        EXISTING,
    );
    assert_eq!(
        fs::read(workspace.root.join("Collision 2.png")).unwrap(),
        IMPORTED,
    );
    assert!(result
        .warnings
        .iter()
        .any(|warning| warning.contains("existing vault path")));

    let repeated = import_workspace_images(
        &workspace.root,
        &source.root,
        &["Assets/Diagram.png".into()],
        result.revision,
    )
    .expect("an identical existing image should be reusable");
    assert_eq!(repeated.image_count, 1);
    assert_eq!(
        repeated.path_mappings,
        BTreeMap::from([(
            "Assets/Diagram.png".to_owned(),
            "Assets/Diagram.png".to_owned(),
        )]),
    );
    assert!(repeated.warnings.is_empty());
    assert_eq!(repeated.revision, result.revision);
}

#[test]
fn imports_reuse_portable_parent_directory_casing() {
    const IMAGE: &[u8] = b"\x89PNG\r\n\x1a\nportable-parent-image";
    const ATTACHMENT: &[u8] = b"portable-parent-attachment";
    let source = TestWorkspace::new("portable-parent-source");
    let workspace = TestWorkspace::new("portable-parent-target");
    fs::create_dir_all(source.root.join("assets/diagrams"))
        .expect("source image folder should be created");
    fs::create_dir_all(source.root.join("assets/reports"))
        .expect("source attachment folder should be created");
    fs::create_dir_all(workspace.root.join("Assets/Diagrams"))
        .expect("target image folder should be created");
    fs::create_dir_all(workspace.root.join("Assets/Reports"))
        .expect("target attachment folder should be created");
    fs::write(source.root.join("assets/diagrams/Diagram.png"), IMAGE)
        .expect("source image should be written");
    fs::write(source.root.join("assets/reports/Report.pdf"), ATTACHMENT)
        .expect("source attachment should be written");
    fs::write(
        workspace.root.join("Assets/Diagrams/Diagram.png"),
        b"existing-image",
    )
    .expect("existing image should be written");
    fs::write(
        workspace.root.join("Assets/Reports/Report.pdf"),
        b"existing-attachment",
    )
    .expect("existing attachment should be written");

    assert_eq!(
        unique_image_relative_path(&workspace.root, "assets/diagrams", "Fresh.png").unwrap(),
        "Assets/Diagrams/Fresh.png",
    );
    assert_eq!(
        unique_attachment_relative_path(&workspace.root, "assets/reports", "Fresh.pdf").unwrap(),
        "Assets/Reports/Fresh.pdf",
    );

    let result = begin_workspace_asset_import(
        &workspace.root,
        &source.root,
        &["assets/diagrams/Diagram.png".to_owned()],
        &["assets/reports/Report.pdf".to_owned()],
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect("portable parent-directory collisions should be resolved");

    assert_eq!(
        result.path_mappings,
        BTreeMap::from([
            (
                "assets/diagrams/Diagram.png".to_owned(),
                "Assets/Diagrams/Diagram 2.png".to_owned(),
            ),
            (
                "assets/reports/Report.pdf".to_owned(),
                "Assets/Reports/Report 2.pdf".to_owned(),
            ),
        ]),
    );
    assert_eq!(
        fs::read(workspace.root.join("Assets/Diagrams/Diagram 2.png")).unwrap(),
        IMAGE,
    );
    assert_eq!(
        fs::read(workspace.root.join("Assets/Reports/Report 2.pdf")).unwrap(),
        ATTACHMENT,
    );
    assert!(!workspace.root.join("assets").exists());

    let transaction_id = result
        .transaction_id
        .as_deref()
        .expect("copied assets should retain a transaction");
    finalize_workspace_image_import(
        &workspace.root,
        transaction_id,
        &mut WarningCollector::default(),
    )
    .expect("the completed import should be finalized");
}

#[test]
fn imports_images_and_streamed_attachments_in_one_transaction() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\ncombined-import-image";
    const REPORT: &[u8] = b"combined-import-attachment";
    const EXISTING: &[u8] = b"existing-report";
    let source = TestWorkspace::new("portable-asset-source");
    let workspace = TestWorkspace::new("portable-asset-target");
    fs::create_dir_all(source.root.join("Assets")).unwrap();
    fs::create_dir_all(workspace.root.join("Assets")).unwrap();
    fs::write(source.root.join("Assets/Diagram.png"), PNG).unwrap();
    fs::write(source.root.join("Assets/Report.pdf"), REPORT).unwrap();
    fs::write(source.root.join("Assets/Empty.bin"), []).unwrap();
    fs::write(workspace.root.join("Assets/Report.pdf"), EXISTING).unwrap();

    let mut result = begin_workspace_asset_import(
        &workspace.root,
        &source.root,
        &["Assets/Diagram.png".to_owned()],
        &[
            "Assets/Report.pdf".to_owned(),
            "Assets/Empty.bin".to_owned(),
        ],
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect("images and attachments should share an import transaction");

    assert_eq!(result.image_count, 1);
    assert_eq!(result.attachment_count, 2);
    assert_eq!(
        result.path_mappings.get("Assets/Report.pdf"),
        Some(&"Assets/Report 2.pdf".to_owned()),
    );
    assert_eq!(
        result.attachment_files,
        vec![
            VaultAttachmentFile {
                asset_id: None,
                relative_path: "Assets/Report 2.pdf".to_owned(),
                media_type: "application/pdf".to_owned(),
                byte_length: REPORT.len() as u64,
                opening_disabled: false,
            },
            VaultAttachmentFile {
                asset_id: None,
                relative_path: "Assets/Empty.bin".to_owned(),
                media_type: "application/octet-stream".to_owned(),
                byte_length: 0,
                opening_disabled: true,
            },
        ],
    );
    assert_eq!(
        fs::read(workspace.root.join("Assets/Diagram.png")).unwrap(),
        PNG,
    );
    assert_eq!(
        fs::read(workspace.root.join("Assets/Report.pdf")).unwrap(),
        EXISTING,
    );
    assert_eq!(
        fs::read(workspace.root.join("Assets/Report 2.pdf")).unwrap(),
        REPORT,
    );
    assert_eq!(
        fs::metadata(workspace.root.join("Assets/Empty.bin"))
            .unwrap()
            .len(),
        0,
    );

    let transaction_id = result
        .transaction_id
        .take()
        .expect("copied assets should retain a transaction");
    let (_, manifest) = pending_workspace_image_import(&workspace.root, &transaction_id)
        .expect("both asset kinds should produce a valid pending import");
    assert!(manifest
        .targets
        .iter()
        .any(|target| target.kind == TransactionTargetKind::Image));
    assert_eq!(
        manifest
            .targets
            .iter()
            .filter(|target| target.kind == TransactionTargetKind::Attachment)
            .count(),
        2,
    );
    finalize_workspace_image_import(
        &workspace.root,
        &transaction_id,
        &mut WarningCollector::default(),
    )
    .expect("the combined transaction should finalize");
}

#[test]
fn failed_note_import_rolls_back_its_copied_assets() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrollback-with-notes";
    const PDF: &[u8] = b"rollback-attachment-with-notes";
    let source = TestWorkspace::new("portable-image-note-rollback-source");
    let workspace = TestWorkspace::new("portable-image-note-rollback-target");
    fs::write(source.root.join("Image.png"), PNG).expect("source image should be written");
    fs::write(source.root.join("Report.pdf"), PDF).expect("source attachment should be written");
    let original_revision = revision_for_root(&workspace.root).unwrap();
    let image_result = begin_workspace_asset_import(
        &workspace.root,
        &source.root,
        &["Image.png".to_owned()],
        &["Report.pdf".to_owned()],
        original_revision,
    )
    .expect("asset import should begin");
    let transaction_id = image_result
        .transaction_id
        .as_deref()
        .expect("copied images should retain a transaction");
    assert!(workspace.root.join("Image.png").exists());
    assert!(workspace.root.join("Report.pdf").exists());

    let mut invalid_vault = empty_vault("Invalid import");
    invalid_vault.folders.push(Folder {
        id: "invalid-folder".to_owned(),
        name: String::new(),
        parent_id: None,
        created_at: 1,
    });
    let result = save_workspace_files_with_image_import(
        &workspace.root,
        &invalid_vault,
        image_result.revision,
        transaction_id,
    )
    .expect("a rejected note save should report a completed rollback");

    assert!(!result.saved);
    assert!(result.error.is_some_and(|error| error.contains("folder")));
    assert!(!workspace.root.join("Image.png").exists());
    assert!(!workspace.root.join("Report.pdf").exists());
    assert_eq!(result.revision, original_revision);
    assert!(existing_transaction_root(&workspace.root, transaction_id).is_err());
}

#[test]
fn failed_import_rollback_preserves_concurrent_vault_edits() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrollback-around-external-edit";
    let source = TestWorkspace::new("portable-image-external-rollback-source");
    let workspace = TestWorkspace::new("portable-image-external-rollback-target");
    fs::write(source.root.join("Image.png"), PNG).expect("source image should be written");
    let image_result = begin_workspace_image_import(
        &workspace.root,
        &source.root,
        &["Image.png".to_owned()],
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect("image import should begin");
    let transaction_id = image_result
        .transaction_id
        .as_deref()
        .expect("copied images should retain a transaction");
    fs::write(workspace.root.join("External.md"), "external edit")
        .expect("the vault should change outside the import");

    let result = save_workspace_files_with_image_import(
        &workspace.root,
        &empty_vault("Concurrent import"),
        image_result.revision,
        transaction_id,
    )
    .expect("the copied image should roll back around the external edit");

    assert!(!result.saved);
    assert!(result.error.is_some_and(|error| error.contains("changed")));
    assert!(!workspace.root.join("Image.png").exists());
    assert_eq!(
        fs::read_to_string(workspace.root.join("External.md")).unwrap(),
        "external edit",
    );
    assert_eq!(result.revision, revision_for_root(&workspace.root).unwrap());
}

#[test]
fn successful_note_import_commits_its_copied_images() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\ncommit-with-notes";
    let source = TestWorkspace::new("portable-image-note-commit-source");
    let workspace = TestWorkspace::new("portable-image-note-commit-target");
    fs::write(source.root.join("Image.png"), PNG).expect("source image should be written");
    let image_result = begin_workspace_image_import(
        &workspace.root,
        &source.root,
        &["Image.png".to_owned()],
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect("image import should begin");
    let transaction_id = image_result
        .transaction_id
        .as_deref()
        .expect("copied images should retain a transaction");
    let mut vault = empty_vault("Committed import");
    let mut note = test_note("![Image](Image.png)");
    note.title = "Imported note".to_owned();
    note.relative_path = "Imported note.md".to_owned();
    vault.notes.push(note);

    let result = save_workspace_files_with_image_import(
        &workspace.root,
        &vault,
        image_result.revision,
        transaction_id,
    )
    .expect("notes and copied images should commit together");

    assert!(result.saved);
    assert_eq!(fs::read(workspace.root.join("Image.png")).unwrap(), PNG);
    assert_eq!(
        fs::read_to_string(workspace.root.join("Imported note.md")).unwrap(),
        "![Image](Image.png)",
    );
    assert!(existing_transaction_root(&workspace.root, transaction_id).is_err());
    let (state, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    assert_eq!(
        state.unwrap().last_committed_image_import_id.as_deref(),
        Some(transaction_id),
    );
}

#[test]
fn interrupted_pending_image_import_recovers_at_the_state_commit_boundary() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrecover-pending-import";
    let source = TestWorkspace::new("portable-image-crash-source");
    fs::write(source.root.join("Image.png"), PNG).expect("source image should be written");

    let uncommitted = TestWorkspace::new("portable-image-crash-uncommitted");
    let uncommitted_result = begin_workspace_image_import(
        &uncommitted.root,
        &source.root,
        &["Image.png".to_owned()],
        revision_for_root(&uncommitted.root).unwrap(),
    )
    .expect("uncommitted import should begin");
    let uncommitted_id = uncommitted_result.transaction_id.unwrap();
    let mut warnings = WarningCollector::default();
    recover_workspace_transactions(&uncommitted.root, None, &mut warnings)
        .expect("uncommitted import should recover");
    assert!(!uncommitted.root.join("Image.png").exists());
    assert!(existing_transaction_root(&uncommitted.root, &uncommitted_id).is_err());

    let committed = TestWorkspace::new("portable-image-crash-committed");
    let committed_result = begin_workspace_image_import(
        &committed.root,
        &source.root,
        &["Image.png".to_owned()],
        revision_for_root(&committed.root).unwrap(),
    )
    .expect("committed import should begin");
    let committed_id = committed_result.transaction_id.unwrap();
    let mut state = WorkspaceState::default();
    state.last_committed_image_import_id = Some(committed_id.clone());
    write_workspace_state(&committed.root, &state)
        .expect("the image import commit boundary should be recorded");
    let mut warnings = WarningCollector::default();
    recover_workspace_transactions(&committed.root, Some(&state), &mut warnings)
        .expect("committed import should finalize");
    assert_eq!(fs::read(committed.root.join("Image.png")).unwrap(), PNG);
    assert!(existing_transaction_root(&committed.root, &committed_id).is_err());
}

#[test]
fn image_import_reserves_collision_targets_for_the_whole_batch() {
    const FIRST: &[u8] = b"\x89PNG\r\n\x1a\nfirst-import";
    const SECOND: &[u8] = b"\x89PNG\r\n\x1a\nsecond-import";
    const EXISTING: &[u8] = b"\x89PNG\r\n\x1a\nexisting-image";
    let source = TestWorkspace::new("portable-image-reservation-source");
    let workspace = TestWorkspace::new("portable-image-reservation-target");
    fs::write(source.root.join("Image.png"), FIRST).expect("first source should be written");
    fs::write(source.root.join("Image 2.png"), SECOND).expect("second source should be written");
    fs::write(workspace.root.join("Image.png"), EXISTING)
        .expect("existing target should be written");

    let result = import_workspace_images(
        &workspace.root,
        &source.root,
        &["Image.png".into(), "Image 2.png".into()],
        revision_for_root(&workspace.root).expect("revision should be available"),
    )
    .expect("both colliding paths should be imported safely");

    assert_eq!(result.image_count, 2);
    assert_eq!(
        result.path_mappings,
        BTreeMap::from([
            ("Image.png".to_owned(), "Image 2.png".to_owned()),
            ("Image 2.png".to_owned(), "Image 2 2.png".to_owned()),
        ]),
    );
    assert_eq!(
        fs::read(workspace.root.join("Image.png")).unwrap(),
        EXISTING
    );
    assert_eq!(fs::read(workspace.root.join("Image 2.png")).unwrap(), FIRST);
    assert_eq!(
        fs::read(workspace.root.join("Image 2 2.png")).unwrap(),
        SECOND,
    );
}

#[test]
fn image_import_rejects_changes_while_files_are_prepared() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nprepared-import";
    let workspace = TestWorkspace::new("portable-image-concurrent-target");
    fs::write(workspace.root.join("Note.md"), "before").expect("target note should be written");
    let revision =
        revision_for_root(&workspace.root).expect("initial revision should be available");

    let mut transaction = prepare_workspace_image_import(&workspace.root, revision)
        .expect("the import transaction should be prepared");
    stage_workspace_image_import(&workspace.root, &mut transaction, "Image.png", PNG)
        .expect("the image should be staged privately");
    assert!(transaction
        .transaction_root
        .as_ref()
        .is_some_and(|path| path.is_dir()));
    assert!(!workspace.root.join("Image.png").exists());

    fs::write(workspace.root.join("Note.md"), "external edit")
        .expect("the note should change outside the import");
    let external_revision =
        revision_for_root(&workspace.root).expect("external revision should be available");
    let mut warnings = WarningCollector::default();
    let error = apply_workspace_image_import(&workspace.root, transaction, &mut warnings)
        .expect_err("the concurrent edit must reject the import");

    assert!(error.contains("vault changed"));
    assert!(warnings.finish().is_empty());
    assert_eq!(
        fs::read_to_string(workspace.root.join("Note.md")).unwrap(),
        "external edit",
    );
    assert!(!workspace.root.join("Image.png").exists());
    assert_eq!(
        revision_for_root(&workspace.root).unwrap(),
        external_revision,
    );
}

#[test]
fn image_import_consistency_rolls_back_only_imported_files() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\napplying-import";
    let workspace = TestWorkspace::new("portable-image-applying-target");
    fs::write(workspace.root.join("Note.md"), "before").expect("target note should be written");
    let baseline =
        revision_entries_for_root(&workspace.root).expect("baseline should be available");
    let transaction_root = prepare_transaction_root(&workspace.root, &new_transaction_id())
        .expect("transaction should be created");
    let staged = staged_import_image_path(&transaction_root, "Assets/Image.png")
        .expect("staged path should be valid");
    ensure_private_directory_tree(&transaction_root, staged.parent().unwrap())
        .expect("staged parent should be created");
    atomic_write(&staged, PNG).expect("image should be staged");
    let target = TransactionTarget {
        relative_path: "Assets/Image.png".to_owned(),
        fingerprint: fingerprint_bytes(PNG),
        kind: TransactionTargetKind::Image,
    };
    let manifest = TransactionManifest {
        version: TRANSACTION_VERSION,
        id: transaction_root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        phase: TransactionPhase::Applying,
        originals: Vec::new(),
        targets: vec![target.clone()],
        recovery_targets: Vec::new(),
        folder_case_renames: Vec::new(),
        created_directories: vec!["Assets".to_owned()],
    };
    write_transaction_manifest(&transaction_root, &manifest)
        .expect("applying manifest should be written");
    ensure_asset_parent(&workspace.root, &target.relative_path, "image")
        .expect("target parent should be created");
    apply_staged_import_image(&workspace.root, &transaction_root, &target)
        .expect("image should begin applying");

    fs::write(workspace.root.join("Note.md"), "external edit")
        .expect("the note should change while applying");
    let error = verify_image_import_consistency(&workspace.root, &baseline, &manifest)
        .expect_err("the concurrent edit must fail consistency verification");
    assert!(error.contains("vault changed"));

    let mut warnings = WarningCollector::default();
    let recovered =
        rollback_transaction(&workspace.root, &transaction_root, &manifest, &mut warnings);
    assert!(recovered, "rollback warnings: {:?}", warnings.warnings);
    discard_private_transaction(&workspace.root, &transaction_root, &mut warnings);
    assert!(warnings.finish().is_empty());
    assert_eq!(
        fs::read_to_string(workspace.root.join("Note.md")).unwrap(),
        "external edit",
    );
    assert!(!workspace.root.join("Assets/Image.png").exists());
    assert!(!workspace.root.join("Assets").exists());
}

#[test]
fn image_import_rollback_preserves_an_unowned_matching_file() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nexternal-matching-image";
    let workspace = TestWorkspace::new("portable-image-unowned-target");
    let transaction_root = prepare_transaction_root(&workspace.root, &new_transaction_id())
        .expect("transaction should be created");
    let target = TransactionTarget {
        relative_path: "Image.png".to_owned(),
        fingerprint: fingerprint_bytes(PNG),
        kind: TransactionTargetKind::Image,
    };
    let manifest = TransactionManifest {
        version: TRANSACTION_VERSION,
        id: transaction_root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        phase: TransactionPhase::Applying,
        originals: Vec::new(),
        targets: vec![target],
        recovery_targets: Vec::new(),
        folder_case_renames: Vec::new(),
        created_directories: Vec::new(),
    };
    write_transaction_manifest(&transaction_root, &manifest)
        .expect("applying manifest should be written");
    fs::write(workspace.root.join("Image.png"), PNG)
        .expect("an external process should create the matching target");

    let mut warnings = WarningCollector::default();
    assert!(rollback_transaction(
        &workspace.root,
        &transaction_root,
        &manifest,
        &mut warnings,
    ));
    assert_eq!(fs::read(workspace.root.join("Image.png")).unwrap(), PNG);
    discard_private_transaction(&workspace.root, &transaction_root, &mut warnings);
    assert!(warnings.finish().is_empty());
}

#[test]
fn image_import_rejects_stale_revisions_and_unsafe_paths() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nimport-validation";
    let source = TestWorkspace::new("portable-image-validation-source");
    let workspace = TestWorkspace::new("portable-image-validation-target");
    fs::write(source.root.join("Image.png"), PNG).expect("source image should be written");
    let stale_revision = revision_for_root(&workspace.root).expect("revision should exist");
    fs::write(workspace.root.join("Changed.md"), "changed")
        .expect("external note should be written");

    let error = import_workspace_images(
        &workspace.root,
        &source.root,
        &["Image.png".into()],
        stale_revision,
    )
    .expect_err("a stale import should be rejected");
    assert!(error.contains("vault changed"));
    assert!(!workspace.root.join("Image.png").exists());

    let error = import_workspace_images(
        &workspace.root,
        &source.root,
        &["../Image.png".into()],
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect_err("an unsafe path should be rejected");
    assert!(error.contains("Parent"));
}

#[test]
fn image_storage_rejects_unsafe_or_mismatched_inputs() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nvalidation-fixture";
    assert!(validate_image_bytes(b"<svg></svg>", Some("image.svg")).is_err());
    assert!(validate_image_bytes(PNG, Some("image.jpg")).is_err());
    assert!(resolve_markdown_image_path("Notes/First.md", "../../escape.png").is_err());
    assert!(normalize_image_embed_settings(&ImageEmbedSettings {
        location: ImageEmbedLocation::SpecifiedFolder,
        folder_path: "../outside".to_owned(),
    })
    .is_err());
    assert_eq!(
        percent_decode_utf8("%7B%22fileName%22%3A%22Caf%C3%A9.png%22%7D")
            .expect("encoded metadata should decode"),
        "{\"fileName\":\"Café.png\"}",
    );
    assert!(percent_decode_utf8("%GG").is_err());
    let unicode_name = safe_image_file_name(&format!("{}.png", "😀".repeat(100)), "png");
    assert!(unicode_name.len() <= 180);
    assert!(validate_component_name(&unicode_name, "image").is_ok());
}

#[cfg(unix)]
#[test]
fn image_storage_refuses_source_and_destination_symlinks() {
    use std::os::unix::fs::symlink;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nsymlink-fixture";
    let workspace = TestWorkspace::new("embedded-image-symlinks");
    let outside = TestWorkspace::new("embedded-image-symlinks-outside");
    fs::write(workspace.root.join("Note.md"), "# Note").expect("note should be written");
    fs::write(outside.root.join("Source.png"), PNG).expect("source should be written");
    symlink(
        outside.root.join("Source.png"),
        workspace.root.join("Source link.png"),
    )
    .expect("source symlink should be created");
    symlink(&outside.root, workspace.root.join("Linked"))
        .expect("destination symlink should be created");
    write_workspace_state(&workspace.root, &WorkspaceState::default())
        .expect("workspace state should be written");

    assert!(validate_image_source_file(
        workspace
            .root
            .join("Source link.png")
            .to_str()
            .expect("path should be Unicode")
    )
    .is_err());
    let error = embed_workspace_image(
        &workspace.root,
        "Note.md",
        ImageEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolder,
            folder_path: "Linked".to_owned(),
        },
        "Image.png",
        PNG,
        None,
        revision_for_root(&workspace.root).expect("revision should be available"),
    )
    .expect_err("destination symlink should be rejected");
    assert!(error.contains("symbolic link"));
    assert!(!outside.root.join("Image.png").exists());

    let import_target = TestWorkspace::new("embedded-image-import-symlink-target");
    fs::write(import_target.root.join("Source link.png"), PNG)
        .expect("an unrelated target image should be written");
    let import_result = import_workspace_images(
        &import_target.root,
        &workspace.root,
        &["Source link.png".into()],
        revision_for_root(&import_target.root).expect("revision should be available"),
    )
    .expect("unsafe source images should be reported without mutation");
    assert_eq!(import_result.image_count, 0);
    assert_eq!(
        import_result.path_mappings,
        BTreeMap::from([("Source link.png".to_owned(), "Source link 2.png".to_owned(),)]),
    );
    assert!(import_result
        .warnings
        .iter()
        .any(|warning| warning.contains("symbolic links")));
    assert_eq!(
        fs::read(import_target.root.join("Source link.png")).unwrap(),
        PNG,
    );
    assert!(!import_target.root.join("Source link 2.png").exists());
}
