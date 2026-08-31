#[test]
fn archives_and_reloads_a_note_without_scanning_the_snapshot() {
    let workspace = TestWorkspace::new("archive-round-trip");
    let note = test_note("A recovered thought\n");
    write_saved_note(&workspace, &note);
    let revision =
        revision_for_root(&workspace.root).expect("initial revision should be calculated");

    let (saved, deleted_note) = save_workspace_files_with_archive(
        &workspace.root,
        &empty_vault("Test vault"),
        revision,
        Some(PendingNoteArchive {
            note: note.clone(),
            original_folder_path: String::new(),
            editor_position: Some(editor_position(4)),
        }),
    )
    .expect("note should be archived");
    let deleted_note = deleted_note.expect("archive result should contain the note");

    assert!(!workspace.root.join(&note.relative_path).exists());
    assert_eq!(
        deleted_note.expires_at - deleted_note.deleted_at,
        RECENTLY_DELETED_RETENTION_MILLIS,
    );
    let snapshot_path = recently_deleted_snapshot_path(&workspace.root, &deleted_note.id)
        .expect("snapshot path should be safe");
    assert_eq!(
        snapshot_path.extension().and_then(|value| value.to_str()),
        Some("snapshot"),
    );
    assert!(snapshot_path.is_file());

    let (scanned_notes, _, _, _) =
        scan_workspace_files(&workspace.root, &mut WarningCollector::default())
            .expect("workspace should scan");
    assert!(scanned_notes.is_empty());

    let loaded = load_workspace(&workspace.root, &empty_vault("Test vault"))
        .expect("workspace should reopen");
    assert!(loaded.vault.notes.is_empty());
    assert_eq!(loaded.recently_deleted_notes, vec![deleted_note.clone()]);

    save_workspace_files(&workspace.root, &loaded.vault, loaded.revision)
        .expect("ordinary saves should preserve recovery snapshots");
    let reopened = load_workspace(&workspace.root, &empty_vault("Test vault"))
        .expect("workspace should reopen after an ordinary save");
    assert_eq!(reopened.recently_deleted_notes, vec![deleted_note]);
    assert!(saved.revision > 0);
}

