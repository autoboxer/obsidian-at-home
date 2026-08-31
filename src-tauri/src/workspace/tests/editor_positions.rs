#[test]
fn vault_data_matches_frontend_contract_without_editor_mode() {
    let value = serde_json::json!({
        "name": "Test vault",
        "notes": [],
        "folders": [],
        "templates": [],
        "snippets": [],
        "activeNoteId": null,
        "recentNoteIds": [],
        "selectedFolderId": "all"
    });

    let vault: VaultData = serde_json::from_value(value).expect("vault data should deserialize");
    let serialized = serde_json::to_value(vault).expect("vault data should serialize");

    assert_eq!(serialized["name"], "Test vault");
    assert!(serialized.get("editorMode").is_none());
}

#[test]
fn normalizes_recent_notes() {
    let note_id_values = (1..=12)
        .map(|index| format!("note-{index}"))
        .collect::<Vec<_>>();
    let note_ids = note_id_values.iter().map(String::as_str).collect();
    let stored = [
        "note-2", "missing", "note-3", "note-2", "note-4", "note-5", "note-6", "note-7", "note-8",
        "note-9", "note-10", "note-11", "note-12",
    ]
    .map(str::to_owned);

    let normalized = normalize_recent_note_ids(&stored, Some("note-1"), &note_ids);
    let expected = (1..=10)
        .map(|index| format!("note-{index}"))
        .collect::<Vec<_>>();

    assert_eq!(normalized, expected);
}

#[test]
fn editor_positions_match_frontend_schema_and_filter_invalid_entries() {
    let position = editor_position(7);
    let serialized = serde_json::to_value(&position).expect("position should serialize");

    assert_eq!(serialized["selection"]["anchor"], 7);
    assert_eq!(serialized["selection"]["head"], 9);
    assert_eq!(serialized["viewport"]["anchor"], 7);
    assert_eq!(serialized["viewport"]["offset"], -4.5);
    assert_eq!(serialized["viewport"]["left"], 12.25);

    let mut raw = BTreeMap::new();
    raw.insert("known".to_owned(), serialized);
    raw.insert(
        "invalid".to_owned(),
        serde_json::json!({
            "selection": { "anchor": -1, "head": 2 },
            "viewport": { "anchor": 0, "offset": 0, "left": 0 }
        }),
    );
    raw.insert(
        "missing".to_owned(),
        serde_json::to_value(editor_position(1)).expect("position should serialize"),
    );
    let note_ids = ["known", "invalid"].into_iter().collect();

    let decoded = decode_editor_positions(raw, &note_ids);

    assert_eq!(
        decoded.positions,
        BTreeMap::from([("known".to_owned(), position)])
    );
    assert_eq!(decoded.invalid_count, 1);
    assert_eq!(decoded.unknown_count, 1);
}

#[test]
fn malformed_and_newer_editor_positions_only_produce_warnings() {
    let workspace = TestWorkspace::new("position-warnings");
    let directory = workspace.root.join(STATE_DIRECTORY);
    fs::create_dir(&directory).expect("state directory should be created");
    let path = directory.join(EDITOR_POSITIONS_FILE);
    let note_ids = HashSet::new();

    fs::write(&path, b"not json").expect("malformed positions should be written");
    let mut warnings = WarningCollector::default();
    let (positions, writable, revision) =
        load_editor_positions(&workspace.root, &note_ids, &mut warnings);

    assert!(positions.is_empty());
    assert!(writable);
    assert!(revision.is_some());
    assert!(warnings
        .finish()
        .iter()
        .any(|warning| warning.contains("invalid")));

    fs::write(
        &path,
        format!(
            "{{\"version\":{},\"positions\":{{}}}}",
            EDITOR_POSITIONS_VERSION + 1,
        ),
    )
    .expect("newer positions should be written");
    let mut warnings = WarningCollector::default();
    let (positions, writable, revision) =
        load_editor_positions(&workspace.root, &note_ids, &mut warnings);

    assert!(positions.is_empty());
    assert!(!writable);
    assert!(revision.is_none());
    assert!(warnings
        .finish()
        .iter()
        .any(|warning| warning.contains("not changed")));
}

