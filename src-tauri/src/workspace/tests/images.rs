#[test]
fn embeds_reads_and_recovers_moved_images() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nembedded-image-fixture";
    let workspace = TestWorkspace::new("embedded-image-storage");
    fs::create_dir(workspace.root.join("Notes")).expect("note folder should be created");
    fs::write(workspace.root.join("Notes/First.md"), "# First").expect("note should be written");
    write_workspace_state(&workspace.root, &WorkspaceState::default())
        .expect("workspace state should be written");

    let revision = revision_for_root(&workspace.root).expect("revision should be available");
    let embedded = embed_workspace_image(
        &workspace.root,
        "Notes/First.md",
        ImageEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolder,
            folder_path: "Assets/Images".to_owned(),
        },
        "Photo.png",
        PNG,
        None,
        revision,
    )
    .expect("image should be embedded");

    assert_eq!(embedded.image.relative_path, "Assets/Images/Photo.png");
    assert_eq!(embedded.image.media_type, "image/png");
    assert_eq!(
        fs::read(workspace.root.join(&embedded.image.relative_path))
            .expect("embedded image should be readable"),
        PNG,
    );
    assert_eq!(
        read_workspace_image(
            &workspace.root,
            Some(&embedded.image.id),
            "Notes/First.md",
            "../wrong.png",
        )
        .expect("stable ID should resolve the image"),
        PNG,
    );

    fs::create_dir(workspace.root.join("Moved")).expect("move folder should be created");
    fs::rename(
        workspace.root.join(&embedded.image.relative_path),
        workspace.root.join("Moved/Renamed.png"),
    )
    .expect("image should move outside the app");
    assert_ne!(
        revision_for_root(&workspace.root).expect("moved revision should be available"),
        embedded.revision,
    );
    assert_eq!(
        read_workspace_image(
            &workspace.root,
            Some(&embedded.image.id),
            "Notes/First.md",
            "../missing.png",
        )
        .expect("fingerprint should recover the moved image"),
        PNG,
    );
    let loaded = load_workspace(&workspace.root, &empty_vault("Images"))
        .expect("reloading should persist recovered image metadata");
    assert_eq!(
        loaded
            .vault
            .embedded_images
            .iter()
            .find(|image| image.id == embedded.image.id)
            .expect("asset should remain indexed")
            .relative_path,
        "Moved/Renamed.png",
    );
    assert_eq!(
        loaded.vault.image_files,
        vec![VaultImageFile {
            asset_id: Some(embedded.image.id.clone()),
            relative_path: "Moved/Renamed.png".to_owned(),
            media_type: "image/png".to_owned(),
        }],
    );
    let mut warnings = WarningCollector::default();
    let (state, _) = read_workspace_state(&workspace.root, &mut warnings);
    assert_eq!(
        state
            .expect("state should remain readable")
            .assets
            .get(&embedded.image.id)
            .expect("asset should remain indexed")
            .relative_path,
        "Moved/Renamed.png",
    );
}

