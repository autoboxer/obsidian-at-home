use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use walkdir::{DirEntry, WalkDir};

mod assets;
mod persistence;
mod registry;
mod revision;

use assets::*;
pub(crate) use assets::files::attachments::{
    copy_attachment_file_for_transfer_impl as copy_attachment_file_for_transfer,
};
pub(crate) use assets::files::images::{
    is_supported_image_path_impl as is_supported_image_path,
    validate_image_bytes_impl as validate_image_bytes,
};
use persistence::*;
use registry::*;
use revision::*;

const STATE_DIRECTORY: &str = ".obsidian-at-home";
const STATE_FILE: &str = "state.json";
const EDITOR_POSITIONS_FILE: &str = "editor-positions.json";
const EDITOR_POSITIONS_LOCK_FILE: &str = "editor-positions.lock";
const WORKSPACE_LOCK_FILE: &str = "workspace.lock";
const REGISTRY_FILE: &str = "workspaces.json";
const TRANSACTIONS_DIRECTORY: &str = "transactions";
const TRANSACTION_MANIFEST_FILE: &str = "manifest.json";
const RECENTLY_DELETED_DIRECTORY: &str = "recently-deleted";
const RECENTLY_DELETED_SNAPSHOT_VERSION: u32 = 1;
const STATE_VERSION: u32 = 4;
const EDITOR_POSITIONS_VERSION: u32 = 1;
const REGISTRY_VERSION: u32 = 1;
const TRANSACTION_VERSION: u32 = 5;
const MAX_NOTE_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const MAX_IMAGE_BYTES: u64 = 50 * 1024 * 1024;
pub(crate) const MAX_ATTACHMENT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_TOTAL_NOTE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_NOTES: usize = 100_000;
const MAX_RECENT_NOTES: usize = 10;
const MAX_WARNINGS: usize = 200;
const MAX_PATH_COMPONENTS: usize = 120;
const MAX_TRANSACTION_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EDITOR_POSITIONS_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RECENTLY_DELETED_SNAPSHOT_BYTES: u64 = MAX_NOTE_BYTES * 6 + 1024 * 1024;
const MAX_RECENTLY_DELETED_BYTES: u64 = MAX_TOTAL_NOTE_BYTES;
const MAX_RECENTLY_DELETED_NOTES: usize = MAX_NOTES;
const MAX_SAFE_JAVASCRIPT_INTEGER: u64 = (1_u64 << 53) - 1;
const RECENTLY_DELETED_RETENTION_MILLIS: u64 = 7 * 24 * 60 * 60 * 1000;

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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ImageEmbedLocation {
    VaultRoot,
    NoteFolder,
    SpecifiedFolder,
}

impl<'de> Deserialize<'de> for ImageEmbedLocation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "vault-root" => Ok(Self::VaultRoot),
            "note-folder" => Ok(Self::NoteFolder),
            "specified-folder" | "specified-folder-mirrored" => Ok(Self::SpecifiedFolder),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &[
                    "vault-root",
                    "note-folder",
                    "specified-folder",
                    "specified-folder-mirrored",
                ],
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageEmbedSettings {
    pub location: ImageEmbedLocation,
    #[serde(default)]
    pub folder_path: String,
}

pub type AttachmentEmbedSettings = ImageEmbedSettings;

