use super::*;

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "obsidian-at-home-{label}-{}-{}",
            std::process::id(),
            TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root).expect("test vault should be created");
        Self { root }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn editor_position(anchor: u64) -> NoteEditorPosition {
    NoteEditorPosition {
        selection: NoteEditorSelection {
            anchor,
            head: anchor + 2,
        },
        viewport: NoteEditorViewport {
            anchor,
            offset: -4.5,
            left: 12.25,
        },
    }
}

fn empty_vault(name: &str) -> VaultData {
    VaultData {
        name: name.to_owned(),
        notes: Vec::new(),
        folders: Vec::new(),
        templates: Vec::new(),
        snippets: Vec::new(),
        active_note_id: None,
        recent_note_ids: Vec::new(),
        selected_folder_id: "all".to_owned(),
        embedded_images: Vec::new(),
        image_files: Vec::new(),
        image_embed_settings: ImageEmbedSettings::default(),
        embedded_attachments: Vec::new(),
        attachment_files: Vec::new(),
        attachment_embed_settings: AttachmentEmbedSettings::default(),
    }
}

fn write_legacy_mirrored_workspace_state(root: &Path, state: &WorkspaceState) {
    write_workspace_state(root, state).expect("current workspace state should be written");
    let state_path = workspace_state_path(root);
    let current_state =
        fs::read_to_string(&state_path).expect("current workspace state should be readable");
    let legacy_state =
        current_state.replace("\"specified-folder\"", "\"specified-folder-mirrored\"");
    assert_ne!(legacy_state, current_state);
    fs::write(state_path, legacy_state).expect("legacy workspace state should be written");
}

#[test]
fn workspace_asset_limit_counts_images_and_attachments_together() {
    assert!(!workspace_asset_limit_reached(1, 1, 3));
    assert!(workspace_asset_limit_reached(2, 1, 3));
    assert!(workspace_asset_limit_reached(1, 2, 3));
    assert!(workspace_asset_limit_reached(0, 3, 3));
    assert!(workspace_asset_limit_reached(usize::MAX, 1, usize::MAX));
}

fn test_note(content: &str) -> Note {
    Note {
        id: "note-1".to_owned(),
        relative_path: "First note.md".to_owned(),
        title: "First note".to_owned(),
        content: content.to_owned(),
        folder_id: None,
        tags: Vec::new(),
        pinned: true,
        created_at: 100,
        updated_at: 200,
    }
}

fn write_saved_note(workspace: &TestWorkspace, note: &Note) -> WorkspaceState {
    fs::write(workspace.root.join(&note.relative_path), &note.content)
        .expect("saved note should be written");
    let mut state = WorkspaceState::default();
    state.name = "Test vault".to_owned();
    state
        .note_paths
        .insert(note.id.clone(), note.relative_path.clone());
    state.note_metadata.insert(
        note.id.clone(),
        StoredNoteMetadata {
            pinned: note.pinned,
            created_at: note.created_at,
        },
    );
    state.active_note_id = Some(note.id.clone());
    state.recent_note_ids.push(note.id.clone());
    write_workspace_state(&workspace.root, &state).expect("workspace state should be written");
    state
}

fn prepare_test_archive(
    workspace: &TestWorkspace,
    note: Note,
    state: &WorkspaceState,
) -> PreparedNoteArchive {
    prepare_note_archive(
        &workspace.root,
        &empty_vault("Test vault"),
        state,
        PendingNoteArchive {
            note,
            original_folder_path: String::new(),
            editor_position: Some(editor_position(3)),
        },
        1_000,
    )
    .expect("recovery snapshot should be prepared")
}

fn mark_recovery_expired(workspace: &TestWorkspace, id: &str) -> u64 {
    let (state, _) = read_workspace_state(&workspace.root, &mut WarningCollector::default());
    let mut state = state.expect("state should load");
    let entry = state
        .recently_deleted_notes
        .get_mut(id)
        .expect("recovery entry should exist");
    let path =
        recently_deleted_snapshot_path(&workspace.root, id).expect("snapshot path should be safe");
    let mut snapshot: RecentlyDeletedSnapshot =
        serde_json::from_slice(&fs::read(&path).expect("snapshot should be readable"))
            .expect("snapshot should decode");
    snapshot.deleted_note.deleted_at = 1;
    snapshot.deleted_note.expires_at = 1 + RECENTLY_DELETED_RETENTION_MILLIS;
    let mut bytes = serde_json::to_vec_pretty(&snapshot).expect("expired snapshot should encode");
    bytes.push(b'\n');
    fs::write(&path, &bytes).expect("expired snapshot should be written");
    entry.deleted_at = snapshot.deleted_note.deleted_at;
    entry.expires_at = snapshot.deleted_note.expires_at;
    entry.fingerprint = fingerprint_bytes(&bytes);
    write_workspace_state(&workspace.root, &state)
        .expect("expired recovery metadata should be written");
    revision_for_root(&workspace.root).expect("expired revision should be calculated")
}

include!("core.rs");
include!("recovery.rs");
include!("editor_positions.rs");
include!("images.rs");
include!("attachments.rs");
include!("imports.rs");
include!("external_upload.rs");