#[test]
fn legacy_mirrored_settings_migrate_idempotently_without_moving_assets() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nlegacy-mirrored-image";
    let workspace = TestWorkspace::new("legacy-mirrored-settings-migration");
    fs::create_dir_all(workspace.root.join("Images/Projects"))
        .expect("legacy image folder should be created");
    fs::create_dir_all(workspace.root.join("Files/Projects"))
        .expect("legacy attachment folder should be created");
    fs::create_dir_all(workspace.root.join("Projects")).expect("note folder should be created");
    fs::write(workspace.root.join("Images/Projects/Photo.png"), PNG)
        .expect("legacy image should be written");
    fs::write(
        workspace.root.join("Files/Projects/Report.pdf"),
        b"legacy attachment",
    )
    .expect("legacy attachment should be written");
    fs::write(workspace.root.join("Projects/Note.md"), "# Note").expect("note should be written");

    let mut state = WorkspaceState::default();
    state.image_embed_settings = ImageEmbedSettings {
        location: ImageEmbedLocation::SpecifiedFolder,
        folder_path: "Images".to_owned(),
    };
    state.attachment_embed_settings = AttachmentEmbedSettings {
        location: ImageEmbedLocation::SpecifiedFolder,
        folder_path: "Files".to_owned(),
    };
    write_legacy_mirrored_workspace_state(&workspace.root, &state);
    let state_path = workspace_state_path(&workspace.root);
    assert_eq!(
        fs::read_to_string(&state_path)
            .expect("legacy workspace state should be readable")
            .matches("specified-folder-mirrored")
            .count(),
        2,
    );

    let loaded = load_workspace(&workspace.root, &empty_vault("Legacy settings"))
        .expect("legacy workspace should load");
    assert_eq!(
        loaded.vault.image_embed_settings,
        ImageEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolder,
            folder_path: "Images".to_owned(),
        },
    );
    assert_eq!(
        loaded.vault.attachment_embed_settings,
        AttachmentEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolder,
            folder_path: "Files".to_owned(),
        },
    );
    assert_eq!(
        fs::read(workspace.root.join("Images/Projects/Photo.png")).unwrap(),
        PNG,
    );
    assert_eq!(
        fs::read(workspace.root.join("Files/Projects/Report.pdf")).unwrap(),
        b"legacy attachment",
    );
    assert!(!workspace.root.join("Images/Photo.png").exists());
    assert!(!workspace.root.join("Files/Report.pdf").exists());

    let (persisted, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    let persisted = persisted.expect("migrated workspace state should remain readable");
    assert_eq!(
        persisted.image_embed_settings,
        loaded.vault.image_embed_settings
    );
    assert_eq!(
        persisted.attachment_embed_settings,
        loaded.vault.attachment_embed_settings,
    );
    assert!(!fs::read_to_string(&state_path)
        .expect("migrated workspace state should be readable")
        .contains("specified-folder-mirrored"));

    let reopened = load_workspace(&workspace.root, &empty_vault("Legacy settings"))
        .expect("migrated workspace should reopen");
    assert_eq!(
        reopened.vault.image_embed_settings,
        loaded.vault.image_embed_settings
    );
    assert_eq!(
        reopened.vault.attachment_embed_settings,
        loaded.vault.attachment_embed_settings,
    );
    assert!(workspace.root.join("Images/Projects/Photo.png").exists());
    assert!(workspace.root.join("Files/Projects/Report.pdf").exists());
}

#[test]
fn specified_image_storage_is_shared_across_note_folders() {
    const FIRST: &[u8] = b"\x89PNG\r\n\x1a\nfirst-shared-image";
    const SECOND: &[u8] = b"\x89PNG\r\n\x1a\nsecond-shared-image";
    let workspace = TestWorkspace::new("shared-image-location");
    fs::create_dir(workspace.root.join("test1")).expect("test1 should be created");
    fs::create_dir(workspace.root.join("test2")).expect("test2 should be created");
    fs::write(workspace.root.join("test1/doc1.md"), "# Doc 1").expect("doc1 should be written");
    fs::write(workspace.root.join("test2/doc2.md"), "# Doc 2").expect("doc2 should be written");
    write_workspace_state(&workspace.root, &WorkspaceState::default())
        .expect("workspace state should be written");
    let settings = ImageEmbedSettings {
        location: ImageEmbedLocation::SpecifiedFolder,
        folder_path: "Images".to_owned(),
    };

    let first = embed_workspace_image(
        &workspace.root,
        "test1/doc1.md",
        settings.clone(),
        "First.png",
        FIRST,
        None,
        revision_for_root(&workspace.root).expect("revision should be available"),
    )
    .expect("first shared image should be embedded");
    let second = embed_workspace_image(
        &workspace.root,
        "test2/doc2.md",
        settings,
        "Second.png",
        SECOND,
        None,
        first.revision,
    )
    .expect("second shared image should be embedded");

    assert_eq!(first.image.relative_path, "Images/First.png");
    assert_eq!(second.image.relative_path, "Images/Second.png");
    assert_eq!(
        fs::read(workspace.root.join(&first.image.relative_path)).unwrap(),
        FIRST,
    );
    assert_eq!(
        fs::read(workspace.root.join(&second.image.relative_path)).unwrap(),
        SECOND,
    );
}