#[test]
fn restored_snapshot_keeps_updated_time_after_workspace_reload() {
    let workspace = TestWorkspace::new("restore-round-trip");
    let mut note = test_note("Remember this\n");
    note.relative_path = "First note.markdown".to_owned();
    write_saved_note(&workspace, &note);
    let revision =
        revision_for_root(&workspace.root).expect("initial revision should be calculated");
    let (archived, deleted_note) = save_workspace_files_with_archive(
        &workspace.root,
        &empty_vault("Test vault"),
        revision,
        Some(PendingNoteArchive {
            note: note.clone(),
            original_folder_path: String::new(),
            editor_position: Some(editor_position(7)),
        }),
    )
    .expect("note should be archived");
    let deleted_note = deleted_note.expect("archive should return the deleted note");
    let (state, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    let state = state.expect("archived state should load");
    let mut vault = empty_vault("Test vault");
    let (restored_note, preferred_relative_path) =
        build_restored_note(&workspace.root, &vault, &state, &deleted_note)
            .expect("restore destination should be selected");
    vault.notes.push(restored_note.clone());
    vault.active_note_id = Some(restored_note.id.clone());
    vault.recent_note_ids.push(restored_note.id.clone());

    let (_, restored) = save_workspace_files_with_restore(
        &workspace.root,
        &vault,
        archived.revision,
        PendingNoteRestore {
            deleted_note_id: deleted_note.id.clone(),
            restored_note: restored_note.clone(),
            preferred_relative_path,
        },
    )
    .expect("snapshot should restore");

    assert_eq!(restored.restored_note, restored_note);
    assert_eq!(restored.restored_note.updated_at, note.updated_at);
    assert_eq!(restored.editor_position, Some(editor_position(7)));
    assert_eq!(restored.restored_note.relative_path, "First note.markdown");
    assert_eq!(
        fs::read_to_string(workspace.root.join("First note.markdown"))
            .expect("restored note should be readable"),
        note.content,
    );
    assert!(
        !recently_deleted_snapshot_path(&workspace.root, &deleted_note.id)
            .expect("snapshot path should be safe")
            .exists()
    );
    let (state, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    assert!(state
        .expect("restored state should load")
        .recently_deleted_notes
        .is_empty());
    let reopened = load_workspace(&workspace.root, &empty_vault("Test vault"))
        .expect("restored workspace should reopen");
    let reopened_note = reopened
        .vault
        .notes
        .iter()
        .find(|candidate| candidate.id == restored.restored_note.id)
        .expect("restored note should reopen");
    assert_eq!(reopened_note.updated_at, note.updated_at);
}

#[test]
fn expired_snapshot_cannot_be_restored() {
    let workspace = TestWorkspace::new("restore-expired");
    let note = test_note("Too late\n");
    write_saved_note(&workspace, &note);
    let revision =
        revision_for_root(&workspace.root).expect("initial revision should be calculated");
    let (_, deleted_note) = save_workspace_files_with_archive(
        &workspace.root,
        &empty_vault("Test vault"),
        revision,
        Some(PendingNoteArchive {
            note,
            original_folder_path: String::new(),
            editor_position: None,
        }),
    )
    .expect("note should be archived");
    let deleted_note = deleted_note.expect("archive should return the deleted note");
    let expired_revision = mark_recovery_expired(&workspace, &deleted_note.id);

    let error = read_recovery_for_restore(&workspace.root, &deleted_note.id, expired_revision)
        .expect_err("expired snapshot should not be restorable");

    assert_eq!(
        error,
        "That deleted note has expired and can no longer be restored.",
    );
    assert!(
        recently_deleted_snapshot_path(&workspace.root, &deleted_note.id)
            .expect("snapshot path should be safe")
            .exists()
    );
}

#[test]
fn restoring_never_overwrites_an_occupied_original_path() {
    let workspace = TestWorkspace::new("restore-path-conflict");
    let note = test_note("Recoverable\n");
    write_saved_note(&workspace, &note);
    let revision =
        revision_for_root(&workspace.root).expect("initial revision should be calculated");
    let (archived, deleted_note) = save_workspace_files_with_archive(
        &workspace.root,
        &empty_vault("Test vault"),
        revision,
        Some(PendingNoteArchive {
            note: note.clone(),
            original_folder_path: String::new(),
            editor_position: None,
        }),
    )
    .expect("note should be archived");
    let deleted_note = deleted_note.expect("archive should return the deleted note");
    fs::write(workspace.root.join("First note.md"), "External content\n")
        .expect("conflicting note should be written");
    let conflict_revision =
        revision_for_root(&workspace.root).expect("conflict revision should be calculated");
    assert_ne!(conflict_revision, archived.revision);
    let (state, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    let state = state.expect("archived state should load");
    let mut vault = empty_vault("Test vault");
    let (restored_note, preferred_relative_path) =
        build_restored_note(&workspace.root, &vault, &state, &deleted_note)
            .expect("a conflict-safe destination should be selected");
    assert_eq!(restored_note.relative_path, "First note 2.md");
    vault.notes.push(restored_note.clone());

    save_workspace_files_with_restore(
        &workspace.root,
        &vault,
        conflict_revision,
        PendingNoteRestore {
            deleted_note_id: deleted_note.id,
            restored_note,
            preferred_relative_path,
        },
    )
    .expect("snapshot should restore beside the conflict");

    assert_eq!(
        fs::read_to_string(workspace.root.join("First note.md"))
            .expect("conflicting file should remain"),
        "External content\n",
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join("First note 2.md"))
            .expect("restored file should be readable"),
        note.content,
    );
}

#[test]
fn manual_and_expiry_cleanup_remove_only_verified_snapshots() {
    let workspace = TestWorkspace::new("recovery-removal");
    let first_note = test_note("First recovery\n");
    write_saved_note(&workspace, &first_note);
    let revision =
        revision_for_root(&workspace.root).expect("initial revision should be calculated");
    let (first_archive, first_deleted) = save_workspace_files_with_archive(
        &workspace.root,
        &empty_vault("Test vault"),
        revision,
        Some(PendingNoteArchive {
            note: first_note,
            original_folder_path: String::new(),
            editor_position: None,
        }),
    )
    .expect("first note should be archived");
    let first_deleted = first_deleted.expect("first archive should return metadata");

    let mut second_note = test_note("Second recovery\n");
    second_note.id = "note-2".to_owned();
    second_note.title = "Second note".to_owned();
    second_note.relative_path = "Second note.md".to_owned();
    let mut second_vault = empty_vault("Test vault");
    second_vault.notes.push(second_note.clone());
    let saved = save_workspace_files(&workspace.root, &second_vault, first_archive.revision)
        .expect("second note should be saved");
    assert_eq!(
        saved.note_paths.get(&second_note.id).map(String::as_str),
        Some("Second note.md"),
    );
    let (second_archive, second_deleted) = save_workspace_files_with_archive(
        &workspace.root,
        &empty_vault("Test vault"),
        saved.revision,
        Some(PendingNoteArchive {
            note: second_note,
            original_folder_path: String::new(),
            editor_position: None,
        }),
    )
    .expect("second note should be archived");
    let second_deleted = second_deleted.expect("second archive should return metadata");

    let removed = remove_recently_deleted_notes(
        &workspace.root,
        vec![first_deleted.id.clone()],
        second_archive.revision,
        false,
    )
    .expect("one selected snapshot should be removed");
    assert_eq!(removed.removed_ids, vec![first_deleted.id.clone()]);
    assert!(
        !recently_deleted_snapshot_path(&workspace.root, &first_deleted.id)
            .expect("first snapshot path should be safe")
            .exists()
    );
    assert!(
        recently_deleted_snapshot_path(&workspace.root, &second_deleted.id)
            .expect("second snapshot path should be safe")
            .exists()
    );

    let path = recently_deleted_snapshot_path(&workspace.root, &second_deleted.id)
        .expect("second snapshot path should be safe");
    let expired_revision = mark_recovery_expired(&workspace, &second_deleted.id);

    let pruned = remove_recently_deleted_notes(&workspace.root, Vec::new(), expired_revision, true)
        .expect("expired snapshot should be pruned");
    assert_eq!(pruned.removed_ids, vec![second_deleted.id.clone()]);
    assert!(!path.exists());
}

#[test]
fn loading_finishes_expiry_after_snapshot_cleanup_preceded_state_cleanup() {
    let workspace = TestWorkspace::new("interrupted-expiry-cleanup");
    let note = test_note("Expired recovery\n");
    write_saved_note(&workspace, &note);
    let revision =
        revision_for_root(&workspace.root).expect("initial revision should be calculated");
    let (_, deleted_note) = save_workspace_files_with_archive(
        &workspace.root,
        &empty_vault("Test vault"),
        revision,
        Some(PendingNoteArchive {
            note,
            original_folder_path: String::new(),
            editor_position: None,
        }),
    )
    .expect("note should be archived");
    let deleted_note = deleted_note.expect("archive should return metadata");
    mark_recovery_expired(&workspace, &deleted_note.id);
    let path = recently_deleted_snapshot_path(&workspace.root, &deleted_note.id)
        .expect("snapshot path should be safe");
    remove_file_durable(&path).expect("snapshot cleanup should be interrupted before state");

    let loaded = load_workspace(&workspace.root, &empty_vault("Test vault"))
        .expect("workspace should finish the interrupted cleanup");
    let (state, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());

    assert!(loaded.recently_deleted_notes.is_empty());
    assert!(state
        .expect("cleaned state should load")
        .recently_deleted_notes
        .is_empty());
}

#[cfg(unix)]
#[test]
fn failed_expiry_cleanup_remains_available_for_retry_but_not_restore() {
    use std::os::unix::fs::PermissionsExt;

    let workspace = TestWorkspace::new("recoverable-expiry-failure");
    let note = test_note("Still recoverable\n");
    write_saved_note(&workspace, &note);
    let revision =
        revision_for_root(&workspace.root).expect("initial revision should be calculated");
    let (_, deleted_note) = save_workspace_files_with_archive(
        &workspace.root,
        &empty_vault("Test vault"),
        revision,
        Some(PendingNoteArchive {
            note,
            original_folder_path: String::new(),
            editor_position: Some(editor_position(5)),
        }),
    )
    .expect("note should be archived");
    let deleted_note = deleted_note.expect("archive should return metadata");
    let expired_revision = mark_recovery_expired(&workspace, &deleted_note.id);
    let directory = inspect_recently_deleted_directory(&workspace.root)
        .expect("recovery directory should exist");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555))
        .expect("recovery directory should become read-only");

    let pruned = remove_recently_deleted_notes(&workspace.root, Vec::new(), expired_revision, true)
        .expect("failed physical cleanup should remain a successful prune check");
    let restore_error =
        read_recovery_for_restore(&workspace.root, &deleted_note.id, pruned.revision)
            .expect_err("an expired snapshot should not remain restorable");
    assert!(
        recently_deleted_snapshot_path(&workspace.root, &deleted_note.id)
            .expect("snapshot path should be safe")
            .exists()
    );
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))
        .expect("recovery directory permissions should be restored");
    let retried = remove_recently_deleted_notes(&workspace.root, Vec::new(), pruned.revision, true)
        .expect("expiry cleanup should succeed when retried");

    assert!(pruned.removed_ids.is_empty());
    assert!(pruned
        .warnings
        .iter()
        .any(|warning| warning.contains("remains recoverable")));
    assert_eq!(
        restore_error,
        "That deleted note has expired and can no longer be restored.",
    );
    assert_eq!(retried.removed_ids, vec![deleted_note.id.clone()]);
    assert!(
        !recently_deleted_snapshot_path(&workspace.root, &deleted_note.id)
            .expect("snapshot path should be safe")
            .exists()
    );
}

