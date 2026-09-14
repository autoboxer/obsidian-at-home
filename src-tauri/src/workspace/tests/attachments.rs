fn asset_deletion_fixture() -> (TestWorkspace, Note, WorkspaceImageNoteUpdate) {
    let workspace = TestWorkspace::new("asset-deletion");
    let note = test_note("[Custom label](Report.pdf#oah-asset=attachment-report)");
    let mut state = write_saved_note(&workspace, &note);
    let path = workspace.root.join("Report.pdf");
    fs::write(&path, b"report bytes").unwrap();
    state.assets.insert(
        "attachment-report".to_owned(),
        StoredVaultAsset {
            kind: VaultAssetKind::Attachment,
            relative_path: "Report.pdf".to_owned(),
            media_type: "application/pdf".to_owned(),
            fingerprint: fingerprint_attachment_file(&path).unwrap(),
            modified_nanos: file_modified_nanos_for_path(&path).unwrap(),
        },
    );
    write_workspace_state(&workspace.root, &state).unwrap();
    let update = WorkspaceImageNoteUpdate {
        note_id: note.id.clone(),
        relative_path: note.relative_path.clone(),
        expected_content: note.content.clone(),
        content: "Reference to Report.pdf deleted".to_owned(),
    };
    (workspace, note, update)
}

fn asset_deletion_recovery_fixture(
) -> (TestWorkspace, RecentlyDeletedNote, WorkspaceImageNoteUpdate) {
    let (workspace, note, update) = asset_deletion_fixture();
    let (_, deleted) = save_workspace_files_with_archive(
        &workspace.root,
        &empty_vault("Test vault"),
        revision_for_root(&workspace.root).unwrap(),
        Some(PendingNoteArchive {
            note,
            original_folder_path: String::new(),
            editor_position: Some(editor_position(3)),
        }),
    )
    .unwrap();
    let deleted = deleted.unwrap();
    let update = WorkspaceImageNoteUpdate {
        note_id: deleted.id.clone(),
        ..update
    };
    (workspace, deleted, update)
}

#[test]
fn asset_deletion_commits_markers_and_removes_only_the_selected_file() {
    let (workspace, note, update) = asset_deletion_fixture();
    fs::write(workspace.root.join("Other.pdf"), "other bytes").unwrap();
    let saved = delete_workspace_asset(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "Report.pdf",
        Some("attachment-report"),
        &[update.clone()],
        &[],
        revision_for_root(&workspace.root).unwrap(),
    )
    .unwrap();
    assert!(!workspace.root.join("Report.pdf").exists());
    assert_eq!(
        fs::read_to_string(workspace.root.join("Other.pdf")).unwrap(),
        "other bytes"
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join(&note.relative_path)).unwrap(),
        update.content
    );
    let loaded = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    assert_eq!(loaded.vault.notes[0].content, update.content);
    assert!(loaded.vault.embedded_attachments.is_empty());
    assert_eq!(saved.revision, revision_for_root(&workspace.root).unwrap());
    assert!(!workspace
        .root
        .join(STATE_DIRECTORY)
        .join(TRANSACTIONS_DIRECTORY)
        .exists());
}

#[test]
fn asset_deletion_rejects_stale_content_revision_and_identity() {
    let (workspace, note, update) = asset_deletion_fixture();
    let revision = revision_for_root(&workspace.root).unwrap();
    let metadata = fs::read(workspace_state_path(&workspace.root)).unwrap();
    for (id, candidate, expected_revision) in [
        (Some("attachment-report"), update.clone(), revision + 1),
        (Some("missing-id"), update.clone(), revision),
        (
            Some("attachment-report"),
            WorkspaceImageNoteUpdate {
                expected_content: "stale".to_owned(),
                ..update.clone()
            },
            revision,
        ),
    ] {
        assert!(delete_workspace_asset(
            &workspace.root,
            ExternalFileUploadKind::Attachment,
            "Report.pdf",
            id,
            &[candidate],
            &[],
            expected_revision
        )
        .is_err());
    }
    assert!(delete_workspace_asset(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "Other.pdf",
        Some("attachment-report"),
        &[update.clone()],
        &[],
        revision
    )
    .is_err());
    assert!(delete_workspace_asset(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "Report.pdf",
        Some("attachment-report"),
        &[update.clone(), update],
        &[],
        revision
    )
    .is_err());
    assert_eq!(
        fs::read_to_string(workspace.root.join(&note.relative_path)).unwrap(),
        note.content
    );
    assert_eq!(
        fs::read(workspace.root.join("Report.pdf")).unwrap(),
        b"report bytes"
    );
    assert_eq!(
        fs::read(workspace_state_path(&workspace.root)).unwrap(),
        metadata
    );
}

#[test]
fn asset_deletion_accepts_untracked_images_and_extensionless_files() {
    let workspace = TestWorkspace::new("asset-deletion-untracked");
    for (path, kind) in [
        ("Photo.png", ExternalFileUploadKind::Image),
        ("LICENSE", ExternalFileUploadKind::Attachment),
    ] {
        fs::write(workspace.root.join(path), "unreferenced bytes").unwrap();
        delete_workspace_asset(
            &workspace.root,
            kind,
            path,
            None,
            &[],
            &[],
            revision_for_root(&workspace.root).unwrap(),
        )
        .unwrap();
        assert!(!workspace.root.join(path).exists());
    }
}

#[test]
fn asset_deletion_retains_recovery_metadata_and_rejects_stale_snapshots() {
    let (workspace, deleted, update) = asset_deletion_recovery_fixture();
    let revision = revision_for_root(&workspace.root).unwrap();
    let snapshot_path = recently_deleted_snapshot_path(&workspace.root, &deleted.id).unwrap();
    let original_bytes = fs::read(&snapshot_path).unwrap();
    let stale = WorkspaceImageNoteUpdate {
        expected_content: "stale".to_owned(),
        ..update.clone()
    };
    assert!(delete_workspace_asset(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "Report.pdf",
        Some("attachment-report"),
        &[],
        &[stale],
        revision
    )
    .is_err());
    assert_eq!(fs::read(&snapshot_path).unwrap(), original_bytes);
    delete_workspace_asset(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "Report.pdf",
        Some("attachment-report"),
        &[],
        &[update.clone()],
        revision,
    )
    .unwrap();
    let loaded = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    let mut expected = deleted.clone();
    expected.note.content = update.content;
    expected.editor_position = None;
    assert_eq!(loaded.recently_deleted_notes, vec![expected]);
    assert!(loaded.vault.attachment_files.is_empty());
}