#[test]
fn reorganizing_an_image_moves_it_and_updates_references() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nreorganized-image";
    let workspace = TestWorkspace::new("reorganized-image");
    fs::create_dir(workspace.root.join("Images")).expect("image folder should be created");
    fs::create_dir(workspace.root.join("Images/Sub")).expect("subfolder should be created");
    fs::create_dir(workspace.root.join("Other Images"))
        .expect("sibling image folder should be created");
    fs::write(workspace.root.join("Note.md"), "# Note").expect("note should be written");
    let mut state = WorkspaceState::default();
    state
        .note_paths
        .insert("note-1".to_owned(), "Note.md".to_owned());
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");
    let embedded = embed_workspace_image(
        &workspace.root,
        "Note.md",
        ImageEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolder,
            folder_path: "Images".to_owned(),
        },
        "Photo.png",
        PNG,
        None,
        revision_for_root(&workspace.root).expect("revision should be available"),
    )
    .expect("image should be embedded");
    let original = format!(
        "![Tracked](Images/Photo.png#oah-image={})\n![Path only](Images/Photo.png)",
        embedded.image.id,
    );
    fs::write(workspace.root.join("Note.md"), &original).expect("references should be written");
    let moved_content = format!(
            "![Tracked](Images/Sub/Photo.png#oah-image={})\n![Path only](Images/Sub/Photo.png#oah-image={})",
            embedded.image.id, embedded.image.id,
        );
    let moved = relocate_workspace_image(
        &workspace.root,
        "Images/Photo.png",
        "Images/Sub/Photo.png",
        &embedded.image.id,
        &[WorkspaceImageNoteUpdate {
            note_id: "note-1".to_owned(),
            relative_path: "Note.md".to_owned(),
            expected_content: original,
            content: moved_content.clone(),
        }],
        revision_for_root(&workspace.root).expect("revision should be available"),
    )
    .expect("image should move into its subfolder");

    assert!(!workspace.root.join("Images/Photo.png").exists());
    assert_eq!(
        fs::read(workspace.root.join("Images/Sub/Photo.png")).unwrap(),
        PNG,
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join("Note.md")).unwrap(),
        moved_content
    );
    assert_eq!(moved.image.relative_path, "Images/Sub/Photo.png");

    let renamed_content =
        moved_content.replace("Images/Sub/Photo.png", "Other%20Images/Renamed.png");
    let renamed = relocate_workspace_image(
        &workspace.root,
        "Images/Sub/Photo.png",
        "Other Images/Renamed.png",
        &embedded.image.id,
        &[WorkspaceImageNoteUpdate {
            note_id: "note-1".to_owned(),
            relative_path: "Note.md".to_owned(),
            expected_content: moved_content,
            content: renamed_content.clone(),
        }],
        moved.revision,
    )
    .expect("image should move to a sibling folder and be renamed");

    assert!(!workspace.root.join("Images/Sub/Photo.png").exists());
    assert_eq!(
        fs::read(workspace.root.join("Other Images/Renamed.png")).unwrap(),
        PNG,
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join("Note.md")).unwrap(),
        renamed_content
    );
    assert_eq!(renamed.image.relative_path, "Other Images/Renamed.png");
    let mut warnings = WarningCollector::default();
    let (state, _) = read_workspace_state(&workspace.root, &mut warnings);
    assert_eq!(
        state.unwrap().assets[&embedded.image.id].relative_path,
        "Other Images/Renamed.png",
    );
}

#[test]
fn reorganizing_registers_an_untracked_image_and_rejects_collisions() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nuntracked-image";
    let workspace = TestWorkspace::new("reorganized-untracked-image");
    fs::create_dir(workspace.root.join("Images")).expect("image folder should be created");
    fs::create_dir(workspace.root.join("Archive")).expect("archive should be created");
    fs::write(workspace.root.join("Images/Loose.png"), PNG).expect("loose image should be written");
    fs::write(workspace.root.join("Archive/Loose.png"), PNG).expect("collision should be written");
    fs::write(workspace.root.join("Note.md"), "![Loose](Images/Loose.png)")
        .expect("note should be written");
    let mut state = WorkspaceState::default();
    state
        .note_paths
        .insert("note-1".to_owned(), "Note.md".to_owned());
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");
    let revision = revision_for_root(&workspace.root).expect("revision should be available");
    let error = relocate_workspace_image(
        &workspace.root,
        "Images/Loose.png",
        "Archive/Loose.png",
        "image-loose",
        &[],
        revision,
    )
    .expect_err("an existing image should block the move");
    assert!(error.contains("already exists"));
    assert!(workspace.root.join("Images/Loose.png").exists());

    let updated = "![Loose](Archive/Loose%202.png#oah-image=image-loose)";
    relocate_workspace_image(
        &workspace.root,
        "Images/Loose.png",
        "Archive/Loose 2.png",
        "image-loose",
        &[WorkspaceImageNoteUpdate {
            note_id: "note-1".to_owned(),
            relative_path: "Note.md".to_owned(),
            expected_content: "![Loose](Images/Loose.png)".to_owned(),
            content: updated.to_owned(),
        }],
        revision,
    )
    .expect("the untracked image should be registered while moving");
    let mut warnings = WarningCollector::default();
    let (state, _) = read_workspace_state(&workspace.root, &mut warnings);
    assert_eq!(
        state.unwrap().assets["image-loose"].relative_path,
        "Archive/Loose 2.png",
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join("Note.md")).unwrap(),
        updated
    );
}