#[test]
fn post_commit_cleanup_warns_without_deleting_a_changed_snapshot() {
    let workspace = TestWorkspace::new("changed-recovery-cleanup");
    ensure_recently_deleted_directory(&workspace.root)
        .expect("recovery directory should be created");
    let id = "deleted-changed-cleanup";
    let path =
        recently_deleted_snapshot_path(&workspace.root, id).expect("snapshot path should be safe");
    fs::write(&path, b"expected recovery").expect("snapshot should be written");
    let expected = fingerprint_bytes(b"expected recovery");
    fs::write(&path, b"changed recovery").expect("snapshot should change");
    let mut warnings = WarningCollector::default();

    assert!(!remove_recovery_snapshot_if_matches(
        &workspace.root,
        id,
        &expected,
        &mut warnings,
    ));
    assert_eq!(
        fs::read(&path).expect("changed snapshot should remain"),
        b"changed recovery",
    );
    assert!(warnings
        .finish()
        .iter()
        .any(|warning| warning.contains("left untouched")));
}

#[test]
fn removed_snapshot_with_a_sync_error_is_not_treated_as_recoverable() {
    let workspace = TestWorkspace::new("recovery-sync-error");
    ensure_recently_deleted_directory(&workspace.root)
        .expect("recovery directory should be created");
    let id = "deleted-sync-error";
    let path =
        recently_deleted_snapshot_path(&workspace.root, id).expect("snapshot path should be safe");
    fs::write(&path, b"recovery").expect("snapshot should be written");
    fs::remove_file(&path).expect("snapshot should be unlinked");

    let result = classify_recovery_snapshot_removal_error(
        &path,
        id,
        io::Error::other("directory sync failed"),
    )
    .expect("an absent snapshot should count as removed");

    assert_eq!(
        result,
        RecoverySnapshotRemoval::RemovedWithoutDurability("directory sync failed".to_owned(),),
    );
}

