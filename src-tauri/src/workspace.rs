use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use walkdir::{DirEntry, WalkDir};

const STATE_DIRECTORY: &str = ".obsidian-at-home";
const STATE_FILE: &str = "state.json";
const REGISTRY_FILE: &str = "workspaces.json";
const TRANSACTIONS_DIRECTORY: &str = "transactions";
const TRANSACTION_MANIFEST_FILE: &str = "manifest.json";
const STATE_VERSION: u32 = 1;
const REGISTRY_VERSION: u32 = 1;
const TRANSACTION_VERSION: u32 = 2;
const MAX_NOTE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_TOTAL_NOTE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_NOTES: usize = 100_000;
const MAX_WARNINGS: usize = 200;
const MAX_PATH_COMPONENTS: usize = 120;
const MAX_TRANSACTION_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SAFE_JAVASCRIPT_INTEGER: u64 = (1_u64 << 53) - 1;

static WORKSPACE_IO_LOCK: Mutex<()> = Mutex::new(());
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    #[serde(default)]
    pub relative_path: String,
    pub title: String,
    #[serde(default)]
    pub content: String,
    pub folder_id: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub pinned: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteTemplate {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub title_pattern: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub glyph: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_in: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CssSnippet {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub css: String,
    #[serde(default)]
    pub enabled: bool,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_in: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultData {
    pub name: String,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default)]
    pub folders: Vec<Folder>,
    #[serde(default)]
    pub templates: Vec<NoteTemplate>,
    #[serde(default)]
    pub snippets: Vec<CssSnippet>,
    pub active_note_id: Option<String>,
    pub selected_folder_id: String,
    pub editor_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultDescriptor {
    pub name: String,
    pub path: String,
    pub last_opened_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLoad {
    pub vault: VaultData,
    pub descriptor: VaultDescriptor,
    pub warnings: Vec<String>,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResult {
    pub workspace: Option<WorkspaceLoad>,
    pub recent_vaults: Vec<VaultDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
struct StoredNoteMetadata {
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WorkspaceState {
    #[serde(default = "state_version")]
    version: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    note_paths: BTreeMap<String, String>,
    #[serde(default)]
    folder_paths: BTreeMap<String, String>,
    #[serde(default)]
    note_metadata: BTreeMap<String, StoredNoteMetadata>,
    #[serde(default)]
    templates: Vec<NoteTemplate>,
    #[serde(default)]
    snippets: Vec<CssSnippet>,
    #[serde(default)]
    active_note_id: Option<String>,
    #[serde(default = "default_folder_selection")]
    selected_folder_id: String,
    #[serde(default = "default_editor_mode")]
    editor_mode: String,
    #[serde(default)]
    last_committed_transaction_id: Option<String>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            name: String::new(),
            note_paths: BTreeMap::new(),
            folder_paths: BTreeMap::new(),
            note_metadata: BTreeMap::new(),
            templates: Vec::new(),
            snippets: Vec::new(),
            active_note_id: None,
            selected_folder_id: default_folder_selection(),
            editor_mode: default_editor_mode(),
            last_committed_transaction_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WorkspaceRegistry {
    #[serde(default = "registry_version")]
    version: u32,
    #[serde(default)]
    active_path: Option<String>,
    #[serde(default)]
    recent_vaults: Vec<VaultDescriptor>,
}

impl Default for WorkspaceRegistry {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            active_path: None,
            recent_vaults: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct ScannedNote {
    relative_path: String,
    content: String,
    created_at: u64,
    updated_at: u64,
    tags: Vec<String>,
}

#[derive(Debug)]
struct ScannedFolder {
    relative_path: String,
    created_at: u64,
}

#[derive(Debug)]
struct NoteWritePlan {
    id: String,
    old_relative_path: Option<String>,
    new_relative_path: String,
    content: String,
    needs_write: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum TransactionPhase {
    Prepared,
    Applying,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FileFingerprint {
    length: u64,
    hash: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct TransactionOriginal {
    relative_path: String,
    backup_relative_path: String,
    fingerprint: FileFingerprint,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct TransactionTarget {
    relative_path: String,
    fingerprint: FileFingerprint,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FolderCaseRename {
    from_relative_path: String,
    to_relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct TransactionManifest {
    version: u32,
    id: String,
    phase: TransactionPhase,
    originals: Vec<TransactionOriginal>,
    targets: Vec<TransactionTarget>,
    folder_case_renames: Vec<FolderCaseRename>,
    created_directories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    length: u64,
    modified_nanos: u128,
}

#[derive(Debug)]
struct SaveConsistency {
    unaffected: BTreeMap<String, FileStamp>,
    targets: Vec<TransactionTarget>,
}

#[tauri::command]
pub fn workspace_bootstrap(
    app: AppHandle,
    defaults: VaultData,
) -> Result<BootstrapResult, String> {
    let _guard = lock_workspace_io()?;
    let mut registry = read_registry(&app)?;
    sort_descriptors(&mut registry.recent_vaults);

    let workspace = match registry.active_path.clone() {
        Some(active_path) => {
            let path = PathBuf::from(&active_path);
            if !path.is_dir() {
                registry.active_path = None;
                write_registry(&app, &registry)?;
                None
            } else {
                let canonical = validate_workspace_root_path(&path)?;
                reject_home_vault(&app, &canonical)?;
                let loaded = load_workspace(&canonical, &defaults)?;
                remember_workspace(&mut registry, &loaded.descriptor);
                write_registry(&app, &registry)?;
                Some(loaded)
            }
        }
        None => None,
    };

    sort_descriptors(&mut registry.recent_vaults);
    Ok(BootstrapResult {
        workspace,
        recent_vaults: registry.recent_vaults,
    })
}

#[tauri::command]
pub fn workspace_open(
    app: AppHandle,
    path: String,
    defaults: VaultData,
) -> Result<WorkspaceLoad, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let mut registry = read_registry(&app)?;
    reject_nested_registered_vault(&root, &registry, true)?;
    let loaded = load_workspace(&root, &defaults)?;
    remember_workspace(&mut registry, &loaded.descriptor);
    write_registry(&app, &registry)?;
    Ok(loaded)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_create(
    app: AppHandle,
    parent_path: String,
    name: String,
    mut initial: VaultData,
) -> Result<WorkspaceLoad, String> {
    let _guard = lock_workspace_io()?;
    let parent = validate_parent_directory(&parent_path)?;
    validate_component_name(name.trim(), "vault")?;
    let root = parent.join(name.trim());
    let mut registry = read_registry(&app)?;
    reject_nested_registered_vault(&root, &registry, false)?;
    if root.exists() {
        return Err(format!(
            "A file or folder already exists at {}.",
            root.display()
        ));
    }
    create_directory_durable(&root)
        .map_err(|error| format!("Could not create the vault folder: {error}"))?;
    let root = root
        .canonicalize()
        .map_err(|error| format!("Could not resolve the new vault folder: {error}"))?;

    initial.name = name.trim().to_owned();
    let expected_revision = revision_for_root(&root)?;
    save_workspace_files(&root, &initial, expected_revision)?;
    let mut loaded = load_workspace(&root, &initial)?;

    match (|| {
        remember_workspace(&mut registry, &loaded.descriptor);
        write_registry(&app, &registry)
    })() {
        Ok(()) => {}
        Err(error) => loaded
            .warnings
            .push(format!("The vault was created, but it could not be added to Recents: {error}")),
    }
    Ok(loaded)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_save(
    app: AppHandle,
    path: String,
    vault: VaultData,
    expected_revision: u64,
) -> Result<SaveResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let mut result = save_workspace_files(&root, &vault, expected_revision)?;

    let registry_result = (|| {
        let mut registry = read_registry(&app)?;
        let descriptor = VaultDescriptor {
            name: display_vault_name(&vault.name, &root),
            path: path_string(&root)?,
            last_opened_at: result.saved_at,
        };
        remember_workspace(&mut registry, &descriptor);
        write_registry(&app, &registry)
    })();
    if let Err(error) = registry_result {
        result
            .warnings
            .push(format!("The vault was saved, but Recents could not be updated: {error}"));
    }
    Ok(result)
}

#[tauri::command]
pub fn workspace_forget(app: AppHandle, path: String) -> Result<Vec<VaultDescriptor>, String> {
    let _guard = lock_workspace_io()?;
    let mut registry = read_registry(&app)?;
    let comparison_path = canonical_path_if_available(&path);
    registry.recent_vaults.retain(|vault| {
        canonical_path_if_available(&vault.path) != comparison_path
    });
    if registry
        .active_path
        .as_deref()
        .is_some_and(|active| canonical_path_if_available(active) == comparison_path)
    {
        registry.active_path = None;
    }
    sort_descriptors(&mut registry.recent_vaults);
    write_registry(&app, &registry)?;
    Ok(registry.recent_vaults)
}

#[tauri::command]
pub fn workspace_revision(app: AppHandle, path: String) -> Result<u64, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    revision_for_root(&root)
}

fn load_workspace(root: &Path, defaults: &VaultData) -> Result<WorkspaceLoad, String> {
    let root = validate_workspace_root_path(root)?;
    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(&root, &mut warnings);
    let state_was_present = stored_state.is_some();
    if state_was_present || !state_file_was_present {
        recover_workspace_transactions(&root, stored_state.as_ref(), &mut warnings)?;
    } else {
        warnings.push(
            "Save transactions were not recovered because workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    let mut state = stored_state.unwrap_or_default();
    let (scanned_notes, scanned_folders) = scan_markdown_files(&root, &mut warnings)?;

    let mut used_note_ids = HashSet::new();
    let note_id_by_path = reverse_valid_paths(&state.note_paths, "note", &mut warnings);
    let mut notes = Vec::with_capacity(scanned_notes.len());
    let mut note_paths = BTreeMap::new();
    let mut note_metadata = BTreeMap::new();

    for scanned in scanned_notes {
        let id = note_id_by_path
            .get(&scanned.relative_path)
            .filter(|id| used_note_ids.insert((*id).clone()))
            .cloned()
            .unwrap_or_else(|| fresh_id("note", &scanned.relative_path, &mut used_note_ids));
        let metadata = state.note_metadata.get(&id).cloned().unwrap_or_default();
        let title = Path::new(&scanned.relative_path)
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Untitled note")
            .to_owned();
        note_paths.insert(id.clone(), scanned.relative_path.clone());
        note_metadata.insert(
            id.clone(),
            StoredNoteMetadata {
                pinned: metadata.pinned,
                created_at: if metadata.created_at > 0 {
                    metadata.created_at
                } else {
                    scanned.created_at
                },
            },
        );
        notes.push(Note {
            id,
            relative_path: scanned.relative_path,
            title,
            content: scanned.content,
            folder_id: None,
            tags: scanned.tags,
            pinned: metadata.pinned,
            created_at: if metadata.created_at > 0 {
                metadata.created_at
            } else {
                scanned.created_at
            },
            updated_at: scanned.updated_at,
        });
    }

    let mut used_folder_ids = HashSet::new();
    let folder_id_by_path = reverse_valid_paths(&state.folder_paths, "folder", &mut warnings);
    let mut folder_ids = HashMap::new();
    let mut folder_created_at = HashMap::new();
    for scanned in &scanned_folders {
        let id = folder_id_by_path
            .get(&scanned.relative_path)
            .filter(|id| used_folder_ids.insert((*id).clone()))
            .cloned()
            .unwrap_or_else(|| {
                fresh_id("folder", &scanned.relative_path, &mut used_folder_ids)
            });
        folder_ids.insert(scanned.relative_path.clone(), id);
        folder_created_at.insert(scanned.relative_path.clone(), scanned.created_at);
    }

    let mut folders = Vec::with_capacity(scanned_folders.len());
    let mut folder_paths = BTreeMap::new();
    for scanned in scanned_folders {
        let id = folder_ids
            .get(&scanned.relative_path)
            .expect("scanned folder should have an ID")
            .clone();
        let relative = Path::new(&scanned.relative_path);
        let name = relative
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Folder")
            .to_owned();
        let parent_id = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .and_then(path_to_slash_string)
            .and_then(|parent| folder_ids.get(&parent).cloned());
        folder_paths.insert(id.clone(), scanned.relative_path.clone());
        folders.push(Folder {
            id,
            name,
            parent_id,
            created_at: *folder_created_at
                .get(&scanned.relative_path)
                .unwrap_or(&now_millis()),
        });
    }

    for note in &mut notes {
        note.folder_id = Path::new(&note.relative_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .and_then(path_to_slash_string)
            .and_then(|parent| folder_ids.get(&parent).cloned());
    }

    notes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    folders.sort_by(|left, right| {
        folder_paths
            .get(&left.id)
            .cmp(&folder_paths.get(&right.id))
    });

    let note_ids: HashSet<&str> = notes.iter().map(|note| note.id.as_str()).collect();
    let active_note_id = state
        .active_note_id
        .filter(|id| note_ids.contains(id.as_str()))
        .or_else(|| notes.first().map(|note| note.id.clone()));
    let selected_folder_id = if is_virtual_folder_selection(&state.selected_folder_id) {
        state.selected_folder_id.clone()
    } else {
        "all".to_owned()
    };
    let templates = if state_was_present {
        state.templates.clone()
    } else {
        defaults.templates.clone()
    };
    let snippets = if state_was_present {
        state.snippets.clone()
    } else {
        defaults.snippets.clone()
    };
    let vault_name = display_vault_name(
        if state_was_present && !state.name.trim().is_empty() {
            &state.name
        } else {
            ""
        },
        &root,
    );

    state = WorkspaceState {
        version: STATE_VERSION,
        name: vault_name.clone(),
        note_paths,
        folder_paths,
        note_metadata,
        templates: templates.clone(),
        snippets: snippets.clone(),
        active_note_id: active_note_id.clone(),
        selected_folder_id: selected_folder_id.clone(),
        editor_mode: normalize_editor_mode(if state_was_present {
            &state.editor_mode
        } else {
            &defaults.editor_mode
        }),
        last_committed_transaction_id: state.last_committed_transaction_id.clone(),
    };
    if state_was_present || !state_file_was_present {
        if let Err(error) = write_workspace_state(&root, &state) {
            warnings.push(format!("Could not save workspace metadata: {error}"));
        }
    } else {
        warnings.push(
            "Workspace metadata was not replaced because the existing file could not be read."
                .to_owned(),
        );
    }

    let opened_at = now_millis();
    let revision = revision_for_root(&root)?;
    Ok(WorkspaceLoad {
        vault: VaultData {
            name: vault_name.clone(),
            notes,
            folders,
            templates,
            snippets,
            active_note_id,
            selected_folder_id,
            editor_mode: state.editor_mode,
        },
        descriptor: VaultDescriptor {
            name: vault_name,
            path: path_string(&root)?,
            last_opened_at: opened_at,
        },
        warnings: warnings.finish(),
        revision,
    })
}

fn save_workspace_files(
    root: &Path,
    vault: &VaultData,
    expected_revision: u64,
) -> Result<SaveResult, String> {
    let root = validate_workspace_root_path(root)?;
    let mut warnings = WarningCollector::default();
    let state_path = workspace_state_path(&root);
    let expected_state_fingerprint = fingerprint_regular_file(&state_path)?;
    let (old_state, state_file_was_present) = read_workspace_state(&root, &mut warnings);
    if fingerprint_regular_file(&state_path)? != expected_state_fingerprint {
        return Err(
            "Workspace metadata changed while it was being read. Reload the vault before saving."
                .to_owned(),
        );
    }
    if old_state.is_none() && state_file_was_present {
        return Err(
            "The existing .obsidian-at-home/state.json file could not be read. Move or repair it before saving so it is not overwritten."
                .to_owned(),
        );
    }
    let old_state = old_state.unwrap_or_default();
    recover_workspace_transactions(&root, Some(&old_state), &mut warnings)?;
    if revision_for_root(&root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before saving so those changes are not overwritten."
                .to_owned(),
        );
    }

    let desired_folder_paths = build_folder_paths(&vault.folders)?;
    let plans = build_note_write_plans(&root, vault, &old_state, &desired_folder_paths)?;
    if revision_for_root(&root)? != expected_revision {
        return Err(
            "The vault changed while it was being saved. Reload it before trying again."
                .to_owned(),
        );
    }

    let mut paths_to_replace = BTreeSet::new();
    for (id, old_relative_path) in &old_state.note_paths {
        let new_path = plans
            .iter()
            .find(|plan| plan.id == *id)
            .map(|plan| plan.new_relative_path.as_str());
        if new_path != Some(old_relative_path.as_str()) {
            paths_to_replace.insert(old_relative_path.clone());
        }
    }
    for plan in &plans {
        if plan.needs_write {
            if let Some(old_relative_path) = &plan.old_relative_path {
                paths_to_replace.insert(old_relative_path.clone());
            }
        }
    }
    validate_managed_path_ownership(&old_state.note_paths)?;
    validate_save_targets(&root, &plans, &paths_to_replace, &old_state.note_paths)?;
    let folder_case_renames = build_folder_case_renames(
        &old_state.folder_paths,
        &desired_folder_paths,
    )?;
    validate_folder_case_renames(&root, &folder_case_renames)?;
    let created_directories = collect_created_directories(
        &root,
        desired_folder_paths.values(),
        &folder_case_renames,
    )?;
    let baseline = note_file_stamps(&root)?;
    let consistency = build_save_consistency(&baseline, &paths_to_replace, &plans)?;
    if revision_for_root(&root)? != expected_revision {
        return Err(
            "The vault changed while the save was being prepared. Reload it before trying again."
                .to_owned(),
        );
    }

    let saved_at = now_millis();
    let mut note_paths = BTreeMap::new();
    let mut note_metadata = BTreeMap::new();
    for (note, plan) in vault.notes.iter().zip(plans.iter()) {
        note_paths.insert(note.id.clone(), plan.new_relative_path.clone());
        note_metadata.insert(
            note.id.clone(),
            StoredNoteMetadata {
                pinned: note.pinned,
                created_at: if note.created_at > 0 {
                    note.created_at
                } else {
                    saved_at
                },
            },
        );
    }
    let mut state = WorkspaceState {
        version: STATE_VERSION,
        name: display_vault_name(&vault.name, &root),
        note_paths,
        folder_paths: desired_folder_paths,
        note_metadata,
        templates: vault.templates.clone(),
        snippets: vault.snippets.clone(),
        active_note_id: vault.active_note_id.clone(),
        selected_folder_id: vault.selected_folder_id.clone(),
        editor_mode: normalize_editor_mode(&vault.editor_mode),
        last_committed_transaction_id: old_state.last_committed_transaction_id.clone(),
    };

    let needs_transaction = !paths_to_replace.is_empty()
        || plans.iter().any(|plan| plan.needs_write)
        || !folder_case_renames.is_empty()
        || !created_directories.is_empty();
    if needs_transaction {
        let transaction_id = new_transaction_id();
        let (transaction_root, mut manifest) = prepare_transaction(
            &root,
            transaction_id,
            &paths_to_replace,
            &plans,
            folder_case_renames,
            created_directories,
        )?;
        if revision_for_root(&root)? != expected_revision {
            discard_private_transaction(&root, &transaction_root, &mut warnings);

            return Err(
                "The vault changed while the save transaction was being prepared. Reload it before trying again."
                    .to_owned(),
            );
        }
        manifest.phase = TransactionPhase::Applying;
        write_transaction_manifest(&transaction_root, &manifest)?;

        if let Err(error) = apply_transaction(&root, &manifest, &plans) {
            let recovered = rollback_transaction(&root, &transaction_root, &manifest, &mut warnings);
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(error);
        }
        if let Err(error) = verify_save_consistency(&root, &consistency) {
            let recovered = rollback_transaction(&root, &transaction_root, &manifest, &mut warnings);
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(error);
        }
        if fingerprint_regular_file(&state_path)? != expected_state_fingerprint {
            let recovered = rollback_transaction(&root, &transaction_root, &manifest, &mut warnings);
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(
                "Workspace metadata changed outside Obsidian At Home while saving. Reload before editing again."
                    .to_owned(),
            );
        }

        state.last_committed_transaction_id = Some(manifest.id.clone());
        if let Err(error) = write_workspace_state(&root, &state) {
            let recovered = rollback_transaction(&root, &transaction_root, &manifest, &mut warnings);
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(format!("Could not save workspace metadata: {error}"));
        }
        // The state file is the commit boundary. Persist the same fact in the
        // manifest before cleanup so an undeletable old transaction can never
        // be mistaken for an uncommitted save after a later transaction.
        manifest.phase = TransactionPhase::Committed;
        write_transaction_manifest(&transaction_root, &manifest).map_err(|error| {
            format!(
                "The vault was saved, but its transaction could not be finalized. Reopen the vault before editing again. {error}"
            )
        })?;
        if let Err(error) = verify_save_consistency(&root, &consistency) {
            return Err(format!(
                "The vault changed as the save was committed. Reload before editing again. {error}"
            ));
        }
        discard_private_transaction(&root, &transaction_root, &mut warnings);
    } else {
        verify_save_consistency(&root, &consistency)?;
        if fingerprint_regular_file(&state_path)? != expected_state_fingerprint {
            return Err(
                "Workspace metadata changed outside Obsidian At Home while saving. Reload before editing again."
                    .to_owned(),
            );
        }
        write_workspace_state(&root, &state)?;
    }

    remove_empty_managed_directories(&root, &old_state.folder_paths, &state.folder_paths, &mut warnings);
    verify_save_consistency(&root, &consistency)?;
    let revision = revision_for_root(&root)?;
    verify_save_consistency(&root, &consistency)?;
    if revision_for_root(&root)? != revision {
        return Err(
            "The vault changed immediately after saving. Reload it before editing again."
                .to_owned(),
        );
    }
    Ok(SaveResult {
        revision,
        saved_at,
        warnings: warnings.finish(),
    })
}

fn build_note_write_plans(
    root: &Path,
    vault: &VaultData,
    old_state: &WorkspaceState,
    folder_paths: &BTreeMap<String, String>,
) -> Result<Vec<NoteWritePlan>, String> {
    if vault.notes.len() > MAX_NOTES {
        return Err(format!("A vault can contain at most {MAX_NOTES} Markdown notes."));
    }
    let mut plans = Vec::with_capacity(vault.notes.len());
    let mut desired_paths = HashSet::new();
    let mut note_ids = HashSet::new();
    let mut total_note_bytes = 0_u64;

    for note in &vault.notes {
        if note.id.trim().is_empty() || !note_ids.insert(note.id.clone()) {
            return Err("Every note must have a unique, non-empty ID.".to_owned());
        }
        if note.content.len() as u64 > MAX_NOTE_BYTES {
            return Err(format!(
                "The note {:?} is larger than {} MiB.",
                note.title,
                MAX_NOTE_BYTES / 1024 / 1024
            ));
        }
        let folder_path = match note.folder_id.as_deref() {
            Some(folder_id) => folder_paths.get(folder_id).ok_or_else(|| {
                format!("The note {:?} refers to a folder that does not exist.", note.title)
            })?,
            None => "",
        };
        let old_relative_path = old_state
            .note_paths
            .get(&note.id)
            .cloned();
        let extension = old_relative_path
            .as_deref()
            .and_then(|path| Path::new(path).extension())
            .and_then(|value| value.to_str())
            .filter(|value| value.eq_ignore_ascii_case("markdown"))
            .unwrap_or("md");
        let stem = safe_file_stem(&note.title, "Untitled note");

        let preserve_old_name = old_relative_path.as_deref().is_some_and(|old_path| {
            let path = Path::new(old_path);
            let old_folder = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .and_then(path_to_slash_string)
                .unwrap_or_default();
            let old_title = path.file_stem().and_then(|value| value.to_str()).unwrap_or("");
            old_folder == folder_path && old_title == note.title
        });
        let new_relative_path = if preserve_old_name {
            old_relative_path.clone().expect("preserved path should exist")
        } else if folder_path.is_empty() {
            format!("{stem}.{extension}")
        } else {
            format!("{folder_path}/{stem}.{extension}")
        };
        validate_markdown_relative_path(&new_relative_path)?;
        let portable_key = new_relative_path.to_lowercase();
        if !desired_paths.insert(portable_key) {
            return Err(format!(
                "More than one note would be saved as {new_relative_path}. Rename one of them first."
            ));
        }

        let old_content = match old_relative_path.as_deref() {
            Some(old_path) => {
                let path = resolve_workspace_file(root, old_path, true)?;
                match fs::read_to_string(&path) {
                    Ok(content) => Some(content),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    Err(error) => {
                        return Err(format!("Could not read {old_path} before saving: {error}"));
                    }
                }
            }
            None => None,
        };
        let content = content_with_requested_tags(note, old_content.as_deref())?;
        if content.len() as u64 > MAX_NOTE_BYTES {
            return Err(format!(
                "The note {:?} is too large after writing its frontmatter.",
                note.title
            ));
        }
        total_note_bytes = total_note_bytes.saturating_add(content.len() as u64);
        if total_note_bytes > MAX_TOTAL_NOTE_BYTES {
            return Err(format!(
                "The vault contains more than {} MiB of Markdown text.",
                MAX_TOTAL_NOTE_BYTES / 1024 / 1024
            ));
        }
        let needs_write = match old_relative_path.as_deref() {
            Some(old_path) if old_path == new_relative_path => old_content
                .as_deref()
                .is_none_or(|existing| existing.as_bytes() != content.as_bytes()),
            _ => true,
        };
        plans.push(NoteWritePlan {
            id: note.id.clone(),
            old_relative_path,
            new_relative_path,
            content,
            needs_write,
        });
    }
    Ok(plans)
}

fn build_folder_paths(folders: &[Folder]) -> Result<BTreeMap<String, String>, String> {
    let by_id: HashMap<&str, &Folder> = folders
        .iter()
        .map(|folder| (folder.id.as_str(), folder))
        .collect();
    if by_id.len() != folders.len() || by_id.contains_key("") {
        return Err("Every folder must have a unique, non-empty ID.".to_owned());
    }

    fn resolve(
        id: &str,
        by_id: &HashMap<&str, &Folder>,
        result: &mut BTreeMap<String, String>,
        visiting: &mut HashSet<String>,
    ) -> Result<String, String> {
        if let Some(path) = result.get(id) {
            return Ok(path.clone());
        }
        if !visiting.insert(id.to_owned()) {
            return Err("The folder tree contains a cycle.".to_owned());
        }
        let folder = by_id
            .get(id)
            .ok_or_else(|| "A folder refers to a parent that does not exist.".to_owned())?;
        validate_component_name(&folder.name, "folder")?;
        let path = match folder.parent_id.as_deref() {
            Some(parent_id) => format!(
                "{}/{}",
                resolve(parent_id, by_id, result, visiting)?,
                folder.name.trim()
            ),
            None => folder.name.trim().to_owned(),
        };
        validate_relative_path(&path, false)?;
        visiting.remove(id);
        result.insert(id.to_owned(), path.clone());
        Ok(path)
    }

    let mut result = BTreeMap::new();
    for folder in folders {
        resolve(&folder.id, &by_id, &mut result, &mut HashSet::new())?;
    }
    let mut portable_paths = HashSet::new();
    for path in result.values() {
        if !portable_paths.insert(path.to_lowercase()) {
            return Err(format!("More than one folder would be saved as {path}."));
        }
    }
    Ok(result)
}

fn scan_markdown_files(
    root: &Path,
    warnings: &mut WarningCollector,
) -> Result<(Vec<ScannedNote>, Vec<ScannedFolder>), String> {
    let mut notes = Vec::new();
    let mut folders = Vec::new();
    let mut total_bytes = 0_u64;
    let walker = WalkDir::new(root)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
        .filter_entry(should_visit_workspace_entry);

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("Could not inspect a vault entry: {error}"));
                continue;
            }
        };
        if entry.depth() == 0 || entry.file_type().is_symlink() {
            continue;
        }
        let relative_path = match entry
            .path()
            .strip_prefix(root)
            .ok()
            .and_then(path_to_slash_string)
        {
            Some(path) if validate_relative_path(&path, entry.file_type().is_file()).is_ok() => {
                path
            }
            _ => {
                warnings.push(format!(
                    "Skipped a vault entry with an unsupported path: {}",
                    entry.path().display()
                ));
                continue;
            }
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!("Could not inspect {relative_path}: {error}"));
                continue;
            }
        };
        if entry.file_type().is_dir() {
            folders.push(ScannedFolder {
                relative_path,
                created_at: metadata_time_millis(&metadata, true),
            });
            continue;
        }
        if !entry.file_type().is_file() || !is_markdown_path(entry.path()) {
            continue;
        }
        if notes.len() >= MAX_NOTES {
            warnings.push(format!("Stopped after {MAX_NOTES} Markdown notes."));
            break;
        }
        if metadata.len() > MAX_NOTE_BYTES {
            warnings.push(format!(
                "Skipped {relative_path} because it is larger than {} MiB.",
                MAX_NOTE_BYTES / 1024 / 1024
            ));
            continue;
        }
        if total_bytes.saturating_add(metadata.len()) > MAX_TOTAL_NOTE_BYTES {
            warnings.push(format!(
                "Stopped after reading {} MiB of Markdown notes.",
                MAX_TOTAL_NOTE_BYTES / 1024 / 1024
            ));
            break;
        }
        let content = match fs::read_to_string(entry.path()) {
            Ok(content) => content,
            Err(error) => {
                warnings.push(format!(
                    "Skipped {relative_path} because it is not readable UTF-8: {error}"
                ));
                continue;
            }
        };
        total_bytes += metadata.len();
        let tags = parse_frontmatter_tags(&content);
        notes.push(ScannedNote {
            relative_path,
            content,
            created_at: metadata_time_millis(&metadata, true),
            updated_at: metadata_time_millis(&metadata, false),
            tags,
        });
    }

    notes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    folders.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok((notes, folders))
}

fn read_workspace_state(
    root: &Path,
    warnings: &mut WarningCollector,
) -> (Option<WorkspaceState>, bool) {
    let path = workspace_state_path(root);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return (None, false),
        Err(error) => {
            warnings.push(format!("Could not inspect workspace metadata: {error}"));

            return (None, false);
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        warnings.push("Ignored workspace metadata because it is not a regular file.".to_owned());

        return (None, true);
    }
    if metadata.len() > 64 * 1024 * 1024 {
        warnings.push("Ignored workspace metadata because it is unexpectedly large.".to_owned());

        return (None, true);
    }
    match fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            serde_json::from_slice::<WorkspaceState>(&bytes).map_err(|error| error.to_string())
        })
    {
        Ok(state) if state.version <= STATE_VERSION => (Some(state), true),
        Ok(state) => {
            warnings.push(format!(
                "Workspace metadata uses version {}, but this app supports up to version {STATE_VERSION}. It was opened read-only and was not changed.",
                state.version
            ));
            (None, true)
        }
        Err(error) => {
            warnings.push(format!("Ignored invalid workspace metadata: {error}"));
            (None, true)
        }
    }
}

fn write_workspace_state(root: &Path, state: &WorkspaceState) -> Result<(), String> {
    let directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &directory)?;
    let mut bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("Could not encode workspace metadata: {error}"))?;
    bytes.push(b'\n');
    atomic_write(&directory.join(STATE_FILE), &bytes)
        .map_err(|error| format!("Could not write workspace metadata: {error}"))
}

fn read_registry(app: &AppHandle) -> Result<WorkspaceRegistry, String> {
    let path = registry_path(app)?;
    match fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<WorkspaceRegistry>(&bytes) {
            Ok(registry) if registry.version <= REGISTRY_VERSION => Ok(registry),
            Ok(registry) => Err(format!(
                "The vault list uses version {}, but this app supports up to version {REGISTRY_VERSION}. Update the app before changing the vault list.",
                registry.version
            )),
            Err(parse_error) => {
                let backup = unique_registry_backup_path(&path);
                fs::rename(&path, &backup).map_err(|backup_error| {
                    format!(
                        "The vault list is invalid ({parse_error}) and could not be preserved at {}: {backup_error}",
                        backup.display()
                    )
                })?;
                Ok(WorkspaceRegistry::default())
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(WorkspaceRegistry::default()),
        Err(error) => Err(format!("Could not read the vault list: {error}")),
    }
}

fn unique_registry_backup_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("workspaces");
    for index in 0..10_000_u32 {
        let suffix = if index == 0 {
            now_millis().to_string()
        } else {
            format!("{}-{index}", now_millis())
        };
        let candidate = parent.join(format!("{stem}.corrupt-{suffix}.json"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem}.corrupt-{}.json", std::process::id()))
}

fn write_registry(app: &AppHandle, registry: &WorkspaceRegistry) -> Result<(), String> {
    let path = registry_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create the app configuration folder: {error}"))?;
    }
    let mut bytes = serde_json::to_vec_pretty(registry)
        .map_err(|error| format!("Could not encode the vault list: {error}"))?;
    bytes.push(b'\n');
    atomic_write(&path, &bytes).map_err(|error| format!("Could not save the vault list: {error}"))
}

fn remember_workspace(registry: &mut WorkspaceRegistry, descriptor: &VaultDescriptor) {
    let descriptor_path = canonical_path_if_available(&descriptor.path);
    registry.recent_vaults.retain(|existing| {
        canonical_path_if_available(&existing.path) != descriptor_path
    });
    registry.recent_vaults.push(descriptor.clone());
    registry.active_path = Some(descriptor.path.clone());
    sort_descriptors(&mut registry.recent_vaults);
    registry.recent_vaults.truncate(50);
}

fn sort_descriptors(descriptors: &mut [VaultDescriptor]) {
    descriptors.sort_by(|left, right| {
        right
            .last_opened_at
            .cmp(&left.last_opened_at)
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn reverse_valid_paths(
    paths: &BTreeMap<String, String>,
    kind: &str,
    warnings: &mut WarningCollector,
) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for (id, path) in paths {
        if id.trim().is_empty() || validate_relative_path(path, kind == "note").is_err() {
            warnings.push(format!("Ignored an invalid {kind} path mapping."));
            continue;
        }
        if result.insert(path.clone(), id.clone()).is_some() {
            warnings.push(format!("Ignored a duplicate {kind} path mapping for {path}."));
        }
    }
    result
}

fn revision_for_root(root: &Path) -> Result<u64, String> {
    let mut entries = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
        .filter_entry(should_visit_revision_entry)
    {
        let entry = entry.map_err(|error| format!("Could not inspect the vault: {error}"))?;
        if entry.depth() == 0 || entry.file_type().is_symlink() {
            continue;
        }
        let Some(relative) = entry
            .path()
            .strip_prefix(root)
            .ok()
            .and_then(path_to_slash_string)
        else {
            continue;
        };
        if entry.file_type().is_dir() && relative != STATE_DIRECTORY {
            entries.push((format!("D:{relative}"), None));
        } else if entry.file_type().is_file()
            && (is_markdown_path(entry.path()) || relative == format!("{STATE_DIRECTORY}/{STATE_FILE}"))
        {
            let metadata = entry.metadata().map_err(|error| {
                format!("Could not inspect {}: {error}", entry.path().display())
            })?;
            let modified_nanos = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            entries.push((
                format!("F:{relative}"),
                Some((metadata.len(), modified_nanos)),
            ));
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hash = 0xcbf29ce484222325_u64;
    for (label, metadata) in entries {
        fnv_update(&mut hash, label.as_bytes());
        fnv_update(&mut hash, &[0]);
        if let Some((length, modified_nanos)) = metadata {
            fnv_update(&mut hash, &length.to_le_bytes());
            fnv_update(&mut hash, &modified_nanos.to_le_bytes());
        }
        fnv_update(&mut hash, &[0xff]);
    }
    let revision = hash & MAX_SAFE_JAVASCRIPT_INTEGER;
    Ok(if revision == 0 { 1 } else { revision })
}

fn fnv_update(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn content_with_requested_tags(note: &Note, old_content: Option<&str>) -> Result<String, String> {
    let requested_tags = normalize_tags(&note.tags);
    if parse_frontmatter_tags(&note.content) == requested_tags {
        return Ok(note.content.clone());
    }

    let action = if old_content.is_some() { "update" } else { "write" };
    update_frontmatter_tags_conservatively(&note.content, &requested_tags).map_err(|error| {
        format!(
            "Could not {action} tags for {:?}: {error} Edit the tags in Markdown source instead.",
            note.title,
        )
    })
}

fn update_frontmatter_tags_conservatively(
    content: &str,
    normalized_tags: &[String],
) -> Result<String, String> {
    let Some((body_start, body_end, line_ending)) = frontmatter_bounds(content) else {
        if normalized_tags.is_empty() {
            return Ok(content.to_owned());
        }
        if content
            .strip_prefix('\u{feff}')
            .unwrap_or(content)
            .lines()
            .next()
            .is_some_and(|line| line.trim() == "---")
        {
            return Err("the existing frontmatter is not closed".to_owned());
        }
        let (bom, body) = content
            .strip_prefix('\u{feff}')
            .map_or(("", content), |body| ("\u{feff}", body));
        let line_ending = if body.contains("\r\n") { "\r\n" } else { "\n" };
        let mut output = String::from(bom);
        output.push_str("---");
        output.push_str(line_ending);
        append_tag_block(&mut output, normalized_tags, line_ending);
        output.push_str("---");
        output.push_str(line_ending);
        output.push_str(line_ending);
        output.push_str(body);

        return Ok(output);
    };

    let body = &content[body_start..body_end];
    let tag_span = find_conservative_tag_span(body)?;
    let mut new_body = String::new();
    match tag_span {
        Some((start, end)) => {
            new_body.push_str(&body[..start]);
            if !normalized_tags.is_empty() {
                append_tag_block(&mut new_body, normalized_tags, line_ending);
            }
            new_body.push_str(&body[end..]);
        }
        None => {
            new_body.push_str(body);
            if !normalized_tags.is_empty() {
                if !new_body.is_empty() && !new_body.ends_with('\n') {
                    new_body.push_str(line_ending);
                }
                append_tag_block(&mut new_body, normalized_tags, line_ending);
            }
        }
    }
    Ok(format!(
        "{}{}{}",
        &content[..body_start],
        new_body,
        &content[body_end..]
    ))
}

fn find_conservative_tag_span(body: &str) -> Result<Option<(usize, usize)>, String> {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let mut spans = Vec::new();
    let mut offset = 0;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let without_ending = trim_line_ending(line);
        let indented = without_ending.starts_with(' ') || without_ending.starts_with('\t');
        let Some((key, value)) = without_ending.split_once(':') else {
            offset += line.len();
            index += 1;
            continue;
        };
        if indented || !key.trim().eq_ignore_ascii_case("tags") {
            offset += line.len();
            index += 1;
            continue;
        }
        if value.contains('#')
            || matches!(
                value.trim().chars().next(),
                Some('&' | '*' | '!' | '|' | '>' | '{')
            )
        {
            return Err("the tags field uses comments, anchors, or complex YAML".to_owned());
        }
        let block_list = value.trim().is_empty();
        let start = offset;
        offset += line.len();
        index += 1;
        while block_list && index < lines.len() {
            let continuation = trim_line_ending(lines[index]);
            let trimmed = continuation.trim();
            let is_indented = continuation.starts_with(' ') || continuation.starts_with('\t');
            let is_unindented_list = !is_indented
                && (trimmed == "-" || trimmed.starts_with("- ") || trimmed.starts_with("-\t"));
            let is_list = is_unindented_list
                || (is_indented
                    && (trimmed == "-"
                        || trimmed.starts_with("- ")
                        || trimmed.starts_with("-\t")));
            if trimmed.is_empty() {
                offset += lines[index].len();
                index += 1;
                continue;
            }
            if is_list {
                if trimmed.starts_with('#') || trimmed.contains(" #") {
                    return Err("the tags field contains comments".to_owned());
                }
                let scalar = trimmed.trim_start_matches('-').trim();
                if matches!(
                    scalar.chars().next(),
                    Some('&' | '*' | '!' | '|' | '>' | '{' | '[')
                ) || (!scalar.starts_with(['\'', '"']) && scalar.contains(": "))
                {
                    return Err("the tags field uses complex YAML".to_owned());
                }
                offset += lines[index].len();
                index += 1;
                continue;
            }
            if is_indented {
                return Err("the tags field uses complex YAML".to_owned());
            }
            break;
        }
        spans.push((start, offset));
    }
    if spans.len() > 1 {
        return Err("frontmatter contains more than one top-level tags field".to_owned());
    }
    Ok(spans.into_iter().next())
}

fn append_tag_block(output: &mut String, tags: &[String], line_ending: &str) {
    output.push_str("tags:");
    output.push_str(line_ending);
    for tag in tags {
        output.push_str("  - \"");
        output.push_str(&escape_yaml_double_quoted(tag));
        output.push('"');
        output.push_str(line_ending);
    }
}

fn parse_frontmatter_tags(content: &str) -> Vec<String> {
    let Some((body_start, body_end, _)) = frontmatter_bounds(content) else {
        return Vec::new();
    };
    let body = &content[body_start..body_end];
    let mut tags = Vec::new();
    let mut reading_tag_list = false;
    for raw_line in body.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indented = raw_line.starts_with(' ') || raw_line.starts_with('\t');
        if reading_tag_list
            && (indented
                || trimmed == "-"
                || trimmed.starts_with("- ")
                || trimmed.starts_with("-\t"))
        {
            if let Some(value) = trimmed.strip_prefix('-') {
                push_tag(&mut tags, parse_yaml_scalar(value.trim()));
            }
            continue;
        }
        reading_tag_list = false;
        if indented {
            continue;
        }
        let Some((key, value)) = raw_line.split_once(':') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("tags") {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            reading_tag_list = true;
        } else if value.starts_with('[') {
            for value in parse_inline_yaml_list(value) {
                push_tag(&mut tags, value);
            }
        } else {
            push_tag(&mut tags, parse_yaml_scalar(value));
        }
    }
    tags
}

fn frontmatter_bounds(content: &str) -> Option<(usize, usize, &str)> {
    let bom_length = if content.starts_with('\u{feff}') { 3 } else { 0 };
    let remaining = &content[bom_length..];
    let first_end = remaining.find('\n').map(|index| index + 1).unwrap_or(remaining.len());
    let first = &remaining[..first_end];
    if trim_line_ending(first).trim() != "---" {
        return None;
    }
    let line_ending = if first.ends_with("\r\n") { "\r\n" } else { "\n" };
    let body_start = bom_length + first_end;
    let mut cursor = body_start;
    for line in content[body_start..].split_inclusive('\n') {
        let trimmed = trim_line_ending(line).trim();
        if trimmed == "---" || trimmed == "..." {
            return Some((body_start, cursor, line_ending));
        }
        cursor += line.len();
    }
    None
}

fn parse_inline_yaml_list(value: &str) -> Vec<String> {
    let Some(end) = value.rfind(']') else {
        return vec![parse_yaml_scalar(value)];
    };
    let inner = &value[1..end];
    let mut values = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('"') {
            escaped = true;
        } else if character == '\'' || character == '"' {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        } else if character == ',' && quote.is_none() {
            values.push(parse_yaml_scalar(inner[start..index].trim()));
            start = index + 1;
        }
    }
    values.push(parse_yaml_scalar(inner[start..].trim()));
    values
}

fn parse_yaml_scalar(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].replace("''", "'");
    }
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let mut output = String::new();
        let mut escaped = false;
        for character in value[1..value.len() - 1].chars() {
            if escaped {
                output.push(match character {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                });
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else {
                output.push(character);
            }
        }

        return output;
    }
    value
        .find(" #")
        .map(|index| &value[..index])
        .unwrap_or(value)
        .trim()
        .to_owned()
}

fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for tag in tags {
        push_tag(&mut result, tag.clone());
    }
    result
}

fn push_tag(tags: &mut Vec<String>, tag: String) {
    let tag = tag.trim().trim_start_matches('#').trim();
    if !tag.is_empty() && !tags.iter().any(|existing| existing == tag) {
        tags.push(tag.to_owned());
    }
}

fn escape_yaml_double_quoted(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            other if other.is_control() => output.push(' '),
            other => output.push(other),
        }
    }
    output
}

fn remove_empty_managed_directories(
    root: &Path,
    old_paths: &BTreeMap<String, String>,
    new_paths: &BTreeMap<String, String>,
    warnings: &mut WarningCollector,
) {
    let desired: HashSet<String> = new_paths
        .values()
        .map(|path| portable_path_key(path))
        .collect();
    let mut obsolete: Vec<&String> = old_paths
        .values()
        .filter(|path| !desired.contains(&portable_path_key(path)))
        .collect();
    obsolete.sort_by_key(|path| std::cmp::Reverse(path.split('/').count()));
    for relative_path in obsolete {
        let Ok(path) = resolve_workspace_directory(root, relative_path) else {
            continue;
        };
        match remove_directory_durable(&path) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    || error.kind() == io::ErrorKind::DirectoryNotEmpty => {}
            Err(error) => warnings.push(format!(
                "Could not remove the empty folder {relative_path}: {error}"
            )),
        }
    }
}

fn validate_managed_path_ownership(paths: &BTreeMap<String, String>) -> Result<(), String> {
    let mut portable_paths = HashSet::new();
    for path in paths.values() {
        validate_markdown_relative_path(path)?;
        if !portable_paths.insert(portable_path_key(path)) {
            return Err(format!(
                "Workspace metadata contains more than one managed note for {path}."
            ));
        }
    }
    Ok(())
}

fn validate_save_targets(
    root: &Path,
    plans: &[NoteWritePlan],
    paths_to_replace: &BTreeSet<String>,
    managed_paths: &BTreeMap<String, String>,
) -> Result<(), String> {
    let replace_keys: HashSet<String> = paths_to_replace
        .iter()
        .map(|path| portable_path_key(path))
        .collect();
    let managed_keys: HashSet<String> = managed_paths
        .values()
        .map(|path| portable_path_key(path))
        .collect();
    for plan in plans.iter().filter(|plan| plan.needs_write) {
        let target = resolve_workspace_file(root, &plan.new_relative_path, true)?;
        if fs::symlink_metadata(&target).is_ok() {
            let key = portable_path_key(&plan.new_relative_path);
            if !replace_keys.contains(&key) || !managed_keys.contains(&key) {
                return Err(format!(
                    "Cannot save {:?} because {} already exists and is not owned by this vault.",
                    plan.id, plan.new_relative_path
                ));
            }
        }
    }
    Ok(())
}

fn build_folder_case_renames(
    old_paths: &BTreeMap<String, String>,
    new_paths: &BTreeMap<String, String>,
) -> Result<Vec<FolderCaseRename>, String> {
    let mut candidates: Vec<(String, String)> = old_paths
        .iter()
        .filter_map(|(id, old_path)| {
            let new_path = new_paths.get(id)?;
            (old_path != new_path && portable_path_key(old_path) == portable_path_key(new_path))
                .then(|| (old_path.clone(), new_path.clone()))
        })
        .collect();
    candidates.sort_by_key(|(old_path, _)| old_path.split('/').count());

    let mut operations: Vec<FolderCaseRename> = Vec::new();
    for (old_path, new_path) in candidates {
        let mut current_from = old_path;
        for operation in &operations {
            if current_from == operation.from_relative_path {
                current_from = operation.to_relative_path.clone();
                continue;
            }
            let prefix = format!("{}/", operation.from_relative_path);
            if let Some(remainder) = current_from.strip_prefix(&prefix) {
                current_from = format!("{}/{remainder}", operation.to_relative_path);
            }
        }
        if current_from == new_path {
            continue;
        }
        validate_relative_path(&current_from, false)?;
        validate_relative_path(&new_path, false)?;
        if portable_path_key(&current_from) != portable_path_key(&new_path) {
            return Err("A case-only folder rename could not be planned safely.".to_owned());
        }
        operations.push(FolderCaseRename {
            from_relative_path: current_from,
            to_relative_path: new_path,
        });
    }
    Ok(operations)
}

fn validate_folder_case_renames(
    root: &Path,
    operations: &[FolderCaseRename],
) -> Result<(), String> {
    for (index, operation) in operations.iter().enumerate() {
        // Nested case-only renames are expressed in the path produced by their
        // parent rename. Map them back to the current on-disk path while doing
        // the preflight checks; apply_transaction performs them in order.
        let source_relative = path_before_folder_renames(
            &operation.from_relative_path,
            &operations[..index],
        );
        let target_relative = path_before_folder_renames(
            &operation.to_relative_path,
            &operations[..index],
        );
        let source = resolve_workspace_directory(root, &source_relative)?;
        let metadata = fs::symlink_metadata(&source).map_err(|error| {
            format!(
                "Could not inspect the folder {} before renaming it: {error}",
                operation.from_relative_path
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "{} is not a regular folder.",
                operation.from_relative_path
            ));
        }
        if directory_contains_nested_vault(&source) {
            return Err(format!(
                "Cannot rename {} because it contains another vault.",
                operation.from_relative_path
            ));
        }
        let target = resolve_workspace_directory(root, &target_relative)?;
        match fs::symlink_metadata(&target) {
            Ok(target_metadata) if target_metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Cannot rename {} because {} is a symbolic link.",
                    operation.from_relative_path, operation.to_relative_path
                ));
            }
            Ok(target_metadata) if !target_metadata.is_dir() => {
                return Err(format!(
                    "Cannot rename {} because {} is not a folder.",
                    operation.from_relative_path, operation.to_relative_path
                ));
            }
            Ok(_) => {
                let same_location = source
                    .canonicalize()
                    .ok()
                    .zip(target.canonicalize().ok())
                    .is_some_and(|(left, right)| left == right);
                if !same_location {
                    return Err(format!(
                        "Cannot rename {} because {} already exists.",
                        operation.from_relative_path, operation.to_relative_path
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Could not inspect {} before renaming it: {error}",
                    operation.to_relative_path
                ));
            }
        }
    }
    Ok(())
}