impl Default for ImageEmbedSettings {
    fn default() -> Self {
        Self {
            location: ImageEmbedLocation::VaultRoot,
            folder_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedImage {
    pub id: String,
    pub relative_path: String,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultImageFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    pub relative_path: String,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedAttachment {
    pub id: String,
    pub relative_path: String,
    pub media_type: String,
    pub byte_length: u64,
    pub opening_disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultAttachmentFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    pub relative_path: String,
    pub media_type: String,
    pub byte_length: u64,
    pub opening_disabled: bool,
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
    #[serde(default)]
    pub recent_note_ids: Vec<String>,
    pub selected_folder_id: String,
    #[serde(default)]
    pub embedded_images: Vec<EmbeddedImage>,
    #[serde(default)]
    pub image_files: Vec<VaultImageFile>,
    #[serde(default)]
    pub image_embed_settings: ImageEmbedSettings,
    #[serde(default)]
    pub embedded_attachments: Vec<EmbeddedAttachment>,
    #[serde(default)]
    pub attachment_files: Vec<VaultAttachmentFile>,
    #[serde(default)]
    pub attachment_embed_settings: AttachmentEmbedSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultDescriptor {
    pub name: String,
    pub path: String,
    pub last_opened_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NoteEditorSelection {
    pub anchor: u64,
    pub head: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NoteEditorViewport {
    pub anchor: u64,
    pub offset: f64,
    pub left: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NoteEditorPosition {
    pub selection: NoteEditorSelection,
    pub viewport: NoteEditorViewport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecentlyDeletedNote {
    pub id: String,
    pub note: Note,
    pub original_folder_path: String,
    pub deleted_at: u64,
    pub expires_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_position: Option<NoteEditorPosition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLoad {
    pub vault: VaultData,
    pub descriptor: VaultDescriptor,
    #[serde(default)]
    pub recently_deleted_notes: Vec<RecentlyDeletedNote>,
    pub editor_positions: BTreeMap<String, NoteEditorPosition>,
    pub editor_positions_revision: Option<String>,
    pub editor_positions_writable: bool,
    pub warnings: Vec<String>,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResult {
    pub workspace: Option<WorkspaceLoad>,
    pub recent_vaults: Vec<VaultDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    pub note_paths: BTreeMap<String, String>,
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceArchiveResult {
    pub deleted_note: RecentlyDeletedNote,
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRestoreResult {
    pub restored_note: Note,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_position: Option<NoteEditorPosition>,
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecoveryMutationResult {
    pub removed_ids: Vec<String>,
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEmbedImageResult {
    pub image: EmbeddedImage,
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEmbedAttachmentResult {
    pub attachment: EmbeddedAttachment,
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceExternalFileUpload {
    pub id: String,
    pub chunk_bytes: usize,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExternalFileUploadKind {
    Image,
    Attachment,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceVaultItemKind {
    Note,
    Folder,
    Image,
    Attachment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceExternalAssetDiscardResult {
    pub discarded: bool,
    pub note_paths: BTreeMap<String, String>,
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceImageNoteUpdate {
    pub note_id: String,
    pub relative_path: String,
    pub expected_content: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRelocateImageResult {
    pub image: EmbeddedImage,
    pub previous_relative_path: String,
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRelocateAttachmentResult {
    pub attachment: EmbeddedAttachment,
    pub previous_relative_path: String,
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAttachmentCopyResult {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceImportImagesResult {
    pub image_count: usize,
    pub image_files: Vec<VaultImageFile>,
    pub attachment_count: usize,
    pub attachment_files: Vec<VaultAttachmentFile>,
    pub path_mappings: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceImportSaveResult {
    pub saved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub note_paths: BTreeMap<String, String>,
    pub revision: u64,
    pub saved_at: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmbedImageBytesMetadata {
    path: String,
    file_name: String,
    note_relative_path: String,
    settings: ImageEmbedSettings,
    expected_revision: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppendExternalFileUploadMetadata {
    upload_id: String,
    offset: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEditorPositions {
    #[serde(default = "editor_positions_version")]
    version: u32,
    #[serde(default)]
    positions: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredEditorPositions<'a> {
    version: u32,
    positions: &'a BTreeMap<String, NoteEditorPosition>,
}

#[derive(Debug)]
enum EditorPositionsRead {
    Missing,
    Loaded(RawEditorPositions, FileFingerprint),
    Invalid(String, FileFingerprint),
    Newer(u32, FileFingerprint),
}

impl EditorPositionsRead {
    fn fingerprint(&self) -> Option<&FileFingerprint> {
        match self {
            Self::Missing => None,
            Self::Loaded(_, fingerprint)
            | Self::Invalid(_, fingerprint)
            | Self::Newer(_, fingerprint) => Some(fingerprint),
        }
    }
}

#[derive(Debug)]
struct DecodedEditorPositions {
    positions: BTreeMap<String, NoteEditorPosition>,
    invalid_count: usize,
    unknown_count: usize,
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
struct ScannedImage {
    relative_path: String,
    media_type: String,
}

#[derive(Debug)]
struct ScannedAttachment {
    relative_path: String,
    media_type: String,
    byte_length: u64,
    opening_disabled: bool,
}

#[derive(Debug)]
struct NoteWritePlan {
    id: String,
    old_relative_path: Option<String>,
    new_relative_path: String,
    content: String,
    needs_write: bool,
    preserved_modified_at: Option<u64>,
}

#[derive(Debug)]
struct PendingNoteArchive {
    note: Note,
    original_folder_path: String,
    editor_position: Option<NoteEditorPosition>,
}

#[derive(Debug)]
struct PreparedNoteArchive {
    deleted_note: RecentlyDeletedNote,
    bytes: Vec<u8>,
    fingerprint: FileFingerprint,
}

#[derive(Debug)]
struct PendingNoteRestore {
    deleted_note_id: String,
    restored_note: Note,
    preferred_relative_path: String,
}

#[derive(Debug)]
struct PreparedNoteRestore {
    restored_note: Note,
    editor_position: Option<NoteEditorPosition>,
    recovery_id: String,
    fingerprint: FileFingerprint,
}

#[derive(Debug, PartialEq, Eq)]
enum RecoverySnapshotRemoval {
    Removed,
    AlreadyMissing,
    RemovedWithoutDurability(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum TransactionPhase {
    Prepared,
    Applying,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
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
enum TransactionTargetKind {
    Markdown,
    Image,
    Attachment,
}

impl Default for TransactionTargetKind {
    fn default() -> Self {
        Self::Markdown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct TransactionTarget {
    relative_path: String,
    fingerprint: FileFingerprint,
    #[serde(default)]
    kind: TransactionTargetKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct TransactionRecoveryTarget {
    id: String,
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
    #[serde(default)]
    recovery_targets: Vec<TransactionRecoveryTarget>,
    folder_case_renames: Vec<FolderCaseRename>,
    created_directories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    length: u64,
    modified_nanos: u128,
    content_hash: Option<u64>,
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
                let _workspace_guard = lock_workspace_files(&canonical)?;
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
    let _workspace_guard = lock_workspace_files(&root)?;
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
    let _workspace_guard = lock_workspace_files(&root)?;

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
    let _workspace_guard = lock_workspace_files(&root)?;
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

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_save_with_image_import(
    app: AppHandle,
    path: String,
    vault: VaultData,
    expected_revision: u64,
    transaction_id: String,
) -> Result<WorkspaceImportSaveResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let mut result = save_workspace_files_with_image_import(
        &root,
        &vault,
        expected_revision,
        &transaction_id,
    )?;

    if result.saved {
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
    }

    Ok(result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_archive_note(
    app: AppHandle,
    path: String,
    vault: VaultData,
    note: Note,
    original_folder_path: String,
    editor_position: Option<NoteEditorPosition>,
    expected_revision: u64,
) -> Result<WorkspaceArchiveResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let pending_archive = PendingNoteArchive {
        note,
        original_folder_path,
        editor_position,
    };
    let (mut result, deleted_note) = save_workspace_files_with_archive(
        &root,
        &vault,
        expected_revision,
        Some(pending_archive),
    )?;
    let deleted_note = deleted_note
        .ok_or_else(|| "The note was saved without a recovery snapshot.".to_owned())?;

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

    Ok(WorkspaceArchiveResult {
        deleted_note,
        revision: result.revision,
        saved_at: result.saved_at,
        warnings: result.warnings,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_restore_recently_deleted_note(
    app: AppHandle,
    path: String,
    deleted_note_id: String,
    vault: VaultData,
    expected_revision: u64,
) -> Result<WorkspaceRestoreResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let (state, deleted_note) = read_recovery_for_restore(
        &root,
        &deleted_note_id,
        expected_revision,
    )?;
    let (restored_note, preferred_relative_path) = build_restored_note(
        &root,
        &vault,
        &state,
        &deleted_note,
    )?;
    let mut restored_vault = vault;
    restored_vault.notes.push(restored_note.clone());
    restored_vault.active_note_id = Some(restored_note.id.clone());
    restored_vault.recent_note_ids.retain(|id| id != &restored_note.id);
    restored_vault
        .recent_note_ids
        .insert(0, restored_note.id.clone());
    restored_vault.recent_note_ids.truncate(MAX_RECENT_NOTES);
    restored_vault.selected_folder_id = "all".to_owned();

    let (mut result, prepared_restore) = save_workspace_files_with_restore(
        &root,
        &restored_vault,
        expected_revision,
        PendingNoteRestore {
            deleted_note_id,
            restored_note,
            preferred_relative_path,
        },
    )?;

    let registry_result = (|| {
        let mut registry = read_registry(&app)?;
        let descriptor = VaultDescriptor {
            name: display_vault_name(&restored_vault.name, &root),
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

    Ok(WorkspaceRestoreResult {
        restored_note: prepared_restore.restored_note,
        editor_position: prepared_restore.editor_position,
        revision: result.revision,
        saved_at: result.saved_at,
        warnings: result.warnings,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_delete_recently_deleted_notes(
    app: AppHandle,
    path: String,
    deleted_note_ids: Vec<String>,
    expected_revision: u64,
) -> Result<WorkspaceRecoveryMutationResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    remove_recently_deleted_notes(
        &root,
        deleted_note_ids,
        expected_revision,
        false,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_prune_recently_deleted_notes(
    app: AppHandle,
    path: String,
    expected_revision: u64,
) -> Result<WorkspaceRecoveryMutationResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    remove_recently_deleted_notes(&root, Vec::new(), expected_revision, true)
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
    let _workspace_guard = lock_workspace_files(&root)?;
    revision_for_root(&root)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_embed_image_file(
    app: AppHandle,
    path: String,
    source_path: String,
    note_relative_path: String,
    settings: ImageEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedImageResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let source = validate_image_source_file(&source_path)?;
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Image.png")
        .to_owned();
    let bytes = read_image_file(&source)?;
    let existing_relative_path = source
        .strip_prefix(&root)
        .ok()
        .and_then(path_to_slash_string)
        .filter(|relative| validate_image_relative_path(relative).is_ok());

    embed_workspace_image(
        &root,
        &note_relative_path,
        settings,
        &file_name,
        &bytes,
        existing_relative_path.as_deref(),
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_embed_vault_image(
    app: AppHandle,
    path: String,
    image_relative_path: String,
    note_relative_path: String,
    settings: ImageEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedImageResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    validate_image_relative_path(&image_relative_path)?;
    let source = resolve_workspace_image_file(&root, &image_relative_path, false)?;
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Image.png")
        .to_owned();
    let bytes = read_image_file(&source)?;

    embed_workspace_image(
        &root,
        &note_relative_path,
        settings,
        &file_name,
        &bytes,
        Some(&image_relative_path),
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_embed_attachment_file(
    app: AppHandle,
    path: String,
    source_path: String,
    note_relative_path: String,
    settings: AttachmentEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedAttachmentResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let source = validate_attachment_source_file(&source_path)?;
    let existing_relative_path = source
        .strip_prefix(&root)
        .ok()
        .and_then(path_to_slash_string)
        .filter(|relative| validate_attachment_relative_path(relative).is_ok());

    embed_workspace_attachment(
        &root,
        &note_relative_path,
        settings,
        &source,
        existing_relative_path.as_deref(),
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_begin_external_file_upload(
    app: AppHandle,
    path: String,
    file_name: String,
    byte_length: u64,
    kind: ExternalFileUploadKind,
    note_relative_path: String,
    expected_revision: u64,
) -> Result<WorkspaceExternalFileUpload, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    validate_external_file_drop_note(&root, &note_relative_path)?;
    if revision_for_root(&root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before dropping the file."
                .to_owned(),
        );
    }
    let staging_directory = external_file_staging_directory(&app)?;
    begin_external_file_upload(
        &staging_directory,
        file_name,
        byte_length,
        kind,
        root,
        note_relative_path,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_append_external_file_upload(
    request: tauri::ipc::Request<'_>,
) -> Result<u64, String> {
    let encoded_metadata = request
        .headers()
        .get("x-oah-external-file-upload")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "Dropped-file transfer metadata is missing.".to_owned())?;
    let metadata: AppendExternalFileUploadMetadata = serde_json::from_str(&percent_decode_utf8(
        encoded_metadata,
    )?)
    .map_err(|error| format!("Dropped-file transfer metadata is invalid: {error}"))?;
    let bytes: Cow<'_, [u8]> = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => Cow::Borrowed(bytes),
        tauri::ipc::InvokeBody::Json(serde_json::Value::Array(values)) => Cow::Owned(
            values
                .iter()
                .map(|value| {
                    value
                        .as_u64()
                        .filter(|value| *value <= u64::from(u8::MAX))
                        .map(|value| value as u8)
                        .ok_or_else(|| "Dropped-file bytes are invalid.".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        _ => return Err("Dropped-file bytes are missing.".to_owned()),
    };
    append_external_file_upload(&metadata.upload_id, metadata.offset, &bytes)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_cancel_external_file_upload(upload_id: String) -> Result<bool, String> {
    cancel_external_file_upload(&upload_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_finish_external_image_upload(
    app: AppHandle,
    upload_id: String,
    settings: ImageEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedImageResult, String> {
    let staged = finish_external_file_upload(&upload_id, ExternalFileUploadKind::Image)?;
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root_path(&staged.root)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    validate_external_file_drop_note(&root, &staged.note_relative_path)?;
    let source = validate_image_source_path(&staged.path)?;
    let bytes = read_image_file(&source)?;
    embed_workspace_image(
        &root,
        &staged.note_relative_path,
        settings,
        &staged.file_name,
        &bytes,
        None,
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_finish_external_attachment_upload(
    app: AppHandle,
    upload_id: String,
    settings: AttachmentEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedAttachmentResult, String> {
    let staged = finish_external_file_upload(&upload_id, ExternalFileUploadKind::Attachment)?;
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root_path(&staged.root)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    validate_external_file_drop_note(&root, &staged.note_relative_path)?;
    let source = validate_attachment_source_path(&staged.path)?;
    embed_workspace_attachment(
        &root,
        &staged.note_relative_path,
        settings,
        &source,
        None,
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_discard_external_asset(
    app: AppHandle,
    path: String,
    asset_id: String,
    relative_path: String,
    expected_revision: u64,
) -> Result<WorkspaceExternalAssetDiscardResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    discard_workspace_external_asset(
        &root,
        &asset_id,
        &relative_path,
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_embed_vault_attachment(
    app: AppHandle,
    path: String,
    attachment_relative_path: String,
    note_relative_path: String,
    settings: AttachmentEmbedSettings,
    expected_revision: u64,
) -> Result<WorkspaceEmbedAttachmentResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    validate_attachment_relative_path(&attachment_relative_path)?;
    let source = resolve_workspace_asset_file(
        &root,
        &attachment_relative_path,
        false,
    )?;

    embed_workspace_attachment(
        &root,
        &note_relative_path,
        settings,
        &source,
        Some(&attachment_relative_path),
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_relocate_image(
    app: AppHandle,
    path: String,
    image_relative_path: String,
    target_relative_path: String,
    asset_id: String,
    note_updates: Vec<WorkspaceImageNoteUpdate>,
    expected_revision: u64,
) -> Result<WorkspaceRelocateImageResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    relocate_workspace_image(
        &root,
        &image_relative_path,
        &target_relative_path,
        &asset_id,
        &note_updates,
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_relocate_attachment(
    app: AppHandle,
    path: String,
    attachment_relative_path: String,
    target_relative_path: String,
    asset_id: String,
    note_updates: Vec<WorkspaceImageNoteUpdate>,
    expected_revision: u64,
) -> Result<WorkspaceRelocateAttachmentResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    relocate_workspace_attachment(
        &root,
        &attachment_relative_path,
        &target_relative_path,
        &asset_id,
        &note_updates,
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_locate_vault_item(
    app: AppHandle,
    path: String,
    kind: WorkspaceVaultItemKind,
    relative_path: String,
    asset_id: Option<String>,
) -> Result<String, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    locate_workspace_vault_item(&root, kind, &relative_path, asset_id.as_deref())
        .map(|(resolved_relative_path, _)| resolved_relative_path)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_show_vault_item_in_folder(
    app: AppHandle,
    path: String,
    kind: WorkspaceVaultItemKind,
    relative_path: String,
    asset_id: Option<String>,
) -> Result<(), String> {
    let target = {
        let _guard = lock_workspace_io()?;
        let root = validate_workspace_root(&path)?;
        reject_home_vault(&app, &root)?;
        let _workspace_guard = lock_workspace_files(&root)?;
        let (_, target) =
            locate_workspace_vault_item(&root, kind, &relative_path, asset_id.as_deref())?;
        target
    };
    app.opener()
        .reveal_item_in_dir(&target)
        .map_err(|error| format!("Could not show the vault item in its folder: {error}"))
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_open_attachment(
    app: AppHandle,
    path: String,
    attachment_relative_path: String,
    asset_id: Option<String>,
) -> Result<(), String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    let (_, source) = resolve_attachment_action_source(
        &root,
        &attachment_relative_path,
        asset_id.as_deref(),
    )?;
    if is_archive_attachment_path(&source) {
        return Err("Archives must be saved to a location outside the vault before opening."
            .to_owned());
    }
    if attachment_opening_is_disabled(&source)? {
        return Err(
            "Opening executable or installer attachments is not supported.".to_owned(),
        );
    }
    app.opener()
        .open_path(path_string(&source)?, None::<&str>)
        .map_err(|error| format!("Could not open the attachment: {error}"))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn workspace_save_attachment_copy(
    app: AppHandle,
    path: String,
    attachment_relative_path: String,
    asset_id: Option<String>,
    preferred_directory: Option<String>,
) -> Result<Option<WorkspaceAttachmentCopyResult>, String> {
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    save_workspace_attachment_copy(
        &app,
        &root,
        &attachment_relative_path,
        asset_id.as_deref(),
        preferred_directory.as_deref(),
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_embed_image_bytes(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<WorkspaceEmbedImageResult, String> {
    let encoded_metadata = request
        .headers()
        .get("x-oah-image-metadata")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "Image metadata is missing from the clipboard request.".to_owned())?;
    let metadata: EmbedImageBytesMetadata = serde_json::from_str(&percent_decode_utf8(
        encoded_metadata,
    )?)
    .map_err(|error| format!("Image metadata is invalid: {error}"))?;
    let bytes: Cow<'_, [u8]> = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => Cow::Borrowed(bytes),
        tauri::ipc::InvokeBody::Json(serde_json::Value::Array(values)) => Cow::Owned(
            values
                .iter()
                .map(|value| {
                    value
                        .as_u64()
                        .filter(|value| *value <= u64::from(u8::MAX))
                        .map(|value| value as u8)
                        .ok_or_else(|| "Image bytes are invalid.".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        _ => return Err("Image bytes are missing from the clipboard request.".to_owned()),
    };
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&metadata.path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    embed_workspace_image(
        &root,
        &metadata.note_relative_path,
        metadata.settings,
        &metadata.file_name,
        &bytes,
        None,
        metadata.expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_read_image(
    app: AppHandle,
    path: String,
    asset_id: Option<String>,
    note_relative_path: String,
    destination: String,
) -> Result<tauri::ipc::Response, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    read_workspace_image(
        &root,
        asset_id.as_deref(),
        &note_relative_path,
        &destination,
    )
    .map(tauri::ipc::Response::new)
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_import_images(
    app: AppHandle,
    path: String,
    source_path: String,
    image_paths: Vec<String>,
    expected_revision: u64,
) -> Result<WorkspaceImportImagesResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let source_root = validate_image_import_root(&source_path)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    begin_workspace_asset_import(
        &root,
        &source_root,
        &image_paths,
        &[],
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_import_assets(
    app: AppHandle,
    path: String,
    source_path: String,
    image_paths: Vec<String>,
    attachment_paths: Vec<String>,
    expected_revision: u64,
) -> Result<WorkspaceImportImagesResult, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let source_root = validate_image_import_root(&source_path)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    begin_workspace_asset_import(
        &root,
        &source_root,
        &image_paths,
        &attachment_paths,
        expected_revision,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn workspace_save_editor_positions(
    app: AppHandle,
    path: String,
    positions: BTreeMap<String, NoteEditorPosition>,
    expected_revision: Option<String>,
) -> Result<String, String> {
    let _guard = lock_workspace_io()?;
    let root = validate_workspace_root(&path)?;
    reject_home_vault(&app, &root)?;
    let _workspace_guard = lock_workspace_files(&root)?;
    save_editor_positions(&root, positions, expected_revision)
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
    validate_transaction_id(transaction_id)?;
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

fn validate_transaction_id(transaction_id: &str) -> Result<(), String> {
    if transaction_id.is_empty()
        || transaction_id.len() > 180
        || transaction_id
            .chars()
            .any(|character| !character.is_ascii_alphanumeric() && character != '-')
    {
        return Err("The save transaction ID is invalid.".to_owned());
    }

    Ok(())
}

fn existing_transaction_root(root: &Path, transaction_id: &str) -> Result<PathBuf, String> {
    validate_transaction_id(transaction_id)?;
    let state_directory = root.join(STATE_DIRECTORY);
    let transactions_directory = state_directory.join(TRANSACTIONS_DIRECTORY);
    let transaction_root = transactions_directory.join(transaction_id);
    for (path, label) in [
        (&state_directory, "workspace metadata"),
        (&transactions_directory, "save transactions"),
        (&transaction_root, "asset import transaction"),
    ] {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("Could not inspect the {label} folder: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!("The {label} path is not a regular folder."));
        }
    }

    Ok(transaction_root)
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

fn resolve_workspace_image_file(
    root: &Path,
    relative_path: &str,
    allow_missing: bool,
) -> Result<PathBuf, String> {
    validate_image_relative_path(relative_path)?;
    let relative = checked_relative_path(relative_path, false)?;
    let target = root.join(&relative);
    if let Some(parent) = target.parent() {
        let mut current = root.to_path_buf();
        for component in parent
            .strip_prefix(root)
            .map_err(|_| "An image path escaped the vault.".to_owned())?
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

fn resolve_workspace_asset_file(
    root: &Path,
    relative_path: &str,
    allow_missing: bool,
) -> Result<PathBuf, String> {
    validate_attachment_relative_path(relative_path)?;
    let relative = checked_relative_path(relative_path, false)?;
    let target = root.join(&relative);
    if let Some(parent) = target.parent() {
        let mut current = root.to_path_buf();
        for component in parent
            .strip_prefix(root)
            .map_err(|_| "An attachment path escaped the vault.".to_owned())?
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

fn set_file_modified_millis(path: &Path, modified_at: u64) -> io::Result<()> {
    let modified = UNIX_EPOCH
        .checked_add(Duration::from_millis(modified_at))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "modified time is too large"))?;
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "modified-time target is not a regular file",
        ));
    }
    file.set_times(FileTimes::new().set_modified(modified))?;
    file.sync_all()
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

fn recently_deleted_snapshot_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    validate_recently_deleted_id(id)?;
    Ok(root
        .join(STATE_DIRECTORY)
        .join(RECENTLY_DELETED_DIRECTORY)
        .join(format!("{id}.snapshot")))
}

fn transaction_recovery_snapshot_path(
    transaction_root: &Path,
    id: &str,
) -> Result<PathBuf, String> {
    validate_recently_deleted_id(id)?;
    let relative_path = format!("recoveries/{id}.snapshot");
    Ok(transaction_root.join(checked_internal_transaction_path(
        &relative_path,
        true,
    )?))
}

fn ensure_recently_deleted_directory(root: &Path) -> Result<PathBuf, String> {
    let state_directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &state_directory)?;
    let directory = state_directory.join(RECENTLY_DELETED_DIRECTORY);
    ensure_regular_directory(&directory, "Recently Deleted")?;

    Ok(directory)
}

fn inspect_recently_deleted_directory(root: &Path) -> Result<PathBuf, String> {
    let state_directory = root.join(STATE_DIRECTORY);
    let state_metadata = fs::symlink_metadata(&state_directory)
        .map_err(|error| format!("Could not inspect workspace metadata: {error}"))?;
    if state_metadata.file_type().is_symlink() || !state_metadata.is_dir() {
        return Err("The workspace metadata path is not a regular folder.".to_owned());
    }
    let directory = state_directory.join(RECENTLY_DELETED_DIRECTORY);
    let metadata = fs::symlink_metadata(&directory)
        .map_err(|error| format!("Could not inspect Recently Deleted: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The Recently Deleted path is not a regular folder.".to_owned());
    }

    Ok(directory)
}

fn editor_positions_path(root: &Path) -> PathBuf {
    root.join(STATE_DIRECTORY).join(EDITOR_POSITIONS_FILE)
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

fn normalize_recent_note_ids(
    recent_note_ids: &[String],
    active_note_id: Option<&str>,
    note_ids: &HashSet<&str>,
) -> Vec<String> {
    let candidates = active_note_id
        .into_iter()
        .chain(recent_note_ids.iter().map(String::as_str));
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();

    for id in candidates {
        if normalized.len() >= MAX_RECENT_NOTES {
            break;
        }
        if note_ids.contains(id) && seen.insert(id.to_owned()) {
            normalized.push(id.to_owned());
        }
    }

    normalized
}

fn is_virtual_folder_selection(value: &str) -> bool {
    matches!(value, "all" | "favorites" | "recent")
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

fn editor_positions_version() -> u32 {
    EDITOR_POSITIONS_VERSION
}

fn registry_version() -> u32 {
    REGISTRY_VERSION
}

fn default_folder_selection() -> String {
    "all".to_owned()
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

#[cfg(test)]
mod tests {
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
        let current_state = fs::read_to_string(&state_path)
            .expect("current workspace state should be readable");
        let legacy_state = current_state.replace(
            "\"specified-folder\"",
            "\"specified-folder-mirrored\"",
        );
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
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");
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
        let (state, _) = read_workspace_state(
            &workspace.root,
            &mut WarningCollector::default(),
        );
        let mut state = state.expect("state should load");
        let entry = state
            .recently_deleted_notes
            .get_mut(id)
            .expect("recovery entry should exist");
        let path = recently_deleted_snapshot_path(&workspace.root, id)
            .expect("snapshot path should be safe");
        let mut snapshot: RecentlyDeletedSnapshot = serde_json::from_slice(
            &fs::read(&path).expect("snapshot should be readable"),
        )
        .expect("snapshot should decode");
        snapshot.deleted_note.deleted_at = 1;
        snapshot.deleted_note.expires_at = 1 + RECENTLY_DELETED_RETENTION_MILLIS;
        let mut bytes = serde_json::to_vec_pretty(&snapshot)
            .expect("expired snapshot should encode");
        bytes.push(b'\n');
        fs::write(&path, &bytes).expect("expired snapshot should be written");
        entry.deleted_at = snapshot.deleted_note.deleted_at;
        entry.expires_at = snapshot.deleted_note.expires_at;
        entry.fingerprint = fingerprint_bytes(&bytes);
        write_workspace_state(&workspace.root, &state)
            .expect("expired recovery metadata should be written");
        revision_for_root(&workspace.root)
            .expect("expired revision should be calculated")
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
        write_workspace_state(&workspace.root, &state)
            .expect("legacy state should be written");

        let loaded = load_workspace(&workspace.root, &empty_vault("Test vault"))
            .expect("legacy workspace should load");
        let (migrated, _) = read_workspace_state(
            &workspace.root,
            &mut WarningCollector::default(),
        );

        assert!(loaded.recently_deleted_notes.is_empty());
        assert_eq!(migrated.expect("migrated state should exist").version, STATE_VERSION);
    }

    #[test]
    fn version_three_image_assets_migrate_to_vault_assets() {
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\nlegacy-image-asset";
        let workspace = TestWorkspace::new("state-v3-image-asset-migration");
        fs::write(workspace.root.join("Legacy.png"), PNG)
            .expect("legacy image should be written");
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
        let revision_before = revision_for_root(&workspace.root)
            .expect("initial revision should be calculated");
        let first = lock_workspace_files(&workspace.root)
            .expect("first workspace handle should lock");
        let second = open_workspace_lock_file(&workspace.root)
            .expect("second workspace handle should open");

        assert!(second.try_lock().is_err());
        drop(first);
        second
            .try_lock()
            .expect("second workspace handle should lock after release");

        let revision_after = revision_for_root(&workspace.root)
            .expect("updated revision should be calculated");
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
        let expected_revision = revision_for_root(&workspace.root)
            .expect("initial revision should be calculated");
        let baseline_stamps = note_file_stamps(&workspace.root)
            .expect("initial note stamps should be calculated");

        fs::write(&note_path, "edited").expect("external note edit should be written");
        File::options()
            .write(true)
            .open(&note_path)
            .expect("external note should reopen")
            .set_times(FileTimes::new().set_modified(original_modified))
            .expect("the original modified time should be restored");
        let edited_metadata = fs::metadata(&note_path)
            .expect("edited metadata should be readable");
        assert_eq!(edited_metadata.len(), note.content.len() as u64);
        assert_eq!(edited_metadata.modified().unwrap(), original_modified);

        let current_revision = revision_for_root(&workspace.root)
            .expect("edited revision should be calculated");
        let current_stamps = note_file_stamps(&workspace.root)
            .expect("edited note stamps should be calculated");
        assert_ne!(current_revision, expected_revision);
        assert_ne!(current_stamps, baseline_stamps);

        let mut stale_vault = empty_vault("Test vault");
        stale_vault.notes.push(note);
        let error = save_workspace_files(
            &workspace.root,
            &stale_vault,
            expected_revision,
        )
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
        fs::write(workspace.root.join("Note.md"), "note")
            .expect("note should be written");
        fs::write(workspace.root.join("Archive.zip"), "asset")
            .expect("asset should be written");
        write_workspace_state(&workspace.root, &WorkspaceState::default())
            .expect("workspace state should be written");

        let entries = revision_entries_for_root(&workspace.root)
            .expect("revision entries should be calculated");
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
        let original_state_modified = fs::metadata(&state_path)
            .unwrap()
            .modified()
            .unwrap();
        let mut changed_state = fs::read(&state_path)
            .expect("workspace state should be readable");
        let version_digit = changed_state
            .windows(b"\"version\": 4".len())
            .position(|window| window == b"\"version\": 4")
            .map(|position| position + b"\"version\": ".len())
            .expect("workspace state should contain its version");
        changed_state[version_digit] = b'3';
        let original_revision = revision_for_entries(&entries);
        fs::write(&state_path, changed_state)
            .expect("external state edit should be written");
        File::options()
            .write(true)
            .open(&state_path)
            .expect("workspace state should reopen")
            .set_times(FileTimes::new().set_modified(original_state_modified))
            .expect("the original state modified time should be restored");
        assert_ne!(
            revision_for_root(&workspace.root)
                .expect("changed state revision should be calculated"),
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
        fs::write(&path, &bytes)
            .expect("fingerprint fixture should be written");

        assert_eq!(
            fingerprint_regular_file(&path)
                .expect("file should be fingerprinted"),
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
        let value = serde_json::to_value(&deleted_note)
            .expect("deleted note should serialize");

        assert_eq!(value["id"], "deleted-contract");
        assert_eq!(value["originalFolderPath"], "Projects");
        assert_eq!(value["deletedAt"], 5_000);
        assert_eq!(
            value["expiresAt"],
            5_000 + RECENTLY_DELETED_RETENTION_MILLIS,
        );
        assert!(value.get("editorPosition").is_some());
    }

    #[test]
    fn archives_and_reloads_a_note_without_scanning_the_snapshot() {
        let workspace = TestWorkspace::new("archive-round-trip");
        let note = test_note("A recovered thought\n");
        write_saved_note(&workspace, &note);
        let revision = revision_for_root(&workspace.root)
            .expect("initial revision should be calculated");

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
        let snapshot_path = recently_deleted_snapshot_path(
            &workspace.root,
            &deleted_note.id,
        )
        .expect("snapshot path should be safe");
        assert_eq!(
            snapshot_path.extension().and_then(|value| value.to_str()),
            Some("snapshot"),
        );
        assert!(snapshot_path.is_file());

        let (scanned_notes, _, _, _) = scan_workspace_files(
            &workspace.root,
            &mut WarningCollector::default(),
        )
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
        let revision = revision_for_root(&workspace.root)
            .expect("initial revision should be calculated");
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
        let (state, _) = read_workspace_state(
            &workspace.root,
            &mut WarningCollector::default(),
        );
        let state = state.expect("archived state should load");
        let mut vault = empty_vault("Test vault");
        let (restored_note, preferred_relative_path) = build_restored_note(
            &workspace.root,
            &vault,
            &state,
            &deleted_note,
        )
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
        assert!(!recently_deleted_snapshot_path(&workspace.root, &deleted_note.id)
            .expect("snapshot path should be safe")
            .exists());
        let (state, _) = read_workspace_state(
            &workspace.root,
            &mut WarningCollector::default(),
        );
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
        let revision = revision_for_root(&workspace.root)
            .expect("initial revision should be calculated");
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

        let error = read_recovery_for_restore(
            &workspace.root,
            &deleted_note.id,
            expired_revision,
        )
        .expect_err("expired snapshot should not be restorable");

        assert_eq!(
            error,
            "That deleted note has expired and can no longer be restored.",
        );
        assert!(recently_deleted_snapshot_path(&workspace.root, &deleted_note.id)
            .expect("snapshot path should be safe")
            .exists());
    }

    #[test]
    fn restoring_never_overwrites_an_occupied_original_path() {
        let workspace = TestWorkspace::new("restore-path-conflict");
        let note = test_note("Recoverable\n");
        write_saved_note(&workspace, &note);
        let revision = revision_for_root(&workspace.root)
            .expect("initial revision should be calculated");
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
        let conflict_revision = revision_for_root(&workspace.root)
            .expect("conflict revision should be calculated");
        assert_ne!(conflict_revision, archived.revision);
        let (state, _) = read_workspace_state(
            &workspace.root,
            &mut WarningCollector::default(),
        );
        let state = state.expect("archived state should load");
        let mut vault = empty_vault("Test vault");
        let (restored_note, preferred_relative_path) = build_restored_note(
            &workspace.root,
            &vault,
            &state,
            &deleted_note,
        )
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
        let revision = revision_for_root(&workspace.root)
            .expect("initial revision should be calculated");
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
        let saved = save_workspace_files(
            &workspace.root,
            &second_vault,
            first_archive.revision,
        )
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
        assert!(!recently_deleted_snapshot_path(&workspace.root, &first_deleted.id)
            .expect("first snapshot path should be safe")
            .exists());
        assert!(recently_deleted_snapshot_path(&workspace.root, &second_deleted.id)
            .expect("second snapshot path should be safe")
            .exists());

        let path = recently_deleted_snapshot_path(&workspace.root, &second_deleted.id)
            .expect("second snapshot path should be safe");
        let expired_revision = mark_recovery_expired(&workspace, &second_deleted.id);

        let pruned = remove_recently_deleted_notes(
            &workspace.root,
            Vec::new(),
            expired_revision,
            true,
        )
        .expect("expired snapshot should be pruned");
        assert_eq!(pruned.removed_ids, vec![second_deleted.id.clone()]);
        assert!(!path.exists());
    }

    #[test]
    fn loading_finishes_expiry_after_snapshot_cleanup_preceded_state_cleanup() {
        let workspace = TestWorkspace::new("interrupted-expiry-cleanup");
        let note = test_note("Expired recovery\n");
        write_saved_note(&workspace, &note);
        let revision = revision_for_root(&workspace.root)
            .expect("initial revision should be calculated");
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
        let (state, _) = read_workspace_state(
            &workspace.root,
            &mut WarningCollector::default(),
        );

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
        let revision = revision_for_root(&workspace.root)
            .expect("initial revision should be calculated");
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

        let pruned = remove_recently_deleted_notes(
            &workspace.root,
            Vec::new(),
            expired_revision,
            true,
        )
        .expect("failed physical cleanup should remain a successful prune check");
        let restore_error = read_recovery_for_restore(
            &workspace.root,
            &deleted_note.id,
            pruned.revision,
        )
        .expect_err("an expired snapshot should not remain restorable");
        assert!(recently_deleted_snapshot_path(&workspace.root, &deleted_note.id)
            .expect("snapshot path should be safe")
            .exists());
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))
            .expect("recovery directory permissions should be restored");
        let retried = remove_recently_deleted_notes(
            &workspace.root,
            Vec::new(),
            pruned.revision,
            true,
        )
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
        assert!(!recently_deleted_snapshot_path(&workspace.root, &deleted_note.id)
            .expect("snapshot path should be safe")
            .exists());
    }

    #[test]
    fn post_commit_cleanup_warns_without_deleting_a_changed_snapshot() {
        let workspace = TestWorkspace::new("changed-recovery-cleanup");
        ensure_recently_deleted_directory(&workspace.root)
            .expect("recovery directory should be created");
        let id = "deleted-changed-cleanup";
        let path = recently_deleted_snapshot_path(&workspace.root, id)
            .expect("snapshot path should be safe");
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
        let path = recently_deleted_snapshot_path(&workspace.root, id)
            .expect("snapshot path should be safe");
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
            RecoverySnapshotRemoval::RemovedWithoutDurability(
                "directory sync failed".to_owned(),
            ),
        );
    }

    #[test]
    fn refuses_to_archive_stale_note_content() {
        let workspace = TestWorkspace::new("stale-archive");
        let saved_note = test_note("Saved content");
        write_saved_note(&workspace, &saved_note);
        let revision = revision_for_root(&workspace.root)
            .expect("initial revision should be calculated");
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
        let transaction_root = prepare_transaction_root(
            &workspace.root,
            &new_transaction_id(),
        )
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
        let transaction_root = prepare_transaction_root(
            &workspace.root,
            &new_transaction_id(),
        )
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
        write_transaction_manifest(&transaction_root, &manifest)
            .expect("manifest should be updated");
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
        assert!(!recently_deleted_snapshot_path(
            &workspace.root,
            &archive.deleted_note.id,
        )
        .expect("snapshot path should be safe")
        .exists());
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
        write_transaction_manifest(&transaction_root, &manifest)
            .expect("manifest should be updated");
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
        write_workspace_state(&workspace.root, &state)
            .expect("committed state should be written");

        let mut warnings = WarningCollector::default();
        recover_workspace_transactions(&workspace.root, Some(&state), &mut warnings)
            .expect("committed transaction should recover");

        assert!(!workspace.root.join(&note.relative_path).exists());
        assert!(!transaction_root.exists());
        assert!(recently_deleted_snapshot_path(
            &workspace.root,
            &archive.deleted_note.id,
        )
        .expect("snapshot path should be safe")
        .is_file());
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
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");
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
        write_transaction_manifest(&transaction_root, &manifest)
            .expect("manifest should be written");

        recover_workspace_transactions(
            &workspace.root,
            Some(&state),
            &mut WarningCollector::default(),
        )
        .expect("stale committed transaction should be cleaned");

        assert!(!transaction_root.exists());
        assert!(!recently_deleted_snapshot_path(
            &workspace.root,
            "deleted-stale-transaction",
        )
        .expect("snapshot path should be safe")
        .exists());
    }

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

        let vault: VaultData =
            serde_json::from_value(value).expect("vault data should deserialize");
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
            "note-2", "missing", "note-3", "note-2", "note-4", "note-5", "note-6",
            "note-7", "note-8", "note-9", "note-10", "note-11", "note-12",
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
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");
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
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");
        let revision_before = revision_for_root(&workspace.root)
            .expect("initial revision should be calculated");
        let positions = BTreeMap::from([("known".to_owned(), editor_position(4))]);

        save_editor_positions(&workspace.root, positions, None)
            .expect("editor positions should be saved");

        let revision_after = revision_for_root(&workspace.root)
            .expect("updated revision should be calculated");
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
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");
        let initial = BTreeMap::from([
            ("first".to_owned(), editor_position(1)),
            ("second".to_owned(), editor_position(2)),
        ]);
        save_editor_positions(&workspace.root, initial, None)
            .expect("initial positions should be saved");
        let note_ids = ["first", "second"].into_iter().collect();
        let (mut first_instance, _, first_revision) = load_editor_positions(
            &workspace.root,
            &note_ids,
            &mut WarningCollector::default(),
        );
        let (mut second_instance, _, second_revision) = load_editor_positions(
            &workspace.root,
            &note_ids,
            &mut WarningCollector::default(),
        );
        first_instance.insert("first".to_owned(), editor_position(11));
        let current_revision = save_editor_positions(
            &workspace.root,
            first_instance,
            first_revision,
        )
        .expect("the first instance should save");
        let positions_path = editor_positions_path(&workspace.root);
        let current_bytes = fs::read(&positions_path)
            .expect("current positions should be readable");
        second_instance.insert("second".to_owned(), editor_position(22));

        let error = save_editor_positions(
            &workspace.root,
            second_instance.clone(),
            second_revision,
        )
        .expect_err("a stale position snapshot should be rejected");

        assert!(error.contains("another app window"));
        assert_eq!(
            fs::read(&positions_path).expect("current positions should remain readable"),
            current_bytes,
        );
        save_editor_positions(
            &workspace.root,
            second_instance,
            Some(current_revision),
        )
        .expect("a snapshot with the current revision should save");
    }

    #[test]
    fn saving_rejects_a_file_created_after_positions_were_loaded() {
        let workspace = TestWorkspace::new("created-position-revision");
        let mut state = WorkspaceState::default();
        state
            .note_paths
            .insert("known".to_owned(), "Known.md".to_owned());
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");
        let note_ids = ["known"].into_iter().collect();
        let (_, _, revision) = load_editor_positions(
            &workspace.root,
            &note_ids,
            &mut WarningCollector::default(),
        );
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
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");

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
        fs::write(workspace.root.join("Known.md"), "Known note")
            .expect("note should be written");
        fs::write(workspace_state_path(&workspace.root), b"not json")
            .expect("malformed state should be written");
        let positions_path = editor_positions_path(&workspace.root);
        let positions_before = serde_json::to_vec(&serde_json::json!({
            "version": EDITOR_POSITIONS_VERSION,
            "positions": { "preserved": editor_position(3) }
        }))
        .expect("positions should serialize");
        fs::write(&positions_path, &positions_before)
            .expect("positions should be written");
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

        let loaded = load_workspace(&workspace.root, &defaults)
            .expect("workspace should open with warnings");

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
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");
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

    #[test]
    fn embeds_reads_and_recovers_moved_images() {
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\nembedded-image-fixture";
        let workspace = TestWorkspace::new("embedded-image-storage");
        fs::create_dir(workspace.root.join("Notes")).expect("note folder should be created");
        fs::write(workspace.root.join("Notes/First.md"), "# First")
            .expect("note should be written");
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
        fs::create_dir_all(workspace.root.join("Projects"))
            .expect("note folder should be created");
        fs::write(workspace.root.join("Images/Projects/Photo.png"), PNG)
            .expect("legacy image should be written");
        fs::write(
            workspace.root.join("Files/Projects/Report.pdf"),
            b"legacy attachment",
        )
        .expect("legacy attachment should be written");
        fs::write(workspace.root.join("Projects/Note.md"), "# Note")
            .expect("note should be written");

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

        let (persisted, _) = read_workspace_state(
            &workspace.root,
            &mut WarningCollector::default(),
        );
        let persisted = persisted.expect("migrated workspace state should remain readable");
        assert_eq!(persisted.image_embed_settings, loaded.vault.image_embed_settings);
        assert_eq!(
            persisted.attachment_embed_settings,
            loaded.vault.attachment_embed_settings,
        );
        assert!(!fs::read_to_string(&state_path)
            .expect("migrated workspace state should be readable")
            .contains("specified-folder-mirrored"));

        let reopened = load_workspace(&workspace.root, &empty_vault("Legacy settings"))
            .expect("migrated workspace should reopen");
        assert_eq!(reopened.vault.image_embed_settings, loaded.vault.image_embed_settings);
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
        fs::write(workspace.root.join("test1/doc1.md"), "# Doc 1")
            .expect("doc1 should be written");
        fs::write(workspace.root.join("test2/doc2.md"), "# Doc 2")
            .expect("doc2 should be written");
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
        fs::write(workspace.root.join("Note.md"), "# Note")
            .expect("note should be written");
        let mut state = WorkspaceState::default();
        state
            .note_paths
            .insert("note-1".to_owned(), "Note.md".to_owned());
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");
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
        fs::write(workspace.root.join("Note.md"), &original)
            .expect("references should be written");
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
        assert_eq!(fs::read_to_string(workspace.root.join("Note.md")).unwrap(), moved_content);
        assert_eq!(moved.image.relative_path, "Images/Sub/Photo.png");

        let renamed_content = moved_content.replace("Images/Sub/Photo.png", "Other%20Images/Renamed.png");
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
        assert_eq!(fs::read_to_string(workspace.root.join("Note.md")).unwrap(), renamed_content);
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
        fs::write(workspace.root.join("Images/Loose.png"), PNG)
            .expect("loose image should be written");
        fs::write(workspace.root.join("Archive/Loose.png"), PNG)
            .expect("collision should be written");
        fs::write(workspace.root.join("Note.md"), "![Loose](Images/Loose.png)")
            .expect("note should be written");
        let mut state = WorkspaceState::default();
        state
            .note_paths
            .insert("note-1".to_owned(), "Note.md".to_owned());
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");
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
        assert_eq!(fs::read_to_string(workspace.root.join("Note.md")).unwrap(), updated);
    }

    #[test]
    fn former_mirrored_images_can_be_reorganized_after_migration() {
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\nformer-mirrored-image";
        let workspace = TestWorkspace::new("former-mirrored-image");
        fs::create_dir_all(workspace.root.join("Images/Notes"))
            .expect("legacy image folder should be created");
        fs::create_dir(workspace.root.join("Elsewhere"))
            .expect("destination should be created");
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
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");

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
        fs::write(workspace.root.join("Retained.png"), PNG)
            .expect("image should be written");
        let loaded = load_workspace(&workspace.root, &empty_vault("Images"))
            .expect("workspace should load");
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

        save_workspace_files(
            &workspace.root,
            &without_reference,
            reference_revision,
        )
        .expect("note without the image reference should save");

        assert_eq!(fs::read(workspace.root.join("Retained.png")).unwrap(), PNG);
        let reopened = load_workspace(&workspace.root, &empty_vault("Images"))
            .expect("workspace should reopen");
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
        fs::write(workspace.root.join("Projects/Plan.md"), "# Plan")
            .expect("note should be written");
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
        fs::write(workspace.root.join("Projects/Plan.md"), "# Plan")
            .expect("note should be written");
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

        assert_eq!(first.attachment.relative_path, "Projects/Quarterly report.pdf");
        assert_eq!(second.attachment.relative_path, "Projects/Quarterly report 2.pdf");
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
        assert_eq!(root_attachment.attachment.relative_path, "Quarterly report.pdf");

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
        fs::write(workspace.root.join("test1/doc1.md"), "# Doc 1")
            .expect("doc1 should be written");
        fs::write(workspace.root.join("test2/doc2.md"), "# Doc 2")
            .expect("doc2 should be written");
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
        assert_eq!(fs::read(workspace.root.join(&second.attachment.relative_path)).unwrap(), b"");
        let loaded = load_workspace(&workspace.root, &empty_vault("Attachments"))
            .expect("shared attachments should reload");
        assert_eq!(loaded.vault.attachment_embed_settings, settings);
        assert!(loaded.vault.folders.iter().any(|folder| folder.name == "test1"));
        assert!(loaded.vault.folders.iter().any(|folder| folder.name == "test2"));
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
        fs::write(workspace.root.join("Note.md"), "# Note")
            .expect("note should be written");
        let mut state = WorkspaceState::default();
        state
            .note_paths
            .insert("note-1".to_owned(), "Note.md".to_owned());
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");
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
        assert_eq!(fs::read(workspace.root.join("Archive/Report.pdf")).unwrap(), bytes);
        assert_eq!(fs::read_to_string(workspace.root.join("Note.md")).unwrap(), updated);
        assert_eq!(moved.attachment.relative_path, "Archive/Report.pdf");
        assert_eq!(moved.attachment.byte_length, bytes.len() as u64);
        let (state, _) = read_workspace_state(
            &workspace.root,
            &mut WarningCollector::default(),
        );
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
        assert_eq!(fs::read(workspace.root.join("Elsewhere/Report.pdf")).unwrap(), bytes);
        assert!(!workspace.root.join("Files/Notes/Report.pdf").exists());
    }

    #[test]
    fn attachment_reconciliation_recovers_external_moves_by_stable_id() {
        let source = TestWorkspace::new("moved-attachment-source");
        let workspace = TestWorkspace::new("moved-attachment-target");
        let source_path = source.root.join("Archive.zip");
        fs::write(&source_path, b"unique archive bytes")
            .expect("source archive should be written");
        fs::write(workspace.root.join("Note.md"), "# Note")
            .expect("note should be written");
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
        fs::create_dir(workspace.root.join("Moved"))
            .expect("external destination should be created");
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
        fs::create_dir(workspace.root.join("Files"))
            .expect("attachment folder should be created");
        fs::write(workspace.root.join("Files/Report#1.pdf"), b"portable report")
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
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");
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
        fs::create_dir_all(workspace.root.join(nested))
            .expect("nested vault folder should be created");
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
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");

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
            workspace.root.join(format!("{nested}/Plan.md")).canonicalize().unwrap(),
        );
        let (folder_relative, folder_path) = locate_workspace_vault_item(
            &workspace.root,
            WorkspaceVaultItemKind::Folder,
            nested,
            None,
        )
        .expect("the nested folder should resolve");
        assert_eq!(folder_relative, nested);
        assert_eq!(folder_path, workspace.root.join(nested).canonicalize().unwrap());

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
        fs::write(&extensionless_text, b"ordinary text")
            .expect("extensionless text should be written");
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
        fs::write(&outside_target, b"existing")
            .expect("existing target should be written");
        assert!(validate_external_attachment_copy_target(
            &workspace.root,
            &outside_target,
        )
        .expect_err("archive copies should not overwrite existing files")
        .contains("already exists"));
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

        assert!(validate_attachment_source_file(
            source.root.join("Note.md").to_str().unwrap(),
        )
        .is_err());
        assert!(validate_attachment_source_file(
            source.root.join("Image.png").to_str().unwrap(),
        )
        .is_err());
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
        assert_eq!(loaded.vault.attachment_files[0].relative_path, "External.zip");
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
            unique_attachment_relative_path(&workspace.root, "assets/reports", "Fresh.pdf")
                .unwrap(),
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
        fs::write(source.root.join("Image.png"), PNG)
            .expect("source image should be written");
        fs::write(source.root.join("Report.pdf"), PDF)
            .expect("source attachment should be written");
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
        fs::write(source.root.join("Image.png"), PNG)
            .expect("source image should be written");
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
        fs::write(source.root.join("Image.png"), PNG)
            .expect("source image should be written");
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
        let (state, _) = read_workspace_state(
            &workspace.root,
            &mut WarningCollector::default(),
        );
        assert_eq!(
            state.unwrap().last_committed_image_import_id.as_deref(),
            Some(transaction_id),
        );
    }

    #[test]
    fn interrupted_pending_image_import_recovers_at_the_state_commit_boundary() {
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrecover-pending-import";
        let source = TestWorkspace::new("portable-image-crash-source");
        fs::write(source.root.join("Image.png"), PNG)
            .expect("source image should be written");

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
        fs::write(source.root.join("Image 2.png"), SECOND)
            .expect("second source should be written");
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
        assert_eq!(fs::read(workspace.root.join("Image.png")).unwrap(), EXISTING);
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
        fs::write(workspace.root.join("Note.md"), "before")
            .expect("target note should be written");
        let revision = revision_for_root(&workspace.root)
            .expect("initial revision should be available");

        let mut transaction = prepare_workspace_image_import(&workspace.root, revision)
            .expect("the import transaction should be prepared");
        stage_workspace_image_import(
            &workspace.root,
            &mut transaction,
            "Image.png",
            PNG,
        )
        .expect("the image should be staged privately");
        assert!(transaction
            .transaction_root
            .as_ref()
            .is_some_and(|path| path.is_dir()));
        assert!(!workspace.root.join("Image.png").exists());

        fs::write(workspace.root.join("Note.md"), "external edit")
            .expect("the note should change outside the import");
        let external_revision = revision_for_root(&workspace.root)
            .expect("external revision should be available");
        let mut warnings = WarningCollector::default();
        let error = apply_workspace_image_import(
            &workspace.root,
            transaction,
            &mut warnings,
        )
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
        fs::write(workspace.root.join("Note.md"), "before")
            .expect("target note should be written");
        let baseline = revision_entries_for_root(&workspace.root)
            .expect("baseline should be available");
        let transaction_root = prepare_transaction_root(
            &workspace.root,
            &new_transaction_id(),
        )
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
        let error = verify_image_import_consistency(
            &workspace.root,
            &baseline,
            &manifest,
        )
        .expect_err("the concurrent edit must fail consistency verification");
        assert!(error.contains("vault changed"));

        let mut warnings = WarningCollector::default();
        let recovered = rollback_transaction(
            &workspace.root,
            &transaction_root,
            &manifest,
            &mut warnings,
        );
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
        let transaction_root = prepare_transaction_root(
            &workspace.root,
            &new_transaction_id(),
        )
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
            workspace.root.join("Source link.png").to_str().expect("path should be Unicode")
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
            BTreeMap::from([(
                "Source link.png".to_owned(),
                "Source link 2.png".to_owned(),
            )]),
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
            append_external_file_upload(&upload.id, 0, b"abc")
                .expect("first chunk should append"),
            3,
        );
        assert_eq!(
            append_external_file_upload(&upload.id, 3, b"def")
                .expect("second chunk should append"),
            6,
        );

        let staged = finish_external_file_upload(
            &upload.id,
            ExternalFileUploadKind::Attachment,
        )
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
        append_external_file_upload(&upload.id, 0, b"ab")
            .expect("partial chunk should append");
        let error = finish_external_file_upload(
            &upload.id,
            ExternalFileUploadKind::Attachment,
        )
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
        let staged_empty = finish_external_file_upload(
            &empty_attachment.id,
            ExternalFileUploadKind::Attachment,
        )
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
        append_external_file_upload(&image.id, 0, b"x")
            .expect("the declared image byte should append");
        let mismatch = finish_external_file_upload(
            &image.id,
            ExternalFileUploadKind::Attachment,
        )
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
        append_external_file_upload(&image.id, 0, PNG)
            .expect("image bytes should append");
        let staged_image = finish_external_file_upload(
            &image.id,
            ExternalFileUploadKind::Image,
        )
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
        let staged_attachment = finish_external_file_upload(
            &attachment.id,
            ExternalFileUploadKind::Attachment,
        )
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
                    last_activity: if index == 0 {
                        abandoned_activity
                    } else {
                        now
                    },
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
        fs::write(vault.root.join("Note.md"), "# Note")
            .expect("saved note should be written");
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
        set_file_modified_millis(&stale_file, 0)
            .expect("stale modified time should be set");
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
        fs::write(vault.root.join("Note.md"), "# Note")
            .expect("note should be written");
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
        append_external_file_upload(&image_upload.id, 0, PNG)
            .expect("image bytes should append");
        let staged_image = finish_external_file_upload(
            &image_upload.id,
            ExternalFileUploadKind::Image,
        )
        .expect("image upload should finish");
        let image_source = validate_image_source_path(&staged_image.path)
            .expect("the staged image should remain a safe source file");
        let image_bytes = read_image_file(&image_source)
            .expect("the staged image should be readable");
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
        let staged_attachment = finish_external_file_upload(
            &attachment_upload.id,
            ExternalFileUploadKind::Attachment,
        )
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

        assert_eq!(
            fs::read(vault.root.join("Dragged photo.png")).unwrap(),
            PNG,
        );
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
        fs::write(vault.root.join("Note.md"), "# Note")
            .expect("saved note should be written");
        let mut state = WorkspaceState::default();
        state
            .note_paths
            .insert("note-1".to_owned(), "Note.md".to_owned());
        write_workspace_state(&vault.root, &state)
            .expect("workspace state should be written");

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

        let (stored, _) = read_workspace_state(
            &vault.root,
            &mut WarningCollector::default(),
        );
        assert!(stored.unwrap().assets.is_empty());
    }

    #[test]
    fn external_asset_cleanup_retains_referenced_changed_and_stale_files() {
        let source = TestWorkspace::new("retained-external-asset-source");
        let vault = TestWorkspace::new("retained-external-asset-vault");
        fs::write(vault.root.join("Note.md"), "# Note")
            .expect("saved note should be written");
        let mut state = WorkspaceState::default();
        state
            .note_paths
            .insert("note-1".to_owned(), "Note.md".to_owned());
        write_workspace_state(&vault.root, &state)
            .expect("workspace state should be written");
        let source_path = source.root.join("Retained.pdf");
        fs::write(&source_path, b"original")
            .expect("attachment source should be written");

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

        fs::write(vault.root.join("Note.md"), "# Note")
            .expect("saved reference should be removed");
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
        fs::write(vault.root.join("Note.md"), "# Note")
            .expect("note should be written");
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
        append_external_file_upload(&upload.id, 0, b"dropped")
            .expect("file bytes should append");

        fs::write(vault.root.join("Note.md"), "# Note\n\nTyped while streaming")
            .expect("the acknowledged note save should be written");
        let finish_revision = revision_for_root(&vault.root).unwrap();
        assert_ne!(finish_revision, begin_revision);

        let staged = finish_external_file_upload(
            &upload.id,
            ExternalFileUploadKind::Attachment,
        )
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
        assert_eq!(
            attachment.attachment.relative_path,
            "Typed during drop.txt",
        );
        assert_eq!(
            fs::read(vault.root.join("Typed during drop.txt")).unwrap(),
            b"dropped",
        );
    }
}