#[test]
fn loading_prunes_invalid_and_unknown_editor_positions() {
    let workspace = TestWorkspace::new("position-pruning");
    let directory = workspace.root.join(STATE_DIRECTORY);
    fs::create_dir(&directory).expect("state directory should be created");
    let path = directory.join(EDITOR_POSITIONS_FILE);
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "version": EDITOR_POSITIONS_VERSION,
            "positions": {
                "known": editor_position(3),
                "invalid": {
                    "selection": { "anchor": -1, "head": 2 },
                    "viewport": { "anchor": 0, "offset": 0, "left": 0 }
                },
                "missing": editor_position(7)
            }
        }))
        .expect("positions should serialize"),
    )
    .expect("positions should be written");
    let note_ids = ["known", "invalid"].into_iter().collect();
    let mut warnings = WarningCollector::default();

    let (positions, writable, revision) =
        load_editor_positions(&workspace.root, &note_ids, &mut warnings);

    assert_eq!(
        positions,
        BTreeMap::from([("known".to_owned(), editor_position(3))])
    );
    assert!(writable);
    assert!(revision.is_some());
    let EditorPositionsRead::Loaded(raw, _) =
        read_editor_positions(&workspace.root).expect("positions should be readable")
    else {
        panic!("positions should use the supported schema");
    };
    assert_eq!(raw.positions.len(), 1);
    assert!(raw.positions.contains_key("known"));
    assert_eq!(warnings.finish().len(), 2);
}

#[test]
fn saving_replaces_malformed_editor_positions() {
    let workspace = TestWorkspace::new("malformed-position-save");
    let mut state = WorkspaceState::default();
    state
        .note_paths
        .insert("known".to_owned(), "Known.md".to_owned());
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");
    let path = editor_positions_path(&workspace.root);
    fs::write(&path, b"not json").expect("malformed positions should be written");

    let (_, _, revision) = load_editor_positions(
        &workspace.root,
        &HashSet::new(),
        &mut WarningCollector::default(),
    );
    save_editor_positions(
        &workspace.root,
        BTreeMap::from([("known".to_owned(), editor_position(2))]),
        revision,
    )
    .expect("malformed positions should be replaced");

    let EditorPositionsRead::Loaded(raw, _) =
        read_editor_positions(&workspace.root).expect("positions should be readable")
    else {
        panic!("positions should use the supported schema");
    };
    assert_eq!(raw.positions.len(), 1);
    assert!(raw.positions.contains_key("known"));
}

#[test]
fn saving_editor_positions_does_not_change_the_vault_revision() {
    let workspace = TestWorkspace::new("position-revision");
    let mut state = WorkspaceState::default();
    state
        .note_paths
        .insert("known".to_owned(), "Known.md".to_owned());
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");
    let revision_before =
        revision_for_root(&workspace.root).expect("initial revision should be calculated");
    let positions = BTreeMap::from([("known".to_owned(), editor_position(4))]);

    save_editor_positions(&workspace.root, positions, None)
        .expect("editor positions should be saved");

    let revision_after =
        revision_for_root(&workspace.root).expect("updated revision should be calculated");
    assert_eq!(revision_after, revision_before);

    let EditorPositionsRead::Loaded(raw, _) =
        read_editor_positions(&workspace.root).expect("positions should be readable")
    else {
        panic!("positions should use the supported schema");
    };
    assert_eq!(raw.positions.len(), 1);
    let note_ids = ["known"].into_iter().collect();
    let decoded = decode_editor_positions(raw.positions, &note_ids);
    assert_eq!(decoded.positions.len(), 1);
    assert_eq!(decoded.positions["known"], editor_position(4));
}

#[test]
fn saving_rejects_a_stale_editor_position_revision() {
    let workspace = TestWorkspace::new("stale-position-revision");
    let mut state = WorkspaceState::default();
    state
        .note_paths
        .insert("first".to_owned(), "First.md".to_owned());
    state
        .note_paths
        .insert("second".to_owned(), "Second.md".to_owned());
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");
    let initial = BTreeMap::from([
        ("first".to_owned(), editor_position(1)),
        ("second".to_owned(), editor_position(2)),
    ]);
    save_editor_positions(&workspace.root, initial, None)
        .expect("initial positions should be saved");
    let note_ids = ["first", "second"].into_iter().collect();
    let (mut first_instance, _, first_revision) =
        load_editor_positions(&workspace.root, &note_ids, &mut WarningCollector::default());
    let (mut second_instance, _, second_revision) =
        load_editor_positions(&workspace.root, &note_ids, &mut WarningCollector::default());
    first_instance.insert("first".to_owned(), editor_position(11));
    let current_revision = save_editor_positions(&workspace.root, first_instance, first_revision)
        .expect("the first instance should save");
    let positions_path = editor_positions_path(&workspace.root);
    let current_bytes = fs::read(&positions_path).expect("current positions should be readable");
    second_instance.insert("second".to_owned(), editor_position(22));

    let error = save_editor_positions(&workspace.root, second_instance.clone(), second_revision)
        .expect_err("a stale position snapshot should be rejected");

    assert!(error.contains("another app window"));
    assert_eq!(
        fs::read(&positions_path).expect("current positions should remain readable"),
        current_bytes,
    );
    save_editor_positions(&workspace.root, second_instance, Some(current_revision))
        .expect("a snapshot with the current revision should save");
}