fn path_before_folder_renames(path: &str, operations: &[FolderCaseRename]) -> String {
    let mut current = path.to_owned();
    for operation in operations.iter().rev() {
        if current == operation.to_relative_path {
            current = operation.from_relative_path.clone();
            continue;
        }
        let prefix = format!("{}/", operation.to_relative_path);
        if let Some(remainder) = current.strip_prefix(&prefix) {
            current = format!("{}/{remainder}", operation.from_relative_path);
        }
    }
    current
}

fn collect_created_directories<'a>(
    root: &Path,
    desired_paths: impl Iterator<Item = &'a String>,
    case_renames: &[FolderCaseRename],
) -> Result<Vec<String>, String> {
    let rename_targets: HashSet<String> = case_renames
        .iter()
        .map(|operation| portable_path_key(&operation.to_relative_path))
        .collect();
    let mut created = BTreeSet::new();
    for desired_path in desired_paths {
        validate_relative_path(desired_path, false)?;
        let mut prefix = String::new();
        for component in desired_path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if rename_targets.contains(&portable_path_key(&prefix)) {
                continue;
            }
            let path = root.join(checked_relative_path(&prefix, false)?);
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(format!("Refusing to use the symbolic link {}.", path.display()));
                }
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => return Err(format!("{} is not a folder.", path.display())),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    created.insert(prefix.clone());
                }
                Err(error) => {
                    return Err(format!("Could not inspect {}: {error}", path.display()));
                }
            }
        }
    }
    Ok(created.into_iter().collect())
}