#[test]
fn refuses_to_archive_stale_note_content() {
    let workspace = TestWorkspace::new("stale-archive");
    let saved_note = test_note("Saved content");
    write_saved_note(&workspace, &saved_note);
    let revision =
        revision_for_root(&workspace.root).expect("initial revision should be calculated");
    let mut stale_note = saved_note.clone();
    stale_note.content = "Unsaved content".to_owned();

    let error = save_workspace_files_with_archive(
        &workspace.root,
        &empty_vault("Test vault"),
        revision,
        Some(PendingNoteArchive {
            note: stale_note,
            original_folder_path: String::new(),
            editor_position: None,
        }),
    )
    .expect_err("stale note content should be rejected");

    assert!(error.contains("changed"));
    assert_eq!(
        fs::read_to_string(workspace.root.join(&saved_note.relative_path))
            .expect("live note should remain"),
        saved_note.content,
    );
}

#[test]
fn refuses_to_archive_a_note_changed_during_transaction_preparation() {
    let workspace = TestWorkspace::new("archive-preparation-race");
    let note = test_note("Original");
    let state = write_saved_note(&workspace, &note);
    let archive = prepare_test_archive(&workspace, note.clone(), &state);
    fs::write(workspace.root.join(&note.relative_path), "Replaced")
        .expect("the saved note should change");
    let replaced = BTreeSet::from([note.relative_path.clone()]);

    let error = prepare_transaction(
        &workspace.root,
        new_transaction_id(),
        &replaced,
        &[],
        std::slice::from_ref(&archive),
        Vec::new(),
        Vec::new(),
    )
    .expect_err("changed note content should not be archived");

    assert!(error.contains("changed"));
    assert_eq!(
        fs::read_to_string(workspace.root.join(&note.relative_path))
            .expect("the changed note should remain live"),
        "Replaced",
    );
}