#[test]
fn asset_deletion_rolls_back_each_precommit_failure() {
    for stage in ["prepared", "references-written", "asset-removed"] {
        let (workspace, note, update) = asset_deletion_fixture();
        let state_bytes = fs::read(workspace_state_path(&workspace.root)).unwrap();
        let error = delete_workspace_asset_with_hook(
            &workspace.root,
            ExternalFileUploadKind::Attachment,
            "Report.pdf",
            Some("attachment-report"),
            &[update],
            &[],
            revision_for_root(&workspace.root).unwrap(),
            |checkpoint| {
                if checkpoint == stage {
                    Err("Injected write failure".to_owned())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert!(error.contains("Injected write failure"), "{stage}: {error}");
        assert!(error.contains("rolled back"), "{stage}: {error}");
        assert_eq!(
            fs::read(workspace_state_path(&workspace.root)).unwrap(),
            state_bytes
        );
        assert_eq!(
            fs::read_to_string(workspace.root.join(&note.relative_path)).unwrap(),
            note.content
        );
        assert_eq!(
            fs::read(workspace.root.join("Report.pdf")).unwrap(),
            b"report bytes"
        );
        load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    }
}

#[test]
fn asset_deletion_rolls_back_recovery_and_live_references_together() {
    let (workspace, deleted, recovery_update) = asset_deletion_recovery_fixture();
    fs::write(workspace.root.join("Live.md"), &deleted.note.content).unwrap();
    let loaded = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
    let live = &loaded.vault.notes[0];
    let update = WorkspaceImageNoteUpdate {
        note_id: live.id.clone(),
        relative_path: live.relative_path.clone(),
        expected_content: live.content.clone(),
        content: recovery_update.content.clone(),
    };
    let snapshot_path = recently_deleted_snapshot_path(&workspace.root, &deleted.id).unwrap();
    let snapshot_bytes = fs::read(&snapshot_path).unwrap();
    let error = delete_workspace_asset_with_hook(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "Report.pdf",
        Some("attachment-report"),
        &[update],
        &[recovery_update],
        loaded.revision,
        |checkpoint| {
            if checkpoint == "asset-removed" {
                Err("Injected write failure".to_owned())
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    assert!(error.contains("Injected write failure"), "{error}");
    assert!(error.contains("rolled back"), "{error}");
    assert_eq!(fs::read(&snapshot_path).unwrap(), snapshot_bytes);
    assert_eq!(
        fs::read_to_string(workspace.root.join("Live.md")).unwrap(),
        live.content
    );
    assert_eq!(
        fs::read(workspace.root.join("Report.pdf")).unwrap(),
        b"report bytes"
    );
}

#[test]
fn asset_deletion_recovers_interrupted_deletions_and_retains_committed_ones() {
    for stage in ["references-written", "asset-removed", "committed"] {
        let (workspace, deleted, update) = asset_deletion_recovery_fixture();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            delete_workspace_asset_with_hook(
                &workspace.root,
                ExternalFileUploadKind::Attachment,
                "Report.pdf",
                Some("attachment-report"),
                &[],
                &[update.clone()],
                revision_for_root(&workspace.root).unwrap(),
                |checkpoint| {
                    assert_ne!(checkpoint, stage, "Simulated interruption");
                    Ok(())
                },
            )
        }));
        assert!(result.is_err());
        let loaded = load_workspace(&workspace.root, &empty_vault("Test vault")).unwrap();
        assert_eq!(
            workspace.root.join("Report.pdf").exists(),
            stage != "committed"
        );
        assert_eq!(
            loaded.recently_deleted_notes[0].note.content,
            if stage == "committed" {
                update.content.clone()
            } else {
                deleted.note.content.clone()
            }
        );
        if stage != "committed" {
            assert_eq!(loaded.recently_deleted_notes[0], deleted);
        }
    }
}

#[test]
fn asset_deletion_preserves_concurrently_edited_notes_and_backups() {
    let (workspace, note, update) = asset_deletion_fixture();
    let error = delete_workspace_asset_with_hook(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "Report.pdf",
        Some("attachment-report"),
        &[update],
        &[],
        revision_for_root(&workspace.root).unwrap(),
        |checkpoint| {
            if checkpoint == "references-written" {
                fs::write(workspace.root.join(&note.relative_path), "External edit").unwrap();
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert!(error.contains("could not be fully rolled back"), "{error}");
    assert_eq!(
        fs::read_to_string(workspace.root.join(&note.relative_path)).unwrap(),
        "External edit"
    );
    assert_eq!(
        fs::read(workspace.root.join("Report.pdf")).unwrap(),
        b"report bytes"
    );
    assert!(workspace
        .root
        .join(STATE_DIRECTORY)
        .join(TRANSACTIONS_DIRECTORY)
        .exists());
}

#[test]
fn asset_deletion_detects_unrelated_external_changes() {
    let (workspace, note, update) = asset_deletion_fixture();
    let error = delete_workspace_asset_with_hook(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "Report.pdf",
        Some("attachment-report"),
        &[update],
        &[],
        revision_for_root(&workspace.root).unwrap(),
        |checkpoint| {
            if checkpoint == "references-written" {
                fs::write(workspace.root.join("Other.md"), "External note").unwrap();
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert!(error.contains("vault changed"), "{error}");
    assert_eq!(
        fs::read_to_string(workspace.root.join(&note.relative_path)).unwrap(),
        note.content
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join("Other.md")).unwrap(),
        "External note"
    );
    assert_eq!(
        fs::read(workspace.root.join("Report.pdf")).unwrap(),
        b"report bytes"
    );
}

#[cfg(unix)]
#[test]
fn asset_deletion_rolls_back_after_a_real_metadata_write_failure() {
    use std::os::unix::fs::PermissionsExt;
    let (workspace, note, update) = asset_deletion_fixture();
    let directory = workspace.root.join(STATE_DIRECTORY);
    let permissions = fs::metadata(&directory).unwrap().permissions();
    let result = delete_workspace_asset_with_hook(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "Report.pdf",
        Some("attachment-report"),
        &[update],
        &[],
        revision_for_root(&workspace.root).unwrap(),
        |checkpoint| {
            if checkpoint == "asset-removed" {
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();
            }
            Ok(())
        },
    );
    fs::set_permissions(&directory, permissions).unwrap();
    assert!(
        result.is_err(),
        "metadata replacement must fail without directory write permission"
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join(&note.relative_path)).unwrap(),
        note.content
    );
    assert_eq!(
        fs::read(workspace.root.join("Report.pdf")).unwrap(),
        b"report bytes"
    );
}

#[test]
fn asset_deletion_refuses_unknown_metadata_and_unreadable_recovery() {
    let (workspace, deleted, _) = asset_deletion_recovery_fixture();
    let path = recently_deleted_snapshot_path(&workspace.root, &deleted.id).unwrap();
    fs::write(path, "damaged snapshot").unwrap();
    assert!(delete_workspace_asset(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "Report.pdf",
        Some("attachment-report"),
        &[],
        &[],
        revision_for_root(&workspace.root).unwrap()
    )
    .is_err());
    fs::write(workspace_state_path(&workspace.root), "{\"version\":999}").unwrap();
    assert!(delete_workspace_asset(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "Report.pdf",
        None,
        &[],
        &[],
        revision_for_root(&workspace.root).unwrap()
    )
    .is_err());
    assert_eq!(
        fs::read(workspace.root.join("Report.pdf")).unwrap(),
        b"report bytes"
    );
}

#[cfg(unix)]
#[test]
fn asset_deletion_rejects_unsafe_targets_and_keeps_case_siblings() {
    let (workspace, note, _) = asset_deletion_fixture();
    let outside = TestWorkspace::new("asset-deletion-outside");
    fs::write(outside.root.join("Outside.pdf"), "outside bytes").unwrap();
    std::os::unix::fs::symlink(
        outside.root.join("Outside.pdf"),
        workspace.root.join("Link.pdf"),
    )
    .unwrap();
    fs::write(workspace.root.join(".DS_Store"), "Finder state").unwrap();
    fs::write(workspace.root.join("report.pdf"), "case sibling").unwrap();
    for path in [
        note.relative_path.as_str(),
        "Link.pdf",
        "../Outside.pdf",
        ".DS_Store",
    ] {
        assert!(
            delete_workspace_asset(
                &workspace.root,
                ExternalFileUploadKind::Attachment,
                path,
                None,
                &[],
                &[],
                revision_for_root(&workspace.root).unwrap()
            )
            .is_err(),
            "{path}"
        );
    }
    delete_workspace_asset(
        &workspace.root,
        ExternalFileUploadKind::Attachment,
        "report.pdf",
        None,
        &[],
        &[],
        revision_for_root(&workspace.root).unwrap(),
    )
    .unwrap();
    assert_eq!(
        fs::read(workspace.root.join("Report.pdf")).unwrap(),
        b"report bytes"
    );
    assert_eq!(
        fs::read_to_string(outside.root.join("Outside.pdf")).unwrap(),
        "outside bytes"
    );
}

#[test]
fn finder_metadata_is_removed_from_loaded_attachment_inventories() {
    let workspace = TestWorkspace::new("finder-metadata-inventory");
    fs::create_dir(workspace.root.join("Assets")).unwrap();
    fs::write(workspace.root.join("Note.md"), "# Note").unwrap();
    let mut state = WorkspaceState::default();
    for (index, path) in [".DS_Store", "Assets/.ds_store", "Assets/Report.pdf"]
        .iter()
        .enumerate()
    {
        fs::write(workspace.root.join(path), path.as_bytes()).unwrap();
        state.assets.insert(
            format!("asset-{index}"),
            StoredVaultAsset {
                kind: VaultAssetKind::Attachment,
                relative_path: (*path).to_owned(),
                media_type: "application/octet-stream".to_owned(),
                fingerprint: fingerprint_bytes(path.as_bytes()),
                modified_nanos: 0,
            },
        );
    }
    fs::write(workspace.root.join("Assets/.DS_Store.txt"), "keep this").unwrap();
    fs::write(workspace.root.join("Assets/.env"), "also keep this").unwrap();
    write_workspace_state(&workspace.root, &state).unwrap();

    let loaded = load_workspace(&workspace.root, &empty_vault("Finder metadata")).unwrap();
    let mut paths = loaded
        .vault
        .attachment_files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(
        paths,
        ["Assets/.DS_Store.txt", "Assets/.env", "Assets/Report.pdf"]
    );
    assert_eq!(loaded.vault.embedded_attachments.len(), 1);
    assert_eq!(loaded.vault.embedded_attachments[0].id, "asset-2");
    let (stored, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    assert_eq!(stored.unwrap().assets.len(), 1);
    let reopened = load_workspace(&workspace.root, &loaded.vault).unwrap();
    assert_eq!(reopened.revision, loaded.revision);
    for path in [".DS_Store", "Assets/.ds_store"] {
        assert_eq!(
            fs::read(workspace.root.join(path)).unwrap(),
            path.as_bytes()
        );
    }
}

#[test]
fn finder_metadata_changes_do_not_invalidate_note_saves() {
    let workspace = TestWorkspace::new("finder-metadata-revision");
    fs::create_dir(workspace.root.join("Assets")).unwrap();
    fs::write(workspace.root.join("Note.md"), "# Original").unwrap();
    let loaded = load_workspace(&workspace.root, &empty_vault("Finder metadata")).unwrap();
    for path in [".DS_Store", "Assets/.DS_Store"] {
        fs::write(workspace.root.join(path), "Finder state").unwrap();
        assert_eq!(revision_for_root(&workspace.root).unwrap(), loaded.revision);
        fs::write(workspace.root.join(path), "Updated Finder state").unwrap();
        assert_eq!(revision_for_root(&workspace.root).unwrap(), loaded.revision);
        fs::remove_file(workspace.root.join(path)).unwrap();
        assert_eq!(revision_for_root(&workspace.root).unwrap(), loaded.revision);
    }
    fs::write(
        workspace.root.join("Assets/.DS_Store"),
        "Final Finder state",
    )
    .unwrap();
    let mut edited = loaded.vault;
    edited.notes[0].content = "# Edited".to_owned();
    let saved = save_workspace_files(&workspace.root, &edited, loaded.revision).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.root.join("Note.md")).unwrap(),
        "# Edited"
    );
    fs::write(workspace.root.join("Assets/Report.pdf"), "real attachment").unwrap();
    assert_ne!(revision_for_root(&workspace.root).unwrap(), saved.revision);
    assert!(save_workspace_files(&workspace.root, &edited, saved.revision).is_err());
}

#[test]
fn finder_metadata_insertion_is_rejected_before_copying_or_staging() {
    let source = TestWorkspace::new("finder-metadata-source");
    let workspace = TestWorkspace::new("finder-metadata-target");
    let staging = TestWorkspace::new("finder-metadata-upload");
    fs::write(workspace.root.join("Note.md"), "# Note").unwrap();
    let loaded = load_workspace(&workspace.root, &empty_vault("Finder metadata")).unwrap();
    for name in [".DS_Store", ".ds_store", ".DS_STORE"] {
        let path = source.root.join(name);
        fs::write(&path, "Finder state").unwrap();
        assert!(validate_attachment_source_path(&path).is_err());
        assert!(validate_attachment_relative_path(&format!("Assets/{name}")).is_err());
        assert!(safe_attachment_file_name(name).is_err());
        assert!(embed_workspace_attachment(
            &workspace.root,
            "Note.md",
            AttachmentEmbedSettings::default(),
            &path,
            None,
            loaded.revision,
        )
        .is_err());
        assert!(begin_external_file_upload(
            &staging.root,
            name.to_owned(),
            12,
            ExternalFileUploadKind::Attachment,
            workspace.root.clone(),
            "Note.md".to_owned(),
        )
        .is_err());
        assert!(begin_workspace_asset_import(
            &workspace.root,
            &source.root,
            &[],
            &[name.to_owned()],
            loaded.revision,
        )
        .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "Finder state");
        assert!(!workspace.root.join(name).exists());
        assert!(!workspace.root.join(name.trim_start_matches('.')).exists());
    }
    assert_eq!(fs::read_dir(&staging.root).unwrap().count(), 0);
    assert_eq!(revision_for_root(&workspace.root).unwrap(), loaded.revision);
    assert!(validate_attachment_relative_path("Assets/.DS_Store.txt").is_ok());
    assert!(validate_attachment_relative_path("Assets/.env").is_ok());
    assert!(validate_attachment_relative_path(".DS_Store/Report.pdf").is_ok());
}

#[test]
fn attachment_storage_streams_files_handles_collisions_and_reuses_ids() {
    let source = TestWorkspace::new("embedded-attachment-source");
    let workspace = TestWorkspace::new("embedded-attachment-target");
    let bytes = (0..ATTACHMENT_COPY_BUFFER_BYTES * 3 + 37)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let source_path = source.root.join("Quarterly report.pdf");
    fs::write(&source_path, &bytes).expect("source attachment should be written");
    fs::create_dir(workspace.root.join("Projects")).expect("note folder should be created");
    fs::write(workspace.root.join("Projects/Plan.md"), "# Plan").expect("note should be written");
    write_workspace_state(&workspace.root, &WorkspaceState::default())
        .expect("workspace state should be written");

    let note_folder_settings = AttachmentEmbedSettings {
        location: ImageEmbedLocation::NoteFolder,
        folder_path: "ignored".to_owned(),
    };
    let first = embed_workspace_attachment(
        &workspace.root,
        "Projects/Plan.md",
        note_folder_settings.clone(),
        &source_path,
        None,
        revision_for_root(&workspace.root).expect("revision should be available"),
    )
    .expect("streamed attachment should be embedded");
    let second = embed_workspace_attachment(
        &workspace.root,
        "Projects/Plan.md",
        note_folder_settings,
        &source_path,
        None,
        first.revision,
    )
    .expect("colliding attachment should receive a portable unique name");

    assert_eq!(
        first.attachment.relative_path,
        "Projects/Quarterly report.pdf"
    );
    assert_eq!(
        second.attachment.relative_path,
        "Projects/Quarterly report 2.pdf"
    );
    assert_eq!(first.attachment.media_type, "application/pdf");
    assert_eq!(first.attachment.byte_length, bytes.len() as u64);
    assert_eq!(
        fs::read(workspace.root.join(&first.attachment.relative_path)).unwrap(),
        bytes,
    );
    assert_eq!(
        fingerprint_attachment_file(&source_path).unwrap(),
        fingerprint_bytes(&bytes),
    );

    let root_attachment = embed_workspace_attachment(
        &workspace.root,
        "Projects/Plan.md",
        AttachmentEmbedSettings::default(),
        &source_path,
        None,
        second.revision,
    )
    .expect("vault-root attachment should be embedded");
    assert_eq!(
        root_attachment.attachment.relative_path,
        "Quarterly report.pdf"
    );

    let existing_path = workspace.root.join("Projects/Archive.zip");
    fs::write(&existing_path, b"existing vault archive")
        .expect("existing vault attachment should be written");
    let existing = embed_workspace_attachment(
        &workspace.root,
        "Projects/Plan.md",
        AttachmentEmbedSettings::default(),
        &existing_path,
        Some("Projects/Archive.zip"),
        revision_for_root(&workspace.root).expect("revision should include the new file"),
    )
    .expect("existing vault attachment should be registered without copying");
    let reused = embed_workspace_attachment(
        &workspace.root,
        "Projects/Plan.md",
        AttachmentEmbedSettings::default(),
        &existing_path,
        Some("Projects/Archive.zip"),
        existing.revision,
    )
    .expect("registered attachment should reuse its stable ID");
    assert_eq!(reused.attachment.id, existing.attachment.id);
    assert_eq!(reused.attachment.relative_path, "Projects/Archive.zip");
    assert!(!workspace.root.join("Archive.zip").exists());

    let case_collision_path = workspace.root.join("Projects/archive.ZIP");
    fs::write(&case_collision_path, b"different case-only attachment")
        .expect("case-only collision should be written on this test filesystem");
    let error = embed_workspace_attachment(
        &workspace.root,
        "Projects/Plan.md",
        AttachmentEmbedSettings::default(),
        &case_collision_path,
        Some("Projects/archive.ZIP"),
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect_err("case-only vault attachment collisions should be rejected");
    assert!(error.contains("differ only by letter case"));
    fs::remove_file(case_collision_path).expect("case-only fixture should be removed");

    let loaded = load_workspace(&workspace.root, &empty_vault("Attachments"))
        .expect("attachment inventory should reload");
    assert_eq!(loaded.vault.embedded_attachments.len(), 4);
    assert_eq!(loaded.vault.attachment_files.len(), 4);
    let loaded_first = loaded
        .vault
        .attachment_files
        .iter()
        .find(|file| file.asset_id.as_deref() == Some(first.attachment.id.as_str()))
        .expect("streamed attachment should keep its stable ID");
    assert_eq!(loaded_first.relative_path, first.attachment.relative_path);
    assert_eq!(loaded_first.byte_length, bytes.len() as u64);
    assert_eq!(
        loaded.vault.attachment_embed_settings,
        AttachmentEmbedSettings::default(),
    );
}

#[test]
fn attachment_storage_honors_shared_locations_and_empty_files() {
    let source = TestWorkspace::new("shared-attachment-source");
    let workspace = TestWorkspace::new("shared-attachment-target");
    let first_source = source.root.join("First.zip");
    let empty_source = source.root.join("Empty export");
    fs::write(&first_source, b"first archive").expect("first source should be written");
    File::create(&empty_source).expect("empty source should be created");
    fs::create_dir(workspace.root.join("test1")).expect("test1 should be created");
    fs::create_dir(workspace.root.join("test2")).expect("test2 should be created");
    fs::write(workspace.root.join("test1/doc1.md"), "# Doc 1").expect("doc1 should be written");
    fs::write(workspace.root.join("test2/doc2.md"), "# Doc 2").expect("doc2 should be written");
    write_workspace_state(&workspace.root, &WorkspaceState::default())
        .expect("workspace state should be written");
    let settings = AttachmentEmbedSettings {
        location: ImageEmbedLocation::SpecifiedFolder,
        folder_path: "Files".to_owned(),
    };

    let first = embed_workspace_attachment(
        &workspace.root,
        "test1/doc1.md",
        settings.clone(),
        &first_source,
        None,
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect("first shared attachment should be embedded");
    let second = embed_workspace_attachment(
        &workspace.root,
        "test2/doc2.md",
        settings.clone(),
        &empty_source,
        None,
        first.revision,
    )
    .expect("empty extensionless attachment should be embedded");

    assert_eq!(first.attachment.relative_path, "Files/First.zip");
    assert_eq!(second.attachment.relative_path, "Files/Empty export");
    assert_eq!(second.attachment.byte_length, 0);
    assert_eq!(second.attachment.media_type, "application/octet-stream");
    assert_eq!(
        fs::read(workspace.root.join(&second.attachment.relative_path)).unwrap(),
        b""
    );
    let loaded = load_workspace(&workspace.root, &empty_vault("Attachments"))
        .expect("shared attachments should reload");
    assert_eq!(loaded.vault.attachment_embed_settings, settings);
    assert!(loaded
        .vault
        .folders
        .iter()
        .any(|folder| folder.name == "test1"));
    assert!(loaded
        .vault
        .folders
        .iter()
        .any(|folder| folder.name == "test2"));
}

#[test]
fn reorganizing_an_attachment_moves_it_and_updates_references() {
    let source = TestWorkspace::new("reorganized-attachment-source");
    let workspace = TestWorkspace::new("reorganized-attachment-target");
    let bytes = b"portable report contents";
    let source_path = source.root.join("Quarterly report.pdf");
    fs::write(&source_path, bytes).expect("source attachment should be written");
    fs::create_dir(workspace.root.join("Files")).expect("file folder should be created");
    fs::create_dir(workspace.root.join("Archive")).expect("archive folder should be created");
    fs::write(workspace.root.join("Note.md"), "# Note").expect("note should be written");
    let mut state = WorkspaceState::default();
    state
        .note_paths
        .insert("note-1".to_owned(), "Note.md".to_owned());
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");
    let embedded = embed_workspace_attachment(
        &workspace.root,
        "Note.md",
        AttachmentEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolder,
            folder_path: "Files".to_owned(),
        },
        &source_path,
        None,
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect("attachment should be embedded");
    let original = format!(
            "[Tracked](Files/Quarterly%20report.pdf#oah-asset={})\n[Path only](Files/Quarterly%20report.pdf)",
            embedded.attachment.id,
        );
    fs::write(workspace.root.join("Note.md"), &original)
        .expect("attachment references should be written");
    let updated = format!(
        "[Tracked](Archive/Report.pdf#oah-asset={})\n[Path only](Archive/Report.pdf#oah-asset={})",
        embedded.attachment.id, embedded.attachment.id,
    );

    let moved = relocate_workspace_attachment(
        &workspace.root,
        "Files/Quarterly report.pdf",
        "Archive/Report.pdf",
        &embedded.attachment.id,
        &[WorkspaceImageNoteUpdate {
            note_id: "note-1".to_owned(),
            relative_path: "Note.md".to_owned(),
            expected_content: original,
            content: updated.clone(),
        }],
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect("attachment should move and be renamed");

    assert!(!workspace.root.join("Files/Quarterly report.pdf").exists());
    assert_eq!(
        fs::read(workspace.root.join("Archive/Report.pdf")).unwrap(),
        bytes
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join("Note.md")).unwrap(),
        updated
    );
    assert_eq!(moved.attachment.relative_path, "Archive/Report.pdf");
    assert_eq!(moved.attachment.byte_length, bytes.len() as u64);
    let (state, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    assert_eq!(
        state.unwrap().assets[&embedded.attachment.id].relative_path,
        "Archive/Report.pdf",
    );
}

#[test]
fn former_mirrored_attachments_can_be_reorganized_after_migration() {
    let workspace = TestWorkspace::new("former-mirrored-attachment");
    let bytes = b"former mirrored attachment";
    fs::create_dir_all(workspace.root.join("Files/Notes"))
        .expect("legacy attachment folder should be created");
    fs::create_dir(workspace.root.join("Elsewhere"))
        .expect("ordinary destination should be created");
    fs::write(workspace.root.join("Files/Notes/Report.pdf"), bytes)
        .expect("legacy attachment should be written");
    let mut state = WorkspaceState::default();
    state.attachment_embed_settings = AttachmentEmbedSettings {
        location: ImageEmbedLocation::SpecifiedFolder,
        folder_path: "Files".to_owned(),
    };
    state.assets.insert(
        "attachment-managed".to_owned(),
        StoredVaultAsset {
            kind: VaultAssetKind::Attachment,
            relative_path: "Files/Notes/Report.pdf".to_owned(),
            media_type: "application/pdf".to_owned(),
            fingerprint: fingerprint_bytes(bytes),
            modified_nanos: file_modified_nanos_for_path(
                &workspace.root.join("Files/Notes/Report.pdf"),
            )
            .unwrap(),
        },
    );
    write_legacy_mirrored_workspace_state(&workspace.root, &state);
    let loaded = load_workspace(&workspace.root, &empty_vault("Former mirror"))
        .expect("legacy workspace should migrate");

    let moved = relocate_workspace_attachment(
        &workspace.root,
        "Files/Notes/Report.pdf",
        "Elsewhere/Report.pdf",
        "attachment-managed",
        &[],
        loaded.revision,
    )
    .expect("a former mirrored attachment should move normally");
    assert_eq!(moved.attachment.relative_path, "Elsewhere/Report.pdf");
    assert_eq!(
        fs::read(workspace.root.join("Elsewhere/Report.pdf")).unwrap(),
        bytes
    );
    assert!(!workspace.root.join("Files/Notes/Report.pdf").exists());
}

#[test]
fn attachment_reconciliation_recovers_external_moves_by_stable_id() {
    let source = TestWorkspace::new("moved-attachment-source");
    let workspace = TestWorkspace::new("moved-attachment-target");
    let source_path = source.root.join("Archive.zip");
    fs::write(&source_path, b"unique archive bytes").expect("source archive should be written");
    fs::write(workspace.root.join("Note.md"), "# Note").expect("note should be written");
    write_workspace_state(&workspace.root, &WorkspaceState::default())
        .expect("workspace state should be written");
    let embedded = embed_workspace_attachment(
        &workspace.root,
        "Note.md",
        AttachmentEmbedSettings::default(),
        &source_path,
        None,
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect("archive should be embedded");
    fs::create_dir(workspace.root.join("Moved")).expect("external destination should be created");
    fs::rename(
        workspace.root.join("Archive.zip"),
        workspace.root.join("Moved/Renamed.zip"),
    )
    .expect("archive should move outside the app");

    let loaded = load_workspace(&workspace.root, &empty_vault("Attachments"))
        .expect("workspace should recover the moved archive");
    let recovered = loaded
        .vault
        .embedded_attachments
        .iter()
        .find(|attachment| attachment.id == embedded.attachment.id)
        .expect("stable attachment should remain indexed");
    assert_eq!(recovered.relative_path, "Moved/Renamed.zip");
    assert_eq!(
        loaded
            .vault
            .attachment_files
            .iter()
            .find(|attachment| attachment.asset_id.as_deref()
                == Some(embedded.attachment.id.as_str()))
            .expect("moved file should retain its stable ID")
            .relative_path,
        "Moved/Renamed.zip",
    );
    let (_, resolved) = resolve_attachment_action_source(
        &workspace.root,
        "Archive.zip",
        Some(&embedded.attachment.id),
    )
    .expect("stable action resolution should use the recovered path");
    assert_eq!(resolved, workspace.root.join("Moved/Renamed.zip"));
}

#[test]
fn attachment_actions_use_portable_paths_only_when_stable_metadata_is_absent() {
    let workspace = TestWorkspace::new("portable-attachment-action");
    fs::create_dir(workspace.root.join("Files")).expect("attachment folder should be created");
    fs::write(
        workspace.root.join("Files/Report#1.pdf"),
        b"portable report",
    )
    .expect("portable attachment should be written");

    let (relative_path, source) = resolve_attachment_action_source(
        &workspace.root,
        "Files/Report#1.pdf",
        Some("attachment-exported"),
    )
    .expect("an exported stable ID should fall back to its portable path");
    assert_eq!(relative_path, "Files/Report#1.pdf");
    assert_eq!(source, workspace.root.join("Files/Report#1.pdf"));

    write_workspace_state(&workspace.root, &WorkspaceState::default())
        .expect("empty workspace state should be written");
    let (_, source_with_empty_state) = resolve_attachment_action_source(
        &workspace.root,
        "Files/Report#1.pdf",
        Some("attachment-exported"),
    )
    .expect("an empty stable index should also use the portable path");
    assert_eq!(
        source_with_empty_state,
        workspace.root.join("Files/Report#1.pdf"),
    );

    let invalid_id_error = resolve_attachment_action_source(
        &workspace.root,
        "Files/Report#1.pdf",
        Some("attachment/invalid"),
    )
    .expect_err("an invalid stable ID should not fall back to the portable path");
    assert!(invalid_id_error.contains("invalid stable ID"));

    let mut state = WorkspaceState::default();
    state.assets.insert(
        "attachment-exported".to_owned(),
        StoredVaultAsset {
            kind: VaultAssetKind::Image,
            relative_path: "Image.png".to_owned(),
            media_type: "image/png".to_owned(),
            fingerprint: fingerprint_bytes(b"different asset kind"),
            modified_nanos: 0,
        },
    );
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");
    let wrong_kind_error = resolve_attachment_action_source(
        &workspace.root,
        "Files/Report#1.pdf",
        Some("attachment-exported"),
    )
    .expect_err("a wrong-kind stable record should remain authoritative");
    assert!(wrong_kind_error.contains("different file type"));

    fs::write(workspace_state_path(&workspace.root), b"not json")
        .expect("unreadable workspace metadata should be written");
    let unreadable_state_error = resolve_attachment_action_source(
        &workspace.root,
        "Files/Report#1.pdf",
        Some("attachment-exported"),
    )
    .expect_err("unreadable metadata should not permit a portable fallback");
    assert!(unreadable_state_error.contains("metadata is unreadable or newer"));
}

#[test]
fn vault_item_locations_are_canonical_strict_and_kind_safe() {
    let workspace = TestWorkspace::new("vault-item-location");
    let nested = "Deep Folder/Ångström";
    fs::create_dir_all(workspace.root.join(nested)).expect("nested vault folder should be created");
    fs::write(workspace.root.join(format!("{nested}/Plan.md")), "# Plan")
        .expect("nested note should be written");
    fs::write(
        workspace.root.join(format!("{nested}/Diagram.png")),
        b"image bytes",
    )
    .expect("nested image should be written");
    fs::write(
        workspace.root.join(format!("{nested}/Report.pdf")),
        b"tracked report",
    )
    .expect("tracked attachment should be written");
    fs::write(workspace.root.join("Report.pdf"), b"different root report")
        .expect("duplicate root attachment should be written");
    let mut state = WorkspaceState::default();
    state.assets.insert(
        "image-location".to_owned(),
        StoredVaultAsset {
            kind: VaultAssetKind::Image,
            relative_path: format!("{nested}/Diagram.png"),
            media_type: "image/png".to_owned(),
            fingerprint: fingerprint_bytes(b"image bytes"),
            modified_nanos: 0,
        },
    );
    state.assets.insert(
        "attachment-location".to_owned(),
        StoredVaultAsset {
            kind: VaultAssetKind::Attachment,
            relative_path: format!("{nested}/Report.pdf"),
            media_type: "application/pdf".to_owned(),
            fingerprint: fingerprint_bytes(b"tracked report"),
            modified_nanos: 0,
        },
    );
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");

    let (note_relative, note_path) = locate_workspace_vault_item(
        &workspace.root,
        WorkspaceVaultItemKind::Note,
        &format!("{nested}/Plan.md"),
        None,
    )
    .expect("the nested note should resolve");
    assert_eq!(note_relative, format!("{nested}/Plan.md"));
    assert_eq!(
        note_path,
        workspace
            .root
            .join(format!("{nested}/Plan.md"))
            .canonicalize()
            .unwrap(),
    );
    let (folder_relative, folder_path) = locate_workspace_vault_item(
        &workspace.root,
        WorkspaceVaultItemKind::Folder,
        nested,
        None,
    )
    .expect("the nested folder should resolve");
    assert_eq!(folder_relative, nested);
    assert_eq!(
        folder_path,
        workspace.root.join(nested).canonicalize().unwrap()
    );

    let (image_relative, _) = locate_workspace_vault_item(
        &workspace.root,
        WorkspaceVaultItemKind::Image,
        "Old/Diagram.png",
        Some("image-location"),
    )
    .expect("the stable image record should override its old path");
    assert_eq!(image_relative, format!("{nested}/Diagram.png"));
    let (attachment_relative, attachment_path) = locate_workspace_vault_item(
        &workspace.root,
        WorkspaceVaultItemKind::Attachment,
        "Report.pdf",
        Some("attachment-location"),
    )
    .expect("the stable attachment record should win over a duplicate name");
    assert_eq!(attachment_relative, format!("{nested}/Report.pdf"));
    assert_eq!(
        fs::read(attachment_path).unwrap(),
        b"tracked report",
        "the duplicate root attachment must not be selected",
    );
    let (portable_relative, _) = locate_workspace_vault_item(
        &workspace.root,
        WorkspaceVaultItemKind::Attachment,
        "Report.pdf",
        None,
    )
    .expect("an untracked root attachment should resolve by its exact path");
    assert_eq!(portable_relative, "Report.pdf");

    let stale_error = locate_workspace_vault_item(
        &workspace.root,
        WorkspaceVaultItemKind::Attachment,
        "Report.pdf",
        Some("attachment-stale"),
    )
    .expect_err("a stale stable ID must not fall back to the duplicate root path");
    assert!(stale_error.contains("no longer has a stable record"));
    let wrong_kind_error = locate_workspace_vault_item(
        &workspace.root,
        WorkspaceVaultItemKind::Attachment,
        "Report.pdf",
        Some("image-location"),
    )
    .expect_err("an image stable ID must not resolve as an attachment");
    assert!(wrong_kind_error.contains("different vault item type"));
    let platform_path_error = locate_workspace_vault_item(
        &workspace.root,
        WorkspaceVaultItemKind::Note,
        r"C:\Users\Person\Plan.md",
        None,
    )
    .expect_err("a platform path must not be accepted as a vault-relative path");
    assert!(platform_path_error.contains("relative to the vault"));

    fs::remove_file(workspace.root.join(format!("{nested}/Report.pdf")))
        .expect("tracked attachment should be removed externally");
    let deleted_error = locate_workspace_vault_item(
        &workspace.root,
        WorkspaceVaultItemKind::Attachment,
        "Report.pdf",
        Some("attachment-location"),
    )
    .expect_err("an externally deleted tracked attachment must not fall back");
    assert!(deleted_error.contains("Could not inspect the vault item"));

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            workspace.root.join("Report.pdf"),
            workspace.root.join("Linked.pdf"),
        )
        .expect("attachment symlink should be created");
        let symlink_error = locate_workspace_vault_item(
            &workspace.root,
            WorkspaceVaultItemKind::Attachment,
            "Linked.pdf",
            None,
        )
        .expect_err("a vault item symlink must not be revealed");
        assert!(symlink_error.contains("symbolic link"));
    }
}

#[test]
fn attachment_actions_classify_risky_files_and_keep_archive_copies_outside_the_vault() {
    let workspace = TestWorkspace::new("attachment-action-vault");
    let outside = TestWorkspace::new("attachment-action-outside");
    assert!(is_archive_attachment_path(Path::new("Backup.ZIP")));
    assert!(is_executable_attachment_path(Path::new("Installer.MSI")));
    for path in [
        "Script.command",
        "Script.vbs",
        "Page.hta",
        "Screen.scr",
        "Control.cpl",
        "Package.msix",
    ] {
        assert!(
            is_executable_attachment_path(Path::new(path)),
            "{path} should be blocked"
        );
    }
    assert!(!is_executable_attachment_path(Path::new("Report.pdf")));

    let extensionless_script = workspace.root.join("extensionless-script");
    fs::write(&extensionless_script, b"#!/bin/sh\necho unsafe\n")
        .expect("extensionless script should be written");
    assert!(attachment_opening_is_disabled(&extensionless_script).unwrap());

    let disguised_binary = workspace.root.join("disguised-document.txt");
    fs::write(&disguised_binary, b"\x7fELF\x02\x01\x01\x00")
        .expect("disguised executable should be written");
    assert!(attachment_opening_is_disabled(&disguised_binary).unwrap());

    let extensionless_text = workspace.root.join("extensionless-text");
    fs::write(&extensionless_text, b"ordinary text").expect("extensionless text should be written");
    assert!(!attachment_opening_is_disabled(&extensionless_text).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&extensionless_text).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&extensionless_text, permissions).unwrap();
        assert!(attachment_opening_is_disabled(&extensionless_text).unwrap());
    }

    let inside = workspace.root.join("Copy.zip");
    let error = validate_external_attachment_copy_target(&workspace.root, &inside)
        .expect_err("archive copies inside the vault should be rejected");
    assert!(error.contains("outside the active vault"));

    let outside_target = outside.root.join("Copy.zip");
    assert_eq!(
        validate_external_attachment_copy_target(&workspace.root, &outside_target)
            .expect("an unused external path should be accepted"),
        outside_target,
    );
    fs::write(&outside_target, b"existing").expect("existing target should be written");
    assert!(
        validate_external_attachment_copy_target(&workspace.root, &outside_target,)
            .expect_err("archive copies should not overwrite existing files")
            .contains("already exists")
    );
}

#[test]
fn attachment_storage_rejects_stale_unsafe_and_oversized_sources() {
    let source = TestWorkspace::new("attachment-validation-source");
    let workspace = TestWorkspace::new("attachment-validation-target");
    fs::write(workspace.root.join("Note.md"), "# Note").expect("note should be written");
    write_workspace_state(&workspace.root, &WorkspaceState::default())
        .expect("workspace state should be written");
    let source_path = source.root.join("Document.pdf");
    fs::write(&source_path, b"document").expect("source should be written");
    fs::write(source.root.join("Note.md"), "# Not an attachment")
        .expect("Markdown source should be written");
    fs::write(source.root.join("Image.png"), b"not relevant")
        .expect("image source should be written");

    assert!(
        validate_attachment_source_file(source.root.join("Note.md").to_str().unwrap(),).is_err()
    );
    assert!(
        validate_attachment_source_file(source.root.join("Image.png").to_str().unwrap(),).is_err()
    );
    assert!(validate_attachment_source_file(source.root.to_str().unwrap()).is_err());
    assert!(validate_attachment_source_file("relative.pdf").is_err());

    let oversized = source.root.join("Oversized.zip");
    File::create(&oversized)
        .expect("oversized fixture should be created")
        .set_len(MAX_ATTACHMENT_BYTES + 1)
        .expect("sparse oversized fixture should be sized");
    let oversized_error = validate_attachment_source_file(oversized.to_str().unwrap())
        .expect_err("oversized attachment should be rejected before reading");
    assert!(oversized_error.contains("larger than"));

    let stale_revision = revision_for_root(&workspace.root).unwrap();
    fs::write(workspace.root.join("External.zip"), b"external change")
        .expect("external attachment should be written");
    assert_ne!(revision_for_root(&workspace.root).unwrap(), stale_revision);
    let error = embed_workspace_attachment(
        &workspace.root,
        "Note.md",
        AttachmentEmbedSettings::default(),
        &source_path,
        None,
        stale_revision,
    )
    .expect_err("stale revision should reject the attachment copy");
    assert!(error.contains("vault changed"));
    assert!(!workspace.root.join("Document.pdf").exists());

    let loaded = load_workspace(&workspace.root, &empty_vault("Attachments"))
        .expect("untracked attachment should be inventoried");
    assert_eq!(loaded.vault.attachment_files.len(), 1);
    assert_eq!(
        loaded.vault.attachment_files[0].relative_path,
        "External.zip"
    );
    assert_eq!(loaded.vault.attachment_files[0].asset_id, None);
}

#[cfg(unix)]
#[test]
fn attachment_storage_refuses_source_and_destination_symlinks() {
    use std::os::unix::fs::symlink;

    let source = TestWorkspace::new("attachment-symlink-source");
    let workspace = TestWorkspace::new("attachment-symlink-target");
    let outside = TestWorkspace::new("attachment-symlink-outside");
    let source_path = source.root.join("Archive.zip");
    fs::write(&source_path, b"archive").expect("source should be written");
    symlink(&source_path, source.root.join("Archive link.zip"))
        .expect("source symlink should be created");
    symlink(&outside.root, workspace.root.join("Linked"))
        .expect("destination symlink should be created");
    fs::write(workspace.root.join("Note.md"), "# Note").expect("note should be written");
    write_workspace_state(&workspace.root, &WorkspaceState::default())
        .expect("workspace state should be written");

    assert!(validate_attachment_source_file(
        source.root.join("Archive link.zip").to_str().unwrap(),
    )
    .is_err());
    let error = embed_workspace_attachment(
        &workspace.root,
        "Note.md",
        AttachmentEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolder,
            folder_path: "Linked".to_owned(),
        },
        &source_path,
        None,
        revision_for_root(&workspace.root).unwrap(),
    )
    .expect_err("destination symlink should be rejected");
    assert!(error.contains("symbolic link"));
    assert!(!outside.root.join("Archive.zip").exists());
}