fn prepare_transaction(
    root: &Path,
    id: String,
    paths_to_replace: &BTreeSet<String>,
    plans: &[NoteWritePlan],
    folder_case_renames: Vec<FolderCaseRename>,
    created_directories: Vec<String>,
) -> Result<(PathBuf, TransactionManifest), String> {
    let transaction_root = prepare_transaction_root(root, &id)?;
    let mut originals = Vec::new();
    for relative_path in paths_to_replace {
        let source = resolve_workspace_file(root, relative_path, true)?;
        let metadata = match fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("Could not inspect {relative_path}: {error}"));
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!("{relative_path} is not a regular Markdown file."));
        }
        let bytes = fs::read(&source)
            .map_err(|error| format!("Could not back up {relative_path}: {error}"))?;
        let backup_relative_path = format!("backups/{relative_path}");
        let backup = transaction_root.join(
            checked_internal_transaction_path(&backup_relative_path, true)?,
        );
        if let Some(parent) = backup.parent() {
            ensure_private_directory_tree(&transaction_root, parent)
                .map_err(|error| format!("Could not prepare a note backup: {error}"))?;
        }
        atomic_write(&backup, &bytes)
            .map_err(|error| format!("Could not back up {relative_path}: {error}"))?;
        originals.push(TransactionOriginal {
            relative_path: relative_path.clone(),
            backup_relative_path,
            fingerprint: fingerprint_bytes(&bytes),
        });
    }
    let targets = plans
        .iter()
        .filter(|plan| plan.needs_write)
        .map(|plan| TransactionTarget {
            relative_path: plan.new_relative_path.clone(),
            fingerprint: fingerprint_bytes(plan.content.as_bytes()),
        })
        .collect();
    let manifest = TransactionManifest {
        version: TRANSACTION_VERSION,
        id,
        phase: TransactionPhase::Prepared,
        originals,
        targets,
        folder_case_renames,
        created_directories,
    };
    write_transaction_manifest(&transaction_root, &manifest)?;
    Ok((transaction_root, manifest))
}