#[test]
fn former_mirrored_images_can_be_reorganized_after_migration() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nformer-mirrored-image";
    let workspace = TestWorkspace::new("former-mirrored-image");
    fs::create_dir_all(workspace.root.join("Images/Notes"))
        .expect("legacy image folder should be created");
    fs::create_dir(workspace.root.join("Elsewhere")).expect("destination should be created");
    fs::write(workspace.root.join("Images/Notes/Photo.png"), PNG)
        .expect("legacy image should be written");
    let mut state = WorkspaceState::default();
    state.image_embed_settings = ImageEmbedSettings {
        location: ImageEmbedLocation::SpecifiedFolder,
        folder_path: "Images".to_owned(),
    };
    state.assets.insert(
        "image-managed".to_owned(),
        StoredVaultAsset {
            kind: VaultAssetKind::Image,
            relative_path: "Images/Notes/Photo.png".to_owned(),
            media_type: "image/png".to_owned(),
            fingerprint: fingerprint_bytes(PNG),
            modified_nanos: image_modified_nanos_for_path(
                &workspace.root,
                "Images/Notes/Photo.png",
            )
            .unwrap(),
        },
    );
    write_legacy_mirrored_workspace_state(&workspace.root, &state);
    let loaded = load_workspace(&workspace.root, &empty_vault("Former mirror"))
        .expect("legacy workspace should migrate");

    let moved = relocate_workspace_image(
        &workspace.root,
        "Images/Notes/Photo.png",
        "Elsewhere/Photo.png",
        "image-managed",
        &[],
        loaded.revision,
    )
    .expect("a former mirrored image should move normally");

    assert_eq!(moved.image.relative_path, "Elsewhere/Photo.png");
    assert!(!workspace.root.join("Images/Notes/Photo.png").exists());
    assert_eq!(
        fs::read(workspace.root.join("Elsewhere/Photo.png")).unwrap(),
        PNG,
    );
}

#[test]
fn image_relocation_requires_an_existing_destination_folder() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nmissing-destination-image";
    let workspace = TestWorkspace::new("missing-image-destination");
    fs::create_dir_all(workspace.root.join("Images/Notes"))
        .expect("source image folder should be created");
    fs::write(workspace.root.join("Images/Notes/Photo.png"), PNG)
        .expect("source image should be written");
    let mut state = WorkspaceState::default();
    state.image_embed_settings = ImageEmbedSettings {
        location: ImageEmbedLocation::SpecifiedFolder,
        folder_path: "Images".to_owned(),
    };
    state.assets.insert(
        "image-managed".to_owned(),
        StoredVaultAsset {
            kind: VaultAssetKind::Image,
            relative_path: "Images/Notes/Photo.png".to_owned(),
            media_type: "image/png".to_owned(),
            fingerprint: fingerprint_bytes(PNG),
            modified_nanos: image_modified_nanos_for_path(
                &workspace.root,
                "Images/Notes/Photo.png",
            )
            .unwrap(),
        },
    );
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");

    let error = relocate_workspace_image(
        &workspace.root,
        "Images/Notes/Photo.png",
        "Images/Archive/Renamed.png",
        "image-managed",
        &[],
        revision_for_root(&workspace.root).expect("revision should be available"),
    )
    .expect_err("an image move should not create an arbitrary destination folder");

    assert!(error.contains("destination folder"));
    assert!(workspace.root.join("Images/Notes/Photo.png").exists());
    assert!(!workspace.root.join("Images/Archive").exists());
}