#[cfg(unix)]
#[test]
fn refuses_to_follow_a_staged_recovery_symlink() {
    use std::os::unix::fs::symlink;

    let workspace = TestWorkspace::new("staged-recovery-symlink");
    let outside = TestWorkspace::new("staged-recovery-outside");
    let transaction_root = prepare_transaction_root(&workspace.root, &new_transaction_id())
        .expect("transaction should be created");
    let recovery_directory = transaction_root.join("recoveries");
    fs::create_dir(&recovery_directory).expect("recovery directory should be created");
    let outside_path = outside.root.join("outside.snapshot");
    let bytes = b"outside recovery data";
    fs::write(&outside_path, bytes).expect("outside data should be written");
    let id = "deleted-symlink";
    symlink(
        &outside_path,
        recovery_directory.join(format!("{id}.snapshot")),
    )
    .expect("staged snapshot symlink should be created");
    let target = TransactionRecoveryTarget {
        id: id.to_owned(),
        fingerprint: fingerprint_bytes(bytes),
    };

    let error = read_staged_recovery_snapshot(&transaction_root, &target)
        .expect_err("staged symlinks should be rejected");

    assert!(error.contains("regular file"));
}

#[test]
fn refuses_to_read_an_oversized_staged_recovery_snapshot() {
    let workspace = TestWorkspace::new("oversized-staged-recovery");
    let transaction_root = prepare_transaction_root(&workspace.root, &new_transaction_id())
        .expect("transaction should be created");
    let recovery_directory = transaction_root.join("recoveries");
    fs::create_dir(&recovery_directory).expect("recovery directory should be created");
    let id = "deleted-oversized";
    let path = recovery_directory.join(format!("{id}.snapshot"));
    File::create(&path)
        .and_then(|file| file.set_len(MAX_RECENTLY_DELETED_SNAPSHOT_BYTES + 1))
        .expect("oversized staged snapshot should be created");
    let target = TransactionRecoveryTarget {
        id: id.to_owned(),
        fingerprint: FileFingerprint {
            length: MAX_RECENTLY_DELETED_SNAPSHOT_BYTES + 1,
            hash: 0,
        },
    };

    let error = read_staged_recovery_snapshot(&transaction_root, &target)
        .expect_err("oversized staged snapshots should be rejected");

    assert!(error.contains("unexpectedly large"));
}

#[test]
fn rolling_back_an_archive_restores_the_note_and_removes_the_snapshot() {
    let workspace = TestWorkspace::new("archive-rollback");
    let note = test_note("Rollback content");
    let state = write_saved_note(&workspace, &note);
    let archive = prepare_test_archive(&workspace, note.clone(), &state);
    let replaced = BTreeSet::from([note.relative_path.clone()]);
    let (transaction_root, mut manifest) = prepare_transaction(
        &workspace.root,
        new_transaction_id(),
        &replaced,
        &[],
        std::slice::from_ref(&archive),
        Vec::new(),
        Vec::new(),
    )
    .expect("transaction should be prepared");
    manifest.phase = TransactionPhase::Applying;
    write_transaction_manifest(&transaction_root, &manifest).expect("manifest should be updated");
    apply_transaction(
        &workspace.root,
        &transaction_root,
        &manifest,
        &[],
        &mut WarningCollector::default(),
    )
    .expect("transaction should apply");
    assert!(!workspace.root.join(&note.relative_path).exists());

    let mut warnings = WarningCollector::default();
    assert!(rollback_transaction(
        &workspace.root,
        &transaction_root,
        &manifest,
        &mut warnings,
    ));

    assert_eq!(
        fs::read_to_string(workspace.root.join(&note.relative_path))
            .expect("live note should be restored"),
        note.content,
    );
    assert!(
        !recently_deleted_snapshot_path(&workspace.root, &archive.deleted_note.id,)
            .expect("snapshot path should be safe")
            .exists()
    );
    assert!(warnings.finish().is_empty());
}