fn apply_transaction(
    root: &Path,
    manifest: &TransactionManifest,
    plans: &[NoteWritePlan],
) -> Result<(), String> {
    for original in &manifest.originals {
        let source = resolve_workspace_file(root, &original.relative_path, true)?;
        let current = fingerprint_regular_file(&source)?.ok_or_else(|| {
            format!("{} disappeared while saving.", original.relative_path)
        })?;
        if current != original.fingerprint {
            return Err(format!(
                "{} changed in another app while saving. Reload the vault before trying again.",
                original.relative_path
            ));
        }
        remove_file_durable(&source).map_err(|error| {
            format!("Could not replace {}: {error}", original.relative_path)
        })?;
    }

    for operation in &manifest.folder_case_renames {
        let source = resolve_workspace_directory(root, &operation.from_relative_path)?;
        let target = root.join(checked_relative_path(
            &operation.to_relative_path,
            false,
        )?);
        rename_durable(&source, &target).map_err(|error| {
            format!(
                "Could not rename {} to {}: {error}",
                operation.from_relative_path, operation.to_relative_path
            )
        })?;
    }
    for relative_path in &manifest.created_directories {
        ensure_directory_path(root, relative_path)?;
    }
    for plan in plans.iter().filter(|plan| plan.needs_write) {
        let target = resolve_workspace_file(root, &plan.new_relative_path, true)?;
        if fs::symlink_metadata(&target).is_ok() {
            return Err(format!(
                "{} appeared while saving. It was not overwritten.",
                plan.new_relative_path
            ));
        }
        if let Some(parent) = target.parent() {
            ensure_existing_directory_without_symlink(root, parent)?;
        }
        atomic_write(&target, plan.content.as_bytes())
            .map_err(|error| format!("Could not save {}: {error}", plan.new_relative_path))?;
    }
    Ok(())
}

