#[test]
fn recognizes_virtual_folder_selections() {
    assert!(is_virtual_folder_selection("all"));
    assert!(is_virtual_folder_selection("favorites"));
    assert!(is_virtual_folder_selection("recent"));
    assert!(!is_virtual_folder_selection("folder-id"));
}

#[test]
fn external_file_upload_streams_ordered_chunks_and_cleans_staging() {
    let staging = TestWorkspace::new("external-file-upload-stream");
    let vault = TestWorkspace::new("external-file-upload-stream-vault");
    let upload = begin_external_file_upload(
        &staging.root,
        "Report.zip".to_owned(),
        6,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("upload should begin");
    let upload_directory = staging.root.join(&upload.id);

    assert_eq!(upload.chunk_bytes, EXTERNAL_FILE_UPLOAD_CHUNK_BYTES);
    assert_eq!(
        append_external_file_upload(&upload.id, 0, b"abc").expect("first chunk should append"),
        3,
    );
    assert_eq!(
        append_external_file_upload(&upload.id, 3, b"def").expect("second chunk should append"),
        6,
    );

    let staged = finish_external_file_upload(&upload.id, ExternalFileUploadKind::Attachment)
        .expect("complete upload should finish");
    let staged_path = staged.path.clone();
    assert_eq!(staged.file_name, "Report.zip");
    assert_eq!(staged.root, vault.root);
    assert_eq!(staged.note_relative_path, "Note.md");
    assert_eq!(fs::read(&staged_path).unwrap(), b"abcdef");
    drop(staged);

    assert!(!staged_path.exists());
    assert!(!upload_directory.exists());
}

#[test]
fn external_file_upload_invalid_chunks_cancel_and_remove_staging() {
    let staging = TestWorkspace::new("external-file-upload-invalid");
    let vault = TestWorkspace::new("external-file-upload-invalid-vault");
    let upload = begin_external_file_upload(
        &staging.root,
        "Report.pdf".to_owned(),
        4,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("upload should begin");
    let upload_directory = staging.root.join(&upload.id);
    let error = append_external_file_upload(&upload.id, 1, b"data")
        .expect_err("out-of-order chunk should fail");
    assert!(error.contains("out of order"));
    assert!(!upload_directory.exists());
    assert!(!cancel_external_file_upload(&upload.id).unwrap());

    let upload = begin_external_file_upload(
        &staging.root,
        "Report.pdf".to_owned(),
        4,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("second upload should begin");
    let upload_directory = staging.root.join(&upload.id);
    append_external_file_upload(&upload.id, 0, b"ab").expect("partial chunk should append");
    let error = finish_external_file_upload(&upload.id, ExternalFileUploadKind::Attachment)
        .expect_err("incomplete upload should fail");
    assert!(error.contains("did not finish"));
    assert!(!upload_directory.exists());

    let upload = begin_external_file_upload(
        &staging.root,
        "Report.pdf".to_owned(),
        4,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("third upload should begin");
    let upload_directory = staging.root.join(&upload.id);
    assert!(cancel_external_file_upload(&upload.id).unwrap());
    assert!(!upload_directory.exists());

    let upload = begin_external_file_upload(
        &staging.root,
        "Report.pdf".to_owned(),
        (EXTERNAL_FILE_UPLOAD_CHUNK_BYTES + 1) as u64,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("fourth upload should begin");
    let upload_directory = staging.root.join(&upload.id);
    let oversized_chunk = vec![0_u8; EXTERNAL_FILE_UPLOAD_CHUNK_BYTES + 1];
    let error = append_external_file_upload(&upload.id, 0, &oversized_chunk)
        .expect_err("oversized chunk should fail");
    assert!(error.contains("invalid size"));
    assert!(!upload_directory.exists());
}

#[test]
fn external_file_upload_enforces_asset_kind_and_size_before_staging() {
    let staging = TestWorkspace::new("external-file-upload-kind");
    let vault = TestWorkspace::new("external-file-upload-kind-vault");

    let empty_image = begin_external_file_upload(
        &staging.root,
        "Empty.png".to_owned(),
        0,
        ExternalFileUploadKind::Image,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect_err("an empty image should be rejected before staging");
    assert!(empty_image.contains("between 1 byte"));

    let oversized_image = begin_external_file_upload(
        &staging.root,
        "Large.png".to_owned(),
        MAX_IMAGE_BYTES + 1,
        ExternalFileUploadKind::Image,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect_err("an oversized image should be rejected before staging");
    assert!(oversized_image.contains("50 MiB"));

    let image_as_attachment = begin_external_file_upload(
        &staging.root,
        "Image.png".to_owned(),
        1,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect_err("an image filename should not bypass the image limit as an attachment");
    assert!(image_as_attachment.contains("image embedding"));
    assert_eq!(
        fs::read_dir(&staging.root)
            .expect("staging should be readable")
            .count(),
        0,
    );

    let empty_attachment = begin_external_file_upload(
        &staging.root,
        "Empty.txt".to_owned(),
        0,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("empty attachments should remain supported");
    let staged_empty =
        finish_external_file_upload(&empty_attachment.id, ExternalFileUploadKind::Attachment)
            .expect("an empty attachment should finish");
    assert_eq!(fs::metadata(&staged_empty.path).unwrap().len(), 0);
    drop(staged_empty);

    let large_attachment = begin_external_file_upload(
        &staging.root,
        "Large.pdf".to_owned(),
        MAX_IMAGE_BYTES + 1,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("the attachment limit should remain larger than the image limit");
    assert!(cancel_external_file_upload(&large_attachment.id).unwrap());

    let image = begin_external_file_upload(
        &staging.root,
        "Image.png".to_owned(),
        1,
        ExternalFileUploadKind::Image,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("an image upload should begin");
    let upload_directory = staging.root.join(&image.id);
    append_external_file_upload(&image.id, 0, b"x").expect("the declared image byte should append");
    let mismatch = finish_external_file_upload(&image.id, ExternalFileUploadKind::Attachment)
        .expect_err("an image upload should not finish through the attachment path");
    assert!(mismatch.contains("asset type"));
    assert!(!upload_directory.exists());
    assert!(!cancel_external_file_upload(&image.id).unwrap());
}

#[cfg(unix)]
#[test]
fn external_file_upload_validates_non_utf8_staging_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnon-utf8-staging";
    let staging = TestWorkspace::new("external-file-upload-non-utf8");
    let staging_directory = staging
        .root
        .join(OsString::from_vec(b"cache-\xff".to_vec()));
    let vault = TestWorkspace::new("external-file-upload-non-utf8-vault");

    let image = begin_external_file_upload(
        &staging_directory,
        "Image.png".to_owned(),
        PNG.len() as u64,
        ExternalFileUploadKind::Image,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("an image should stage below a non-UTF-8 cache path");
    append_external_file_upload(&image.id, 0, PNG).expect("image bytes should append");
    let staged_image = finish_external_file_upload(&image.id, ExternalFileUploadKind::Image)
        .expect("the image should finish staging");
    let image_source = validate_image_source_path(&staged_image.path)
        .expect("the staged image path should not require UTF-8");
    assert_eq!(read_image_file(&image_source).unwrap(), PNG);
    drop(staged_image);

    let attachment = begin_external_file_upload(
        &staging_directory,
        "Empty.txt".to_owned(),
        0,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("an attachment should stage below a non-UTF-8 cache path");
    let staged_attachment =
        finish_external_file_upload(&attachment.id, ExternalFileUploadKind::Attachment)
            .expect("the attachment should finish staging");
    let attachment_source = validate_attachment_source_path(&staged_attachment.path)
        .expect("the staged attachment path should not require UTF-8");
    assert_eq!(fs::metadata(&attachment_source).unwrap().len(), 0);
    drop(staged_attachment);
}

#[test]
fn external_file_uploads_release_abandoned_capacity_and_staging() {
    let staging = TestWorkspace::new("external-file-upload-abandoned");
    let vault = TestWorkspace::new("external-file-upload-abandoned-vault");
    let now = Instant::now();
    let timeout = Duration::from_millis(ABANDONED_EXTERNAL_FILE_UPLOAD_MILLIS);
    let abandoned_activity = now
        .checked_sub(timeout)
        .expect("the inactivity deadline should fit in an instant");
    let mut uploads = HashMap::new();
    let mut active_directory = None;

    for index in 0..MAX_EXTERNAL_FILE_UPLOADS {
        let id = format!("drop-capacity-{index}");
        let directory = staging.root.join(&id);
        fs::create_dir(&directory).expect("upload directory should be created");
        let file_name = format!("file-{index}.bin");
        let path = directory.join(&file_name);
        let file = File::create(&path).expect("staged upload should be created");
        if index == 1 {
            active_directory = Some(directory.clone());
        }
        uploads.insert(
            id,
            ExternalFileUpload {
                directory,
                path,
                file: Some(file),
                file_name,
                expected_length: 1,
                received_length: 0,
                root: vault.root.clone(),
                note_relative_path: "Note.md".to_owned(),
                kind: ExternalFileUploadKind::Attachment,
                last_activity: if index == 0 { abandoned_activity } else { now },
                cleanup_on_drop: true,
            },
        );
    }
    let abandoned_directory = staging.root.join("drop-capacity-0");
    assert_eq!(uploads.len(), MAX_EXTERNAL_FILE_UPLOADS);

    remove_abandoned_external_file_uploads(&mut uploads, now);

    assert_eq!(uploads.len(), MAX_EXTERNAL_FILE_UPLOADS - 1);
    assert!(!abandoned_directory.exists());
    let active_directory = active_directory.expect("an active upload should be retained");
    assert!(active_directory.exists());
    drop(uploads);
    assert!(!active_directory.exists());
}

#[test]
fn external_file_upload_sanitizes_names_and_removes_stale_files() {
    let staging = TestWorkspace::new("external-file-upload-cleanup");
    let vault = TestWorkspace::new("external-file-upload-cleanup-vault");
    assert!(validate_external_file_drop_note(&vault.root, "Note.md").is_err());
    fs::write(vault.root.join("Note.md"), "# Note").expect("saved note should be written");
    assert!(validate_external_file_drop_note(&vault.root, "Note.md").is_ok());
    assert!(begin_external_file_upload(
        &staging.root,
        "../escape.pdf".to_owned(),
        1,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .is_err());
    assert!(begin_external_file_upload(
        &staging.root,
        "large.pdf".to_owned(),
        MAX_ATTACHMENT_BYTES + 1,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .is_err());

    let stale_directory = staging.root.join("drop-stale-fixture");
    fs::create_dir(&stale_directory).expect("stale directory should be created");
    let stale_file = stale_directory.join("orphan.tmp");
    fs::write(&stale_file, b"orphan").expect("stale file should be written");
    set_file_modified_millis(&stale_file, 0).expect("stale modified time should be set");
    prepare_external_file_staging_directory(&staging.root)
        .expect("staging directory should be prepared");
    assert!(!stale_file.exists());
    assert!(!stale_directory.exists());
}

#[test]
fn staged_external_files_reuse_image_and_attachment_pipelines() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nexternal-drop-fixture";
    let staging = TestWorkspace::new("external-file-upload-pipelines");
    let vault = TestWorkspace::new("external-file-upload-pipelines-vault");
    fs::write(vault.root.join("Note.md"), "# Note").expect("note should be written");
    write_workspace_state(&vault.root, &WorkspaceState::default())
        .expect("workspace state should be written");

    let image_revision = revision_for_root(&vault.root).unwrap();
    let image_upload = begin_external_file_upload(
        &staging.root,
        "Dragged photo.png".to_owned(),
        PNG.len() as u64,
        ExternalFileUploadKind::Image,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("image upload should begin");
    append_external_file_upload(&image_upload.id, 0, PNG).expect("image bytes should append");
    let staged_image = finish_external_file_upload(&image_upload.id, ExternalFileUploadKind::Image)
        .expect("image upload should finish");
    let image_source = validate_image_source_path(&staged_image.path)
        .expect("the staged image should remain a safe source file");
    let image_bytes = read_image_file(&image_source).expect("the staged image should be readable");
    let image = embed_workspace_image(
        &vault.root,
        "Note.md",
        ImageEmbedSettings::default(),
        &staged_image.file_name,
        &image_bytes,
        None,
        image_revision,
    )
    .expect("staged image should embed");
    assert_eq!(image.image.relative_path, "Dragged photo.png");
    drop(staged_image);

    let attachment_revision = revision_for_root(&vault.root).unwrap();
    let attachment_upload = begin_external_file_upload(
        &staging.root,
        "Dragged report.pdf".to_owned(),
        6,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("attachment upload should begin");
    append_external_file_upload(&attachment_upload.id, 0, b"report")
        .expect("attachment bytes should append");
    let staged_attachment =
        finish_external_file_upload(&attachment_upload.id, ExternalFileUploadKind::Attachment)
            .expect("attachment upload should finish");
    let attachment_source = validate_attachment_source_path(&staged_attachment.path)
        .expect("the staged attachment should remain a safe source file");
    let attachment = embed_workspace_attachment(
        &vault.root,
        "Note.md",
        AttachmentEmbedSettings::default(),
        &attachment_source,
        None,
        attachment_revision,
    )
    .expect("staged attachment should embed");
    assert_eq!(attachment.attachment.relative_path, "Dragged report.pdf");
    drop(staged_attachment);

    assert_eq!(fs::read(vault.root.join("Dragged photo.png")).unwrap(), PNG,);
    assert_eq!(
        fs::read(vault.root.join("Dragged report.pdf")).unwrap(),
        b"report",
    );
}

#[test]
fn unused_completed_external_assets_are_discarded_with_their_stable_records() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nunused-external-image";
    let source = TestWorkspace::new("discarded-external-asset-source");
    let vault = TestWorkspace::new("discarded-external-asset-vault");
    fs::write(vault.root.join("Note.md"), "# Note").expect("saved note should be written");
    let mut state = WorkspaceState::default();
    state
        .note_paths
        .insert("note-1".to_owned(), "Note.md".to_owned());
    write_workspace_state(&vault.root, &state).expect("workspace state should be written");

    let image = embed_workspace_image(
        &vault.root,
        "Note.md",
        ImageEmbedSettings::default(),
        "Unused.png",
        PNG,
        None,
        revision_for_root(&vault.root).unwrap(),
    )
    .expect("external image should embed");
    let discarded_image = discard_workspace_external_asset(
        &vault.root,
        &image.image.id,
        &image.image.relative_path,
        image.revision,
    )
    .expect("unused external image should be discarded");
    assert!(discarded_image.discarded);
    assert!(!vault.root.join("Unused.png").exists());

    let attachment_source = source.root.join("Unused.pdf");
    fs::write(&attachment_source, b"unused attachment")
        .expect("attachment source should be written");
    let attachment = embed_workspace_attachment(
        &vault.root,
        "Note.md",
        AttachmentEmbedSettings::default(),
        &attachment_source,
        None,
        discarded_image.revision,
    )
    .expect("external attachment should embed");
    let discarded_attachment = discard_workspace_external_asset(
        &vault.root,
        &attachment.attachment.id,
        &attachment.attachment.relative_path,
        attachment.revision,
    )
    .expect("unused external attachment should be discarded");
    assert!(discarded_attachment.discarded);
    assert!(!vault.root.join("Unused.pdf").exists());

    let (stored, _) = read_workspace_state(&vault.root, &mut WarningCollector::default());
    assert!(stored.unwrap().assets.is_empty());
}

#[test]
fn external_asset_cleanup_retains_referenced_changed_and_stale_files() {
    let source = TestWorkspace::new("retained-external-asset-source");
    let vault = TestWorkspace::new("retained-external-asset-vault");
    fs::write(vault.root.join("Note.md"), "# Note").expect("saved note should be written");
    let mut state = WorkspaceState::default();
    state
        .note_paths
        .insert("note-1".to_owned(), "Note.md".to_owned());
    write_workspace_state(&vault.root, &state).expect("workspace state should be written");
    let source_path = source.root.join("Retained.pdf");
    fs::write(&source_path, b"original").expect("attachment source should be written");

    let referenced = embed_workspace_attachment(
        &vault.root,
        "Note.md",
        AttachmentEmbedSettings::default(),
        &source_path,
        None,
        revision_for_root(&vault.root).unwrap(),
    )
    .expect("referenced attachment should embed");
    fs::write(
        vault.root.join("Note.md"),
        format!(
            "[Retained](Retained.pdf#oah-asset={})",
            referenced.attachment.id,
        ),
    )
    .expect("saved reference should be written");
    let referenced_cleanup = discard_workspace_external_asset(
        &vault.root,
        &referenced.attachment.id,
        &referenced.attachment.relative_path,
        revision_for_root(&vault.root).unwrap(),
    )
    .expect("a saved reference should safely retain the attachment");
    assert!(!referenced_cleanup.discarded);
    assert!(referenced_cleanup
        .warnings
        .iter()
        .any(|warning| warning.contains("already referenced")));

    fs::write(vault.root.join("Note.md"), "# Note").expect("saved reference should be removed");
    fs::write(vault.root.join("Retained.pdf"), b"externally modified")
        .expect("embedded attachment should be modified externally");
    let changed_cleanup = discard_workspace_external_asset(
        &vault.root,
        &referenced.attachment.id,
        &referenced.attachment.relative_path,
        revision_for_root(&vault.root).unwrap(),
    )
    .expect("a changed attachment should safely be retained");
    assert!(!changed_cleanup.discarded);
    assert!(changed_cleanup
        .warnings
        .iter()
        .any(|warning| warning.contains("changed before cleanup")));
    assert_eq!(
        fs::read(vault.root.join("Retained.pdf")).unwrap(),
        b"externally modified",
    );

    fs::write(vault.root.join("Unrelated.txt"), b"revision change")
        .expect("unrelated external change should be written");
    let stale_cleanup = discard_workspace_external_asset(
        &vault.root,
        &referenced.attachment.id,
        &referenced.attachment.relative_path,
        changed_cleanup.revision,
    )
    .expect("a stale cleanup request should retain the attachment");
    assert!(!stale_cleanup.discarded);
    assert!(stale_cleanup
        .warnings
        .iter()
        .any(|warning| warning.contains("vault changed")));
}

#[test]
fn staged_external_file_accepts_an_acknowledged_revision_after_streaming() {
    let staging = TestWorkspace::new("external-file-upload-current-revision");
    let vault = TestWorkspace::new("external-file-upload-current-revision-vault");
    fs::write(vault.root.join("Note.md"), "# Note").expect("note should be written");
    write_workspace_state(&vault.root, &WorkspaceState::default())
        .expect("workspace state should be written");
    let begin_revision = revision_for_root(&vault.root).unwrap();
    let upload = begin_external_file_upload(
        &staging.root,
        "Typed during drop.txt".to_owned(),
        7,
        ExternalFileUploadKind::Attachment,
        vault.root.clone(),
        "Note.md".to_owned(),
    )
    .expect("upload should begin");
    append_external_file_upload(&upload.id, 0, b"dropped").expect("file bytes should append");

    fs::write(
        vault.root.join("Note.md"),
        "# Note\n\nTyped while streaming",
    )
    .expect("the acknowledged note save should be written");
    let finish_revision = revision_for_root(&vault.root).unwrap();
    assert_ne!(finish_revision, begin_revision);

    let staged = finish_external_file_upload(&upload.id, ExternalFileUploadKind::Attachment)
        .expect("complete upload should finish staging");
    let attachment = embed_workspace_attachment(
        &vault.root,
        "Note.md",
        AttachmentEmbedSettings::default(),
        &staged.path,
        None,
        finish_revision,
    )
    .expect("staged attachment should accept the acknowledged revision");
    assert_eq!(attachment.attachment.relative_path, "Typed during drop.txt",);
    assert_eq!(
        fs::read(vault.root.join("Typed during drop.txt")).unwrap(),
        b"dropped",
    );
}