#[test]
fn committed_archive_recovery_retains_the_snapshot() {
    let workspace = TestWorkspace::new("archive-commit-recovery");
    let note = test_note("Committed content");
    let mut state = write_saved_note(&workspace, &note);
    let archive = prepare_test_archive(&workspace, note.clone(), &state);
    let replaced = BTreeSet::from([note.relative_path.clone()]);
    let transaction_id = new_transaction_id();
    let (transaction_root, mut manifest) = prepare_transaction(
        &workspace.root,
        transaction_id.clone(),
        &replaced,
        &[],
        std::slice::from_ref(&archive),
        Vec::new(),
        Vec::new(),
    )
    .expect("transaction should be prepared");
    manifest.phase = TransactionPhase::Applying;
    write_transaction_manifest(&transaction_root, &manifest).expect("manifest should be updated");
    apply_transaction(
        &workspace.root,
        &transaction_root,
        &manifest,
        &[],
        &mut WarningCollector::default(),
    )
    .expect("transaction should apply");

    state.note_paths.clear();
    state.note_metadata.clear();
    state.active_note_id = None;
    state.recent_note_ids.clear();
    state.last_committed_transaction_id = Some(transaction_id);
    state.recently_deleted_notes.insert(
        archive.deleted_note.id.clone(),
        StoredRecentlyDeletedNote {
            deleted_at: archive.deleted_note.deleted_at,
            expires_at: archive.deleted_note.expires_at,
            fingerprint: archive.fingerprint.clone(),
        },
    );
    write_workspace_state(&workspace.root, &state).expect("committed state should be written");

    let mut warnings = WarningCollector::default();
    recover_workspace_transactions(&workspace.root, Some(&state), &mut warnings)
        .expect("committed transaction should recover");

    assert!(!workspace.root.join(&note.relative_path).exists());
    assert!(!transaction_root.exists());
    assert!(
        recently_deleted_snapshot_path(&workspace.root, &archive.deleted_note.id,)
            .expect("snapshot path should be safe")
            .is_file()
    );
    let loaded = load_recently_deleted_notes(
        &workspace.root,
        &state.recently_deleted_notes,
        &mut WarningCollector::default(),
    );
    assert_eq!(loaded, vec![archive.deleted_note]);
}

#[test]
fn stale_committed_transaction_does_not_recreate_a_removed_snapshot() {
    let workspace = TestWorkspace::new("stale-committed-archive");
    let state = WorkspaceState::default();
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");
    let id = "deleted-stale-transaction".to_owned();
    let bytes = b"stale recovery payload\n".to_vec();
    let fingerprint = fingerprint_bytes(&bytes);
    let transaction_id = new_transaction_id();
    let transaction_root = prepare_transaction_root(&workspace.root, &transaction_id)
        .expect("transaction should be created");
    let staged = transaction_recovery_snapshot_path(&transaction_root, &id)
        .expect("staged path should be safe");
    ensure_private_directory_tree(
        &transaction_root,
        staged.parent().expect("staged path should have a parent"),
    )
    .expect("staging directory should be created");
    atomic_write(&staged, &bytes).expect("staged snapshot should be written");
    let manifest = TransactionManifest {
        version: TRANSACTION_VERSION,
        id: transaction_id,
        phase: TransactionPhase::Committed,
        originals: Vec::new(),
        targets: Vec::new(),
        recovery_targets: vec![TransactionRecoveryTarget {
            id: "deleted-stale-transaction".to_owned(),
            fingerprint,
        }],
        folder_case_renames: Vec::new(),
        created_directories: Vec::new(),
    };
    write_transaction_manifest(&transaction_root, &manifest).expect("manifest should be written");

    recover_workspace_transactions(
        &workspace.root,
        Some(&state),
        &mut WarningCollector::default(),
    )
    .expect("stale committed transaction should be cleaned");

    assert!(!transaction_root.exists());
    assert!(
        !recently_deleted_snapshot_path(&workspace.root, "deleted-stale-transaction",)
            .expect("snapshot path should be safe")
            .exists()
    );
}