fn rollback_transaction(
    root: &Path,
    transaction_root: &Path,
    manifest: &TransactionManifest,
    warnings: &mut WarningCollector,
) -> bool {
    let mut recovered = true;
    for target in manifest.targets.iter().rev() {
        let Ok(path) = resolve_workspace_file(root, &target.relative_path, true) else {
            recovered = false;
            continue;
        };
        match fingerprint_regular_file(&path) {
            Ok(Some(current)) if current == target.fingerprint => {
                if let Err(error) = remove_file_durable(&path) {
                    warnings.push(format!(
                        "Could not remove the partial save {}: {error}",
                        target.relative_path
                    ));
                    recovered = false;
                }
            }
            Ok(Some(_)) => {
                warnings.push(format!(
                    "Did not remove {} because it changed after the interrupted save.",
                    target.relative_path
                ));
                recovered = false;
            }
            Ok(None) => {}
            Err(error) => {
                warnings.push(error);
                recovered = false;
            }
        }
    }

    for (index, operation) in manifest.folder_case_renames.iter().enumerate().rev() {
        if !rollback_folder_case_rename(
            root,
            operation,
            &manifest.folder_case_renames[..index],
            warnings,
        ) {
            recovered = false;
        }
    }
    for original in &manifest.originals {
        let backup = match resolve_transaction_backup(transaction_root, original) {
            Ok(path) => path,
            Err(error) => {
                warnings.push(error);
                recovered = false;
                continue;
            }
        };
        let bytes = match fs::read(&backup) {
            Ok(bytes) if fingerprint_bytes(&bytes) == original.fingerprint => bytes,
            Ok(_) => {
                warnings.push(format!(
                    "The backup for {} did not match its manifest.",
                    original.relative_path
                ));
                recovered = false;
                continue;
            }
            Err(error) => {
                warnings.push(format!(
                    "Could not read the backup for {}: {error}",
                    original.relative_path
                ));
                recovered = false;
                continue;
            }
        };
        let original_path = match resolve_workspace_file(root, &original.relative_path, true) {
            Ok(path) => path,
            Err(error) => {
                warnings.push(error);
                recovered = false;
                continue;
            }
        };
        match fingerprint_regular_file(&original_path) {
            Ok(Some(current)) if current == original.fingerprint => {}
            Ok(Some(_)) => {
                warnings.push(format!(
                    "Did not restore {} because another file now occupies that path.",
                    original.relative_path
                ));
                recovered = false;
            }
            Ok(None) => {
                if let Some(parent) = original_path.parent() {
                    if let Err(error) = ensure_existing_directory_without_symlink(root, parent) {
                        warnings.push(error);
                        recovered = false;
                        continue;
                    }
                }
                if let Err(error) = atomic_write(&original_path, &bytes) {
                    warnings.push(format!(
                        "Could not restore {}: {error}",
                        original.relative_path
                    ));
                    recovered = false;
                }
            }
            Err(error) => {
                warnings.push(error);
                recovered = false;
            }
        }
    }
    remove_created_directories(root, &manifest.created_directories, warnings);
    recovered
}