#[test]
fn removing_the_final_markdown_reference_keeps_the_image_file() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nretained-unreferenced-image";
    let workspace = TestWorkspace::new("retained-unreferenced-image");
    fs::write(
        workspace.root.join("Note.md"),
        "# Note\n\n![Retained](Retained.png)",
    )
    .expect("note should be written");
    fs::write(workspace.root.join("Retained.png"), PNG).expect("image should be written");
    let loaded =
        load_workspace(&workspace.root, &empty_vault("Images")).expect("workspace should load");
    assert_eq!(
        loaded.vault.image_files,
        vec![VaultImageFile {
            asset_id: None,
            relative_path: "Retained.png".to_owned(),
            media_type: "image/png".to_owned(),
        }],
    );
    let registered = embed_workspace_image(
        &workspace.root,
        "Note.md",
        ImageEmbedSettings::default(),
        "Retained.png",
        PNG,
        Some("Retained.png"),
        loaded.revision,
    )
    .expect("existing image should be registered");
    let registered_workspace = load_workspace(&workspace.root, &empty_vault("Images"))
        .expect("registered workspace should load");
    let reference_revision = registered_workspace.revision;
    let mut without_reference = registered_workspace.vault;
    without_reference.notes[0].content = "# Note\n\nNo image reference remains.".to_owned();

    save_workspace_files(&workspace.root, &without_reference, reference_revision)
        .expect("note without the image reference should save");

    assert_eq!(fs::read(workspace.root.join("Retained.png")).unwrap(), PNG);
    let reopened =
        load_workspace(&workspace.root, &empty_vault("Images")).expect("workspace should reopen");
    assert_eq!(reopened.vault.image_files.len(), 1);
    assert_eq!(
        reopened.vault.image_files[0].asset_id.as_deref(),
        Some(registered.image.id.as_str()),
    );
}

#[test]
fn image_storage_honors_locations_and_name_collisions() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nlocation-fixture";
    let workspace = TestWorkspace::new("embedded-image-locations");
    fs::create_dir(workspace.root.join("Projects")).expect("note folder should be created");
    fs::write(workspace.root.join("Projects/Plan.md"), "# Plan").expect("note should be written");
    write_workspace_state(&workspace.root, &WorkspaceState::default())
        .expect("workspace state should be written");

    let first = embed_workspace_image(
        &workspace.root,
        "Projects/Plan.md",
        ImageEmbedSettings {
            location: ImageEmbedLocation::NoteFolder,
            folder_path: "ignored".to_owned(),
        },
        "Diagram.png",
        PNG,
        None,
        revision_for_root(&workspace.root).expect("revision should be available"),
    )
    .expect("note-folder image should be embedded");
    let second = embed_workspace_image(
        &workspace.root,
        "Projects/Plan.md",
        ImageEmbedSettings {
            location: ImageEmbedLocation::NoteFolder,
            folder_path: String::new(),
        },
        "Diagram.png",
        PNG,
        None,
        first.revision,
    )
    .expect("colliding image should be embedded");
    let case_collision = embed_workspace_image(
        &workspace.root,
        "Projects/Plan.md",
        ImageEmbedSettings {
            location: ImageEmbedLocation::NoteFolder,
            folder_path: String::new(),
        },
        "diagram.PNG",
        PNG,
        None,
        second.revision,
    )
    .expect("case-only collision should use a portable unique name");
    let root_image = embed_workspace_image(
        &workspace.root,
        "Projects/Plan.md",
        ImageEmbedSettings::default(),
        "Root.png",
        PNG,
        None,
        case_collision.revision,
    )
    .expect("root image should be embedded");

    assert_eq!(first.image.relative_path, "Projects/Diagram.png");
    assert_eq!(second.image.relative_path, "Projects/Diagram 2.png");
    assert_eq!(case_collision.image.relative_path, "Projects/diagram 3.png");
    assert_eq!(root_image.image.relative_path, "Root.png");

    fs::write(workspace.root.join("Projects/Existing.png"), PNG)
        .expect("existing vault image should be written");
    let existing = embed_workspace_image(
        &workspace.root,
        "Projects/Plan.md",
        ImageEmbedSettings::default(),
        "Existing.png",
        PNG,
        Some("Projects/Existing.png"),
        revision_for_root(&workspace.root).expect("revision should be available"),
    )
    .expect("existing vault image should be registered without copying");
    let reused = embed_workspace_image(
        &workspace.root,
        "Projects/Plan.md",
        ImageEmbedSettings::default(),
        "Existing.png",
        PNG,
        Some("Projects/Existing.png"),
        existing.revision,
    )
    .expect("registered image should reuse its stable ID");
    assert_eq!(existing.image.relative_path, "Projects/Existing.png");
    assert_eq!(reused.image.id, existing.image.id);
    assert!(!workspace.root.join("Existing.png").exists());
}