#[test]
fn saving_rejects_a_file_created_after_positions_were_loaded() {
    let workspace = TestWorkspace::new("created-position-revision");
    let mut state = WorkspaceState::default();
    state
        .note_paths
        .insert("known".to_owned(), "Known.md".to_owned());
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");
    let note_ids = ["known"].into_iter().collect();
    let (_, _, revision) =
        load_editor_positions(&workspace.root, &note_ids, &mut WarningCollector::default());
    assert!(revision.is_none());
    write_editor_positions(
        &workspace.root,
        &BTreeMap::from([("known".to_owned(), editor_position(3))]),
    )
    .expect("external positions should be written");

    let error = save_editor_positions(
        &workspace.root,
        BTreeMap::from([("known".to_owned(), editor_position(9))]),
        revision,
    )
    .expect_err("a newly created position file should not be overwritten");

    assert!(error.contains("another app window"));
}

#[test]
fn saving_rejects_positions_for_unsaved_notes() {
    let workspace = TestWorkspace::new("unsaved-position");
    let mut state = WorkspaceState::default();
    state
        .note_paths
        .insert("known".to_owned(), "Known.md".to_owned());
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");

    let error = save_editor_positions(
        &workspace.root,
        BTreeMap::from([("missing".to_owned(), editor_position(8))]),
        None,
    )
    .expect_err("positions for unsaved notes should be rejected");

    assert!(error.contains("have not been saved"));
    assert!(!editor_positions_path(&workspace.root).exists());
}

#[test]
fn unreadable_workspace_metadata_preserves_editor_positions() {
    let workspace = TestWorkspace::new("unsafe-position-load");
    let directory = workspace.root.join(STATE_DIRECTORY);
    fs::create_dir(&directory).expect("state directory should be created");
    fs::write(workspace.root.join("Known.md"), "Known note").expect("note should be written");
    fs::write(workspace_state_path(&workspace.root), b"not json")
        .expect("malformed state should be written");
    let positions_path = editor_positions_path(&workspace.root);
    let positions_before = serde_json::to_vec(&serde_json::json!({
        "version": EDITOR_POSITIONS_VERSION,
        "positions": { "preserved": editor_position(3) }
    }))
    .expect("positions should serialize");
    fs::write(&positions_path, &positions_before).expect("positions should be written");
    let defaults: VaultData = serde_json::from_value(serde_json::json!({
        "name": "Test vault",
        "notes": [],
        "folders": [],
        "templates": [],
        "snippets": [],
        "activeNoteId": null,
        "recentNoteIds": [],
        "selectedFolderId": "all"
    }))
    .expect("defaults should deserialize");

    let loaded =
        load_workspace(&workspace.root, &defaults).expect("workspace should open with warnings");

    assert!(loaded.editor_positions.is_empty());
    assert!(!loaded.editor_positions_writable);
    assert_eq!(
        fs::read(&positions_path).expect("positions should remain readable"),
        positions_before,
    );
}

#[test]
fn saving_refuses_to_replace_newer_editor_positions() {
    let workspace = TestWorkspace::new("newer-position-save");
    let mut state = WorkspaceState::default();
    state
        .note_paths
        .insert("known".to_owned(), "Known.md".to_owned());
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");
    let path = editor_positions_path(&workspace.root);
    let newer = format!(
        "{{\"version\":{},\"positions\":{{}}}}",
        EDITOR_POSITIONS_VERSION + 1,
    );
    fs::write(&path, &newer).expect("newer positions should be written");

    let error = save_editor_positions(
        &workspace.root,
        BTreeMap::from([("known".to_owned(), editor_position(2))]),
        None,
    )
    .expect_err("newer positions should not be overwritten");

    assert!(error.contains("Update the app"));
    assert_eq!(
        fs::read_to_string(&path).expect("newer positions should remain readable"),
        newer,
    );
}