fn rollback_folder_case_rename(
    root: &Path,
    operation: &FolderCaseRename,
    prior_operations: &[FolderCaseRename],
    warnings: &mut WarningCollector,
) -> bool {
    let Ok(from) = resolve_workspace_directory(root, &operation.from_relative_path) else {
        return false;
    };
    let Ok(to_relative) = checked_relative_path(&operation.to_relative_path, false) else {
        return false;
    };
    let to = root.join(to_relative);
    let from_exists = from.is_dir();
    let to_exists = to.is_dir();
    if from_exists && !to_exists {
        return true;
    }
    if !to_exists {
        let original_relative = path_before_folder_renames(
            &operation.from_relative_path,
            prior_operations,
        );
        if original_relative != operation.from_relative_path {
            if let Ok(original) = resolve_workspace_directory(root, &original_relative) {
                if fs::symlink_metadata(original)
                    .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
                    .unwrap_or(false)
                {
                    return true;
                }
            }
        }
        warnings.push(format!(
            "Could not find {} while recovering a folder rename.",
            operation.to_relative_path
        ));

        return false;
    }
    if from_exists {
        let same_location = from
            .canonicalize()
            .ok()
            .zip(to.canonicalize().ok())
            .is_some_and(|(left, right)| left == right);
        if !same_location {
            warnings.push(format!(
                "Did not restore {} because both folder names now exist.",
                operation.from_relative_path
            ));

            return false;
        }
    }
    if let Err(error) = rename_durable(&to, &from) {
        warnings.push(format!(
            "Could not restore folder {}: {error}",
            operation.from_relative_path
        ));

        return false;
    }
    true
}

fn remove_created_directories(
    root: &Path,
    directories: &[String],
    warnings: &mut WarningCollector,
) {
    let mut directories = directories.to_vec();
    directories.sort_by_key(|path| std::cmp::Reverse(path.split('/').count()));
    for relative_path in directories {
        let Ok(path) = resolve_workspace_directory(root, &relative_path) else {
            continue;
        };
        match remove_directory_durable(&path) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    || error.kind() == io::ErrorKind::DirectoryNotEmpty => {}
            Err(error) => warnings.push(format!(
                "Could not remove temporary folder {relative_path}: {error}"
            )),
        }
    }
}

fn recover_workspace_transactions(
    root: &Path,
    state: Option<&WorkspaceState>,
    warnings: &mut WarningCollector,
) -> Result<(), String> {
    let transactions_root = root.join(STATE_DIRECTORY).join(TRANSACTIONS_DIRECTORY);
    let metadata = match fs::symlink_metadata(&transactions_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Could not inspect save transactions: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The save transactions path is not a regular folder.".to_owned());
    }
    let entries = fs::read_dir(&transactions_root)
        .map_err(|error| format!("Could not read save transactions: {error}"))?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("Could not inspect a save transaction: {error}"));
                continue;
            }
        };
        let transaction_root = entry.path();
        let metadata = match fs::symlink_metadata(&transaction_root) {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!("Could not inspect a save transaction: {error}"));
                continue;
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            warnings.push(format!(
                "Ignored an unsafe save transaction at {}.",
                transaction_root.display()
            ));
            continue;
        }
        let mut manifest = match read_transaction_manifest(&transaction_root) {
            Ok(Some(manifest)) => manifest,
            Ok(None) => {
                warnings.push("Removed an incomplete transaction that had not changed the vault.".to_owned());
                discard_private_transaction(root, &transaction_root, warnings);
                continue;
            }
            Err(error) => {
                warnings.push(error);
                continue;
            }
        };
        if manifest.version > TRANSACTION_VERSION {
            warnings.push(format!(
                "A save transaction uses unsupported version {} and was left untouched.",
                manifest.version
            ));
            continue;
        }
        if entry.file_name().to_string_lossy() != manifest.id {
            warnings.push("A save transaction ID did not match its folder and was left untouched.".to_owned());
            continue;
        }
        let committed = state
            .and_then(|state| state.last_committed_transaction_id.as_deref())
            == Some(manifest.id.as_str());
        if committed && manifest.phase != TransactionPhase::Committed {
            manifest.phase = TransactionPhase::Committed;
            write_transaction_manifest(&transaction_root, &manifest).map_err(|error| {
                format!(
                    "A committed save could not be finalized safely. Repair permissions for {} and reopen the vault. {error}",
                    transaction_root.display()
                )
            })?;
        }
        if committed || manifest.phase == TransactionPhase::Committed {
            discard_private_transaction(root, &transaction_root, warnings);
            warnings.push("Finished cleaning up a previously committed save.".to_owned());
            continue;
        }
        if manifest.phase == TransactionPhase::Prepared {
            discard_private_transaction(root, &transaction_root, warnings);
            continue;
        }
        let recovered = rollback_transaction(root, &transaction_root, &manifest, warnings);
        if recovered {
            discard_private_transaction(root, &transaction_root, warnings);
            warnings.push("Recovered an interrupted save without changing the vault.".to_owned());
        } else {
            return Err(format!(
                "An interrupted save could not be fully recovered. Its backups remain at {}. Resolve the conflicting files before reopening this vault.",
                transaction_root.display()
            ));
        }
    }
    Ok(())
}

fn write_transaction_manifest(
    transaction_root: &Path,
    manifest: &TransactionManifest,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("Could not encode the save transaction: {error}"))?;
    if bytes.len() as u64 > MAX_TRANSACTION_MANIFEST_BYTES {
        return Err("The save transaction is too large to recover safely.".to_owned());
    }
    bytes.push(b'\n');
    atomic_write(&transaction_root.join(TRANSACTION_MANIFEST_FILE), &bytes)
        .map_err(|error| format!("Could not write the save transaction: {error}"))
}

fn read_transaction_manifest(
    transaction_root: &Path,
) -> Result<Option<TransactionManifest>, String> {
    let path = transaction_root.join(TRANSACTION_MANIFEST_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not inspect a save manifest: {error}")),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_TRANSACTION_MANIFEST_BYTES
    {
        return Err("A save transaction manifest is unsafe or unexpectedly large.".to_owned());
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("Could not read a save transaction: {error}"))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("Could not parse a save transaction: {error}"))
}

fn discard_private_transaction(
    root: &Path,
    transaction_root: &Path,
    warnings: &mut WarningCollector,
) {
    let expected_parent = root.join(STATE_DIRECTORY).join(TRANSACTIONS_DIRECTORY);
    if transaction_root.parent() != Some(expected_parent.as_path()) {
        warnings.push("Refused to clean a transaction outside the private save folder.".to_owned());

        return;
    }
    let mut entries = Vec::new();
    for entry in WalkDir::new(transaction_root)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
    {
        match entry {
            Ok(entry) => entries.push(entry),
            Err(error) => warnings.push(format!("Could not inspect transaction cleanup: {error}")),
        }
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.depth()));
    for entry in entries {
        let path = entry.path();
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                warnings.push(format!("Could not inspect {}: {error}", path.display()));
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            warnings.push(format!("Refused to follow a symbolic link at {}.", path.display()));
            continue;
        }
        let result = if metadata.is_dir() {
            remove_directory_durable(path)
        } else if metadata.is_file() {
            remove_file_durable(path)
        } else {
            continue;
        };
        if let Err(error) = result {
            if error.kind() != io::ErrorKind::DirectoryNotEmpty
                && error.kind() != io::ErrorKind::NotFound
            {
                warnings.push(format!("Could not clean {}: {error}", path.display()));
            }
        }
    }
    let _ = remove_directory_durable(&expected_parent);
}

fn resolve_transaction_backup(
    transaction_root: &Path,
    original: &TransactionOriginal,
) -> Result<PathBuf, String> {
    let relative = checked_internal_transaction_path(&original.backup_relative_path, true)?;
    let path = transaction_root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Could not inspect a transaction backup: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("A transaction backup is not a regular file.".to_owned());
    }
    Ok(path)
}

fn checked_internal_transaction_path(path: &str, file: bool) -> Result<PathBuf, String> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains('\\') {
        return Err("A transaction path is invalid.".to_owned());
    }
    let mut result = PathBuf::new();
    let components: Vec<&str> = path.split('/').collect();
    for (index, component) in components.iter().enumerate() {
        if component.is_empty() || *component == "." || *component == ".." {
            return Err("A transaction path contains unsafe segments.".to_owned());
        }
        if component.len() > 255 || component.chars().any(char::is_control) {
            return Err("A transaction path contains an unsupported segment.".to_owned());
        }
        if !file || index + 1 < components.len() {
            if component.contains('/') || component.contains('\\') {
                return Err("A transaction folder path is unsafe.".to_owned());
            }
        }
        result.push(component);
    }
    Ok(result)
}

fn build_save_consistency(
    baseline: &BTreeMap<String, FileStamp>,
    paths_to_replace: &BTreeSet<String>,
    plans: &[NoteWritePlan],
) -> Result<SaveConsistency, String> {
    let mut unaffected = baseline.clone();
    for path in paths_to_replace {
        unaffected.remove(&portable_path_key(path));
    }
    let targets: Vec<TransactionTarget> = plans
        .iter()
        .filter(|plan| plan.needs_write)
        .map(|plan| TransactionTarget {
            relative_path: plan.new_relative_path.clone(),
            fingerprint: fingerprint_bytes(plan.content.as_bytes()),
        })
        .collect();
    for target in &targets {
        unaffected.remove(&portable_path_key(&target.relative_path));
    }
    Ok(SaveConsistency { unaffected, targets })
}

fn verify_save_consistency(root: &Path, expected: &SaveConsistency) -> Result<(), String> {
    let current = note_file_stamps(root)?;
    let mut expected_keys: HashSet<String> = expected.unaffected.keys().cloned().collect();
    for (path, stamp) in &expected.unaffected {
        if current.get(path) != Some(stamp) {
            return Err(
                "A Markdown file changed outside Obsidian At Home while the vault was being saved. Reload before editing again."
                    .to_owned(),
            );
        }
    }
    for target in &expected.targets {
        let key = portable_path_key(&target.relative_path);
        expected_keys.insert(key);
        let path = resolve_workspace_file(root, &target.relative_path, true)?;
        if fingerprint_regular_file(&path)? != Some(target.fingerprint.clone()) {
            return Err(format!(
                "{} changed outside Obsidian At Home while the vault was being saved. Reload before editing again.",
                target.relative_path
            ));
        }
    }
    let current_keys: HashSet<String> = current.keys().cloned().collect();
    if current_keys != expected_keys {
        return Err(
            "Markdown files were added, removed, or renamed outside Obsidian At Home while saving. Reload before editing again."
                .to_owned(),
        );
    }
    Ok(())
}

fn note_file_stamps(root: &Path) -> Result<BTreeMap<String, FileStamp>, String> {
    let mut result = BTreeMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
        .filter_entry(should_visit_workspace_entry)
    {
        let entry = entry.map_err(|error| format!("Could not inspect the vault: {error}"))?;
        if entry.file_type().is_symlink() || !entry.file_type().is_file() || !is_markdown_path(entry.path()) {
            continue;
        }
        let Some(relative_path) = entry
            .path()
            .strip_prefix(root)
            .ok()
            .and_then(path_to_slash_string)
        else {
            continue;
        };
        if validate_markdown_relative_path(&relative_path).is_err() {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| format!("Could not inspect {relative_path}: {error}"))?;
        let stamp = FileStamp {
            length: metadata.len(),
            modified_nanos: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0),
        };
        if result
            .insert(portable_path_key(&relative_path), stamp)
            .is_some()
        {
            return Err(format!(
                "The vault contains paths that differ only by letter case near {relative_path}."
            ));
        }
    }
    Ok(result)
}

fn fingerprint_regular_file(path: &Path) -> Result<Option<FileFingerprint>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not inspect {}: {error}", path.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{} is not a regular file.", path.display()));
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    Ok(Some(fingerprint_bytes(&bytes)))
}

fn fingerprint_bytes(bytes: &[u8]) -> FileFingerprint {
    let mut hash = 0xcbf29ce484222325_u64;
    fnv_update(&mut hash, bytes);
    FileFingerprint {
        length: bytes.len() as u64,
        hash,
    }
}

fn portable_path_key(path: &str) -> String {
    path.to_lowercase()
}

fn new_transaction_id() -> String {
    format!(
        "{}-{}-{}",
        now_millis(),
        std::process::id(),
        TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn directory_contains_nested_vault(directory: &Path) -> bool {
    WalkDir::new(directory)
        .follow_links(false)
        .max_depth(128)
        .into_iter()
        .filter_map(Result::ok)
        .any(|entry| is_nested_vault_directory(&entry))
}

fn ensure_directory_path(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let relative = checked_relative_path(relative_path, false)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Refusing to use the symbolic link {}.",
                    current.display()
                ));
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(format!("{} is not a folder.", current.display()));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                create_directory_durable(&current).map_err(|error| {
                    format!("Could not create the folder {}: {error}", current.display())
                })?;
            }
            Err(error) => {
                return Err(format!("Could not inspect {}: {error}", current.display()));
            }
        }
    }
    Ok(current)
}

fn resolve_workspace_directory(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let relative = checked_relative_path(relative_path, false)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Refusing to use the symbolic link {}.",
                    current.display()
                ));
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(format!("{} is not a folder.", current.display())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!("Could not inspect {}: {error}", current.display()));
            }
        }
    }
    Ok(root.join(relative))
}

fn ensure_existing_directory_without_symlink(root: &Path, directory: &Path) -> Result<(), String> {
    let relative = directory
        .strip_prefix(root)
        .map_err(|_| "A note path escaped the vault.".to_owned())?;
    let relative = path_to_slash_string(relative)
        .ok_or_else(|| "A note folder path is not valid Unicode.".to_owned())?;
    if relative.is_empty() {
        return Ok(());
    }
    ensure_directory_path(root, &relative).map(|_| ())
}

fn ensure_state_directory(root: &Path, directory: &Path) -> Result<(), String> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("The .obsidian-at-home folder cannot be a symbolic link.".to_owned())
        }
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(".obsidian-at-home exists but is not a folder.".to_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_directory_durable(directory)
            .map_err(|error| format!("Could not create .obsidian-at-home: {error}")),
        Err(error) => Err(format!("Could not inspect .obsidian-at-home: {error}")),
    }?;
    if directory.parent() != Some(root) {
        return Err("The workspace metadata path escaped the vault.".to_owned());
    }
    Ok(())
}

fn prepare_transaction_root(root: &Path, transaction_id: &str) -> Result<PathBuf, String> {
    let state_directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &state_directory)?;
    if transaction_id.is_empty()
        || transaction_id.len() > 180
        || transaction_id
            .chars()
            .any(|character| !character.is_ascii_alphanumeric() && character != '-')
    {
        return Err("The save transaction ID is invalid.".to_owned());
    }
    let transactions_directory = state_directory.join(TRANSACTIONS_DIRECTORY);
    ensure_regular_directory(&transactions_directory, "save transactions")?;
    let save_directory = transactions_directory.join(transaction_id);
    match create_directory_durable(&save_directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err("A save transaction with the same ID already exists.".to_owned());
        }
        Err(error) => return Err(format!("Could not create the save transaction: {error}")),
    }
    Ok(save_directory)
}

fn ensure_regular_directory(path: &Path, label: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("The {label} folder cannot be a symbolic link."))
        }
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(format!("The {label} path is not a folder.")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_directory_durable(path)
            .map_err(|error| format!("Could not create the {label} folder: {error}")),
        Err(error) => Err(format!("Could not inspect the {label} folder: {error}")),
    }
}

fn resolve_workspace_file(
    root: &Path,
    relative_path: &str,
    allow_missing: bool,
) -> Result<PathBuf, String> {
    let relative = checked_relative_path(relative_path, true)?;
    let target = root.join(&relative);
    if let Some(parent) = target.parent() {
        let mut current = root.to_path_buf();
        for component in parent
            .strip_prefix(root)
            .map_err(|_| "A note path escaped the vault.".to_owned())?
            .components()
        {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(format!(
                        "Refusing to follow the symbolic link {}.",
                        current.display()
                    ));
                }
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => return Err(format!("{} is not a folder.", current.display())),
                Err(error) if error.kind() == io::ErrorKind::NotFound && allow_missing => break,
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => {
                    return Err(format!("Could not inspect {}: {error}", current.display()));
                }
            }
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing to use the symbolic link {}.",
                target.display()
            ));
        }
    }
    Ok(target)
}

fn checked_relative_path(relative_path: &str, file: bool) -> Result<PathBuf, String> {
    validate_relative_path(relative_path, file)?;
    let mut result = PathBuf::new();
    for component in relative_path.split('/') {
        result.push(component);
    }
    Ok(result)
}

fn validate_relative_path(relative_path: &str, file: bool) -> Result<(), String> {
    if relative_path.is_empty()
        || relative_path.starts_with('/')
        || relative_path.starts_with('\\')
        || relative_path.contains('\\')
    {
        return Err("Paths must be relative to the vault and use forward slashes.".to_owned());
    }
    let path = Path::new(relative_path);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("Parent, root, and drive-qualified paths are not allowed.".to_owned());
    }
    let components: Vec<&str> = relative_path.split('/').collect();
    if components.len() > MAX_PATH_COMPONENTS {
        return Err("The path contains too many nested folders.".to_owned());
    }
    for (index, component) in components.iter().enumerate() {
        if component.is_empty() || *component == "." || *component == ".." {
            return Err("Empty, current-directory, and parent-directory segments are not allowed."
                .to_owned());
        }
        if is_reserved_workspace_directory(component) {
            return Err("App settings, Obsidian settings, trash, and Git folders are reserved."
                .to_owned());
        }
        if component.len() > 255 || component.chars().any(char::is_control) {
            return Err("A path segment is too long or contains control characters.".to_owned());
        }
        if !file || index + 1 < components.len() {
            validate_component_name(component, "folder")?;
        }
    }
    if file && !is_markdown_path(path) {
        return Err("Note paths must end in .md or .markdown.".to_owned());
    }
    Ok(())
}

fn validate_markdown_relative_path(path: &str) -> Result<(), String> {
    validate_relative_path(path, true)
}

fn validate_component_name(name: &str, kind: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err(format!("Enter a valid {kind} name."));
    }
    if name.len() > 180
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(is_forbidden_component_character)
        || is_windows_reserved_name(name)
        || is_reserved_workspace_directory(name)
    {
        return Err(format!("The {kind} name contains characters that are not safe in a path."));
    }
    Ok(())
}

fn validate_workspace_root(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose a vault folder.".to_owned());
    }
    validate_workspace_root_path(Path::new(input))
}

fn validate_workspace_root_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("The vault path must be absolute.".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The vault folder could not be opened: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("The vault folder cannot be a symbolic link.".to_owned());
    }
    if !metadata.is_dir() {
        return Err("The vault path is not a folder.".to_owned());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("The vault folder could not be resolved: {error}"))?;
    if canonical.parent().is_none() {
        return Err("The filesystem root cannot be used as a vault.".to_owned());
    }
    if canonical
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(is_reserved_workspace_directory)
    {
        return Err("An app settings, trash, or Git folder cannot be opened as a vault.".to_owned());
    }
    Ok(canonical)
}

fn reject_home_vault(app: &AppHandle, root: &Path) -> Result<(), String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Could not resolve the home folder: {error}"))?;
    if home
        .canonicalize()
        .ok()
        .as_deref()
        .is_some_and(|home| home == root)
    {
        return Err(
            "Your home folder is too broad to use as a vault. Create or choose a dedicated folder inside it instead."
                .to_owned(),
        );
    }
    Ok(())
}

fn reject_nested_registered_vault(
    root: &Path,
    registry: &WorkspaceRegistry,
    allow_same: bool,
) -> Result<(), String> {
    for registered in &registry.recent_vaults {
        let Ok(registered_root) = Path::new(&registered.path).canonicalize() else {
            continue;
        };
        if root == registered_root && allow_same {
            continue;
        }
        if root == registered_root
            || root.starts_with(&registered_root)
            || registered_root.starts_with(root)
        {
            return Err(format!(
                "Vaults cannot be nested. Choose a folder outside {}.",
                registered_root.display()
            ));
        }
    }
    Ok(())
}

fn validate_parent_directory(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose a parent folder for the new vault.".to_owned());
    }
    let path = Path::new(input);
    if !path.is_absolute() {
        return Err("The parent folder path must be absolute.".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The parent folder could not be opened: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The parent path must be a regular folder, not a symbolic link.".to_owned());
    }
    path.canonicalize()
        .map_err(|error| format!("The parent folder could not be resolved: {error}"))
}

fn should_visit_workspace_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if entry.file_type().is_symlink() {
        return false;
    }
    if is_nested_vault_directory(entry) {
        return false;
    }
    !entry.file_type().is_dir()
        || !is_reserved_workspace_directory(&entry.file_name().to_string_lossy())
}

fn should_visit_revision_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if entry.file_type().is_symlink() {
        return false;
    }
    if is_nested_vault_directory(entry) {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    let inside_state_directory = entry
        .path()
        .components()
        .any(|component| component.as_os_str() == STATE_DIRECTORY);
    if inside_state_directory {
        if entry.depth() == 1 && name.eq_ignore_ascii_case(STATE_DIRECTORY) {
            return true;
        }

        return entry.depth() == 2
            && !entry.file_type().is_dir()
            && name.eq_ignore_ascii_case(STATE_FILE);
    }
    if !entry.file_type().is_dir() {
        return true;
    }
    !name.eq_ignore_ascii_case(".obsidian")
        && !name.eq_ignore_ascii_case(".trash")
        && !name.eq_ignore_ascii_case(".git")
}

fn is_nested_vault_directory(entry: &DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return false;
    }
    let state_directory = entry.path().join(STATE_DIRECTORY);
    let Ok(state_directory_metadata) = fs::symlink_metadata(&state_directory) else {
        return false;
    };
    if state_directory_metadata.file_type().is_symlink() || !state_directory_metadata.is_dir() {
        return false;
    }
    let state_file = state_directory.join(STATE_FILE);
    fs::symlink_metadata(state_file)
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn is_reserved_workspace_directory(name: &str) -> bool {
    name.eq_ignore_ascii_case(STATE_DIRECTORY)
        || name.eq_ignore_ascii_case(".obsidian")
        || name.eq_ignore_ascii_case(".trash")
        || name.eq_ignore_ascii_case(".git")
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md")
                || extension.eq_ignore_ascii_case("markdown")
        })
}

fn safe_file_stem(input: &str, fallback: &str) -> String {
    let mut result = String::new();
    let mut previous_was_replacement = false;
    for character in input.trim().chars() {
        if character.is_control()
            || matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        {
            if !previous_was_replacement {
                result.push('-');
                previous_was_replacement = true;
            }
        } else {
            result.push(character);
            previous_was_replacement = false;
        }
        if result.len() >= 120 {
            break;
        }
    }
    let result = result.trim_matches(|character| character == ' ' || character == '.');
    let result = if result.is_empty() { fallback } else { result };
    if is_windows_reserved_name(result) {
        format!("_{result}")
    } else {
        result.to_owned()
    }
}

fn is_forbidden_component_character(character: char) -> bool {
    character.is_control()
        || matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

fn is_windows_reserved_name(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(
        base.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn ensure_private_directory_tree(root: &Path, directory: &Path) -> io::Result<()> {
    let relative = directory.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "private directory escaped its transaction",
        )
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private directory contains a symbolic link",
                ));
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "private directory path is not a folder",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                create_directory_durable(&current)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn create_directory_durable(path: &Path) -> io::Result<()> {
    fs::create_dir(path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn remove_file_durable(path: &Path) -> io::Result<()> {
    fs::remove_file(path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn remove_directory_durable(path: &Path) -> io::Result<()> {
    fs::remove_dir(path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn rename_durable(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)?;
    if let Some(source_parent) = source.parent() {
        sync_directory(source_parent)?;
    }
    if target.parent() != source.parent() {
        if let Some(target_parent) = target.parent() {
            sync_directory(target_parent)?;
        }
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file has no parent"))?;
    fs::create_dir_all(parent)?;
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("workspace");
    let temporary = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        counter
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);

        #[cfg(windows)]
        {
            if path.exists() {
                let backup = parent.join(format!(
                    ".{file_name}.{}.{}.bak",
                    std::process::id(),
                    counter
                ));
                fs::rename(path, &backup)?;
                if let Err(error) = fs::rename(&temporary, path) {
                    let _ = fs::rename(&backup, path);

                    return Err(error);
                }
                let _ = fs::remove_file(backup);
                sync_directory(parent)?;

                return Ok(());
            }
        }

        fs::rename(&temporary, path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn registry_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(REGISTRY_FILE))
        .map_err(|error| format!("Could not locate the app configuration folder: {error}"))
}

fn workspace_state_path(root: &Path) -> PathBuf {
    root.join(STATE_DIRECTORY).join(STATE_FILE)
}

fn path_string(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "The selected path is not valid Unicode.".to_owned())
}

fn path_to_slash_string(path: &Path) -> Option<String> {
    let mut result = String::new();
    for component in path.components() {
        let component = component.as_os_str().to_str()?;
        if !result.is_empty() {
            result.push('/');
        }
        result.push_str(component);
    }
    Some(result)
}

fn canonical_path_if_available(path: &str) -> String {
    Path::new(path)
        .canonicalize()
        .ok()
        .and_then(|path| path_to_slash_string(&path))
        .unwrap_or_else(|| path.replace('\\', "/"))
}

fn metadata_time_millis(metadata: &fs::Metadata, prefer_created: bool) -> u64 {
    let time = if prefer_created {
        metadata.created().or_else(|_| metadata.modified())
    } else {
        metadata.modified().or_else(|_| metadata.created())
    };
    time.ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_else(now_millis)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn fresh_id(prefix: &str, value: &str, used: &mut HashSet<String>) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    fnv_update(&mut hash, value.as_bytes());
    let base = format!("{prefix}-{hash:016x}");
    if used.insert(base.clone()) {
        return base;
    }
    for index in 2..10_000 {
        let candidate = format!("{base}-{index}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    format!("{base}-{}", now_millis())
}

fn display_vault_name(requested: &str, root: &Path) -> String {
    if !requested.trim().is_empty() {
        return requested.trim().to_owned();
    }
    root.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Vault")
        .to_owned()
}

fn normalize_editor_mode(mode: &str) -> String {
    match mode {
        "split" | "reading" => mode.to_owned(),
        _ => "source".to_owned(),
    }
}

fn is_virtual_folder_selection(value: &str) -> bool {
    matches!(value, "all" | "favorites")
}

fn trim_line_ending(line: &str) -> &str {
    line.strip_suffix('\n')
        .unwrap_or(line)
        .strip_suffix('\r')
        .unwrap_or_else(|| line.strip_suffix('\n').unwrap_or(line))
}

fn state_version() -> u32 {
    STATE_VERSION
}

fn registry_version() -> u32 {
    REGISTRY_VERSION
}

fn default_folder_selection() -> String {
    "all".to_owned()
}

fn default_editor_mode() -> String {
    "source".to_owned()
}

fn lock_workspace_io() -> Result<MutexGuard<'static, ()>, String> {
    WORKSPACE_IO_LOCK
        .lock()
        .map_err(|_| "Workspace storage is unavailable because an earlier operation failed.".to_owned())
}

#[derive(Default)]
struct WarningCollector {
    warnings: Vec<String>,
    truncated: bool,
}

impl WarningCollector {
    fn push(&mut self, warning: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(warning);
        } else {
            self.truncated = true;
        }
    }

    fn finish(mut self) -> Vec<String> {
        if self.truncated {
            self.warnings
                .push("Additional warnings were omitted.".to_owned());
        }
        self.warnings
    }
}
