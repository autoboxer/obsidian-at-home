use serde::{Deserialize, Serialize};
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
const MAX_VAULT_ASSETS: usize = 100_000;
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
const ATTACHMENT_COPY_BUFFER_BYTES: usize = 256 * 1024;
const EXTERNAL_FILE_UPLOAD_DIRECTORY: &str = "external-file-drops";
const EXTERNAL_FILE_UPLOAD_CHUNK_BYTES: usize = 512 * 1024;
const MAX_EXTERNAL_FILE_UPLOADS: usize = 16;
const ABANDONED_EXTERNAL_FILE_UPLOAD_MILLIS: u64 = 5 * 60 * 1000;
const STALE_EXTERNAL_FILE_UPLOAD_MILLIS: u64 = 24 * 60 * 60 * 1000;

static WORKSPACE_IO_LOCK: Mutex<()> = Mutex::new(());
static EXTERNAL_FILE_UPLOADS: LazyLock<Mutex<HashMap<String, ExternalFileUpload>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
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
#[serde(rename_all = "kebab-case")]
pub enum ImageEmbedLocation {
    VaultRoot,
    NoteFolder,
    SpecifiedFolder,
    SpecifiedFolderMirrored,
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

#[derive(Debug)]
struct ExternalFileUpload {
    directory: PathBuf,
    path: PathBuf,
    file: Option<File>,
    file_name: String,
    expected_length: u64,
    received_length: u64,
    root: PathBuf,
    note_relative_path: String,
    kind: ExternalFileUploadKind,
    last_activity: Instant,
    cleanup_on_drop: bool,
}

impl Drop for ExternalFileUpload {
    fn drop(&mut self) {
        if !self.cleanup_on_drop {
            return;
        }
        self.file.take();
        let _ = remove_file_durable(&self.path);
        let _ = remove_directory_durable(&self.directory);
    }
}

#[derive(Debug)]
struct StagedExternalFile {
    directory: PathBuf,
    path: PathBuf,
    file_name: String,
    root: PathBuf,
    note_relative_path: String,
}

impl Drop for StagedExternalFile {
    fn drop(&mut self) {
        let _ = remove_file_durable(&self.path);
        let _ = remove_directory_durable(&self.directory);
    }
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
struct StoredRecentlyDeletedNote {
    deleted_at: u64,
    expires_at: u64,
    fingerprint: FileFingerprint,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum VaultAssetKind {
    Image,
    Attachment,
}

impl Default for VaultAssetKind {
    fn default() -> Self {
        Self::Image
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StoredVaultAsset {
    #[serde(default)]
    kind: VaultAssetKind,
    relative_path: String,
    media_type: String,
    fingerprint: FileFingerprint,
    #[serde(default)]
    modified_nanos: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct RecentlyDeletedSnapshot {
    version: u32,
    deleted_note: RecentlyDeletedNote,
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
    #[serde(default)]
    recent_note_ids: Vec<String>,
    #[serde(default)]
    recently_deleted_notes: BTreeMap<String, StoredRecentlyDeletedNote>,
    #[serde(default, alias = "imageAssets")]
    assets: BTreeMap<String, StoredVaultAsset>,
    #[serde(default)]
    image_embed_settings: ImageEmbedSettings,
    #[serde(default)]
    attachment_embed_settings: AttachmentEmbedSettings,
    #[serde(default = "default_folder_selection")]
    selected_folder_id: String,
    #[serde(default)]
    last_committed_transaction_id: Option<String>,
    #[serde(default)]
    last_committed_image_import_id: Option<String>,
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
            recent_note_ids: Vec::new(),
            recently_deleted_notes: BTreeMap::new(),
            assets: BTreeMap::new(),
            image_embed_settings: ImageEmbedSettings::default(),
            attachment_embed_settings: AttachmentEmbedSettings::default(),
            selected_folder_id: default_folder_selection(),
            last_committed_transaction_id: None,
            last_committed_image_import_id: None,
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
}

#[derive(Debug)]
struct SaveConsistency {
    unaffected: BTreeMap<String, FileStamp>,
    targets: Vec<TransactionTarget>,
}

type RevisionEntry = (String, Option<(u64, u128)>);

struct WorkspaceImageImportTransaction {
    baseline: Vec<RevisionEntry>,
    targets: Vec<TransactionTarget>,
    transaction_root: Option<PathBuf>,
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
    managed_by_note_move: bool,
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
        managed_by_note_move,
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
    managed_by_note_move: bool,
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
        managed_by_note_move,
    )
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
    let (scanned_notes, scanned_folders, scanned_images, scanned_attachments) =
        scan_workspace_files(&root, &mut warnings)?;

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
    let state_ids_are_trustworthy = state_was_present || !state_file_was_present;
    let (editor_positions, editor_positions_writable, editor_positions_revision) =
        if state_ids_are_trustworthy {
            load_editor_positions(&root, &note_ids, &mut warnings)
        } else {
            (BTreeMap::new(), false, None)
        };
    let active_note_id = state
        .active_note_id
        .filter(|id| note_ids.contains(id.as_str()))
        .or_else(|| notes.first().map(|note| note.id.clone()));
    let recent_note_ids = normalize_recent_note_ids(
        &state.recent_note_ids,
        active_note_id.as_deref(),
        &note_ids,
    );
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
    let embedded_images = reconcile_image_assets(&root, &mut state.assets, &mut warnings);
    let tracked_image_ids = embedded_images
        .iter()
        .map(|image| (portable_path_key(&image.relative_path), image.id.clone()))
        .collect::<HashMap<_, _>>();
    let image_files = scanned_images
        .into_iter()
        .map(|image| VaultImageFile {
            asset_id: tracked_image_ids
                .get(&portable_path_key(&image.relative_path))
                .cloned(),
            relative_path: image.relative_path,
            media_type: image.media_type,
        })
        .collect();
    let embedded_attachments =
        reconcile_attachment_assets(&root, &mut state.assets, &mut warnings);
    let tracked_attachment_ids = embedded_attachments
        .iter()
        .map(|attachment| {
            (
                portable_path_key(&attachment.relative_path),
                attachment.id.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    let attachment_files = scanned_attachments
        .into_iter()
        .map(|attachment| VaultAttachmentFile {
            asset_id: tracked_attachment_ids
                .get(&portable_path_key(&attachment.relative_path))
                .cloned(),
            relative_path: attachment.relative_path,
            media_type: attachment.media_type,
            byte_length: attachment.byte_length,
            opening_disabled: attachment.opening_disabled,
        })
        .collect();
    let image_embed_settings = match migrate_legacy_image_embed_settings(
        &state.image_embed_settings,
    ) {
        Ok(settings) => settings,
        Err(error) => {
            warnings.push(format!("Reset invalid image embed settings: {error}"));
            ImageEmbedSettings::default()
        }
    };
    let attachment_embed_settings =
        match migrate_legacy_attachment_embed_settings(&state.attachment_embed_settings) {
            Ok(settings) => settings,
            Err(error) => {
                warnings.push(format!("Reset invalid attachment embed settings: {error}"));
                AttachmentEmbedSettings::default()
            }
        };
    let vault_name = display_vault_name(
        if state_was_present && !state.name.trim().is_empty() {
            &state.name
        } else {
            ""
        },
        &root,
    );
    let mut recently_deleted_state = state.recently_deleted_notes.clone();
    let now = now_millis();
    if recently_deleted_state
        .values()
        .any(|entry| now >= entry.expires_at)
    {
        let expired_ids = recently_deleted_state
            .iter()
            .filter_map(|(id, entry)| (now >= entry.expires_at).then(|| id.clone()))
            .collect::<Vec<_>>();
        for id in expired_ids {
            let entry = recently_deleted_state
                .get(&id)
                .expect("expired recovery entry should still exist")
                .clone();
            if remove_expired_recovery_snapshot(&root, &id, &entry, &mut warnings) {
                recently_deleted_state.remove(&id);
            }
        }
    }
    let recently_deleted_notes = load_recently_deleted_notes(
        &root,
        &recently_deleted_state,
        &mut warnings,
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
        recent_note_ids: recent_note_ids.clone(),
        recently_deleted_notes: recently_deleted_state,
        assets: state.assets.clone(),
        image_embed_settings: image_embed_settings.clone(),
        attachment_embed_settings: attachment_embed_settings.clone(),
        selected_folder_id: selected_folder_id.clone(),
        last_committed_transaction_id: state.last_committed_transaction_id.clone(),
        last_committed_image_import_id: state.last_committed_image_import_id.clone(),
    };
    let mut state_was_written = false;
    if state_was_present || !state_file_was_present {
        match write_workspace_state(&root, &state) {
            Ok(()) => state_was_written = true,
            Err(error) => warnings.push(format!("Could not save workspace metadata: {error}")),
        }
    } else {
        warnings.push(
            "Workspace metadata was not replaced because the existing file could not be read."
                .to_owned(),
        );
    }
    if state_was_written {
        cleanup_orphaned_recovery_snapshots(
            &root,
            &state.recently_deleted_notes,
            &HashSet::new(),
            &mut warnings,
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
            recent_note_ids,
            selected_folder_id,
            embedded_images,
            image_files,
            image_embed_settings,
            embedded_attachments,
            attachment_files,
            attachment_embed_settings,
        },
        descriptor: VaultDescriptor {
            name: vault_name,
            path: path_string(&root)?,
            last_opened_at: opened_at,
        },
        recently_deleted_notes,
        editor_positions,
        editor_positions_revision,
        editor_positions_writable,
        warnings: warnings.finish(),
        revision,
    })
}

fn save_workspace_files(
    root: &Path,
    vault: &VaultData,
    expected_revision: u64,
) -> Result<SaveResult, String> {
    save_workspace_files_with_archive(root, vault, expected_revision, None)
        .map(|(result, _)| result)
}

fn save_workspace_files_with_image_import(
    root: &Path,
    vault: &VaultData,
    expected_revision: u64,
    transaction_id: &str,
) -> Result<WorkspaceImportSaveResult, String> {
    pending_workspace_image_import(root, transaction_id)?;
    let result = save_workspace_files_with_recovery(
        root,
        vault,
        expected_revision,
        None,
        None,
        Some(transaction_id),
    );
    match result {
        Ok((mut result, _, _)) => {
            let mut cleanup_warnings = WarningCollector::default();
            if let Err(error) = finalize_workspace_image_import(
                root,
                transaction_id,
                &mut cleanup_warnings,
            ) {
                result.warnings.push(format!(
                    "The imported assets were saved, but their transaction cleanup will be retried when the vault reopens: {error}"
                ));
            }
            result.warnings.extend(cleanup_warnings.finish());

            Ok(WorkspaceImportSaveResult {
                saved: true,
                error: None,
                note_paths: result.note_paths,
                revision: result.revision,
                saved_at: result.saved_at,
                warnings: result.warnings,
            })
        }
        Err(error) => {
            let mut state_warnings = WarningCollector::default();
            let (state, state_file_was_present) = read_workspace_state(root, &mut state_warnings);
            let committed = state.as_ref().is_some_and(|state| {
                state.last_committed_image_import_id.as_deref() == Some(transaction_id)
            });
            if committed {
                let mut cleanup_warnings = WarningCollector::default();
                if let Err(cleanup_error) = finalize_workspace_image_import(
                    root,
                    transaction_id,
                    &mut cleanup_warnings,
                ) {
                    cleanup_warnings.push(format!(
                        "The committed asset import will be cleaned up when the vault reopens: {cleanup_error}"
                    ));
                }
                let state = state.expect("committed state should be available");
                let mut warnings = state_warnings.finish();
                warnings.extend(cleanup_warnings.finish());
                warnings.push(format!(
                    "The import was saved, but its final verification reported: {error}"
                ));

                return Ok(WorkspaceImportSaveResult {
                    saved: true,
                    error: None,
                    note_paths: state.note_paths,
                    revision: revision_for_root(root)?,
                    saved_at: now_millis(),
                    warnings,
                });
            }
            if state.is_none() && state_file_was_present {
                return Err(format!(
                    "{error} The asset import could not be rolled back because workspace metadata became unreadable. Reopen the vault before editing again."
                ));
            }

            let mut rollback_warnings = WarningCollector::default();
            let recovered = rollback_workspace_image_import(
                root,
                transaction_id,
                &mut rollback_warnings,
            )?;
            if !recovered {
                return Err(format!(
                    "{error} The imported assets could not be fully rolled back. Reopen the vault before editing again."
                ));
            }
            let revision = revision_for_root(root)?;
            if revision_for_root(root)? != revision {
                return Err(format!(
                    "{error} The vault changed while the failed asset import was being rolled back. Reload it before editing again."
                ));
            }
            let mut warnings = state_warnings.finish();
            warnings.extend(rollback_warnings.finish());

            Ok(WorkspaceImportSaveResult {
                saved: false,
                error: Some(error),
                note_paths: BTreeMap::new(),
                revision,
                saved_at: now_millis(),
                warnings,
            })
        }
    }
}

fn save_workspace_files_with_archive(
    root: &Path,
    vault: &VaultData,
    expected_revision: u64,
    pending_archive: Option<PendingNoteArchive>,
) -> Result<(SaveResult, Option<RecentlyDeletedNote>), String> {
    save_workspace_files_with_recovery(
        root,
        vault,
        expected_revision,
        pending_archive,
        None,
        None,
    )
    .map(|(result, deleted_note, _)| (result, deleted_note))
}

fn save_workspace_files_with_restore(
    root: &Path,
    vault: &VaultData,
    expected_revision: u64,
    pending_restore: PendingNoteRestore,
) -> Result<(SaveResult, PreparedNoteRestore), String> {
    let (result, _, prepared_restore) = save_workspace_files_with_recovery(
        root,
        vault,
        expected_revision,
        None,
        Some(pending_restore),
        None,
    )?;
    let prepared_restore = prepared_restore
        .ok_or_else(|| "The note was restored without recovery metadata.".to_owned())?;

    Ok((result, prepared_restore))
}

fn save_workspace_files_with_recovery(
    root: &Path,
    vault: &VaultData,
    expected_revision: u64,
    pending_archive: Option<PendingNoteArchive>,
    pending_restore: Option<PendingNoteRestore>,
    pending_image_import_id: Option<&str>,
) -> Result<
    (
        SaveResult,
        Option<RecentlyDeletedNote>,
        Option<PreparedNoteRestore>,
    ),
    String,
> {
    if pending_archive.is_some() && pending_restore.is_some() {
        return Err("A note cannot be archived and restored in the same save.".to_owned());
    }
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
    recover_workspace_transactions_except(
        &root,
        Some(&old_state),
        pending_image_import_id,
        &mut warnings,
    )?;
    if revision_for_root(&root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before saving so those changes are not overwritten."
                .to_owned(),
        );
    }

    let desired_folder_paths = build_folder_paths(&vault.folders)?;
    let prepared_restore = pending_restore
        .map(|restore| {
            prepare_note_restore(
                &root,
                vault,
                &old_state,
                &desired_folder_paths,
                restore,
            )
        })
        .transpose()?;
    let preferred_new_paths = prepared_restore
        .as_ref()
        .map(|restore| {
            BTreeMap::from([(
                restore.restored_note.id.clone(),
                restore.restored_note.relative_path.clone(),
            )])
        })
        .unwrap_or_default();
    let plans = build_note_write_plans(
        &root,
        vault,
        &old_state,
        &desired_folder_paths,
        &preferred_new_paths,
    )?;
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
    let prepared_archive = pending_archive
        .map(|archive| prepare_note_archive(&root, vault, &old_state, archive, saved_at))
        .transpose()?;
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
    let note_ids: HashSet<&str> = vault.notes.iter().map(|note| note.id.as_str()).collect();
    let recent_note_ids = normalize_recent_note_ids(
        &vault.recent_note_ids,
        vault.active_note_id.as_deref(),
        &note_ids,
    );
    let mut recently_deleted_notes = old_state.recently_deleted_notes.clone();
    if let Some(archive) = &prepared_archive {
        recently_deleted_notes.insert(
            archive.deleted_note.id.clone(),
            StoredRecentlyDeletedNote {
                deleted_at: archive.deleted_note.deleted_at,
                expires_at: archive.deleted_note.expires_at,
                fingerprint: archive.fingerprint.clone(),
            },
        );
    }
    if let Some(restore) = &prepared_restore {
        recently_deleted_notes.remove(&restore.recovery_id);
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
        recent_note_ids,
        recently_deleted_notes,
        assets: old_state.assets.clone(),
        image_embed_settings: migrate_legacy_image_embed_settings(&vault.image_embed_settings)?,
        attachment_embed_settings: migrate_legacy_attachment_embed_settings(
            &vault.attachment_embed_settings,
        )?,
        selected_folder_id: vault.selected_folder_id.clone(),
        last_committed_transaction_id: old_state.last_committed_transaction_id.clone(),
        last_committed_image_import_id: pending_image_import_id
            .map(str::to_owned)
            .or_else(|| old_state.last_committed_image_import_id.clone()),
    };

    let needs_transaction = prepared_archive.is_some()
        || !paths_to_replace.is_empty()
        || plans.iter().any(|plan| plan.needs_write)
        || !folder_case_renames.is_empty()
        || !created_directories.is_empty();
    if needs_transaction {
        let transaction_id = new_transaction_id();
        let recovery_archives = prepared_archive
            .as_ref()
            .map(std::slice::from_ref)
            .unwrap_or(&[]);
        let (transaction_root, mut manifest) = prepare_transaction(
            &root,
            transaction_id,
            &paths_to_replace,
            &plans,
            recovery_archives,
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

        if let Err(error) = apply_transaction(
            &root,
            &transaction_root,
            &manifest,
            &plans,
            &mut warnings,
        ) {
            let recovered = rollback_transaction(
                &root,
                &transaction_root,
                &manifest,
                &mut warnings,
            );
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(error);
        }
        if let Err(error) = verify_save_consistency(&root, &consistency) {
            let recovered = rollback_transaction(
                &root,
                &transaction_root,
                &manifest,
                &mut warnings,
            );
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(error);
        }
        if let Err(error) = verify_applied_recovery_targets(
            &root,
            &manifest.recovery_targets,
        ) {
            let recovered = rollback_transaction(
                &root,
                &transaction_root,
                &manifest,
                &mut warnings,
            );
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(error);
        }
        if fingerprint_regular_file(&state_path)? != expected_state_fingerprint {
            let recovered = rollback_transaction(
                &root,
                &transaction_root,
                &manifest,
                &mut warnings,
            );
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(
                "Workspace metadata changed outside Obsidian At Home while saving. Reload before editing again."
                    .to_owned(),
            );
        }
        if let Some(restore) = &prepared_restore {
            if let Err(error) = verify_recovery_snapshot_target(
                &root,
                &restore.recovery_id,
                &restore.fingerprint,
            ) {
                let recovered = rollback_transaction(
                    &root,
                    &transaction_root,
                    &manifest,
                    &mut warnings,
                );
                if recovered {
                    discard_private_transaction(&root, &transaction_root, &mut warnings);
                }

                return Err(error);
            }
        }

        state.last_committed_transaction_id = Some(manifest.id.clone());
        if let Err(error) = write_workspace_state(&root, &state) {
            let recovered = rollback_transaction(
                &root,
                &transaction_root,
                &manifest,
                &mut warnings,
            );
            if recovered {
                discard_private_transaction(&root, &transaction_root, &mut warnings);
            }

            return Err(format!("Could not save workspace metadata: {error}"));
        }
        finalize_committed_recovery_targets(
            &root,
            &transaction_root,
            &manifest.recovery_targets,
        )
        .map_err(|error| {
            format!(
                "The vault was saved, but its recovery snapshot could not be finalized. Reopen the vault before editing again. {error}"
            )
        })?;
        // The state file is the commit boundary. Persist the same fact in the
        // manifest before cleanup so an undeletable old transaction can never
        // be mistaken for an uncommitted save after a later transaction.
        manifest.phase = TransactionPhase::Committed;
        let transaction_was_finalized = match write_transaction_manifest(
            &transaction_root,
            &manifest,
        ) {
            Ok(()) => true,
            Err(error) if prepared_restore.is_some() => {
                warnings.push(format!(
                    "The note was restored, but its save transaction will be cleaned up the next \
                     time the vault opens: {error}"
                ));
                false
            }
            Err(error) => {
                return Err(format!(
                    "The vault was saved, but its transaction could not be finalized. Reopen the \
                     vault before editing again. {error}"
                ));
            }
        };
        if let Err(error) = verify_save_consistency(&root, &consistency) {
            if prepared_restore.is_some() {
                warnings.push(format!(
                    "The note was restored, but the vault changed as the restore was committed. \
                     Reload before editing again. {error}"
                ));
            } else {
                return Err(format!(
                    "The vault changed as the save was committed. Reload before editing again. \
                     {error}"
                ));
            }
        }
        if transaction_was_finalized {
            discard_private_transaction(&root, &transaction_root, &mut warnings);
        }
    } else {
        verify_save_consistency(&root, &consistency)?;
        if fingerprint_regular_file(&state_path)? != expected_state_fingerprint {
            return Err(
                "Workspace metadata changed outside Obsidian At Home while saving. Reload before editing again."
                    .to_owned(),
            );
        }
        if let Some(restore) = &prepared_restore {
            verify_recovery_snapshot_target(
                &root,
                &restore.recovery_id,
                &restore.fingerprint,
            )?;
        }
        write_workspace_state(&root, &state)?;
    }

    if let Some(restore) = &prepared_restore {
        remove_recovery_snapshot_if_matches(
            &root,
            &restore.recovery_id,
            &restore.fingerprint,
            &mut warnings,
        );
    }

    remove_empty_managed_directories(&root, &old_state.folder_paths, &state.folder_paths, &mut warnings);
    let revision = if prepared_restore.is_some() {
        if let Err(error) = verify_save_consistency(&root, &consistency) {
            warnings.push(format!(
                "The note was restored, but the vault changed immediately afterward. Reload \
                 before editing again. {error}"
            ));
        }
        let revision = match revision_for_root(&root) {
            Ok(revision) => revision,
            Err(error) => {
                warnings.push(format!(
                    "The note was restored, but the new vault revision could not be read. Reload \
                     before editing again. {error}"
                ));
                expected_revision
            }
        };
        if let Err(error) = verify_save_consistency(&root, &consistency) {
            warnings.push(format!(
                "The note was restored, but the vault no longer matches the committed restore. \
                 Reload before editing again. {error}"
            ));
        }
        match revision_for_root(&root) {
            Ok(current_revision) if current_revision != revision => warnings.push(
                "The note was restored, but the vault changed immediately afterward. Reload \
                 before editing again."
                    .to_owned(),
            ),
            Ok(_) => {}
            Err(error) => warnings.push(format!(
                "The note was restored, but its revision could not be confirmed. Reload before \
                 editing again. {error}"
            )),
        }
        revision
    } else {
        verify_save_consistency(&root, &consistency)?;
        let revision = revision_for_root(&root)?;
        verify_save_consistency(&root, &consistency)?;
        if revision_for_root(&root)? != revision {
            return Err(
                "The vault changed immediately after saving. Reload it before editing again."
                    .to_owned(),
            );
        }
        revision
    };
    let deleted_note = prepared_archive.map(|archive| archive.deleted_note);
    Ok((
        SaveResult {
            note_paths: state.note_paths.clone(),
            revision,
            saved_at,
            warnings: warnings.finish(),
        },
        deleted_note,
        prepared_restore,
    ))
}

fn prepare_note_archive(
    root: &Path,
    vault: &VaultData,
    old_state: &WorkspaceState,
    mut pending: PendingNoteArchive,
    deleted_at: u64,
) -> Result<PreparedNoteArchive, String> {
    if pending.note.id.trim().is_empty() {
        return Err("The deleted note has an invalid ID.".to_owned());
    }
    if pending.note.content.len() as u64 > MAX_NOTE_BYTES {
        return Err(format!(
            "The note {:?} is larger than {} MiB and cannot be archived.",
            pending.note.title,
            MAX_NOTE_BYTES / 1024 / 1024,
        ));
    }
    if pending
        .editor_position
        .as_ref()
        .is_some_and(|position| !is_valid_editor_position(position))
    {
        return Err("The deleted note has an invalid editor position.".to_owned());
    }
    if vault.notes.iter().any(|note| note.id == pending.note.id) {
        return Err("Remove the note from the live vault before archiving it.".to_owned());
    }

    let removed_note_ids = old_state
        .note_paths
        .keys()
        .filter(|id| !vault.notes.iter().any(|note| note.id == id.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    if removed_note_ids != [pending.note.id.as_str()] {
        return Err(
            "Exactly one saved note must be removed when creating a recovery snapshot."
                .to_owned(),
        );
    }

    let original_relative_path = old_state
        .note_paths
        .get(&pending.note.id)
        .ok_or_else(|| "The note must be saved before it can be archived.".to_owned())?;
    validate_markdown_relative_path(original_relative_path)?;
    let original_path = resolve_workspace_file(root, original_relative_path, false)?;
    let stored_content = fs::read_to_string(&original_path)
        .map_err(|error| format!("Could not read the note before archiving it: {error}"))?;
    let requested_content = content_with_requested_tags(&pending.note, Some(&stored_content))?;
    if requested_content.as_bytes() != stored_content.as_bytes() {
        return Err(
            "The note changed before it could be archived. Save it and try again.".to_owned(),
        );
    }
    pending.note.content = stored_content;
    let stored_folder_path = Path::new(original_relative_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    if pending.original_folder_path != stored_folder_path {
        return Err(
            "The note's original folder changed before it could be archived. Reload the vault and try again."
                .to_owned(),
        );
    }
    if !pending.original_folder_path.is_empty() {
        validate_relative_path(&pending.original_folder_path, false)?;
    }
    pending.note.relative_path = original_relative_path.clone();

    validate_recently_deleted_capacity(&old_state.recently_deleted_notes, 0)?;
    let id = new_recently_deleted_id(root, &old_state.recently_deleted_notes)?;
    let expires_at = deleted_at.saturating_add(RECENTLY_DELETED_RETENTION_MILLIS);
    let deleted_note = RecentlyDeletedNote {
        id,
        note: pending.note,
        original_folder_path: pending.original_folder_path,
        deleted_at,
        expires_at,
        editor_position: pending.editor_position,
    };
    let snapshot = RecentlyDeletedSnapshot {
        version: RECENTLY_DELETED_SNAPSHOT_VERSION,
        deleted_note: deleted_note.clone(),
    };
    let mut bytes = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| format!("Could not encode the recovery snapshot: {error}"))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
        return Err("The recovery snapshot is unexpectedly large.".to_owned());
    }
    validate_recently_deleted_capacity(
        &old_state.recently_deleted_notes,
        bytes.len() as u64,
    )?;
    let fingerprint = fingerprint_bytes(&bytes);

    Ok(PreparedNoteArchive {
        deleted_note,
        bytes,
        fingerprint,
    })
}

fn validate_recently_deleted_capacity(
    stored: &BTreeMap<String, StoredRecentlyDeletedNote>,
    additional_bytes: u64,
) -> Result<(), String> {
    if stored.len() >= MAX_RECENTLY_DELETED_NOTES && additional_bytes > 0 {
        return Err(format!(
            "Recently Deleted can contain at most {MAX_RECENTLY_DELETED_NOTES} notes."
        ));
    }

    let mut total_bytes = additional_bytes;
    for (id, entry) in stored {
        validate_recently_deleted_id(id)?;
        if entry.fingerprint.length > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
            return Err("A stored recovery snapshot is unexpectedly large.".to_owned());
        }
        total_bytes = total_bytes
            .checked_add(entry.fingerprint.length)
            .ok_or_else(|| "Recently Deleted is too large to measure safely.".to_owned())?;
    }
    if total_bytes > MAX_RECENTLY_DELETED_BYTES {
        return Err(format!(
            "Recently Deleted cannot contain more than {} MiB of note snapshots.",
            MAX_RECENTLY_DELETED_BYTES / 1024 / 1024,
        ));
    }

    Ok(())
}

fn new_recently_deleted_id(
    root: &Path,
    stored: &BTreeMap<String, StoredRecentlyDeletedNote>,
) -> Result<String, String> {
    for _ in 0..100 {
        let id = format!("deleted-{}", new_transaction_id());
        if stored.contains_key(&id) {
            continue;
        }
        let path = recently_deleted_snapshot_path(root, &id)?;
        match fs::symlink_metadata(path) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(id),
            Err(error) => {
                return Err(format!("Could not inspect the recovery snapshot folder: {error}"));
            }
        }
    }

    Err("Could not allocate a unique recovery snapshot ID.".to_owned())
}

fn validate_recently_deleted_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 180
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("A recovery snapshot has an invalid ID.".to_owned());
    }

    Ok(())
}

fn read_recovery_for_restore(
    root: &Path,
    deleted_note_id: &str,
    expected_revision: u64,
) -> Result<(WorkspaceState, RecentlyDeletedNote), String> {
    validate_recently_deleted_id(deleted_note_id)?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before restoring the note."
                .to_owned(),
        );
    }

    let state_path = workspace_state_path(root);
    let expected_state_fingerprint = fingerprint_regular_file(&state_path)?;
    let mut warnings = WarningCollector::default();
    let (state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    let state = state.ok_or_else(|| {
        if state_file_was_present {
            "Workspace metadata is unreadable or newer than this app.".to_owned()
        } else {
            "Workspace metadata is missing. Reopen the vault before restoring the note."
                .to_owned()
        }
    })?;
    recover_workspace_transactions(root, Some(&state), &mut warnings)?;
    if fingerprint_regular_file(&state_path)? != expected_state_fingerprint
        || revision_for_root(root)? != expected_revision
    {
        return Err(
            "The vault changed while the deleted note was being read. Reload it and try again."
                .to_owned(),
        );
    }

    inspect_recently_deleted_directory(root)?;
    let stored = state
        .recently_deleted_notes
        .get(deleted_note_id)
        .ok_or_else(|| "That deleted note is no longer available.".to_owned())?;
    if stored.expires_at <= now_millis() {
        return Err("That deleted note has expired and can no longer be restored.".to_owned());
    }
    let deleted_note = read_indexed_recently_deleted_note(root, deleted_note_id, stored)?;
    if fingerprint_regular_file(&state_path)? != expected_state_fingerprint
        || revision_for_root(root)? != expected_revision
    {
        return Err(
            "The vault changed while the deleted note was being read. Reload it and try again."
                .to_owned(),
        );
    }

    Ok((state, deleted_note))
}

fn build_restored_note(
    root: &Path,
    vault: &VaultData,
    state: &WorkspaceState,
    deleted_note: &RecentlyDeletedNote,
) -> Result<(Note, String), String> {
    let folder_paths = build_folder_paths(&vault.folders)?;
    let existing_plans = build_note_write_plans(
        root,
        vault,
        state,
        &folder_paths,
        &BTreeMap::new(),
    )?;
    let original_folder_id = folder_paths
        .iter()
        .find_map(|(id, path)| (path == &deleted_note.original_folder_path).then(|| id.clone()));
    let target_folder_path = original_folder_id
        .as_ref()
        .and_then(|id| folder_paths.get(id))
        .map(String::as_str)
        .unwrap_or("");

    let original_path = Path::new(&deleted_note.note.relative_path);
    let original_stem = original_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Untitled note");
    let extension = original_path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| value.eq_ignore_ascii_case("md") || value.eq_ignore_ascii_case("markdown"))
        .unwrap_or("md");

    let mut occupied_paths = existing_plans
        .iter()
        .map(|plan| portable_path_key(&plan.new_relative_path))
        .collect::<HashSet<_>>();
    occupied_paths.extend(folder_paths.values().map(|path| portable_path_key(path)));
    occupied_paths.extend(note_file_stamps(root)?.into_keys());

    let mut preferred_relative_path = None;
    let mut restored_title = String::new();
    for suffix in 1..=MAX_NOTES {
        let title = if suffix == 1 {
            original_stem.to_owned()
        } else {
            format!("{original_stem} {suffix}")
        };
        let file_name = format!("{title}.{extension}");
        let candidate = if target_folder_path.is_empty() {
            file_name
        } else {
            format!("{target_folder_path}/{file_name}")
        };
        validate_markdown_relative_path(&candidate)?;
        if occupied_paths.contains(&portable_path_key(&candidate)) {
            continue;
        }
        let path = resolve_workspace_file(root, &candidate, true)?;
        match fs::symlink_metadata(path) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                restored_title = title;
                preferred_relative_path = Some(candidate);
                break;
            }
            Err(error) => {
                return Err(format!("Could not inspect the restore destination: {error}"));
            }
        }
    }
    let preferred_relative_path = preferred_relative_path
        .ok_or_else(|| "Could not find a safe file name for the restored note.".to_owned())?;

    let mut used_ids = vault
        .notes
        .iter()
        .map(|note| note.id.clone())
        .collect::<HashSet<_>>();
    let restored_id = if used_ids.insert(deleted_note.note.id.clone()) {
        deleted_note.note.id.clone()
    } else {
        fresh_id("note", &preferred_relative_path, &mut used_ids)
    };
    let mut restored_note = deleted_note.note.clone();
    restored_note.id = restored_id;
    restored_note.title = restored_title;
    restored_note.folder_id = original_folder_id;
    restored_note.relative_path = preferred_relative_path.clone();

    Ok((restored_note, preferred_relative_path))
}

fn prepare_note_restore(
    root: &Path,
    vault: &VaultData,
    old_state: &WorkspaceState,
    folder_paths: &BTreeMap<String, String>,
    pending: PendingNoteRestore,
) -> Result<PreparedNoteRestore, String> {
    validate_recently_deleted_id(&pending.deleted_note_id)?;
    inspect_recently_deleted_directory(root)?;
    let stored = old_state
        .recently_deleted_notes
        .get(&pending.deleted_note_id)
        .ok_or_else(|| "That deleted note is no longer available.".to_owned())?;
    let deleted_note = read_indexed_recently_deleted_note(
        root,
        &pending.deleted_note_id,
        stored,
    )?;

    let restored = &pending.restored_note;
    if restored.content != deleted_note.note.content
        || restored.tags != deleted_note.note.tags
        || restored.pinned != deleted_note.note.pinned
        || restored.created_at != deleted_note.note.created_at
        || restored.updated_at != deleted_note.note.updated_at
    {
        return Err("The restored note changed before it could be saved.".to_owned());
    }
    if restored.relative_path != pending.preferred_relative_path {
        return Err("The restored note path changed before it could be saved.".to_owned());
    }
    let restored_folder_path = match restored.folder_id.as_deref() {
        Some(folder_id) => folder_paths
            .get(folder_id)
            .ok_or_else(|| "The restored note folder no longer exists.".to_owned())?
            .as_str(),
        None => "",
    };
    let path_folder = Path::new(&restored.relative_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    if path_folder != restored_folder_path {
        return Err("The restored note path does not match its folder.".to_owned());
    }

    let old_ids = old_state.note_paths.keys().map(String::as_str).collect::<HashSet<_>>();
    if old_ids
        .iter()
        .any(|id| !vault.notes.iter().any(|note| note.id == **id))
    {
        return Err("Restore the note without removing another live note.".to_owned());
    }
    let new_notes = vault
        .notes
        .iter()
        .filter(|note| !old_ids.contains(note.id.as_str()))
        .collect::<Vec<_>>();
    if new_notes.len() != 1 || new_notes[0] != restored {
        return Err("Exactly one recovery snapshot must be restored at a time.".to_owned());
    }

    Ok(PreparedNoteRestore {
        restored_note: restored.clone(),
        editor_position: deleted_note.editor_position,
        recovery_id: pending.deleted_note_id,
        fingerprint: stored.fingerprint.clone(),
    })
}

fn build_note_write_plans(
    root: &Path,
    vault: &VaultData,
    old_state: &WorkspaceState,
    folder_paths: &BTreeMap<String, String>,
    preferred_new_paths: &BTreeMap<String, String>,
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
        let preferred_new_path = old_relative_path
            .is_none()
            .then(|| preferred_new_paths.get(&note.id))
            .flatten();
        let preserved_modified_at = preferred_new_path.map(|_| note.updated_at);
        let new_relative_path = if let Some(preferred_path) = preferred_new_path {
            validate_markdown_relative_path(preferred_path)?;
            let preferred = Path::new(preferred_path);
            let preferred_folder = preferred
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .and_then(path_to_slash_string)
                .unwrap_or_default();
            let preferred_title = preferred
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if preferred_folder != folder_path || preferred_title != note.title {
                return Err(
                    "The restored note path does not match its title and folder.".to_owned(),
                );
            }
            preferred_path.clone()
        } else if preserve_old_name {
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
            preserved_modified_at,
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

fn normalize_image_embed_settings(
    settings: &ImageEmbedSettings,
) -> Result<ImageEmbedSettings, String> {
    match settings.location {
        ImageEmbedLocation::VaultRoot => Ok(ImageEmbedSettings::default()),
        ImageEmbedLocation::NoteFolder => Ok(ImageEmbedSettings {
            location: ImageEmbedLocation::NoteFolder,
            folder_path: String::new(),
        }),
        ImageEmbedLocation::SpecifiedFolder => {
            let folder_path = settings.folder_path.trim().trim_matches('/').to_owned();
            if folder_path.is_empty() {
                return Err("Choose a vault-relative folder for embedded images.".to_owned());
            }
            validate_relative_path(&folder_path, false)?;
            Ok(ImageEmbedSettings {
                location: ImageEmbedLocation::SpecifiedFolder,
                folder_path,
            })
        }
        ImageEmbedLocation::SpecifiedFolderMirrored => {
            let folder_path = settings.folder_path.trim().trim_matches('/').to_owned();
            if folder_path.is_empty() {
                return Err("Choose a vault-relative folder for embedded images.".to_owned());
            }
            validate_relative_path(&folder_path, false)?;
            Ok(ImageEmbedSettings {
                location: ImageEmbedLocation::SpecifiedFolderMirrored,
                folder_path,
            })
        }
    }
}

fn migrate_legacy_image_embed_settings(
    settings: &ImageEmbedSettings,
) -> Result<ImageEmbedSettings, String> {
    let mut normalized = normalize_image_embed_settings(settings)?;
    if normalized.location == ImageEmbedLocation::SpecifiedFolderMirrored {
        normalized.location = ImageEmbedLocation::SpecifiedFolder;
    }
    Ok(normalized)
}

fn image_destination_folder(
    note_relative_path: &str,
    settings: &ImageEmbedSettings,
) -> Result<String, String> {
    validate_markdown_relative_path(note_relative_path)?;
    let settings = normalize_image_embed_settings(settings)?;
    match settings.location {
        ImageEmbedLocation::VaultRoot => Ok(String::new()),
        ImageEmbedLocation::NoteFolder => Ok(Path::new(note_relative_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .and_then(path_to_slash_string)
            .unwrap_or_default()),
        ImageEmbedLocation::SpecifiedFolder => Ok(settings.folder_path),
        ImageEmbedLocation::SpecifiedFolderMirrored => {
            let note_folder = Path::new(note_relative_path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .and_then(path_to_slash_string)
                .unwrap_or_default();
            Ok(if note_folder.is_empty() {
                settings.folder_path
            } else {
                format!("{}/{note_folder}", settings.folder_path)
            })
        }
    }
}

fn normalize_attachment_embed_settings(
    settings: &AttachmentEmbedSettings,
) -> Result<AttachmentEmbedSettings, String> {
    match settings.location {
        ImageEmbedLocation::VaultRoot => Ok(AttachmentEmbedSettings::default()),
        ImageEmbedLocation::NoteFolder => Ok(AttachmentEmbedSettings {
            location: ImageEmbedLocation::NoteFolder,
            folder_path: String::new(),
        }),
        ImageEmbedLocation::SpecifiedFolder => {
            let folder_path = settings.folder_path.trim().trim_matches('/').to_owned();
            if folder_path.is_empty() {
                return Err("Choose a vault-relative folder for embedded files.".to_owned());
            }
            validate_relative_path(&folder_path, false)?;
            Ok(AttachmentEmbedSettings {
                location: ImageEmbedLocation::SpecifiedFolder,
                folder_path,
            })
        }
        ImageEmbedLocation::SpecifiedFolderMirrored => {
            let folder_path = settings.folder_path.trim().trim_matches('/').to_owned();
            if folder_path.is_empty() {
                return Err("Choose a vault-relative folder for embedded files.".to_owned());
            }
            validate_relative_path(&folder_path, false)?;
            Ok(AttachmentEmbedSettings {
                location: ImageEmbedLocation::SpecifiedFolderMirrored,
                folder_path,
            })
        }
    }
}

fn migrate_legacy_attachment_embed_settings(
    settings: &AttachmentEmbedSettings,
) -> Result<AttachmentEmbedSettings, String> {
    let mut normalized = normalize_attachment_embed_settings(settings)?;
    if normalized.location == ImageEmbedLocation::SpecifiedFolderMirrored {
        normalized.location = ImageEmbedLocation::SpecifiedFolder;
    }
    Ok(normalized)
}

fn attachment_destination_folder(
    note_relative_path: &str,
    settings: &AttachmentEmbedSettings,
) -> Result<String, String> {
    validate_markdown_relative_path(note_relative_path)?;
    let settings = normalize_attachment_embed_settings(settings)?;
    match settings.location {
        ImageEmbedLocation::VaultRoot => Ok(String::new()),
        ImageEmbedLocation::NoteFolder => Ok(Path::new(note_relative_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .and_then(path_to_slash_string)
            .unwrap_or_default()),
        ImageEmbedLocation::SpecifiedFolder => Ok(settings.folder_path),
        ImageEmbedLocation::SpecifiedFolderMirrored => {
            let note_folder = Path::new(note_relative_path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .and_then(path_to_slash_string)
                .unwrap_or_default();
            Ok(if note_folder.is_empty() {
                settings.folder_path
            } else {
                format!("{}/{note_folder}", settings.folder_path)
            })
        }
    }
}

fn embed_workspace_image(
    root: &Path,
    note_relative_path: &str,
    settings: ImageEmbedSettings,
    file_name: &str,
    bytes: &[u8],
    existing_relative_path: Option<&str>,
    expected_revision: u64,
) -> Result<WorkspaceEmbedImageResult, String> {
    let settings = normalize_image_embed_settings(&settings)?;
    let destination_folder = image_destination_folder(note_relative_path, &settings)?;
    let (media_type, extension) = validate_image_bytes(bytes, Some(file_name))?;
    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err(
            "Embedded images cannot be changed while workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    recover_workspace_transactions(root, stored_state.as_ref(), &mut warnings)?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before embedding the image."
                .to_owned(),
        );
    }

    let mut state = stored_state.unwrap_or_default();
    let existing_relative_path = existing_relative_path
        .map(str::to_owned)
        .filter(|path| validate_image_relative_path(path).is_ok());
    let fingerprint = fingerprint_bytes(bytes);

    if let Some(relative_path) = existing_relative_path.as_deref() {
        if let Some((id, stored)) = state.assets.iter_mut().find(|(_, stored)| {
            stored.kind == VaultAssetKind::Image
                && portable_path_key(&stored.relative_path) == portable_path_key(relative_path)
        }) {
            if stored.relative_path != relative_path {
                let old_path = resolve_workspace_image_file(root, &stored.relative_path, true)?;
                if old_path.exists() {
                    return Err(format!(
                        "The vault contains image paths that differ only by letter case near {relative_path}."
                    ));
                }
                stored.relative_path = relative_path.to_owned();
            }
            stored.media_type = media_type.to_owned();
            stored.fingerprint = fingerprint;
            stored.modified_nanos = image_modified_nanos_for_path(root, relative_path)?;
            let id = id.clone();
            state.version = STATE_VERSION;
            state.image_embed_settings = settings;
            write_workspace_state(root, &state)?;

            return Ok(WorkspaceEmbedImageResult {
                image: EmbeddedImage {
                    id,
                    relative_path: relative_path.to_owned(),
                    media_type: media_type.to_owned(),
                },
                revision: revision_for_root(root)?,
                saved_at: now_millis(),
                warnings: warnings.finish(),
            });
        }
    }

    if state.assets.len() >= MAX_VAULT_ASSETS {
        return Err(format!(
            "This vault already tracks the maximum of {MAX_VAULT_ASSETS} embedded images."
        ));
    }

    let mut wrote_image = false;
    let relative_path = if let Some(relative_path) = existing_relative_path {
        relative_path
    } else {
        if !destination_folder.is_empty() {
            ensure_directory_path(root, &destination_folder)?;
        }
        let safe_name = safe_image_file_name(file_name, extension);
        let relative_path = unique_image_relative_path(root, &destination_folder, &safe_name)?;
        let target = resolve_workspace_image_file(root, &relative_path, true)?;
        atomic_write(&target, bytes)
            .map_err(|error| format!("Could not save the embedded image: {error}"))?;
        wrote_image = true;
        relative_path
    };

    let mut used_ids = state.assets.keys().cloned().collect::<HashSet<_>>();
    let id_seed = format!(
        "{relative_path}:{}:{}:{}",
        fingerprint.length,
        fingerprint.hash,
        now_millis(),
    );
    let id = fresh_id("image", &id_seed, &mut used_ids);
    let modified_nanos = match image_modified_nanos_for_path(root, &relative_path) {
        Ok(modified_nanos) => modified_nanos,
        Err(error) => {
            if wrote_image {
                if let Ok(target) = resolve_workspace_image_file(root, &relative_path, false) {
                    let _ = remove_file_durable(&target);
                }
            }
            return Err(error);
        }
    };
    state.version = STATE_VERSION;
    state.image_embed_settings = settings;
    state.assets.insert(
        id.clone(),
        StoredVaultAsset {
            kind: VaultAssetKind::Image,
            relative_path: relative_path.clone(),
            media_type: media_type.to_owned(),
            fingerprint,
            modified_nanos,
        },
    );

    if let Err(error) = write_workspace_state(root, &state) {
        if wrote_image {
            if let Ok(target) = resolve_workspace_image_file(root, &relative_path, false) {
                let _ = remove_file_durable(&target);
            }
        }
        return Err(error);
    }

    Ok(WorkspaceEmbedImageResult {
        image: EmbeddedImage {
            id,
            relative_path,
            media_type: media_type.to_owned(),
        },
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

fn embed_workspace_attachment(
    root: &Path,
    note_relative_path: &str,
    settings: AttachmentEmbedSettings,
    source: &Path,
    existing_relative_path: Option<&str>,
    expected_revision: u64,
) -> Result<WorkspaceEmbedAttachmentResult, String> {
    let settings = normalize_attachment_embed_settings(&settings)?;
    let destination_folder = attachment_destination_folder(note_relative_path, &settings)?;
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "The selected attachment name is not valid Unicode.".to_owned())?;
    let safe_name = safe_attachment_file_name(file_name)?;
    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err(
            "Embedded files cannot be changed while workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    recover_workspace_transactions(root, stored_state.as_ref(), &mut warnings)?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before embedding the file."
                .to_owned(),
        );
    }

    let mut state = stored_state.unwrap_or_default();
    let existing_relative_path = existing_relative_path
        .map(str::to_owned)
        .filter(|path| validate_attachment_relative_path(path).is_ok());
    if let Some(relative_path) = existing_relative_path.as_deref() {
        if let Some((id, stored)) = state.assets.iter_mut().find(|(_, stored)| {
            stored.kind == VaultAssetKind::Attachment
                && portable_path_key(&stored.relative_path) == portable_path_key(relative_path)
        }) {
            if stored.relative_path != relative_path {
                let old_path = resolve_workspace_asset_file(root, &stored.relative_path, true)?;
                if old_path.exists() {
                    return Err(format!(
                        "The vault contains attachment paths that differ only by letter case near {relative_path}."
                    ));
                }
                stored.relative_path = relative_path.to_owned();
            }
            let fingerprint = fingerprint_attachment_file(source)?;
            stored.media_type = attachment_media_type_for_path(Path::new(relative_path)).to_owned();
            stored.fingerprint = fingerprint.clone();
            stored.modified_nanos = file_modified_nanos_for_path(source)?;
            let attachment = EmbeddedAttachment {
                id: id.clone(),
                relative_path: relative_path.to_owned(),
                media_type: stored.media_type.clone(),
                byte_length: fingerprint.length,
                opening_disabled: attachment_opening_is_disabled(source)?,
            };
            state.version = STATE_VERSION;
            state.attachment_embed_settings = settings;
            write_workspace_state(root, &state)?;

            return Ok(WorkspaceEmbedAttachmentResult {
                attachment,
                revision: revision_for_root(root)?,
                saved_at: now_millis(),
                warnings: warnings.finish(),
            });
        }
    }

    if state.assets.len() >= MAX_VAULT_ASSETS {
        return Err(format!(
            "This vault already tracks the maximum of {MAX_VAULT_ASSETS} embedded files."
        ));
    }

    let mut copied_attachment = false;
    let (relative_path, fingerprint) = if let Some(relative_path) = existing_relative_path {
        (relative_path, fingerprint_attachment_file(source)?)
    } else {
        if !destination_folder.is_empty() {
            ensure_directory_path(root, &destination_folder)?;
        }
        let relative_path = unique_attachment_relative_path(
            root,
            &destination_folder,
            &safe_name,
        )?;
        let target = resolve_workspace_asset_file(root, &relative_path, true)?;
        let fingerprint = copy_attachment_file_durable(source, &target)?;
        copied_attachment = true;
        (relative_path, fingerprint)
    };
    let stored_path = resolve_workspace_asset_file(root, &relative_path, false)?;
    let modified_nanos = file_modified_nanos_for_path(&stored_path)?;
    let opening_disabled = attachment_opening_is_disabled(&stored_path)?;
    let media_type = attachment_media_type_for_path(Path::new(&relative_path)).to_owned();
    let mut used_ids = state.assets.keys().cloned().collect::<HashSet<_>>();
    let id_seed = format!(
        "{relative_path}:{}:{}:{}",
        fingerprint.length,
        fingerprint.hash,
        now_millis(),
    );
    let id = fresh_id("asset", &id_seed, &mut used_ids);
    state.version = STATE_VERSION;
    state.attachment_embed_settings = settings;
    state.assets.insert(
        id.clone(),
        StoredVaultAsset {
            kind: VaultAssetKind::Attachment,
            relative_path: relative_path.clone(),
            media_type: media_type.clone(),
            fingerprint: fingerprint.clone(),
            modified_nanos,
        },
    );
    if let Err(error) = write_workspace_state(root, &state) {
        if copied_attachment {
            if let Ok(target) = resolve_workspace_asset_file(root, &relative_path, false) {
                let _ = remove_file_durable(&target);
            }
        }
        return Err(error);
    }

    Ok(WorkspaceEmbedAttachmentResult {
        attachment: EmbeddedAttachment {
            id,
            relative_path,
            media_type,
            byte_length: fingerprint.length,
            opening_disabled,
        },
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

fn discard_workspace_external_asset(
    root: &Path,
    asset_id: &str,
    relative_path: &str,
    expected_revision: u64,
) -> Result<WorkspaceExternalAssetDiscardResult, String> {
    if !is_valid_asset_id(asset_id) {
        return Err("The dropped file has an invalid stable ID.".to_owned());
    }

    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err(
            "The dropped file cannot be cleaned up while workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    let old_state = stored_state.unwrap_or_default();
    recover_workspace_transactions(root, Some(&old_state), &mut warnings)?;
    if revision_for_root(root)? != expected_revision {
        warnings.push(
            "The vault changed before the unused dropped file could be removed; the file was retained."
                .to_owned(),
        );

        return retained_external_asset_result(root, &old_state, warnings);
    }

    let Some(stored) = old_state.assets.get(asset_id) else {
        return Err("The dropped file's stable record is no longer available.".to_owned());
    };
    if stored.relative_path != relative_path {
        warnings.push(
            "The dropped file moved before cleanup, so it was retained at its current location."
                .to_owned(),
        );

        return retained_external_asset_result(root, &old_state, warnings);
    }
    let source = match stored.kind {
        VaultAssetKind::Image => {
            validate_image_relative_path(relative_path)?;
            resolve_workspace_image_file(root, relative_path, false)?
        }
        VaultAssetKind::Attachment => {
            validate_attachment_relative_path(relative_path)?;
            resolve_workspace_asset_file(root, relative_path, false)?
        }
    };
    if workspace_asset_is_referenced(root, &old_state, stored.kind, asset_id)? {
        warnings.push(
            "The dropped file is already referenced by a saved note, so it was retained."
                .to_owned(),
        );

        return retained_external_asset_result(root, &old_state, warnings);
    }
    if !workspace_asset_matches_stored(&source, stored)? {
        warnings.push(
            "The dropped file changed before cleanup, so the modified file was retained."
                .to_owned(),
        );

        return retained_external_asset_result(root, &old_state, warnings);
    }

    let mut next_state = old_state.clone();
    next_state.version = STATE_VERSION;
    next_state.assets.remove(asset_id);
    write_workspace_state(root, &next_state)?;

    let cleanup_result = (|| {
        if !workspace_asset_matches_stored(&source, stored)? {
            return Err("The dropped file changed while cleanup was being committed.".to_owned());
        }
        remove_file_durable(&source)
            .map_err(|error| format!("Could not remove the unused dropped file: {error}"))
    })();
    if let Err(error) = cleanup_result {
        write_workspace_state(root, &old_state).map_err(|rollback_error| {
            format!(
                "{error} Its stable record could not be restored: {rollback_error}. Reopen the vault before editing again."
            )
        })?;
        warnings.push(format!("{error} The file was retained."));

        return retained_external_asset_result(root, &old_state, warnings);
    }

    Ok(WorkspaceExternalAssetDiscardResult {
        discarded: true,
        note_paths: next_state.note_paths,
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

fn retained_external_asset_result(
    root: &Path,
    state: &WorkspaceState,
    warnings: WarningCollector,
) -> Result<WorkspaceExternalAssetDiscardResult, String> {
    Ok(WorkspaceExternalAssetDiscardResult {
        discarded: false,
        note_paths: state.note_paths.clone(),
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

fn workspace_asset_matches_stored(
    path: &Path,
    stored: &StoredVaultAsset,
) -> Result<bool, String> {
    let fingerprint = match stored.kind {
        VaultAssetKind::Image => fingerprint_bytes(&read_image_file(path)?),
        VaultAssetKind::Attachment => fingerprint_attachment_file(path)?,
    };
    Ok(
        fingerprint == stored.fingerprint
            && file_modified_nanos_for_path(path)? == stored.modified_nanos,
    )
}

fn workspace_asset_is_referenced(
    root: &Path,
    state: &WorkspaceState,
    kind: VaultAssetKind,
    asset_id: &str,
) -> Result<bool, String> {
    let fragment = match kind {
        VaultAssetKind::Image => format!("#oah-image={asset_id}"),
        VaultAssetKind::Attachment => format!("#oah-asset={asset_id}"),
    };
    for relative_path in state.note_paths.values() {
        validate_markdown_relative_path(relative_path)?;
        let path = resolve_workspace_file(root, relative_path, false)?;
        let content = fs::read_to_string(&path).map_err(|error| {
            format!(
                "Could not check {} for dropped-file references: {error}",
                path.display(),
            )
        })?;
        if content.contains(&fragment) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Debug)]
struct PreparedAssetNoteUpdate {
    path: PathBuf,
    expected_content: Vec<u8>,
    content: Vec<u8>,
}

fn relocate_workspace_image(
    root: &Path,
    image_relative_path: &str,
    target_relative_path: &str,
    asset_id: &str,
    note_updates: &[WorkspaceImageNoteUpdate],
    expected_revision: u64,
    managed_by_note_move: bool,
) -> Result<WorkspaceRelocateImageResult, String> {
    validate_image_relative_path(image_relative_path)?;
    validate_image_relative_path(target_relative_path)?;
    if image_relative_path == target_relative_path {
        return Err("The image is already at that path.".to_owned());
    }
    if !is_valid_asset_id(asset_id) {
        return Err("The image has an invalid stable ID.".to_owned());
    }

    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err(
            "Images cannot be reorganized while workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    let old_state = stored_state.unwrap_or_default();
    recover_workspace_transactions(root, Some(&old_state), &mut warnings)?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before moving the image."
                .to_owned(),
        );
    }
    let source_is_mirror_managed =
        image_path_is_mirror_managed(image_relative_path, &old_state.image_embed_settings)?;
    if source_is_mirror_managed && !managed_by_note_move {
        return Err(
            "Images in the mirrored image folder are managed by note location and cannot be renamed or moved."
                .to_owned(),
        );
    }
    if managed_by_note_move {
        let target_is_mirror_managed =
            image_path_is_mirror_managed(target_relative_path, &old_state.image_embed_settings)?;
        let source_name = Path::new(image_relative_path).file_name();
        let target_name = Path::new(target_relative_path).file_name();
        if !source_is_mirror_managed
            || !target_is_mirror_managed
            || source_name.is_none()
            || source_name != target_name
        {
            return Err(
                "A note move can only carry a mirrored image, without renaming it."
                    .to_owned(),
            );
        }
    }

    let source = resolve_workspace_image_file(root, image_relative_path, false)?;
    let bytes = read_image_file(&source)?;
    let (media_type, _) = validate_image_bytes(&bytes, Some(target_relative_path))?;
    let target = resolve_workspace_image_file(root, target_relative_path, true)?;
    let target_parent = target
        .parent()
        .ok_or_else(|| "The image destination has no parent folder.".to_owned())?;
    match fs::symlink_metadata(target_parent) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("The image destination is not a regular vault folder.".to_owned());
        }
        Ok(_) => {}
        Err(error)
            if error.kind() == io::ErrorKind::NotFound && managed_by_note_move => {}
        Err(error) => {
            return Err(format!(
                "Could not inspect the image destination folder: {error}"
            ));
        }
    }
    let case_only_rename = portable_path_key(image_relative_path)
        == portable_path_key(target_relative_path);
    if !case_only_rename && image_path_exists_portably(root, target_relative_path)? {
        return Err(format!("A file named {target_relative_path} already exists."));
    }

    if let Some(stored) = old_state.assets.get(asset_id) {
        if stored.kind != VaultAssetKind::Image {
            return Err("The stable image record refers to a different file type.".to_owned());
        }
        if portable_path_key(&stored.relative_path) != portable_path_key(image_relative_path) {
            return Err("The stable image record no longer points to that file. Reload the vault.".to_owned());
        }
    } else {
        if old_state.assets.len() >= MAX_VAULT_ASSETS {
            return Err(format!(
                "This vault already tracks the maximum of {MAX_VAULT_ASSETS} embedded images."
            ));
        }
        if old_state.assets.values().any(|stored| {
            stored.kind == VaultAssetKind::Image
                && portable_path_key(&stored.relative_path)
                    == portable_path_key(image_relative_path)
        }) {
            return Err("The image's stable record changed. Reload the vault.".to_owned());
        }
    }

    let prepared_updates = prepare_asset_note_updates(root, &old_state, note_updates, "image")?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed while the image move was being prepared. Reload it and try again."
                .to_owned(),
        );
    }
    for update in &prepared_updates {
        if fs::read(&update.path).map_err(|error| {
            format!("Could not recheck a note before moving the image: {error}")
        })? != update.expected_content
        {
            return Err(
                "A note changed before the image could be moved. Reload the vault and try again."
                    .to_owned(),
            );
        }
    }

    if managed_by_note_move {
        ensure_asset_parent(root, target_relative_path, "image")?;
    }

    relocate_asset_file_durable(&source, &target).map_err(|error| {
        format!("Could not move the image to {target_relative_path}: {error}")
    })?;

    let mut applied_note_count = 0_usize;
    for update in &prepared_updates {
        if let Err(error) = atomic_write(&update.path, &update.content) {
            let rollback_error = rollback_asset_relocation(
                &source,
                &target,
                &prepared_updates[..=applied_note_count],
                None,
                root,
            );
            return Err(format!(
                "Could not update image references: {error}{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
        applied_note_count += 1;
    }

    let modified_nanos = match image_modified_nanos_for_path(root, target_relative_path) {
        Ok(value) => value,
        Err(error) => {
            let rollback_error = rollback_asset_relocation(
                &source,
                &target,
                &prepared_updates,
                None,
                root,
            );
            return Err(format!(
                "Could not verify the moved image: {error}{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
    };
    for update in &prepared_updates {
        let verification_error = match fs::read(&update.path) {
            Ok(content) if content == update.content => None,
            Ok(_) => Some("an image reference did not match the requested content".to_owned()),
            Err(error) => Some(format!("an image reference could not be read: {error}")),
        };
        if let Some(verification_error) = verification_error {
            let rollback_error = rollback_asset_relocation(
                &source,
                &target,
                &prepared_updates,
                None,
                root,
            );
            return Err(format!(
                "The move could not be verified because {verification_error}.{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
    }

    let mut state = old_state.clone();
    state.version = STATE_VERSION;
    state.assets.insert(
        asset_id.to_owned(),
        StoredVaultAsset {
            kind: VaultAssetKind::Image,
            relative_path: target_relative_path.to_owned(),
            media_type: media_type.to_owned(),
            fingerprint: fingerprint_bytes(&bytes),
            modified_nanos,
        },
    );
    if let Err(error) = write_workspace_state(root, &state) {
        let rollback_error = rollback_asset_relocation(
            &source,
            &target,
            &prepared_updates,
            Some(&old_state),
            root,
        );
        return Err(format!(
            "Could not update the stable image record: {error}{}",
            rollback_error
                .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                .unwrap_or_default(),
        ));
    }

    Ok(WorkspaceRelocateImageResult {
        image: EmbeddedImage {
            id: asset_id.to_owned(),
            relative_path: target_relative_path.to_owned(),
            media_type: media_type.to_owned(),
        },
        previous_relative_path: image_relative_path.to_owned(),
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

fn relocate_workspace_attachment(
    root: &Path,
    attachment_relative_path: &str,
    target_relative_path: &str,
    asset_id: &str,
    note_updates: &[WorkspaceImageNoteUpdate],
    expected_revision: u64,
    managed_by_note_move: bool,
) -> Result<WorkspaceRelocateAttachmentResult, String> {
    validate_attachment_relative_path(attachment_relative_path)?;
    validate_attachment_relative_path(target_relative_path)?;
    if attachment_relative_path == target_relative_path {
        return Err("The attachment is already at that path.".to_owned());
    }
    if !is_valid_asset_id(asset_id) {
        return Err("The attachment has an invalid stable ID.".to_owned());
    }

    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err(
            "Attachments cannot be reorganized while workspace metadata is unreadable or newer than this app."
                .to_owned(),
        );
    }
    let old_state = stored_state.unwrap_or_default();
    recover_workspace_transactions(root, Some(&old_state), &mut warnings)?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before moving the attachment."
                .to_owned(),
        );
    }
    let source_is_mirror_managed = attachment_path_is_mirror_managed(
        attachment_relative_path,
        &old_state.attachment_embed_settings,
    )?;
    if source_is_mirror_managed && !managed_by_note_move {
        return Err(
            "Attachments in the mirrored attachment folder are managed by note location and cannot be renamed or moved."
                .to_owned(),
        );
    }
    if managed_by_note_move {
        let target_is_mirror_managed = attachment_path_is_mirror_managed(
            target_relative_path,
            &old_state.attachment_embed_settings,
        )?;
        let source_name = Path::new(attachment_relative_path).file_name();
        let target_name = Path::new(target_relative_path).file_name();
        if !source_is_mirror_managed
            || !target_is_mirror_managed
            || source_name.is_none()
            || source_name != target_name
        {
            return Err(
                "A note move can only carry a mirrored attachment, without renaming it."
                    .to_owned(),
            );
        }
    }

    let source = resolve_workspace_asset_file(root, attachment_relative_path, false)?;
    let fingerprint = fingerprint_attachment_file(&source)?;
    let media_type = attachment_media_type_for_path(Path::new(target_relative_path));
    let target = resolve_workspace_asset_file(root, target_relative_path, true)?;
    let target_parent = target
        .parent()
        .ok_or_else(|| "The attachment destination has no parent folder.".to_owned())?;
    match fs::symlink_metadata(target_parent) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("The attachment destination is not a regular vault folder.".to_owned());
        }
        Ok(_) => {}
        Err(error)
            if error.kind() == io::ErrorKind::NotFound && managed_by_note_move => {}
        Err(error) => {
            return Err(format!(
                "Could not inspect the attachment destination folder: {error}"
            ));
        }
    }
    let case_only_rename = portable_path_key(attachment_relative_path)
        == portable_path_key(target_relative_path);
    if !case_only_rename && asset_path_exists_portably(root, target_relative_path)? {
        return Err(format!("A file named {target_relative_path} already exists."));
    }

    if let Some(stored) = old_state.assets.get(asset_id) {
        if stored.kind != VaultAssetKind::Attachment {
            return Err("The stable attachment record refers to a different file type.".to_owned());
        }
        if portable_path_key(&stored.relative_path)
            != portable_path_key(attachment_relative_path)
        {
            return Err(
                "The stable attachment record no longer points to that file. Reload the vault."
                    .to_owned(),
            );
        }
    } else {
        if old_state.assets.len() >= MAX_VAULT_ASSETS {
            return Err(format!(
                "This vault already tracks the maximum of {MAX_VAULT_ASSETS} embedded assets."
            ));
        }
        if old_state.assets.values().any(|stored| {
            stored.kind == VaultAssetKind::Attachment
                && portable_path_key(&stored.relative_path)
                    == portable_path_key(attachment_relative_path)
        }) {
            return Err("The attachment's stable record changed. Reload the vault.".to_owned());
        }
    }

    let prepared_updates =
        prepare_asset_note_updates(root, &old_state, note_updates, "attachment")?;
    if revision_for_root(root)? != expected_revision {
        return Err(
            "The vault changed while the attachment move was being prepared. Reload it and try again."
                .to_owned(),
        );
    }
    for update in &prepared_updates {
        if fs::read(&update.path)
            .map_err(|error| format!("Could not recheck a note before moving the attachment: {error}"))?
            != update.expected_content
        {
            return Err(
                "A note changed before the attachment could be moved. Reload the vault and try again."
                    .to_owned(),
            );
        }
    }

    if managed_by_note_move {
        ensure_asset_parent(root, target_relative_path, "attachment")?;
    }
    relocate_asset_file_durable(&source, &target).map_err(|error| {
        format!("Could not move the attachment to {target_relative_path}: {error}")
    })?;

    let mut applied_note_count = 0_usize;
    for update in &prepared_updates {
        if let Err(error) = atomic_write(&update.path, &update.content) {
            let rollback_error = rollback_asset_relocation(
                &source,
                &target,
                &prepared_updates[..=applied_note_count],
                None,
                root,
            );
            return Err(format!(
                "Could not update attachment references: {error}{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
        applied_note_count += 1;
    }

    let target_fingerprint = match fingerprint_attachment_file(&target) {
        Ok(value) if value == fingerprint => value,
        Ok(_) => {
            let rollback_error = rollback_asset_relocation(
                &source,
                &target,
                &prepared_updates,
                None,
                root,
            );
            return Err(format!(
                "The moved attachment failed its integrity check.{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
        Err(error) => {
            let rollback_error = rollback_asset_relocation(
                &source,
                &target,
                &prepared_updates,
                None,
                root,
            );
            return Err(format!(
                "Could not verify the moved attachment: {error}{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
    };
    let modified_nanos = match file_modified_nanos_for_path(&target) {
        Ok(value) => value,
        Err(error) => {
            let rollback_error = rollback_asset_relocation(
                &source,
                &target,
                &prepared_updates,
                None,
                root,
            );
            return Err(format!(
                "Could not inspect the moved attachment: {error}{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
    };
    for update in &prepared_updates {
        let verification_error = match fs::read(&update.path) {
            Ok(content) if content == update.content => None,
            Ok(_) => Some("an attachment reference did not match the requested content".to_owned()),
            Err(error) => Some(format!("an attachment reference could not be read: {error}")),
        };
        if let Some(verification_error) = verification_error {
            let rollback_error = rollback_asset_relocation(
                &source,
                &target,
                &prepared_updates,
                None,
                root,
            );
            return Err(format!(
                "The move could not be verified because {verification_error}.{}",
                rollback_error
                    .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                    .unwrap_or_default(),
            ));
        }
    }

    let mut state = old_state.clone();
    state.version = STATE_VERSION;
    state.assets.insert(
        asset_id.to_owned(),
        StoredVaultAsset {
            kind: VaultAssetKind::Attachment,
            relative_path: target_relative_path.to_owned(),
            media_type: media_type.to_owned(),
            fingerprint: target_fingerprint.clone(),
            modified_nanos,
        },
    );
    if let Err(error) = write_workspace_state(root, &state) {
        let rollback_error = rollback_asset_relocation(
            &source,
            &target,
            &prepared_updates,
            Some(&old_state),
            root,
        );
        return Err(format!(
            "Could not update the stable attachment record: {error}{}",
            rollback_error
                .map(|detail| format!(" The move could not be fully rolled back: {detail}"))
                .unwrap_or_default(),
        ));
    }

    Ok(WorkspaceRelocateAttachmentResult {
        attachment: EmbeddedAttachment {
            id: asset_id.to_owned(),
            relative_path: target_relative_path.to_owned(),
            media_type: media_type.to_owned(),
            byte_length: target_fingerprint.length,
            opening_disabled: attachment_opening_is_disabled(&target)?,
        },
        previous_relative_path: attachment_relative_path.to_owned(),
        revision: revision_for_root(root)?,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

fn prepare_asset_note_updates(
    root: &Path,
    state: &WorkspaceState,
    note_updates: &[WorkspaceImageNoteUpdate],
    asset_label: &str,
) -> Result<Vec<PreparedAssetNoteUpdate>, String> {
    if note_updates.len() > MAX_NOTES {
        return Err(format!("Only {MAX_NOTES} notes can be updated at once."));
    }
    let mut seen_note_ids = HashSet::new();
    let mut seen_paths = HashSet::new();
    let mut total_bytes = 0_u64;
    let mut prepared = Vec::with_capacity(note_updates.len());
    for update in note_updates {
        validate_markdown_relative_path(&update.relative_path)?;
        if state.note_paths.get(&update.note_id).map(String::as_str)
            != Some(update.relative_path.as_str())
        {
            return Err(format!(
                "A note path changed before its {asset_label} reference could be updated."
            ));
        }
        if !seen_note_ids.insert(update.note_id.as_str())
            || !seen_paths.insert(portable_path_key(&update.relative_path))
        {
            return Err(format!(
                "The {asset_label} move contains a duplicate note update."
            ));
        }
        if update.content.len() as u64 > MAX_NOTE_BYTES
            || update.expected_content.len() as u64 > MAX_NOTE_BYTES
        {
            return Err(format!(
                "{} is larger than {} MiB and cannot be updated.",
                update.relative_path,
                MAX_NOTE_BYTES / 1024 / 1024,
            ));
        }
        total_bytes = total_bytes.saturating_add(update.content.len() as u64);
        if total_bytes > MAX_TOTAL_NOTE_BYTES {
            return Err(format!(
                "The {asset_label} move would update too much note content at once."
            ));
        }
        let path = resolve_workspace_file(root, &update.relative_path, false)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Could not inspect {}: {error}", update.relative_path))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!("{} is not a regular Markdown file.", update.relative_path));
        }
        let current = fs::read(&path)
            .map_err(|error| format!("Could not read {}: {error}", update.relative_path))?;
        if current != update.expected_content.as_bytes() {
            return Err(format!(
                "{} changed before its {asset_label} reference could be updated.",
                update.relative_path,
            ));
        }
        prepared.push(PreparedAssetNoteUpdate {
            path,
            expected_content: current,
            content: update.content.as_bytes().to_vec(),
        });
    }
    Ok(prepared)
}

fn image_path_is_mirror_managed(
    image_relative_path: &str,
    settings: &ImageEmbedSettings,
) -> Result<bool, String> {
    let normalized = normalize_image_embed_settings(settings)?;
    if normalized.location != ImageEmbedLocation::SpecifiedFolderMirrored {
        return Ok(false);
    }
    let image_key = portable_path_key(image_relative_path);
    let folder_key = portable_path_key(&normalized.folder_path);
    Ok(image_key.starts_with(&format!("{folder_key}/")))
}

fn attachment_path_is_mirror_managed(
    attachment_relative_path: &str,
    settings: &AttachmentEmbedSettings,
) -> Result<bool, String> {
    let normalized = normalize_attachment_embed_settings(settings)?;
    if normalized.location != ImageEmbedLocation::SpecifiedFolderMirrored {
        return Ok(false);
    }
    let attachment_key = portable_path_key(attachment_relative_path);
    let folder_key = portable_path_key(&normalized.folder_path);
    Ok(attachment_key.starts_with(&format!("{folder_key}/")))
}

fn relocate_asset_file_durable(source: &Path, target: &Path) -> io::Result<()> {
    if source == target {
        return Ok(());
    }
    if source.to_string_lossy().eq_ignore_ascii_case(&target.to_string_lossy()) {
        let parent = source
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "asset has no parent"))?;
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".oah-asset-rename-{}-{counter}.tmp",
            std::process::id(),
        ));
        rename_durable(source, &temporary)?;
        if let Err(error) = rename_durable(&temporary, target) {
            let _ = rename_durable(&temporary, source);
            return Err(error);
        }
        return Ok(());
    }
    rename_durable(source, target)
}

fn rollback_asset_relocation(
    source: &Path,
    target: &Path,
    applied_updates: &[PreparedAssetNoteUpdate],
    old_state: Option<&WorkspaceState>,
    root: &Path,
) -> Option<String> {
    let mut errors = Vec::new();
    for update in applied_updates.iter().rev() {
        if let Err(error) = atomic_write(&update.path, &update.expected_content) {
            errors.push(format!("could not restore a note: {error}"));
        }
    }
    if let Err(error) = relocate_asset_file_durable(target, source) {
        errors.push(format!("could not restore the asset: {error}"));
    }
    if let Some(state) = old_state {
        if let Err(error) = write_workspace_state(root, state) {
            errors.push(format!("could not restore asset metadata: {error}"));
        }
    }
    (!errors.is_empty()).then(|| errors.join("; "))
}

fn read_workspace_image(
    root: &Path,
    asset_id: Option<&str>,
    note_relative_path: &str,
    destination: &str,
) -> Result<Vec<u8>, String> {
    let mut warnings = WarningCollector::default();
    let (stored_state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    if stored_state.is_none() && state_file_was_present {
        return Err("Workspace metadata is unreadable or newer than this app.".to_owned());
    }
    let mut state = stored_state.unwrap_or_default();
    let valid_asset_id = asset_id.filter(|id| is_valid_asset_id(id));
    let tracked_asset_id = valid_asset_id.filter(|id| {
        state
            .assets
            .get(*id)
            .is_some_and(|asset| asset.kind == VaultAssetKind::Image)
    });
    if let Some(relative_path) = tracked_asset_id
        .and_then(|id| state.assets.get(id))
        .map(|asset| asset.relative_path.as_str())
    {
        if let Ok(bytes) = read_relative_workspace_image(root, relative_path) {
            return Ok(bytes);
        }
    }
    if tracked_asset_id.is_some() {
        let _ = reconcile_image_assets(root, &mut state.assets, &mut warnings);
        if let Some(relative_path) = tracked_asset_id
            .and_then(|id| state.assets.get(id))
            .map(|asset| asset.relative_path.as_str())
        {
            if let Ok(bytes) = read_relative_workspace_image(root, relative_path) {
                return Ok(bytes);
            }
        }
    }

    let relative_path = resolve_markdown_image_path(note_relative_path, destination)?;
    read_relative_workspace_image(root, &relative_path)
}

fn read_relative_workspace_image(root: &Path, relative_path: &str) -> Result<Vec<u8>, String> {
    let path = resolve_workspace_image_file(root, relative_path, false)?;
    let bytes = read_image_file(&path)?;
    validate_image_bytes(&bytes, Some(relative_path))?;
    Ok(bytes)
}

fn workspace_image_import_path(
    root: &Path,
    source_relative_path: &str,
    bytes: Option<&[u8]>,
    reserved_paths: &HashSet<String>,
) -> Result<(String, bool), String> {
    let source_key = portable_path_key(source_relative_path);
    if !reserved_paths.contains(&source_key) {
        let target = resolve_workspace_image_file(root, source_relative_path, true)?;
        match fs::symlink_metadata(&target) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => {
                if bytes.is_some_and(|bytes| {
                    fs::read(&target)
                        .is_ok_and(|existing| existing.as_slice() == bytes)
                }) {
                    return Ok((source_relative_path.to_owned(), true));
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if !image_path_exists_portably(root, source_relative_path)? {
                    return Ok((source_relative_path.to_owned(), false));
                }
            }
            Err(_) => {}
        }
    }

    let path = Path::new(source_relative_path);
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Image");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png");
    for index in 2..=10_001_u32 {
        let file_name = format!("{stem} {index}.{extension}");
        let candidate = if folder.is_empty() {
            file_name
        } else {
            format!("{folder}/{file_name}")
        };
        validate_image_relative_path(&candidate)?;
        if !reserved_paths.contains(&portable_path_key(&candidate))
            && !image_path_exists_portably(root, &candidate)?
        {
            return Ok((candidate, false));
        }
    }

    Err(format!(
        "Could not choose a collision-free vault path for {source_relative_path}."
    ))
}

fn workspace_attachment_import_path(
    root: &Path,
    source_relative_path: &str,
    fingerprint: Option<&FileFingerprint>,
    reserved_paths: &HashSet<String>,
) -> Result<(String, bool), String> {
    let source_key = portable_path_key(source_relative_path);
    if !reserved_paths.contains(&source_key) {
        let target = resolve_workspace_asset_file(root, source_relative_path, true)?;
        match fs::symlink_metadata(&target) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => {
                if fingerprint.is_some_and(|expected| {
                    fingerprint_attachment_file(&target).as_ref() == Ok(expected)
                }) {
                    return Ok((source_relative_path.to_owned(), true));
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if !asset_path_exists_portably(root, source_relative_path)? {
                    return Ok((source_relative_path.to_owned(), false));
                }
            }
            Err(_) => {}
        }
    }

    let path = Path::new(source_relative_path);
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Attachment");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 2..=10_001_u32 {
        let file_name = match extension {
            Some(extension) => format!("{stem} {index}.{extension}"),
            None => format!("{stem} {index}"),
        };
        let candidate = if folder.is_empty() {
            file_name
        } else {
            format!("{folder}/{file_name}")
        };
        validate_attachment_relative_path(&candidate)?;
        if !reserved_paths.contains(&portable_path_key(&candidate))
            && !asset_path_exists_portably(root, &candidate)?
        {
            return Ok((candidate, false));
        }
    }

    Err(format!(
        "Could not choose a collision-free vault path for {source_relative_path}."
    ))
}

fn resolve_attachment_import_source(
    root: &Path,
    relative_path: &str,
) -> Result<PathBuf, String> {
    validate_attachment_relative_path(relative_path)?;
    let relative = checked_relative_path(relative_path, false)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("the source attachment could not be inspected: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("symbolic links are not followed".to_owned());
        }
    }
    let metadata = fs::symlink_metadata(&current)
        .map_err(|error| format!("the source attachment could not be inspected: {error}"))?;
    if !metadata.is_file() {
        return Err("the source attachment is not a regular file".to_owned());
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "the source attachment is larger than {} GiB",
            MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024
        ));
    }
    Ok(current)
}

fn stage_workspace_attachment_import(
    root: &Path,
    transaction: &mut WorkspaceImageImportTransaction,
    relative_path: &str,
    source: &Path,
    expected_fingerprint: &FileFingerprint,
) -> Result<(), String> {
    let transaction_root = match transaction.transaction_root.as_ref() {
        Some(transaction_root) => transaction_root,
        None => {
            let created = prepare_transaction_root(root, &new_transaction_id())
                .map_err(|error| format!("Could not prepare the asset import: {error}"))?;
            transaction.transaction_root.insert(created)
        }
    };
    let kind = TransactionTargetKind::Attachment;
    let staged = staged_import_asset_path(transaction_root, relative_path, &kind)?;
    let parent = staged
        .parent()
        .ok_or_else(|| "The staged attachment path has no parent folder.".to_owned())?;
    ensure_private_directory_tree(transaction_root, parent)
        .map_err(|error| format!("Could not prepare an attachment import: {error}"))?;
    let copied_fingerprint = copy_attachment_file_durable(source, &staged)?;
    if copied_fingerprint != *expected_fingerprint {
        let _ = remove_file_durable(&staged);
        return Err(format!(
            "The staged copy of {relative_path} did not match the selected attachment."
        ));
    }
    transaction.targets.push(TransactionTarget {
        relative_path: relative_path.to_owned(),
        fingerprint: copied_fingerprint,
        kind,
    });
    Ok(())
}

fn begin_workspace_asset_import(
    root: &Path,
    source_root: &Path,
    image_paths: &[String],
    attachment_paths: &[String],
    expected_revision: u64,
) -> Result<WorkspaceImportImagesResult, String> {
    if image_paths.len().saturating_add(attachment_paths.len()) > MAX_VAULT_ASSETS {
        return Err(format!(
            "Only {MAX_VAULT_ASSETS} asset files can be imported at once."
        ));
    }
    let mut transaction = prepare_workspace_image_import(root, expected_revision)?;

    let mut unique_paths = HashSet::new();
    for relative_path in image_paths {
        validate_image_relative_path(relative_path)?;
        if !unique_paths.insert(portable_path_key(relative_path)) {
            return Err(format!(
            "The asset import contains a duplicate path near {relative_path}."
            ));
        }
    }
    for relative_path in attachment_paths {
        validate_attachment_relative_path(relative_path)?;
        if !unique_paths.insert(portable_path_key(relative_path)) {
            return Err(format!(
                "The asset import contains a duplicate path near {relative_path}."
            ));
        }
    }

    let mut image_files = Vec::new();
    let mut attachment_files = Vec::new();
    let mut path_mappings = BTreeMap::new();
    let mut reserved_paths = HashSet::new();
    let mut warnings = WarningCollector::default();
    for relative_path in image_paths {
        let source = match resolve_image_import_source(source_root, relative_path) {
            Ok(source) => source,
            Err(error) => {
                match workspace_image_import_path(root, relative_path, None, &reserved_paths) {
                    Ok((target_path, _)) => {
                        reserved_paths.insert(portable_path_key(&target_path));
                        path_mappings.insert(relative_path.clone(), target_path);
                    }
                    Err(path_error) => warnings.push(format!(
                        "Could not reserve a safe path for {relative_path}: {path_error}"
                    )),
                }
                warnings.push(format!("Skipped {relative_path}: {error}"));
                continue;
            }
        };
        let (bytes, media_type) = match read_image_file(&source).and_then(|bytes| {
            validate_image_bytes(&bytes, Some(relative_path))
                .map(|(media_type, _)| (bytes, media_type.to_owned()))
        }) {
            Ok(image) => image,
            Err(error) => {
                match workspace_image_import_path(root, relative_path, None, &reserved_paths) {
                    Ok((target_path, _)) => {
                        reserved_paths.insert(portable_path_key(&target_path));
                        path_mappings.insert(relative_path.clone(), target_path);
                    }
                    Err(path_error) => warnings.push(format!(
                        "Could not reserve a safe path for {relative_path}: {path_error}"
                    )),
                }
                warnings.push(format!("Skipped {relative_path}: {error}"));
                continue;
            }
        };
        let (target_path, reuse_existing) = match workspace_image_import_path(
            root,
            relative_path,
            Some(&bytes),
            &reserved_paths,
        ) {
            Ok(target) => target,
            Err(error) => {
                warnings.push(format!(
                    "Could not reserve a safe path for {relative_path}: {error}"
                ));
                continue;
            }
        };
        reserved_paths.insert(portable_path_key(&target_path));
        path_mappings.insert(relative_path.clone(), target_path.clone());
        if target_path != *relative_path {
            warnings.push(format!(
                "Imported {relative_path} as {target_path} to avoid an existing vault path."
            ));
        }
        if reuse_existing {
            image_files.push(VaultImageFile {
                asset_id: None,
                relative_path: target_path,
                media_type,
            });
            continue;
        }

        if let Err(error) = stage_workspace_image_import(
            root,
            &mut transaction,
            &target_path,
            &bytes,
        ) {
            warnings.push(format!("Skipped {relative_path}: {error}"));
            continue;
        }
        image_files.push(VaultImageFile {
            asset_id: None,
            relative_path: target_path,
            media_type,
        });
    }

    for relative_path in attachment_paths {
        let source = match resolve_attachment_import_source(source_root, relative_path) {
            Ok(source) => source,
            Err(error) => {
                match workspace_attachment_import_path(
                    root,
                    relative_path,
                    None,
                    &reserved_paths,
                ) {
                    Ok((target_path, _)) => {
                        reserved_paths.insert(portable_path_key(&target_path));
                        path_mappings.insert(relative_path.clone(), target_path);
                    }
                    Err(path_error) => warnings.push(format!(
                        "Could not reserve a safe path for {relative_path}: {path_error}"
                    )),
                }
                warnings.push(format!("Skipped {relative_path}: {error}"));
                continue;
            }
        };
        let fingerprint = match fingerprint_attachment_file(&source) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                match workspace_attachment_import_path(
                    root,
                    relative_path,
                    None,
                    &reserved_paths,
                ) {
                    Ok((target_path, _)) => {
                        reserved_paths.insert(portable_path_key(&target_path));
                        path_mappings.insert(relative_path.clone(), target_path);
                    }
                    Err(path_error) => warnings.push(format!(
                        "Could not reserve a safe path for {relative_path}: {path_error}"
                    )),
                }
                warnings.push(format!("Skipped {relative_path}: {error}"));
                continue;
            }
        };
        let (target_path, reuse_existing) = match workspace_attachment_import_path(
            root,
            relative_path,
            Some(&fingerprint),
            &reserved_paths,
        ) {
            Ok(target) => target,
            Err(error) => {
                warnings.push(format!(
                    "Could not reserve a safe path for {relative_path}: {error}"
                ));
                continue;
            }
        };
        reserved_paths.insert(portable_path_key(&target_path));
        path_mappings.insert(relative_path.clone(), target_path.clone());
        if target_path != *relative_path {
            warnings.push(format!(
                "Imported {relative_path} as {target_path} to avoid an existing vault path."
            ));
        }
        let media_type = attachment_media_type_for_path(Path::new(&target_path)).to_owned();
        let opening_disabled = if reuse_existing {
            resolve_workspace_asset_file(root, &target_path, false)
                .and_then(|path| attachment_opening_is_disabled(&path))
                .unwrap_or(true)
        } else {
            attachment_opening_is_disabled(&source).unwrap_or(true)
        };
        if !reuse_existing {
            if let Err(error) = stage_workspace_attachment_import(
                root,
                &mut transaction,
                &target_path,
                &source,
                &fingerprint,
            ) {
                warnings.push(format!("Skipped {relative_path}: {error}"));
                continue;
            }
        }
        attachment_files.push(VaultAttachmentFile {
            asset_id: None,
            relative_path: target_path,
            media_type,
            byte_length: fingerprint.length,
            opening_disabled,
        });
    }

    if let Some(missing_path) = image_paths
        .iter()
        .chain(attachment_paths.iter())
        .find(|path| !path_mappings.contains_key(path.as_str()))
    {
        if let Some(transaction_root) = transaction.transaction_root.take() {
            discard_private_transaction(root, &transaction_root, &mut warnings);
        }
        return Err(format!(
            "A safe destination could not be reserved for {missing_path}."
        ));
    }

    let (revision, transaction_id) =
        apply_workspace_image_import(root, transaction, &mut warnings)?;
    Ok(WorkspaceImportImagesResult {
        image_count: image_files.len(),
        image_files,
        attachment_count: attachment_files.len(),
        attachment_files,
        path_mappings,
        transaction_id,
        revision,
        saved_at: now_millis(),
        warnings: warnings.finish(),
    })
}

#[cfg(test)]
fn begin_workspace_image_import(
    root: &Path,
    source_root: &Path,
    image_paths: &[String],
    expected_revision: u64,
) -> Result<WorkspaceImportImagesResult, String> {
    begin_workspace_asset_import(root, source_root, image_paths, &[], expected_revision)
}

#[cfg(test)]
fn import_workspace_images(
    root: &Path,
    source_root: &Path,
    image_paths: &[String],
    expected_revision: u64,
) -> Result<WorkspaceImportImagesResult, String> {
    let mut result = begin_workspace_image_import(
        root,
        source_root,
        image_paths,
        expected_revision,
    )?;
    if let Some(transaction_id) = result.transaction_id.take() {
        let mut warnings = WarningCollector::default();
        finalize_workspace_image_import(root, &transaction_id, &mut warnings)?;
        result.warnings.extend(warnings.finish());
    }

    Ok(result)
}

fn prepare_workspace_image_import(
    root: &Path,
    expected_revision: u64,
) -> Result<WorkspaceImageImportTransaction, String> {
    let baseline = revision_entries_for_root(root)?;
    if revision_for_entries(&baseline) != expected_revision {
        return Err(
            "The vault changed before its assets could be imported. Reload it and try again."
                .to_owned(),
        );
    }
    Ok(WorkspaceImageImportTransaction {
        baseline,
        targets: Vec::new(),
        transaction_root: None,
    })
}

fn stage_workspace_image_import(
    root: &Path,
    transaction: &mut WorkspaceImageImportTransaction,
    relative_path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let transaction_root = match transaction.transaction_root.as_ref() {
        Some(transaction_root) => transaction_root,
        None => {
            let created = prepare_transaction_root(root, &new_transaction_id())
                .map_err(|error| format!("Could not prepare the asset import: {error}"))?;
            transaction.transaction_root.insert(created)
        }
    };
    let fingerprint = fingerprint_bytes(bytes);
    let staged = staged_import_image_path(transaction_root, relative_path)?;
    let parent = staged
        .parent()
        .ok_or_else(|| "The staged image path has no parent folder.".to_owned())?;
    ensure_private_directory_tree(transaction_root, parent)
        .map_err(|error| format!("Could not prepare an asset import: {error}"))?;
    atomic_write(&staged, bytes)
        .map_err(|error| format!("Could not stage {relative_path}: {error}"))?;
    if fingerprint_regular_file(&staged)? != Some(fingerprint.clone()) {
        return Err(format!(
            "The staged copy of {relative_path} failed its integrity check."
        ));
    }
    transaction.targets.push(TransactionTarget {
        relative_path: relative_path.to_owned(),
        fingerprint,
        kind: TransactionTargetKind::Image,
    });
    Ok(())
}

fn apply_workspace_image_import(
    root: &Path,
    transaction: WorkspaceImageImportTransaction,
    warnings: &mut WarningCollector,
) -> Result<(u64, Option<String>), String> {
    let WorkspaceImageImportTransaction {
        baseline,
        targets,
        transaction_root,
    } = transaction;

    let current = match revision_entries_for_root(root) {
        Ok(current) => current,
        Err(error) => {
            if let Some(transaction_root) = &transaction_root {
                discard_private_transaction(root, transaction_root, warnings);
            }
            return Err(error);
        }
    };
    if current != baseline {
        if let Some(transaction_root) = &transaction_root {
            discard_private_transaction(root, transaction_root, warnings);
        }
        return Err(
            "The vault changed while its assets were being prepared. Reload it and try again."
                .to_owned(),
        );
    }

    let Some(transaction_root) = transaction_root else {
        return Ok((revision_for_entries(&baseline), None));
    };
    if targets.is_empty() {
        discard_private_transaction(root, &transaction_root, warnings);
        return Ok((revision_for_entries(&baseline), None));
    }

    let parent_paths = targets
        .iter()
        .filter_map(|target| {
            Path::new(&target.relative_path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .and_then(path_to_slash_string)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let created_directories = match collect_created_directories(
        root,
        parent_paths.iter(),
        &[],
    ) {
        Ok(created_directories) => created_directories,
        Err(error) => {
            discard_private_transaction(root, &transaction_root, warnings);
            return Err(error);
        }
    };
    let transaction_id = match transaction_root.file_name().and_then(|value| value.to_str()) {
        Some(transaction_id) => transaction_id.to_owned(),
        None => {
            discard_private_transaction(root, &transaction_root, warnings);
            return Err("The asset import transaction ID is invalid.".to_owned());
        }
    };
    let mut manifest = TransactionManifest {
        version: TRANSACTION_VERSION,
        id: transaction_id,
        phase: TransactionPhase::Prepared,
        originals: Vec::new(),
        targets,
        recovery_targets: Vec::new(),
        folder_case_renames: Vec::new(),
        created_directories,
    };
    if let Err(error) = write_transaction_manifest(&transaction_root, &manifest) {
        discard_private_transaction(root, &transaction_root, warnings);
        return Err(error);
    }
    match revision_entries_for_root(root) {
        Ok(current) if current == baseline => {}
        Ok(_) => {
            discard_private_transaction(root, &transaction_root, warnings);
            return Err(
                "The vault changed while its assets were being prepared. Reload it and try again."
                    .to_owned(),
            );
        }
        Err(error) => {
            discard_private_transaction(root, &transaction_root, warnings);
            return Err(error);
        }
    }

    manifest.phase = TransactionPhase::Applying;
    if let Err(error) = write_transaction_manifest(&transaction_root, &manifest) {
        discard_private_transaction(root, &transaction_root, warnings);
        return Err(error);
    }
    let result = (|| {
        if revision_entries_for_root(root)? != baseline {
            return Err(
                "The vault changed before its assets could be committed. Reload it and try again."
                    .to_owned(),
            );
        }
        for target in &manifest.targets {
            let label = match target.kind {
                TransactionTargetKind::Image => "image",
                TransactionTargetKind::Attachment => "attachment",
                TransactionTargetKind::Markdown => {
                    return Err("A Markdown file was included in an asset import.".to_owned())
                }
            };
            ensure_asset_parent(root, &target.relative_path, label)?;
            apply_staged_import_image(root, &transaction_root, target)?;
        }
        let committed_entries = verify_image_import_consistency(root, &baseline, &manifest)?;
        if verify_image_import_consistency(root, &baseline, &manifest)? != committed_entries {
            return Err(
                "The vault changed while its imported assets were being verified. Reload it and try again."
                    .to_owned(),
            );
        }
        Ok(revision_for_entries(&committed_entries))
    })();

    let revision = match result {
        Ok(revision) => revision,
        Err(error) => {
            let recovered = rollback_transaction(
                root,
                &transaction_root,
                &manifest,
                warnings,
            );
            if recovered {
                discard_private_transaction(root, &transaction_root, warnings);
                return Err(error);
            }
            return Err(format!(
                "{error} The interrupted asset import could not be fully rolled back. Reopen the vault before editing again."
            ));
        }
    };

    Ok((revision, Some(manifest.id)))
}

fn pending_workspace_image_import(
    root: &Path,
    transaction_id: &str,
) -> Result<(PathBuf, TransactionManifest), String> {
    let transaction_root = existing_transaction_root(root, transaction_id)?;
    let manifest = read_transaction_manifest(&transaction_root)?
        .ok_or_else(|| "The pending asset import has no transaction manifest.".to_owned())?;
    if manifest.id != transaction_id
        || manifest.version > TRANSACTION_VERSION
        || manifest.phase != TransactionPhase::Applying
        || !manifest.originals.is_empty()
        || !manifest.recovery_targets.is_empty()
        || !manifest.folder_case_renames.is_empty()
        || manifest.targets.is_empty()
        || manifest
            .targets
            .iter()
            .any(|target| target.kind == TransactionTargetKind::Markdown)
    {
        return Err("The pending asset import transaction is invalid.".to_owned());
    }
    for target in &manifest.targets {
        if !import_image_was_applied(&transaction_root, target)? {
            return Err(format!(
                "The pending asset import did not create {}.",
                target.relative_path,
            ));
        }
        let path = resolve_transaction_target_file(root, target, false)?;
        if fingerprint_regular_file(&path)? != Some(target.fingerprint.clone()) {
            return Err(format!(
                "The imported asset {} changed before its notes were saved.",
                target.relative_path,
            ));
        }
    }

    Ok((transaction_root, manifest))
}

fn finalize_workspace_image_import(
    root: &Path,
    transaction_id: &str,
    warnings: &mut WarningCollector,
) -> Result<(), String> {
    let (transaction_root, mut manifest) = pending_workspace_image_import(root, transaction_id)?;
    manifest.phase = TransactionPhase::Committed;
    write_transaction_manifest(&transaction_root, &manifest)?;
    discard_private_transaction(root, &transaction_root, warnings);

    Ok(())
}

fn rollback_workspace_image_import(
    root: &Path,
    transaction_id: &str,
    warnings: &mut WarningCollector,
) -> Result<bool, String> {
    let (transaction_root, manifest) = pending_workspace_image_import(root, transaction_id)?;
    let recovered = rollback_transaction(root, &transaction_root, &manifest, warnings);
    if recovered {
        discard_private_transaction(root, &transaction_root, warnings);
    }

    Ok(recovered)
}

fn staged_import_image_path(
    transaction_root: &Path,
    relative_path: &str,
) -> Result<PathBuf, String> {
    staged_import_asset_path(
        transaction_root,
        relative_path,
        &TransactionTargetKind::Image,
    )
}

fn staged_import_asset_path(
    transaction_root: &Path,
    relative_path: &str,
    kind: &TransactionTargetKind,
) -> Result<PathBuf, String> {
    match kind {
        TransactionTargetKind::Image => validate_image_relative_path(relative_path)?,
        TransactionTargetKind::Attachment => validate_attachment_relative_path(relative_path)?,
        TransactionTargetKind::Markdown => {
            return Err("Markdown files cannot be staged as imported assets.".to_owned())
        }
    }
    let internal_path = checked_internal_transaction_path(
        &format!("assets/{relative_path}"),
        true,
    )?;
    Ok(transaction_root.join(internal_path))
}

fn import_asset_applied_marker_path(
    transaction_root: &Path,
    target: &TransactionTarget,
) -> Result<PathBuf, String> {
    match target.kind {
        TransactionTargetKind::Image => validate_image_relative_path(&target.relative_path)?,
        TransactionTargetKind::Attachment => {
            validate_attachment_relative_path(&target.relative_path)?
        }
        TransactionTargetKind::Markdown => {
            return Err("Markdown files do not use asset import markers.".to_owned())
        }
    }
    let internal_path = checked_internal_transaction_path(
        &format!("applied/{}.json", target.relative_path),
        true,
    )?;
    Ok(transaction_root.join(internal_path))
}

fn validate_private_import_directory(
    transaction_root: &Path,
    directory: &Path,
) -> Result<(), String> {
    let relative = directory
        .strip_prefix(transaction_root)
        .map_err(|_| "A private asset import path escaped its transaction.".to_owned())?;
    let root_metadata = fs::symlink_metadata(transaction_root)
        .map_err(|error| format!("Could not inspect the asset import transaction: {error}"))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("The asset import transaction is not a regular folder.".to_owned());
    }
    let mut current = transaction_root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("Could not inspect a private asset import folder: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("A private asset import path is not a regular folder.".to_owned());
        }
    }
    Ok(())
}

fn mark_import_image_applied(
    transaction_root: &Path,
    target: &TransactionTarget,
) -> Result<(), String> {
    let marker = import_asset_applied_marker_path(transaction_root, target)?;
    let parent = marker
        .parent()
        .ok_or_else(|| "The asset import marker has no parent folder.".to_owned())?;
    ensure_private_directory_tree(transaction_root, parent)
        .map_err(|error| format!("Could not prepare an asset import marker: {error}"))?;
    let bytes = serde_json::to_vec(&target.fingerprint)
        .map_err(|error| format!("Could not encode an asset import marker: {error}"))?;
    atomic_write(&marker, &bytes)
        .map_err(|error| format!("Could not save an asset import marker: {error}"))?;
    if fingerprint_regular_file(&marker)? != Some(fingerprint_bytes(&bytes)) {
        return Err("An asset import marker failed its integrity check.".to_owned());
    }
    Ok(())
}

fn import_image_was_applied(
    transaction_root: &Path,
    target: &TransactionTarget,
) -> Result<bool, String> {
    let marker = import_asset_applied_marker_path(transaction_root, target)?;
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!("Could not inspect an asset import marker: {error}"));
        }
    };
    let marker_parent = marker
        .parent()
        .ok_or_else(|| "The asset import marker has no parent folder.".to_owned())?;
    validate_private_import_directory(transaction_root, marker_parent)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 1024 {
        return Err("An asset import marker is unsafe or unexpectedly large.".to_owned());
    }
    let bytes = fs::read(&marker)
        .map_err(|error| format!("Could not read an asset import marker: {error}"))?;
    let fingerprint: FileFingerprint = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse an asset import marker: {error}"))?;
    if fingerprint != target.fingerprint {
        return Err("An asset import marker does not match its target.".to_owned());
    }
    Ok(true)
}

fn apply_staged_import_image(
    root: &Path,
    transaction_root: &Path,
    target: &TransactionTarget,
) -> Result<(), String> {
    let staged = staged_import_asset_path(
        transaction_root,
        &target.relative_path,
        &target.kind,
    )?;
    let staged_parent = staged
        .parent()
        .ok_or_else(|| "The staged asset has no parent folder.".to_owned())?;
    validate_private_import_directory(transaction_root, staged_parent)?;
    let metadata = fs::symlink_metadata(&staged)
        .map_err(|error| format!("Could not inspect staged asset {}: {error}", target.relative_path))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() != target.fingerprint.length
    {
        return Err(format!(
            "The staged copy of {} is unsafe or incomplete.",
            target.relative_path
        ));
    }
    let source = File::open(&staged)
        .map_err(|error| format!("Could not open staged asset {}: {error}", target.relative_path))?;
    let opened_metadata = source
        .metadata()
        .map_err(|error| format!("Could not inspect staged asset {}: {error}", target.relative_path))?;
    if !opened_metadata.is_file() || opened_metadata.len() != target.fingerprint.length {
        return Err(format!("The staged copy of {} changed.", target.relative_path));
    }

    let destination = resolve_transaction_target_file(root, target, true)?;
    if let Some(parent) = destination.parent() {
        ensure_existing_directory_without_symlink(root, parent)?;
    }
    let mut destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| {
            format!(
                "Could not import {} without overwriting another file: {error}",
                target.relative_path
            )
        })?;
    let copy_result = (|| -> io::Result<()> {
        let copied = io::copy(
            &mut source.take(target.fingerprint.length.saturating_add(1)),
            &mut destination_file,
        )?;
        if copied != target.fingerprint.length {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the staged asset length changed",
            ));
        }
        destination_file.flush()?;
        destination_file.sync_all()?;
        if let Some(parent) = destination.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    })();
    drop(destination_file);
    if let Err(error) = copy_result {
        let _ = remove_file_durable(&destination);
        return Err(format!("Could not import {}: {error}", target.relative_path));
    }
    if fingerprint_regular_file(&destination)? != Some(target.fingerprint.clone()) {
        return Err(format!(
            "The imported copy of {} failed its integrity check.",
            target.relative_path
        ));
    }
    if let Err(error) = mark_import_image_applied(transaction_root, target) {
        if fingerprint_regular_file(&destination)? == Some(target.fingerprint.clone()) {
            let _ = remove_file_durable(&destination);
        }
        return Err(error);
    }

    Ok(())
}

fn verify_image_import_consistency(
    root: &Path,
    baseline: &[RevisionEntry],
    manifest: &TransactionManifest,
) -> Result<Vec<RevisionEntry>, String> {
    for target in &manifest.targets {
        let destination = resolve_transaction_target_file(root, target, false)?;
        if fingerprint_regular_file(&destination)? != Some(target.fingerprint.clone()) {
            return Err(format!(
                "{} changed while its asset import was being committed.",
                target.relative_path
            ));
        }
    }

    let current = revision_entries_for_root(root)?;
    let allowed_labels = manifest
        .targets
        .iter()
        .map(|target| format!("F:{}", target.relative_path))
        .chain(
            manifest
                .created_directories
                .iter()
                .map(|directory| format!("D:{directory}")),
        )
        .collect::<HashSet<_>>();
    if allowed_labels
        .iter()
        .any(|label| !current.iter().any(|entry| &entry.0 == label))
    {
        return Err(
            "The vault changed while its asset folders were being committed. Reload it and try again."
                .to_owned(),
        );
    }
    let unaffected = current
        .iter()
        .filter(|entry| !allowed_labels.contains(&entry.0))
        .cloned()
        .collect::<Vec<_>>();
    if unaffected != baseline {
        return Err(
            "The vault changed outside Obsidian At Home during the asset import. Reload it before editing again."
                .to_owned(),
        );
    }

    Ok(current)
}

fn ensure_asset_parent(
    root: &Path,
    relative_path: &str,
    asset_label: &str,
) -> Result<(), String> {
    let parent = Path::new(relative_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let Some(parent) = parent else {
        return Ok(());
    };
    let mut current = root.to_path_buf();
    for component in parent.components() {
        let name = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| format!("An {asset_label} folder name is not valid Unicode."))?;
        let mut case_collision = false;
        let mut exact_match = false;
        for entry in fs::read_dir(&current)
            .map_err(|error| format!("Could not inspect {}: {error}", current.display()))?
        {
            let entry = entry
                .map_err(|error| format!("Could not inspect {}: {error}", current.display()))?;
            let entry_name = entry.file_name();
            if entry_name.to_string_lossy().eq_ignore_ascii_case(name) {
                if entry_name == component.as_os_str() {
                    exact_match = true;
                } else {
                    case_collision = true;
                }
                break;
            }
        }
        if case_collision {
            return Err(format!(
                "a folder differing only by letter case already exists near {relative_path}."
            ));
        }
        current.push(component.as_os_str());
        if exact_match {
            let metadata = fs::symlink_metadata(&current)
                .map_err(|error| format!("Could not inspect {}: {error}", current.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(format!("Refusing to follow the symbolic link {}.", current.display()));
            }
            if !metadata.is_dir() {
                return Err(format!("{} is not a folder.", current.display()));
            }
        } else {
            create_directory_durable(&current).map_err(|error| {
                format!(
                    "Could not create the {asset_label} folder {}: {error}",
                    current.display()
                )
            })?;
        }
    }
    Ok(())
}

fn validate_image_import_root(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose a vault folder to import assets from.".to_owned());
    }
    let path = Path::new(input);
    if !path.is_absolute() {
        return Err("The asset import path must be absolute.".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The asset import folder could not be opened: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("The asset import folder cannot be a symbolic link.".to_owned());
    }
    if !metadata.is_dir() {
        return Err("The asset import path is not a folder.".to_owned());
    }
    path.canonicalize()
        .map_err(|error| format!("The asset import folder could not be resolved: {error}"))
}

fn resolve_image_import_source(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    validate_image_relative_path(relative_path)?;
    let relative = checked_relative_path(relative_path, false)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("the source image could not be inspected: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("symbolic links are not followed".to_owned());
        }
    }
    let metadata = fs::symlink_metadata(&current)
        .map_err(|error| format!("the source image could not be inspected: {error}"))?;
    if !metadata.is_file() {
        return Err("the source image is not a regular file".to_owned());
    }
    Ok(current)
}

fn external_file_staging_directory(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_cache_dir()
        .map(|directory| directory.join(EXTERNAL_FILE_UPLOAD_DIRECTORY))
        .map_err(|error| format!("Could not locate temporary dropped-file storage: {error}"))
}

fn safe_external_file_name(file_name: &str) -> Result<String, String> {
    if file_name.trim().is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || Path::new(file_name).components().count() != 1
    {
        return Err("The dropped file name is not safe.".to_owned());
    }
    let path = Path::new(file_name);
    let stem = safe_file_stem(
        path.file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Dropped file"),
        "Dropped file",
    );
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|character| {
            !character.is_control() && !is_forbidden_component_character(*character)
        })
        .take(40)
        .collect::<String>();
    let safe_name = if extension.is_empty() {
        stem
    } else {
        format!("{stem}.{extension}")
    };
    validate_component_name(&safe_name, "file")?;
    Ok(safe_name)
}

fn validate_external_file_drop_note(root: &Path, note_relative_path: &str) -> Result<(), String> {
    validate_markdown_relative_path(note_relative_path)?;
    let note_path = resolve_workspace_file(root, note_relative_path, false)?;
    let metadata = fs::symlink_metadata(&note_path)
        .map_err(|_| "Save the active note before dropping a file into it.".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Save the active note before dropping a file into it.".to_owned());
    }
    Ok(())
}

fn prepare_external_file_staging_directory(directory: &Path) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not prepare temporary dropped-file storage: {error}"))?;
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("Could not inspect temporary dropped-file storage: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Temporary dropped-file storage is not a regular folder.".to_owned());
    }
    cleanup_stale_external_file_uploads(directory);
    Ok(())
}

fn cleanup_stale_external_file_uploads(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten().take(256) {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with("drop-") {
            continue;
        }
        let upload_directory = entry.path();
        let Ok(directory_metadata) = fs::symlink_metadata(&upload_directory) else {
            continue;
        };
        if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
            continue;
        }
        let Ok(children) = fs::read_dir(&upload_directory) else {
            continue;
        };
        let children = children.flatten().take(2).collect::<Vec<_>>();
        if children.len() > 1 {
            continue;
        }
        let child = children.first();
        let modified = child
            .and_then(|child| child.metadata().ok())
            .and_then(|metadata| metadata.modified().ok())
            .or_else(|| directory_metadata.modified().ok());
        let is_stale = modified
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| {
                age >= Duration::from_millis(STALE_EXTERNAL_FILE_UPLOAD_MILLIS)
            });
        if !is_stale {
            continue;
        }
        if let Some(child) = child {
            let Ok(metadata) = fs::symlink_metadata(child.path()) else {
                continue;
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            if remove_file_durable(&child.path()).is_err() {
                continue;
            }
        }
        let _ = remove_directory_durable(&upload_directory);
    }
}

fn remove_abandoned_external_file_uploads(
    uploads: &mut HashMap<String, ExternalFileUpload>,
    now: Instant,
) {
    let timeout = Duration::from_millis(ABANDONED_EXTERNAL_FILE_UPLOAD_MILLIS);
    uploads.retain(|_, upload| {
        now.checked_duration_since(upload.last_activity)
            .map_or(true, |inactive| inactive < timeout)
    });
}

fn begin_external_file_upload(
    staging_directory: &Path,
    file_name: String,
    expected_length: u64,
    kind: ExternalFileUploadKind,
    root: PathBuf,
    note_relative_path: String,
) -> Result<WorkspaceExternalFileUpload, String> {
    match kind {
        ExternalFileUploadKind::Image
            if expected_length == 0 || expected_length > MAX_IMAGE_BYTES => {
            return Err(format!(
                "The dropped image must be between 1 byte and {} MiB.",
                MAX_IMAGE_BYTES / 1024 / 1024,
            ));
        }
        ExternalFileUploadKind::Attachment if expected_length > MAX_ATTACHMENT_BYTES => {
            return Err(format!(
                "The dropped file is larger than {} GiB.",
                MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024,
            ));
        }
        _ => {}
    }
    let file_name = safe_external_file_name(&file_name)?;
    match kind {
        ExternalFileUploadKind::Image => validate_image_relative_path(&file_name)?,
        ExternalFileUploadKind::Attachment => validate_attachment_relative_path(&file_name)?,
    }
    prepare_external_file_staging_directory(staging_directory)?;
    let mut uploads = EXTERNAL_FILE_UPLOADS.lock().map_err(|_| {
        "Dropped-file transfers are unavailable because an earlier transfer failed.".to_owned()
    })?;
    remove_abandoned_external_file_uploads(&mut uploads, Instant::now());
    if uploads.len() >= MAX_EXTERNAL_FILE_UPLOADS {
        return Err("Wait for the current dropped files to finish before adding more.".to_owned());
    }

    for _ in 0..10_000 {
        let id = format!(
            "drop-{}-{}-{}",
            std::process::id(),
            now_millis(),
            TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed),
        );
        if uploads.contains_key(&id) {
            continue;
        }
        let upload_directory = staging_directory.join(&id);
        match fs::create_dir(&upload_directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Could not create temporary dropped-file storage: {error}"
                ));
            }
        }
        let upload_path = upload_directory.join(&file_name);
        let file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&upload_path)
        {
            Ok(file) => file,
            Err(error) => {
                let _ = remove_directory_durable(&upload_directory);
                return Err(format!("Could not stage the dropped file: {error}"));
            }
        };
        uploads.insert(
            id.clone(),
            ExternalFileUpload {
                directory: upload_directory,
                path: upload_path,
                file: Some(file),
                file_name,
                expected_length,
                received_length: 0,
                root,
                note_relative_path,
                kind,
                last_activity: Instant::now(),
                cleanup_on_drop: true,
            },
        );
        return Ok(WorkspaceExternalFileUpload {
            id,
            chunk_bytes: EXTERNAL_FILE_UPLOAD_CHUNK_BYTES,
        });
    }
    Err("Could not reserve temporary storage for the dropped file.".to_owned())
}

fn append_external_file_upload(upload_id: &str, offset: u64, bytes: &[u8]) -> Result<u64, String> {
    let mut uploads = EXTERNAL_FILE_UPLOADS.lock().map_err(|_| {
        "Dropped-file transfers are unavailable because an earlier transfer failed.".to_owned()
    })?;
    let result = (|| {
        let upload = uploads
            .get_mut(upload_id)
            .ok_or_else(|| "The dropped-file transfer is no longer available.".to_owned())?;
        if bytes.is_empty() || bytes.len() > EXTERNAL_FILE_UPLOAD_CHUNK_BYTES {
            return Err("A dropped-file transfer chunk has an invalid size.".to_owned());
        }
        if offset != upload.received_length {
            return Err("Dropped-file transfer chunks arrived out of order.".to_owned());
        }
        let received_length = upload
            .received_length
            .checked_add(bytes.len() as u64)
            .filter(|length| *length <= upload.expected_length)
            .ok_or_else(|| "The dropped file contains more data than expected.".to_owned())?;
        upload
            .file
            .as_mut()
            .ok_or_else(|| "The dropped-file transfer is already closed.".to_owned())?
            .write_all(bytes)
            .map_err(|error| format!("Could not stage the dropped file: {error}"))?;
        upload.received_length = received_length;
        upload.last_activity = Instant::now();
        Ok(received_length)
    })();
    if result.is_err() {
        uploads.remove(upload_id);
    }
    result
}

fn cancel_external_file_upload(upload_id: &str) -> Result<bool, String> {
    let mut uploads = EXTERNAL_FILE_UPLOADS.lock().map_err(|_| {
        "Dropped-file transfers are unavailable because an earlier transfer failed.".to_owned()
    })?;
    Ok(uploads.remove(upload_id).is_some())
}

fn finish_external_file_upload(
    upload_id: &str,
    expected_kind: ExternalFileUploadKind,
) -> Result<StagedExternalFile, String> {
    let mut upload = EXTERNAL_FILE_UPLOADS
        .lock()
        .map_err(|_| {
            "Dropped-file transfers are unavailable because an earlier transfer failed."
                .to_owned()
        })?
        .remove(upload_id)
        .ok_or_else(|| "The dropped-file transfer is no longer available.".to_owned())?;
    if upload.kind != expected_kind {
        return Err("The dropped-file transfer does not match the requested asset type.".to_owned());
    }
    if upload.received_length != upload.expected_length {
        return Err("The dropped file did not finish transferring.".to_owned());
    }
    let mut file = upload
        .file
        .take()
        .ok_or_else(|| "The dropped-file transfer is already closed.".to_owned())?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Could not finish staging the dropped file: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect the staged dropped file: {error}"))?;
    if !metadata.is_file() || metadata.len() != upload.expected_length {
        return Err("The staged dropped file is incomplete or unsafe.".to_owned());
    }
    drop(file);
    let staged = StagedExternalFile {
        directory: upload.directory.clone(),
        path: upload.path.clone(),
        file_name: upload.file_name.clone(),
        root: upload.root.clone(),
        note_relative_path: upload.note_relative_path.clone(),
    };
    upload.cleanup_on_drop = false;
    Ok(staged)
}

fn validate_image_source_file(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose an image file.".to_owned());
    }
    validate_image_source_path(Path::new(input))
}

fn validate_image_source_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("The selected image path must be absolute.".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The selected image could not be opened: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Choose a regular image file, not a symbolic link.".to_owned());
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "The selected image is larger than {} MiB.",
            MAX_IMAGE_BYTES / 1024 / 1024,
        ));
    }
    path.canonicalize()
        .map_err(|error| format!("The selected image could not be resolved: {error}"))
}

fn validate_attachment_source_file(input: &str) -> Result<PathBuf, String> {
    if input.trim().is_empty() {
        return Err("Choose a file to embed.".to_owned());
    }
    validate_attachment_source_path(Path::new(input))
}

fn validate_attachment_source_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("The selected attachment path must be absolute.".to_owned());
    }
    validate_attachment_relative_path(
        path.file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "The selected attachment name is not valid Unicode.".to_owned())?,
    )?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("The selected attachment could not be opened: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Choose a regular file, not a folder, symbolic link, or special file.".to_owned());
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "The selected attachment is larger than {} GiB.",
            MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024,
        ));
    }
    path.canonicalize()
        .map_err(|error| format!("The selected attachment could not be resolved: {error}"))
}

fn fingerprint_attachment_file(path: &Path) -> Result<FileFingerprint, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{} is not a regular attachment file.", path.display()));
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "The attachment is larger than {} GiB.",
            MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024,
        ));
    }
    let mut file = File::open(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if !opened_metadata.is_file() || opened_metadata.len() != metadata.len() {
        return Err("The attachment changed while it was being opened.".to_owned());
    }

    let mut buffer = vec![0_u8; ATTACHMENT_COPY_BUFFER_BYTES];
    let mut hash = 0xcbf29ce484222325_u64;
    let mut length = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(read as u64)
            .filter(|value| *value <= MAX_ATTACHMENT_BYTES)
            .ok_or_else(|| "The attachment became too large while it was being read.".to_owned())?;
        fnv_update(&mut hash, &buffer[..read]);
    }
    if length != metadata.len() {
        return Err("The attachment changed while it was being read.".to_owned());
    }

    Ok(FileFingerprint { length, hash })
}

fn copy_attachment_file_durable(
    source: &Path,
    destination: &Path,
) -> Result<FileFingerprint, String> {
    let source_metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("Could not inspect the selected attachment: {error}"))?;
    if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
        return Err("The selected attachment is not a regular file.".to_owned());
    }
    if source_metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "The selected attachment is larger than {} GiB.",
            MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024,
        ));
    }
    let source_modified_nanos = image_modified_nanos(&source_metadata);
    let mut source_file = File::open(source)
        .map_err(|error| format!("Could not open the selected attachment: {error}"))?;
    let opened_metadata = source_file
        .metadata()
        .map_err(|error| format!("Could not inspect the selected attachment: {error}"))?;
    if !opened_metadata.is_file() || opened_metadata.len() != source_metadata.len() {
        return Err("The selected attachment changed while it was being opened.".to_owned());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "The attachment destination has no parent folder.".to_owned())?;
    let mut destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("Could not create the embedded attachment: {error}"))?;

    let copy_result = (|| -> Result<FileFingerprint, String> {
        let mut buffer = vec![0_u8; ATTACHMENT_COPY_BUFFER_BYTES];
        let mut hash = 0xcbf29ce484222325_u64;
        let mut length = 0_u64;
        loop {
            let read = source_file
                .read(&mut buffer)
                .map_err(|error| format!("Could not read the selected attachment: {error}"))?;
            if read == 0 {
                break;
            }
            length = length
                .checked_add(read as u64)
                .filter(|value| *value <= MAX_ATTACHMENT_BYTES)
                .ok_or_else(|| {
                    "The selected attachment became too large while it was copied.".to_owned()
                })?;
            destination_file
                .write_all(&buffer[..read])
                .map_err(|error| format!("Could not copy the embedded attachment: {error}"))?;
            fnv_update(&mut hash, &buffer[..read]);
        }
        let final_source_metadata = source_file
            .metadata()
            .map_err(|error| format!("Could not recheck the selected attachment: {error}"))?;
        if length != source_metadata.len()
            || final_source_metadata.len() != source_metadata.len()
            || image_modified_nanos(&final_source_metadata) != source_modified_nanos
        {
            return Err("The selected attachment changed while it was being copied.".to_owned());
        }
        destination_file
            .flush()
            .and_then(|_| destination_file.sync_all())
            .map_err(|error| format!("Could not finish the embedded attachment: {error}"))?;

        Ok(FileFingerprint { length, hash })
    })();
    drop(destination_file);
    match copy_result {
        Ok(fingerprint) => {
            sync_directory(parent)
                .map_err(|error| format!("Could not finish the attachment folder: {error}"))?;
            Ok(fingerprint)
        }
        Err(error) => {
            let _ = remove_file_durable(destination);
            Err(error)
        }
    }
}

pub(crate) fn copy_attachment_file_for_transfer(
    source: &Path,
    destination: &Path,
) -> Result<(), String> {
    let fingerprint = copy_attachment_file_durable(source, destination)?;
    match fingerprint_attachment_file(destination) {
        Ok(copied) if copied == fingerprint => Ok(()),
        Ok(_) => {
            let _ = remove_file_durable(destination);
            Err("The transferred attachment failed its integrity check.".to_owned())
        }
        Err(error) => {
            let _ = remove_file_durable(destination);
            Err(format!("The transferred attachment could not be verified: {error}"))
        }
    }
}

fn resolve_attachment_action_source(
    root: &Path,
    attachment_relative_path: &str,
    asset_id: Option<&str>,
) -> Result<(String, PathBuf), String> {
    let relative_path = if let Some(asset_id) = asset_id.filter(|id| !id.is_empty()) {
        if !is_valid_asset_id(asset_id) {
            return Err("The attachment has an invalid stable ID.".to_owned());
        }
        let mut warnings = WarningCollector::default();
        let (state, state_file_was_present) = read_workspace_state(root, &mut warnings);
        if state.is_none() && state_file_was_present {
            return Err(
                "The attachment cannot be opened while workspace metadata is unreadable or newer than this app."
                    .to_owned(),
            );
        }
        if let Some(stored) = state.as_ref().and_then(|state| state.assets.get(asset_id)) {
            if stored.kind != VaultAssetKind::Attachment {
                return Err(
                    "The stable attachment record refers to a different file type.".to_owned(),
                );
            }
            stored.relative_path.clone()
        } else {
            attachment_relative_path.to_owned()
        }
    } else {
        attachment_relative_path.to_owned()
    };
    validate_attachment_relative_path(&relative_path)?;
    let source = resolve_workspace_asset_file(root, &relative_path, false)?;
    let metadata = fs::symlink_metadata(&source)
        .map_err(|error| format!("Could not inspect the attachment: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("The attachment is not a regular vault file.".to_owned());
    }

    Ok((relative_path, source))
}

fn is_archive_attachment_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "7z" | "bz2" | "gz" | "rar" | "tar" | "tgz" | "xz" | "zip"
    )
}

fn is_executable_attachment_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "app"
            | "appimage"
            | "appx"
            | "appxbundle"
            | "bat"
            | "bin"
            | "cmd"
            | "command"
            | "com"
            | "cpl"
            | "deb"
            | "desktop"
            | "dmg"
            | "exe"
            | "fish"
            | "hta"
            | "jar"
            | "js"
            | "jse"
            | "lnk"
            | "msc"
            | "msi"
            | "msix"
            | "msixbundle"
            | "msp"
            | "mst"
            | "pif"
            | "pkg"
            | "ps1"
            | "psm1"
            | "py"
            | "pyw"
            | "reg"
            | "rpm"
            | "run"
            | "scr"
            | "sh"
            | "tool"
            | "vbe"
            | "vbs"
            | "wsf"
            | "wsh"
            | "zsh"
    )
}

fn attachment_opening_is_disabled(path: &Path) -> Result<bool, String> {
    if is_archive_attachment_path(path) {
        return Ok(false);
    }
    if is_executable_attachment_path(path) {
        return Ok(true);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(path)
            .map_err(|error| format!("Could not inspect the attachment before opening: {error}"))?;
        if metadata.permissions().mode() & 0o111 != 0 {
            return Ok(true);
        }
    }

    let mut file = File::open(path)
        .map_err(|error| format!("Could not inspect the attachment before opening: {error}"))?;
    let mut prefix = [0_u8; 8];
    let length = file
        .read(&mut prefix)
        .map_err(|error| format!("Could not inspect the attachment before opening: {error}"))?;

    Ok(attachment_prefix_is_executable(&prefix[..length]))
}

fn attachment_prefix_is_executable(prefix: &[u8]) -> bool {
    prefix.starts_with(b"#!")
        || prefix.starts_with(b"MZ")
        || prefix.starts_with(b"\x7fELF")
        || matches!(
            prefix.get(..4),
            Some(
                [0xfe, 0xed, 0xfa, 0xce]
                    | [0xce, 0xfa, 0xed, 0xfe]
                    | [0xfe, 0xed, 0xfa, 0xcf]
                    | [0xcf, 0xfa, 0xed, 0xfe]
                    | [0xca, 0xfe, 0xba, 0xbe]
                    | [0xbe, 0xba, 0xfe, 0xca]
                    | [0xca, 0xfe, 0xba, 0xbf]
                    | [0xbf, 0xba, 0xfe, 0xca]
            )
        )
}

fn safe_external_copy_directory(root: &Path, input: &Path) -> Option<PathBuf> {
    if !input.is_absolute() {
        return None;
    }
    let metadata = fs::symlink_metadata(input).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return None;
    }
    let directory = input.canonicalize().ok()?;
    (!directory.starts_with(root)).then_some(directory)
}

fn validate_external_attachment_copy_target(root: &Path, target: &Path) -> Result<PathBuf, String> {
    if !target.is_absolute() {
        return Err("Choose an absolute location outside the vault.".to_owned());
    }
    let file_name = target
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Choose a file name for the attachment copy.".to_owned())?;
    let parent = target
        .parent()
        .ok_or_else(|| "The attachment copy location has no parent folder.".to_owned())?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| format!("Could not inspect the attachment copy folder: {error}"))?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err("Choose a regular folder, not a symbolic link.".to_owned());
    }
    let parent = parent
        .canonicalize()
        .map_err(|error| format!("Could not resolve the attachment copy folder: {error}"))?;
    if parent.starts_with(root) {
        return Err(
            "Archive copies must be saved outside the active vault to avoid untracked extracted files."
                .to_owned(),
        );
    }
    let target = parent.join(file_name);
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            return Err(
                "A file already exists at that location. Choose a new name for the copy."
                    .to_owned(),
            );
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!("Could not inspect the attachment copy location: {error}"));
        }
    }

    Ok(target)
}

fn save_workspace_attachment_copy(
    app: &AppHandle,
    root: &Path,
    attachment_relative_path: &str,
    asset_id: Option<&str>,
    preferred_directory: Option<&str>,
) -> Result<Option<WorkspaceAttachmentCopyResult>, String> {
    let (source_name, baseline_fingerprint) = {
        let _guard = lock_workspace_io()?;
        let _workspace_guard = lock_workspace_files(root)?;
        let (_, source) = resolve_attachment_action_source(
            root,
            attachment_relative_path,
            asset_id,
        )?;
        if !is_archive_attachment_path(&source) {
            return Err("Only archive attachments use the Save archive as flow.".to_owned());
        }
        let file_name = source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "The archive name is not valid Unicode.".to_owned())?
            .to_owned();
        (file_name, fingerprint_attachment_file(&source)?)
    };

    let preferred = preferred_directory
        .map(Path::new)
        .and_then(|directory| safe_external_copy_directory(root, directory));
    let downloads = app
        .path()
        .download_dir()
        .ok()
        .and_then(|directory| safe_external_copy_directory(root, &directory));
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Save archive as")
        .set_file_name(&source_name);
    if let Some(directory) = preferred.or(downloads) {
        dialog = dialog.set_directory(directory);
    }
    let Some(selected) = dialog.blocking_save_file() else {
        return Ok(None);
    };
    let selected = selected
        .into_path()
        .map_err(|error| format!("The selected copy location is not a local path: {error}"))?;
    let target = validate_external_attachment_copy_target(root, &selected)?;

    let copied_fingerprint = {
        let _guard = lock_workspace_io()?;
        let _workspace_guard = lock_workspace_files(root)?;
        let (_, source) = resolve_attachment_action_source(
            root,
            attachment_relative_path,
            asset_id,
        )?;
        if !is_archive_attachment_path(&source) {
            return Err("The attachment is no longer an archive.".to_owned());
        }
        if fingerprint_attachment_file(&source)? != baseline_fingerprint {
            return Err(
                "The archive changed while the copy location was being chosen. Try again."
                    .to_owned(),
            );
        }
        copy_attachment_file_durable(&source, &target)?
    };
    if copied_fingerprint != baseline_fingerprint
        || fingerprint_attachment_file(&target)? != baseline_fingerprint
    {
        let _ = remove_file_durable(&target);
        return Err("The saved archive copy failed its integrity check.".to_owned());
    }

    Ok(Some(WorkspaceAttachmentCopyResult {
        path: path_string(&target)?,
    }))
}

fn file_modified_nanos_for_path(path: &Path) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{} is not a regular file.", path.display()));
    }
    Ok(image_modified_nanos(&metadata))
}

fn read_image_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{} is not a regular image file.", path.display()));
    }
    if metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "The image must be between 1 byte and {} MiB.",
            MAX_IMAGE_BYTES / 1024 / 1024,
        ));
    }
    fs::read(path).map_err(|error| format!("Could not read {}: {error}", path.display()))
}

fn image_modified_nanos_for_path(root: &Path, relative_path: &str) -> Result<u64, String> {
    let path = resolve_workspace_image_file(root, relative_path, false)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Could not inspect {relative_path}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{relative_path} is not a regular image file."));
    }
    Ok(image_modified_nanos(&metadata))
}

fn image_modified_nanos(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

pub(crate) fn validate_image_bytes(
    bytes: &[u8],
    file_name: Option<&str>,
) -> Result<(&'static str, &'static str), String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(format!(
            "The image must be between 1 byte and {} MiB.",
            MAX_IMAGE_BYTES / 1024 / 1024,
        ));
    }
    let detected = detect_image_type(bytes)
        .ok_or_else(|| "Use a PNG, JPEG, GIF, WebP, BMP, or AVIF image.".to_owned())?;
    if let Some(extension) = file_name
        .and_then(|name| Path::new(name).extension())
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
    {
        if let Some(expected_media_type) = image_media_type_for_extension(&extension) {
            if expected_media_type != detected.0 {
                return Err("The image contents do not match the file extension.".to_owned());
            }
        }
    }
    Ok(detected)
}

fn detect_image_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(("image/png", "png"));
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(("image/jpeg", "jpg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(("image/gif", "gif"));
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(("image/webp", "webp"));
    }
    if bytes.starts_with(b"BM") {
        return Some(("image/bmp", "bmp"));
    }
    if bytes.len() >= 16
        && &bytes[4..8] == b"ftyp"
        && bytes[8..]
            .chunks_exact(4)
            .any(|brand| brand == b"avif" || brand == b"avis")
    {
        return Some(("image/avif", "avif"));
    }
    None
}

fn image_media_type_for_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "avif" => Some("image/avif"),
        _ => None,
    }
}

pub(crate) fn is_supported_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .and_then(image_media_type_for_extension)
        .is_some()
}

fn safe_image_file_name(file_name: &str, extension: &str) -> String {
    let source_stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Image")
        .trim();
    let normalized = source_stem
        .chars()
        .map(|character| {
            if character.is_control() || is_forbidden_component_character(character) {
                '-'
            } else {
                character
            }
        })
        .collect::<String>();
    let mut stem = String::new();
    for character in normalized.trim_matches([' ', '.', '-']).chars() {
        if stem.len() + character.len_utf8() > 140 {
            break;
        }
        stem.push(character);
    }
    if stem.is_empty() || is_windows_reserved_name(&stem) {
        stem = "Image".to_owned();
    }
    format!("{stem}.{extension}")
}

fn safe_attachment_file_name(file_name: &str) -> Result<String, String> {
    let path = Path::new(file_name);
    let stem = safe_file_stem(
        path.file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Attachment"),
        "Attachment",
    );
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|character| {
            !character.is_control() && !is_forbidden_component_character(*character)
        })
        .take(40)
        .collect::<String>();
    let safe_name = if extension.is_empty() {
        stem
    } else {
        format!("{stem}.{extension}")
    };
    validate_attachment_relative_path(&safe_name)?;
    Ok(safe_name)
}

fn unique_attachment_relative_path(
    root: &Path,
    folder: &str,
    file_name: &str,
) -> Result<String, String> {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Attachment");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 1..=10_000_u32 {
        let candidate_name = if index == 1 {
            file_name.to_owned()
        } else if let Some(extension) = extension {
            format!("{stem} {index}.{extension}")
        } else {
            format!("{stem} {index}")
        };
        let relative_path = if folder.is_empty() {
            candidate_name
        } else {
            format!("{folder}/{candidate_name}")
        };
        validate_attachment_relative_path(&relative_path)?;
        if !asset_path_exists_portably(root, &relative_path)? {
            return Ok(relative_path);
        }
    }
    Err("Could not choose a unique file name for the embedded attachment.".to_owned())
}

fn asset_path_exists_portably(root: &Path, relative_path: &str) -> Result<bool, String> {
    let candidate = resolve_workspace_asset_file(root, relative_path, true)?;
    let parent = candidate
        .parent()
        .ok_or_else(|| "The embedded attachment path has no parent folder.".to_owned())?;
    let file_name = candidate
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "The embedded attachment name is not valid Unicode.".to_owned())?;
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!("Could not inspect {}: {error}", parent.display()));
        }
    };
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("Could not inspect {}: {error}", parent.display()))?;
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case(file_name) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn attachment_media_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "json" => "application/json",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "rtf" => "application/rtf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

fn unique_image_relative_path(
    root: &Path,
    folder: &str,
    file_name: &str,
) -> Result<String, String> {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Image");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png");
    for index in 1..=10_000_u32 {
        let candidate_name = if index == 1 {
            file_name.to_owned()
        } else {
            format!("{stem} {index}.{extension}")
        };
        let relative_path = if folder.is_empty() {
            candidate_name
        } else {
            format!("{folder}/{candidate_name}")
        };
        validate_image_relative_path(&relative_path)?;
        if !image_path_exists_portably(root, &relative_path)? {
            return Ok(relative_path);
        }
    }
    Err("Could not choose a unique file name for the embedded image.".to_owned())
}

fn image_path_exists_portably(root: &Path, relative_path: &str) -> Result<bool, String> {
    let candidate = resolve_workspace_image_file(root, relative_path, true)?;
    let parent = candidate
        .parent()
        .ok_or_else(|| "The embedded image path has no parent folder.".to_owned())?;
    let file_name = candidate
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "The embedded image name is not valid Unicode.".to_owned())?;
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!("Could not inspect {}: {error}", parent.display()));
        }
    };
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("Could not inspect {}: {error}", parent.display()))?;
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case(file_name) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_image_relative_path(relative_path: &str) -> Result<(), String> {
    validate_relative_path(relative_path, false)?;
    let extension = Path::new(relative_path)
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Embedded image paths must include a supported extension.".to_owned())?;
    if image_media_type_for_extension(extension).is_none() {
        return Err("Use a PNG, JPEG, GIF, WebP, BMP, or AVIF image path.".to_owned());
    }
    Ok(())
}

fn validate_attachment_relative_path(relative_path: &str) -> Result<(), String> {
    validate_relative_path(relative_path, false)?;
    let path = Path::new(relative_path);
    if is_markdown_path(path) {
        return Err("Markdown notes should be linked as notes, not embedded as attachments.".to_owned());
    }
    if is_supported_image_path(path) {
        return Err("Use image embedding for PNG, JPEG, GIF, WebP, BMP, and AVIF files.".to_owned());
    }
    Ok(())
}

fn is_valid_asset_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 180
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn resolve_markdown_image_path(
    note_relative_path: &str,
    destination: &str,
) -> Result<String, String> {
    validate_markdown_relative_path(note_relative_path)?;
    if destination.is_empty()
        || destination.contains('\\')
        || destination.contains('#')
        || destination.contains('?')
        || destination.chars().any(char::is_control)
    {
        return Err("The Markdown image path is not a safe local vault path.".to_owned());
    }
    let mut components = if destination.starts_with('/') {
        Vec::new()
    } else {
        Path::new(note_relative_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .and_then(path_to_slash_string)
            .unwrap_or_default()
            .split('/')
            .filter(|component| !component.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    for component in destination.trim_start_matches('/').split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err("The Markdown image path escapes the vault.".to_owned());
                }
            }
            value => components.push(value.to_owned()),
        }
    }
    let relative_path = components.join("/");
    validate_image_relative_path(&relative_path)?;
    Ok(relative_path)
}

fn percent_decode_utf8(value: &str) -> Result<String, String> {
    let source = value.as_bytes();
    let mut decoded = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            if index + 2 >= source.len() {
                return Err("Image metadata contains invalid percent encoding.".to_owned());
            }
            let high = hex_value(source[index + 1])
                .ok_or_else(|| "Image metadata contains invalid percent encoding.".to_owned())?;
            let low = hex_value(source[index + 2])
                .ok_or_else(|| "Image metadata contains invalid percent encoding.".to_owned())?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(source[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded)
        .map_err(|_| "Image metadata is not valid UTF-8.".to_owned())
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn reconcile_image_assets(
    root: &Path,
    assets: &mut BTreeMap<String, StoredVaultAsset>,
    warnings: &mut WarningCollector,
) -> Vec<EmbeddedImage> {
    if assets.len() > MAX_VAULT_ASSETS {
        warnings.push(format!(
            "Only the first {MAX_VAULT_ASSETS} embedded image records were loaded."
        ));
        let retained = assets
            .keys()
            .take(MAX_VAULT_ASSETS)
            .cloned()
            .collect::<HashSet<_>>();
        assets.retain(|id, _| retained.contains(id));
    }
    let invalid_ids = assets
        .iter()
        .filter_map(|(id, asset)| {
            let path_is_invalid = match asset.kind {
                VaultAssetKind::Image => {
                    validate_image_relative_path(&asset.relative_path).is_err()
                }
                VaultAssetKind::Attachment => {
                    validate_attachment_relative_path(&asset.relative_path).is_err()
                }
            };
            (!is_valid_asset_id(id) || path_is_invalid).then(|| id.clone())
        })
        .collect::<Vec<_>>();
    for id in invalid_ids {
        assets.remove(&id);
        warnings.push("Ignored an invalid embedded image record.".to_owned());
    }

    let mut assigned_paths = HashSet::new();
    let mut missing_ids = Vec::new();
    for (id, asset) in assets
        .iter_mut()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Image)
    {
        let path = match resolve_workspace_image_file(root, &asset.relative_path, false) {
            Ok(path) => path,
            Err(_) => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => metadata,
            _ => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let expected_media_type = Path::new(&asset.relative_path)
            .extension()
            .and_then(|value| value.to_str())
            .and_then(image_media_type_for_extension);
        let modified_nanos = image_modified_nanos(&metadata);
        if asset.modified_nanos != 0
            && asset.modified_nanos == modified_nanos
            && asset.fingerprint.length == metadata.len()
            && expected_media_type == Some(asset.media_type.as_str())
        {
            assigned_paths.insert(portable_path_key(&asset.relative_path));
            continue;
        }
        match read_image_file(&path) {
            Ok(bytes) => match validate_image_bytes(&bytes, Some(&asset.relative_path)) {
                Ok((media_type, _)) => {
                    asset.media_type = media_type.to_owned();
                    asset.fingerprint = fingerprint_bytes(&bytes);
                    asset.modified_nanos = modified_nanos;
                    assigned_paths.insert(portable_path_key(&asset.relative_path));
                }
                Err(_) => missing_ids.push(id.clone()),
            },
            Err(_) => missing_ids.push(id.clone()),
        }
    }

    if !missing_ids.is_empty() {
        let missing_lengths = missing_ids
            .iter()
            .filter_map(|id| assets.get(id).map(|asset| asset.fingerprint.length))
            .collect::<HashSet<_>>();
        let mut candidates: HashMap<FileFingerprint, Vec<(String, String, u64)>> = HashMap::new();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(128)
            .into_iter()
            .filter_entry(should_visit_workspace_entry)
            .filter_map(Result::ok)
        {
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
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
            if assigned_paths.contains(&portable_path_key(&relative_path))
                || validate_image_relative_path(&relative_path).is_err()
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !missing_lengths.contains(&metadata.len()) {
                continue;
            }
            let Ok(bytes) = read_image_file(entry.path()) else {
                continue;
            };
            let Ok((media_type, _)) = validate_image_bytes(&bytes, Some(&relative_path)) else {
                continue;
            };
            let modified_nanos = image_modified_nanos(&metadata);
            candidates
                .entry(fingerprint_bytes(&bytes))
                .or_default()
                .push((relative_path, media_type.to_owned(), modified_nanos));
        }

        for id in missing_ids {
            let Some(asset) = assets.get_mut(&id) else {
                continue;
            };
            let Some(matches) = candidates.get(&asset.fingerprint) else {
                warnings.push(format!("Could not find the embedded image {}.", asset.relative_path));
                continue;
            };
            if matches.len() != 1 {
                warnings.push(format!(
                    "Could not uniquely locate the moved embedded image {}.",
                    asset.relative_path,
                ));
                continue;
            }
            let (relative_path, media_type, modified_nanos) = matches[0].clone();
            asset.relative_path = relative_path.clone();
            asset.media_type = media_type;
            asset.modified_nanos = modified_nanos;
            assigned_paths.insert(portable_path_key(&relative_path));
            candidates.remove(&asset.fingerprint);
        }
    }

    assets
        .iter()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Image)
        .map(|(id, asset)| EmbeddedImage {
            id: id.clone(),
            relative_path: asset.relative_path.clone(),
            media_type: asset.media_type.clone(),
        })
        .collect()
}

fn reconcile_attachment_assets(
    root: &Path,
    assets: &mut BTreeMap<String, StoredVaultAsset>,
    warnings: &mut WarningCollector,
) -> Vec<EmbeddedAttachment> {
    let mut assigned_paths = HashSet::new();
    let mut missing_ids = Vec::new();
    for (id, asset) in assets
        .iter_mut()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Attachment)
    {
        let path = match resolve_workspace_asset_file(root, &asset.relative_path, false) {
            Ok(path) => path,
            Err(_) => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata)
                if !metadata.file_type().is_symlink()
                    && metadata.is_file()
                    && metadata.len() <= MAX_ATTACHMENT_BYTES => metadata,
            _ => {
                missing_ids.push(id.clone());
                continue;
            }
        };
        let media_type = attachment_media_type_for_path(Path::new(&asset.relative_path));
        let modified_nanos = image_modified_nanos(&metadata);
        if asset.modified_nanos != 0
            && asset.modified_nanos == modified_nanos
            && asset.fingerprint.length == metadata.len()
            && asset.media_type == media_type
        {
            assigned_paths.insert(portable_path_key(&asset.relative_path));
            continue;
        }
        match fingerprint_attachment_file(&path) {
            Ok(fingerprint) => {
                asset.media_type = media_type.to_owned();
                asset.fingerprint = fingerprint;
                asset.modified_nanos = modified_nanos;
                assigned_paths.insert(portable_path_key(&asset.relative_path));
            }
            Err(_) => missing_ids.push(id.clone()),
        }
    }

    if !missing_ids.is_empty() {
        let missing_lengths = missing_ids
            .iter()
            .filter_map(|id| assets.get(id).map(|asset| asset.fingerprint.length))
            .collect::<HashSet<_>>();
        let mut candidates: HashMap<FileFingerprint, Vec<(String, String, u64)>> = HashMap::new();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(128)
            .into_iter()
            .filter_entry(should_visit_workspace_entry)
            .filter_map(Result::ok)
        {
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
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
            if assigned_paths.contains(&portable_path_key(&relative_path))
                || validate_attachment_relative_path(&relative_path).is_err()
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !missing_lengths.contains(&metadata.len()) {
                continue;
            }
            let Ok(fingerprint) = fingerprint_attachment_file(entry.path()) else {
                continue;
            };
            let modified_nanos = image_modified_nanos(&metadata);
            candidates
                .entry(fingerprint)
                .or_default()
                .push((
                    relative_path.clone(),
                    attachment_media_type_for_path(Path::new(&relative_path)).to_owned(),
                    modified_nanos,
                ));
        }

        for id in missing_ids {
            let Some(asset) = assets.get_mut(&id) else {
                continue;
            };
            let Some(matches) = candidates.get(&asset.fingerprint) else {
                warnings.push(format!(
                    "Could not find the embedded attachment {}.",
                    asset.relative_path,
                ));
                continue;
            };
            if matches.len() != 1 {
                warnings.push(format!(
                    "Could not uniquely locate the moved embedded attachment {}.",
                    asset.relative_path,
                ));
                continue;
            }
            let (relative_path, media_type, modified_nanos) = matches[0].clone();
            asset.relative_path = relative_path.clone();
            asset.media_type = media_type;
            asset.modified_nanos = modified_nanos;
            assigned_paths.insert(portable_path_key(&relative_path));
            candidates.remove(&asset.fingerprint);
        }
    }

    assets
        .iter()
        .filter(|(_, asset)| asset.kind == VaultAssetKind::Attachment)
        .map(|(id, asset)| EmbeddedAttachment {
            id: id.clone(),
            relative_path: asset.relative_path.clone(),
            media_type: asset.media_type.clone(),
            byte_length: asset.fingerprint.length,
            opening_disabled: attachment_opening_is_disabled(
                &root.join(Path::new(&asset.relative_path)),
            )
            .unwrap_or(true),
        })
        .collect()
}

fn workspace_asset_limit_reached(
    image_count: usize,
    attachment_count: usize,
    limit: usize,
) -> bool {
    image_count.saturating_add(attachment_count) >= limit
}

fn scan_workspace_files(
    root: &Path,
    warnings: &mut WarningCollector,
) -> Result<(
    Vec<ScannedNote>,
    Vec<ScannedFolder>,
    Vec<ScannedImage>,
    Vec<ScannedAttachment>,
), String> {
    let mut notes = Vec::new();
    let mut folders = Vec::new();
    let mut images = Vec::new();
    let mut attachments = Vec::new();
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
            Some(path)
                if validate_relative_path(
                    &path,
                    entry.file_type().is_file() && is_markdown_path(entry.path()),
                )
                .is_ok() =>
            {
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
        if !entry.file_type().is_file() {
            continue;
        }
        let markdown = is_markdown_path(entry.path());
        if !markdown
            && workspace_asset_limit_reached(images.len(), attachments.len(), MAX_VAULT_ASSETS)
        {
            warnings.push(format!(
                "Only the first {MAX_VAULT_ASSETS} asset files are shown in the vault."
            ));
            continue;
        }
        if is_supported_image_path(entry.path()) {
            if metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES {
                continue;
            }
            let Some(media_type) = Path::new(&relative_path)
                .extension()
                .and_then(|value| value.to_str())
                .and_then(image_media_type_for_extension)
            else {
                continue;
            };
            images.push(ScannedImage {
                relative_path,
                media_type: media_type.to_owned(),
            });
            continue;
        }
        if !markdown {
            if metadata.len() > MAX_ATTACHMENT_BYTES {
                continue;
            }
            attachments.push(ScannedAttachment {
                relative_path,
                media_type: attachment_media_type_for_path(entry.path()).to_owned(),
                byte_length: metadata.len(),
                opening_disabled: attachment_opening_is_disabled(entry.path()).unwrap_or(true),
            });
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
    images.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    attachments.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok((notes, folders, images, attachments))
}

fn load_recently_deleted_notes(
    root: &Path,
    stored: &BTreeMap<String, StoredRecentlyDeletedNote>,
    warnings: &mut WarningCollector,
) -> Vec<RecentlyDeletedNote> {
    if stored.is_empty() {
        return Vec::new();
    }
    if let Err(error) = inspect_recently_deleted_directory(root) {
        warnings.push(error);

        return Vec::new();
    }

    let mut entries = stored.iter().collect::<Vec<_>>();
    entries.sort_by(|(left_id, left), (right_id, right)| {
        right
            .deleted_at
            .cmp(&left.deleted_at)
            .then_with(|| right_id.cmp(left_id))
    });
    if entries.len() > MAX_RECENTLY_DELETED_NOTES {
        warnings.push(format!(
            "Only the newest {MAX_RECENTLY_DELETED_NOTES} recovery snapshots were loaded."
        ));
        entries.truncate(MAX_RECENTLY_DELETED_NOTES);
    }

    let mut deleted_notes = Vec::with_capacity(entries.len());
    let mut total_bytes = 0_u64;
    for (id, entry) in entries {
        if validate_recently_deleted_id(id).is_err() {
            warnings.push("Ignored a recovery snapshot with an invalid ID.".to_owned());
            continue;
        }
        if entry.expires_at
            != entry
                .deleted_at
                .saturating_add(RECENTLY_DELETED_RETENTION_MILLIS)
        {
            warnings.push(format!(
                "Ignored recovery snapshot {id} because its retention period is invalid."
            ));
            continue;
        }
        if entry.fingerprint.length > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
            warnings.push(format!(
                "Ignored recovery snapshot {id} because it is unexpectedly large."
            ));
            continue;
        }
        let Some(next_total) = total_bytes.checked_add(entry.fingerprint.length) else {
            warnings.push(
                "Stopped loading recovery snapshots because their size overflowed.".to_owned(),
            );
            break;
        };
        if next_total > MAX_RECENTLY_DELETED_BYTES {
            warnings.push(format!(
                "Stopped after reading {} MiB of recovery snapshots.",
                MAX_RECENTLY_DELETED_BYTES / 1024 / 1024,
            ));
            break;
        }
        total_bytes = next_total;

        match read_indexed_recently_deleted_note(root, id, entry) {
            Ok(deleted_note) => deleted_notes.push(deleted_note),
            Err(error) => warnings.push(error),
        }
    }

    deleted_notes
}

fn read_indexed_recently_deleted_note(
    root: &Path,
    id: &str,
    entry: &StoredRecentlyDeletedNote,
) -> Result<RecentlyDeletedNote, String> {
    validate_recently_deleted_id(id)?;
    if entry.expires_at
        != entry
            .deleted_at
            .saturating_add(RECENTLY_DELETED_RETENTION_MILLIS)
    {
        return Err(format!(
            "Recovery snapshot {id} has an invalid retention period."
        ));
    }
    if entry.fingerprint.length > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
        return Err(format!("Recovery snapshot {id} is unexpectedly large."));
    }

    let path = recently_deleted_snapshot_path(root, id)?;
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            format!("Recovery snapshot {id} is missing.")
        } else {
            format!("Could not inspect recovery snapshot {id}: {error}")
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "Recovery snapshot {id} is not a regular file."
        ));
    }
    if metadata.len() != entry.fingerprint.length {
        return Err(format!("Recovery snapshot {id} does not match its metadata."));
    }

    let file = File::open(&path)
        .map_err(|error| format!("Could not open recovery snapshot {id}: {error}"))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect recovery snapshot {id}: {error}"))?;
    if !opened_metadata.is_file() || opened_metadata.len() != entry.fingerprint.length {
        return Err(format!(
            "Recovery snapshot {id} changed while it was being opened."
        ));
    }
    let read_limit = entry
        .fingerprint
        .length
        .checked_add(1)
        .ok_or_else(|| format!("Recovery snapshot {id} is too large to read safely."))?;
    let mut bytes = Vec::with_capacity(entry.fingerprint.length as usize);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read recovery snapshot {id}: {error}"))?;
    if bytes.len() as u64 != entry.fingerprint.length
        || fingerprint_bytes(&bytes) != entry.fingerprint
    {
        return Err(format!("Recovery snapshot {id} failed its integrity check."));
    }

    let snapshot = serde_json::from_slice::<RecentlyDeletedSnapshot>(&bytes)
        .map_err(|error| format!("Could not parse recovery snapshot {id}: {error}"))?;
    if snapshot.version == 0 || snapshot.version > RECENTLY_DELETED_SNAPSHOT_VERSION {
        return Err(format!(
            "Recovery snapshot {id} uses unsupported version {}.",
            snapshot.version,
        ));
    }
    validate_loaded_recently_deleted_note(id, entry, &snapshot.deleted_note)?;

    Ok(snapshot.deleted_note)
}

fn verify_recovery_snapshot_target(
    root: &Path,
    id: &str,
    fingerprint: &FileFingerprint,
) -> Result<(), String> {
    let path = recently_deleted_snapshot_path(root, id)?;
    if fingerprint_regular_file(&path)? != Some(fingerprint.clone()) {
        return Err(format!(
            "Recovery snapshot {id} changed while the operation was being prepared."
        ));
    }

    Ok(())
}

fn remove_recovery_snapshot_if_matches(
    root: &Path,
    id: &str,
    fingerprint: &FileFingerprint,
    warnings: &mut WarningCollector,
) -> bool {
    match delete_recovery_snapshot_if_matches(root, id, fingerprint) {
        Ok(RecoverySnapshotRemoval::Removed | RecoverySnapshotRemoval::AlreadyMissing) => true,
        Ok(RecoverySnapshotRemoval::RemovedWithoutDurability(error)) => {
            warnings.push(format!(
                "Recovery snapshot {id} was removed, but its deletion could not be made fully \
                 durable: {error}"
            ));
            true
        }
        Err(error) => {
            warnings.push(format!(
                "The recovery entry was removed, but snapshot {id} could not be cleaned up: \
                 {error}"
            ));
            false
        }
    }
}

fn delete_recovery_snapshot_if_matches(
    root: &Path,
    id: &str,
    fingerprint: &FileFingerprint,
) -> Result<RecoverySnapshotRemoval, String> {
    let directory = root
        .join(STATE_DIRECTORY)
        .join(RECENTLY_DELETED_DIRECTORY);
    match inspect_recently_deleted_directory(root) {
        Ok(_) => {}
        Err(_) if fs::symlink_metadata(&directory).is_err_and(|error| {
            error.kind() == io::ErrorKind::NotFound
        }) => return Ok(RecoverySnapshotRemoval::AlreadyMissing),
        Err(error) => return Err(error),
    }
    let path = recently_deleted_snapshot_path(root, id)?;
    match fingerprint_regular_file(&path)? {
        Some(current) if current == *fingerprint => match remove_file_durable(&path) {
            Ok(()) => Ok(RecoverySnapshotRemoval::Removed),
            Err(error) => classify_recovery_snapshot_removal_error(&path, id, error),
        },
        Some(_) => Err(format!(
            "Recovery snapshot {id} changed during cleanup and was left untouched."
        )),
        None => Ok(RecoverySnapshotRemoval::AlreadyMissing),
    }
}

fn classify_recovery_snapshot_removal_error(
    path: &Path,
    id: &str,
    removal_error: io::Error,
) -> Result<RecoverySnapshotRemoval, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(
            RecoverySnapshotRemoval::RemovedWithoutDurability(removal_error.to_string()),
        ),
        Ok(_) => Err(format!(
            "Could not remove recovery snapshot {id}: {removal_error}"
        )),
        Err(error) => Err(format!(
            "Could not remove recovery snapshot {id}: {removal_error}. Its path could not be \
             checked afterward: {error}"
        )),
    }
}

fn remove_expired_recovery_snapshot(
    root: &Path,
    id: &str,
    entry: &StoredRecentlyDeletedNote,
    warnings: &mut WarningCollector,
) -> bool {
    let directory = root
        .join(STATE_DIRECTORY)
        .join(RECENTLY_DELETED_DIRECTORY);
    match inspect_recently_deleted_directory(root) {
        Ok(_) => {}
        Err(_) if fs::symlink_metadata(&directory).is_err_and(|error| {
            error.kind() == io::ErrorKind::NotFound
        }) => {
            warnings.push(format!(
                "Finished removing expired recovery entry {id} after its snapshot was already \
                 cleaned up."
            ));

            return true;
        }
        Err(error) => {
            warnings.push(format!(
                "Expired recovery entry {id} remains recoverable because cleanup could not start: \
                 {error}"
            ));

            return false;
        }
    }

    match read_indexed_recently_deleted_note(root, id, entry) {
        Ok(_) => {}
        Err(error) => {
            let path = match recently_deleted_snapshot_path(root, id) {
                Ok(path) => path,
                Err(path_error) => {
                    warnings.push(path_error);

                    return false;
                }
            };
            if fs::symlink_metadata(path).is_err_and(|metadata_error| {
                metadata_error.kind() == io::ErrorKind::NotFound
            }) {
                warnings.push(format!(
                    "Finished removing expired recovery entry {id} after its snapshot was already \
                     cleaned up."
                ));

                return true;
            }
            warnings.push(format!(
                "Expired recovery entry {id} was retained because it could not be verified: \
                 {error}"
            ));

            return false;
        }
    }

    match delete_recovery_snapshot_if_matches(root, id, &entry.fingerprint) {
        Ok(RecoverySnapshotRemoval::Removed | RecoverySnapshotRemoval::AlreadyMissing) => true,
        Ok(RecoverySnapshotRemoval::RemovedWithoutDurability(error)) => {
            warnings.push(format!(
                "Expired recovery snapshot {id} was removed, but its deletion could not be made \
                 fully durable: {error}"
            ));
            true
        }
        Err(error) => {
            warnings.push(format!(
                "Expired recovery entry {id} remains recoverable because its snapshot could not \
                 be removed: {error}"
            ));
            false
        }
    }
}

fn remove_recently_deleted_notes(
    root: &Path,
    requested_ids: Vec<String>,
    expected_revision: u64,
    expired_only: bool,
) -> Result<WorkspaceRecoveryMutationResult, String> {
    if !expired_only && requested_ids.is_empty() {
        return Err("Choose at least one deleted note to remove.".to_owned());
    }
    if requested_ids.len() > MAX_RECENTLY_DELETED_NOTES {
        return Err("Too many recovery entries were requested at once.".to_owned());
    }
    let mut requested = HashSet::new();
    for id in &requested_ids {
        validate_recently_deleted_id(id)?;
        if !requested.insert(id.clone()) {
            return Err("A recovery entry was requested more than once.".to_owned());
        }
    }

    let mut warnings = WarningCollector::default();
    let state_path = workspace_state_path(root);
    let expected_state_fingerprint = fingerprint_regular_file(&state_path)?;
    let (state, state_file_was_present) = read_workspace_state(root, &mut warnings);
    let mut state = state.ok_or_else(|| {
        if state_file_was_present {
            "Workspace metadata is unreadable or newer than this app.".to_owned()
        } else {
            "Workspace metadata is missing. Reopen the vault and try again.".to_owned()
        }
    })?;
    recover_workspace_transactions(root, Some(&state), &mut warnings)?;
    if revision_for_root(root)? != expected_revision
        || fingerprint_regular_file(&state_path)? != expected_state_fingerprint
    {
        return Err(
            "The vault changed outside Obsidian At Home. Reload it before changing Recently Deleted."
                .to_owned(),
        );
    }

    if !expired_only && !state.recently_deleted_notes.is_empty() {
        inspect_recently_deleted_directory(root)?;
    }
    let now = now_millis();
    let candidate_ids = if expired_only {
        state
            .recently_deleted_notes
            .iter()
            .filter_map(|(id, entry)| (now >= entry.expires_at).then(|| id.clone()))
            .collect::<Vec<_>>()
    } else {
        requested_ids
    };
    let mut removals = Vec::new();
    for id in candidate_ids {
        let Some(entry) = state.recently_deleted_notes.get(&id) else {
            if expired_only {
                continue;
            }

            return Err(format!("Recovery entry {id} is no longer available."));
        };
        if expired_only {
            removals.push((id, entry.clone()));
        } else {
            match read_indexed_recently_deleted_note(root, &id, entry) {
                Ok(_) => removals.push((id, entry.clone())),
                Err(error) => return Err(error),
            }
        }
    }

    if fingerprint_regular_file(&state_path)? != expected_state_fingerprint
        || revision_for_root(root)? != expected_revision
    {
        return Err(
            "The vault changed while Recently Deleted was being updated. Reload it and try again."
                .to_owned(),
        );
    }
    if expired_only {
        removals.retain(|(id, entry)| {
            remove_expired_recovery_snapshot(root, id, entry, &mut warnings)
        });
    } else {
        for (id, entry) in &removals {
            verify_recovery_snapshot_target(root, id, &entry.fingerprint)?;
        }
    }

    let saved_at = now_millis();
    let removed_ids = removals
        .iter()
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    if !removed_ids.is_empty() {
        state.version = STATE_VERSION;
        for id in &removed_ids {
            state.recently_deleted_notes.remove(id);
        }
        write_workspace_state(root, &state)?;
    }
    let mut protected_ids = HashSet::new();
    if !expired_only && !removed_ids.is_empty() {
        for (id, entry) in &removals {
            if !remove_recovery_snapshot_if_matches(
                root,
                id,
                &entry.fingerprint,
                &mut warnings,
            ) {
                protected_ids.insert(id.clone());
            }
        }
    }

    cleanup_orphaned_recovery_snapshots(
        root,
        &state.recently_deleted_notes,
        &protected_ids,
        &mut warnings,
    );
    let revision = revision_for_root(root)?;

    Ok(WorkspaceRecoveryMutationResult {
        removed_ids,
        revision,
        saved_at,
        warnings: warnings.finish(),
    })
}

fn cleanup_orphaned_recovery_snapshots(
    root: &Path,
    indexed: &BTreeMap<String, StoredRecentlyDeletedNote>,
    protected_ids: &HashSet<String>,
    warnings: &mut WarningCollector,
) {
    let directory = match inspect_recently_deleted_directory(root) {
        Ok(directory) => directory,
        Err(error) => {
            if fs::symlink_metadata(
                root.join(STATE_DIRECTORY)
                    .join(RECENTLY_DELETED_DIRECTORY),
            )
            .is_ok()
            {
                warnings.push(error);
            }

            return;
        }
    };
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!("Could not inspect Recently Deleted cleanup: {error}"));

            return;
        }
    };
    for entry in entries.take(MAX_RECENTLY_DELETED_NOTES.saturating_mul(2)) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("Could not inspect a recovery snapshot: {error}"));
                continue;
            }
        };
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(id) = file_name.strip_suffix(".snapshot") else {
            continue;
        };
        if validate_recently_deleted_id(id).is_err()
            || indexed.contains_key(id)
            || protected_ids.contains(id)
        {
            continue;
        }
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!(
                    "Could not inspect orphaned recovery snapshot {id}: {error}"
                ));
                continue;
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            warnings.push(format!(
                "Orphaned recovery snapshot {id} was left untouched because it is not a regular \
                 file."
            ));
            continue;
        }
        if let Err(error) = remove_file_durable(&path) {
            warnings.push(format!(
                "Could not clean orphaned recovery snapshot {id}: {error}"
            ));
        }
    }
}

fn validate_loaded_recently_deleted_note(
    id: &str,
    stored: &StoredRecentlyDeletedNote,
    deleted_note: &RecentlyDeletedNote,
) -> Result<(), String> {
    if deleted_note.id != id
        || deleted_note.deleted_at != stored.deleted_at
        || deleted_note.expires_at != stored.expires_at
        || deleted_note.note.id.trim().is_empty()
        || deleted_note.note.content.len() as u64 > MAX_NOTE_BYTES
    {
        return Err(format!("Recovery snapshot {id} contains invalid note metadata."));
    }
    validate_markdown_relative_path(&deleted_note.note.relative_path).map_err(|_| {
        format!("Recovery snapshot {id} contains an unsafe original note path.")
    })?;
    let expected_folder_path = Path::new(&deleted_note.note.relative_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .and_then(path_to_slash_string)
        .unwrap_or_default();
    if deleted_note.original_folder_path != expected_folder_path {
        return Err(format!(
            "Recovery snapshot {id} does not match its original folder."
        ));
    }
    if deleted_note
        .editor_position
        .as_ref()
        .is_some_and(|position| !is_valid_editor_position(position))
    {
        return Err(format!(
            "Recovery snapshot {id} contains an invalid editor position."
        ));
    }

    Ok(())
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

            return (None, true);
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

fn load_editor_positions(
    root: &Path,
    note_ids: &HashSet<&str>,
    warnings: &mut WarningCollector,
) -> (BTreeMap<String, NoteEditorPosition>, bool, Option<String>) {
    let raw = match read_editor_positions(root) {
        Ok(EditorPositionsRead::Missing) => return (BTreeMap::new(), true, None),
        Ok(EditorPositionsRead::Invalid(error, fingerprint)) => {
            warnings.push(format!("Ignored saved editor positions: {error}"));
            let positions = BTreeMap::new();
            let revision = rewrite_editor_positions_if_unchanged(
                root,
                &positions,
                &fingerprint,
                warnings,
                "replace invalid saved editor positions",
            );

            return (positions, revision.is_some(), revision);
        }
        Ok(EditorPositionsRead::Newer(version, _)) => {
            warnings.push(format!(
                "Saved editor positions use version {version}, but this app supports up to version {EDITOR_POSITIONS_VERSION}. They were ignored and not changed."
            ));

            return (BTreeMap::new(), false, None);
        }
        Ok(EditorPositionsRead::Loaded(raw, fingerprint)) => (raw, fingerprint),
        Err(error) => {
            warnings.push(format!("Ignored saved editor positions: {error}"));

            return (BTreeMap::new(), false, None);
        }
    };
    let (raw, fingerprint) = raw;
    let decoded = decode_editor_positions(raw.positions, note_ids);
    if decoded.invalid_count > 0 {
        warnings.push(format!(
            "Ignored {} invalid saved editor position{}.",
            decoded.invalid_count,
            if decoded.invalid_count == 1 { "" } else { "s" },
        ));
    }
    if decoded.unknown_count > 0 {
        warnings.push(format!(
            "Ignored {} saved editor position{} for notes that no longer exist.",
            decoded.unknown_count,
            if decoded.unknown_count == 1 { "" } else { "s" },
        ));
    }
    let revision = if decoded.invalid_count > 0 || decoded.unknown_count > 0 {
        rewrite_editor_positions_if_unchanged(
            root,
            &decoded.positions,
            &fingerprint,
            warnings,
            "prune saved editor positions",
        )
    } else {
        Some(editor_positions_revision(&fingerprint))
    };

    (decoded.positions, revision.is_some(), revision)
}

fn rewrite_editor_positions_if_unchanged(
    root: &Path,
    positions: &BTreeMap<String, NoteEditorPosition>,
    fingerprint: &FileFingerprint,
    warnings: &mut WarningCollector,
    action: &str,
) -> Option<String> {
    let _lock = match lock_editor_positions(root) {
        Ok(lock) => lock,
        Err(error) => {
            warnings.push(format!("Could not {action}: {error}"));

            return None;
        }
    };
    let unchanged = fingerprint_regular_file(&editor_positions_path(root))
        .is_ok_and(|current| current.as_ref() == Some(fingerprint));
    if !unchanged {
        warnings.push(format!(
            "Saved editor positions changed while the app tried to {action} and were left untouched."
        ));

        return None;
    }
    if let Err(error) = write_editor_positions(root, positions) {
        warnings.push(format!("Could not {action}: {error}"));

        return None;
    }

    fingerprint_regular_file(&editor_positions_path(root))
        .ok()
        .flatten()
        .as_ref()
        .map(editor_positions_revision)
}

fn save_editor_positions(
    root: &Path,
    positions: BTreeMap<String, NoteEditorPosition>,
    expected_revision: Option<String>,
) -> Result<String, String> {
    let _lock = lock_editor_positions(root)?;
    validate_editor_positions(&positions)?;
    let state_path = workspace_state_path(root);
    let expected_state_fingerprint = fingerprint_regular_file(&state_path)?.ok_or_else(|| {
        "Workspace metadata is missing. Reopen the vault before saving editor positions."
            .to_owned()
    })?;
    let mut state_warnings = WarningCollector::default();
    let (state, state_file_was_present) = read_workspace_state(root, &mut state_warnings);
    let state = state.ok_or_else(|| {
        if state_file_was_present {
            "Workspace metadata is unreadable or newer than this app. Editor positions were not changed."
        } else {
            "Workspace metadata is missing. Reopen the vault before saving editor positions."
        }
        .to_owned()
    })?;
    if positions
        .keys()
        .any(|note_id| !state.note_paths.contains_key(note_id))
    {
        return Err(
            "Editor positions refer to notes that have not been saved yet. Try again.".to_owned(),
        );
    }
    let existing_positions = read_editor_positions(root)?;
    let expected_positions_fingerprint = existing_positions.fingerprint().cloned();
    match existing_positions {
        EditorPositionsRead::Missing => {}
        EditorPositionsRead::Newer(version, _) => {
            return Err(format!(
                "The existing editor positions use version {version}, but this app supports up to version {EDITOR_POSITIONS_VERSION}. Update the app before changing them."
            ));
        }
        EditorPositionsRead::Loaded(_, _) | EditorPositionsRead::Invalid(_, _) => {}
    }
    let current_revision = expected_positions_fingerprint
        .as_ref()
        .map(editor_positions_revision);
    if current_revision != expected_revision {
        return Err(
            "Editor positions changed in another app window. Reopen the vault before saving them."
                .to_owned(),
        );
    }
    if fingerprint_regular_file(&state_path)? != Some(expected_state_fingerprint) {
        return Err(
            "Workspace metadata changed while editor positions were being saved. Try again."
                .to_owned(),
        );
    }
    if fingerprint_regular_file(&editor_positions_path(root))? != expected_positions_fingerprint {
        return Err(
            "Editor positions changed while they were being saved. Try again.".to_owned(),
        );
    }
    write_editor_positions(root, &positions)?;
    fingerprint_regular_file(&editor_positions_path(root))?
        .as_ref()
        .map(editor_positions_revision)
        .ok_or_else(|| "Editor positions disappeared after they were saved.".to_owned())
}

fn read_editor_positions(root: &Path) -> Result<EditorPositionsRead, String> {
    let directory = root.join(STATE_DIRECTORY);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("the .obsidian-at-home folder is a symbolic link".to_owned());
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(".obsidian-at-home is not a folder".to_owned());
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(EditorPositionsRead::Missing);
        }
        Err(error) => {
            return Err(format!("the .obsidian-at-home folder could not be inspected: {error}"));
        }
    }

    let path = editor_positions_path(root);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(EditorPositionsRead::Missing);
        }
        Err(error) => return Err(format!("editor-positions.json could not be inspected: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("editor-positions.json is not a regular file".to_owned());
    }
    if metadata.len() > MAX_EDITOR_POSITIONS_BYTES {
        return Err("editor-positions.json is unexpectedly large".to_owned());
    }

    let bytes = fs::read(&path)
        .map_err(|error| format!("editor-positions.json could not be read: {error}"))?;
    let fingerprint = fingerprint_bytes(&bytes);
    let raw = match serde_json::from_slice::<RawEditorPositions>(&bytes) {
        Ok(raw) => raw,
        Err(error) => {
            return Ok(EditorPositionsRead::Invalid(
                format!("editor-positions.json is invalid: {error}"),
                fingerprint,
            ));
        }
    };
    if raw.version == 0 {
        return Ok(EditorPositionsRead::Invalid(
            "editor-positions.json has an invalid version".to_owned(),
            fingerprint,
        ));
    }
    if raw.version > EDITOR_POSITIONS_VERSION {
        return Ok(EditorPositionsRead::Newer(raw.version, fingerprint));
    }
    Ok(EditorPositionsRead::Loaded(raw, fingerprint))
}

fn decode_editor_positions(
    positions: BTreeMap<String, serde_json::Value>,
    note_ids: &HashSet<&str>,
) -> DecodedEditorPositions {
    let mut decoded = BTreeMap::new();
    let mut invalid_count = 0;
    let mut unknown_count = 0;

    for (note_id, value) in positions {
        if !note_ids.contains(note_id.as_str()) {
            unknown_count += 1;
            continue;
        }
        match serde_json::from_value::<NoteEditorPosition>(value) {
            Ok(position) if is_valid_editor_position(&position) => {
                decoded.insert(note_id, position);
            }
            _ => invalid_count += 1,
        }
    }

    DecodedEditorPositions {
        positions: decoded,
        invalid_count,
        unknown_count,
    }
}

fn validate_editor_positions(
    positions: &BTreeMap<String, NoteEditorPosition>,
) -> Result<(), String> {
    if positions.len() > MAX_NOTES {
        return Err(format!(
            "A vault can store positions for at most {MAX_NOTES} notes."
        ));
    }
    if positions
        .iter()
        .any(|(note_id, position)| note_id.trim().is_empty() || !is_valid_editor_position(position))
    {
        return Err("Editor positions contain an invalid entry.".to_owned());
    }

    Ok(())
}

fn is_valid_editor_position(position: &NoteEditorPosition) -> bool {
    let maximum = MAX_SAFE_JAVASCRIPT_INTEGER as f64;

    position.selection.anchor <= MAX_SAFE_JAVASCRIPT_INTEGER
        && position.selection.head <= MAX_SAFE_JAVASCRIPT_INTEGER
        && position.viewport.anchor <= MAX_SAFE_JAVASCRIPT_INTEGER
        && position.viewport.offset.is_finite()
        && position.viewport.offset.abs() <= maximum
        && position.viewport.left.is_finite()
        && (0.0..=maximum).contains(&position.viewport.left)
}

fn write_editor_positions(
    root: &Path,
    positions: &BTreeMap<String, NoteEditorPosition>,
) -> Result<(), String> {
    let directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &directory)?;
    let stored = StoredEditorPositions {
        version: EDITOR_POSITIONS_VERSION,
        positions,
    };
    let mut bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|error| format!("Could not encode editor positions: {error}"))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_EDITOR_POSITIONS_BYTES {
        return Err("There are too many editor positions to save safely.".to_owned());
    }
    atomic_write(&editor_positions_path(root), &bytes)
        .map_err(|error| format!("Could not write editor positions: {error}"))
}

fn lock_workspace_files(root: &Path) -> Result<File, String> {
    let file = open_workspace_lock_file(root)?;
    file.lock()
        .map_err(|error| format!("Could not lock the vault: {error}"))?;

    Ok(file)
}

fn open_workspace_lock_file(root: &Path) -> Result<File, String> {
    let directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &directory)?;
    let path = directory.join(WORKSPACE_LOCK_FILE);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("The workspace lock is not a regular file.".to_owned());
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not inspect the workspace lock: {error}")),
    }

    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|error| format!("Could not open the workspace lock: {error}"))
}

fn lock_editor_positions(root: &Path) -> Result<File, String> {
    let directory = root.join(STATE_DIRECTORY);
    ensure_state_directory(root, &directory)?;
    let path = directory.join(EDITOR_POSITIONS_LOCK_FILE);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("The editor-position lock is not a regular file.".to_owned());
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!("Could not inspect the editor-position lock: {error}"));
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|error| format!("Could not open the editor-position lock: {error}"))?;
    file.lock()
        .map_err(|error| format!("Could not lock editor positions: {error}"))?;

    Ok(file)
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
    revision_entries_for_root(root).map(|entries| revision_for_entries(&entries))
}

fn revision_entries_for_root(root: &Path) -> Result<Vec<RevisionEntry>, String> {
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
        } else if entry.file_type().is_file() {
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

    Ok(entries)
}

fn revision_for_entries(entries: &[RevisionEntry]) -> u64 {
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
    if revision == 0 { 1 } else { revision }
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
            concat!(
                "Could not {} tags for {:?}: {} ",
                "Edit the tags in Markdown source instead. ",
                "If the frontmatter is hidden, reveal it from the note toolbar."
            ),
            action,
            note.title,
            error,
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
    recovery_archives: &[PreparedNoteArchive],
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
            kind: TransactionTargetKind::Markdown,
        })
        .collect();
    let mut recovery_targets = Vec::with_capacity(recovery_archives.len());
    for archive in recovery_archives {
        validate_recently_deleted_id(&archive.deleted_note.id)?;
        if fingerprint_bytes(&archive.bytes) != archive.fingerprint {
            return Err("A recovery snapshot changed while the save was being prepared.".to_owned());
        }
        let archived_content_fingerprint =
            fingerprint_bytes(archive.deleted_note.note.content.as_bytes());
        let original = originals
            .iter()
            .find(|original| {
                original.relative_path == archive.deleted_note.note.relative_path
            })
            .ok_or_else(|| {
                "The note being archived was not included in the save transaction.".to_owned()
            })?;
        if original.fingerprint != archived_content_fingerprint {
            return Err(
                "The note changed while its recovery snapshot was being prepared. Try again."
                    .to_owned(),
            );
        }
        let staged = transaction_recovery_snapshot_path(
            &transaction_root,
            &archive.deleted_note.id,
        )?;
        if let Some(parent) = staged.parent() {
            ensure_private_directory_tree(&transaction_root, parent)
                .map_err(|error| format!("Could not prepare a recovery snapshot: {error}"))?;
        }
        atomic_write(&staged, &archive.bytes)
            .map_err(|error| format!("Could not stage a recovery snapshot: {error}"))?;
        recovery_targets.push(TransactionRecoveryTarget {
            id: archive.deleted_note.id.clone(),
            fingerprint: archive.fingerprint.clone(),
        });
    }
    let manifest = TransactionManifest {
        version: TRANSACTION_VERSION,
        id,
        phase: TransactionPhase::Prepared,
        originals,
        targets,
        recovery_targets,
        folder_case_renames,
        created_directories,
    };
    write_transaction_manifest(&transaction_root, &manifest)?;
    Ok((transaction_root, manifest))
}

fn apply_transaction(
    root: &Path,
    transaction_root: &Path,
    manifest: &TransactionManifest,
    plans: &[NoteWritePlan],
    warnings: &mut WarningCollector,
) -> Result<(), String> {
    apply_recovery_targets(root, transaction_root, &manifest.recovery_targets)?;

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
        if let Some(modified_at) = plan.preserved_modified_at {
            if let Err(error) = set_file_modified_millis(&target, modified_at) {
                warnings.push(format!(
                    "The note was restored, but the modified time for {} could not be preserved: \
                     {error}",
                    plan.new_relative_path,
                ));
            }
        }
    }
    Ok(())
}

fn apply_recovery_targets(
    root: &Path,
    transaction_root: &Path,
    targets: &[TransactionRecoveryTarget],
) -> Result<(), String> {
    if targets.is_empty() {
        return Ok(());
    }
    ensure_recently_deleted_directory(root)?;

    for target in targets {
        let bytes = read_staged_recovery_snapshot(transaction_root, target)?;
        let destination = recently_deleted_snapshot_path(root, &target.id)?;
        match fs::symlink_metadata(&destination) {
            Ok(_) => {
                return Err(format!(
                    "A recovery snapshot already exists for {}.",
                    target.id,
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("Could not inspect a recovery snapshot: {error}"));
            }
        }
        atomic_write(&destination, &bytes)
            .map_err(|error| format!("Could not save a recovery snapshot: {error}"))?;
        if fingerprint_regular_file(&destination)? != Some(target.fingerprint.clone()) {
            return Err("A recovery snapshot changed while it was being saved.".to_owned());
        }
    }

    Ok(())
}

fn finalize_committed_recovery_targets(
    root: &Path,
    transaction_root: &Path,
    targets: &[TransactionRecoveryTarget],
) -> Result<(), String> {
    if targets.is_empty() {
        return Ok(());
    }
    ensure_recently_deleted_directory(root)?;

    for target in targets {
        let destination = recently_deleted_snapshot_path(root, &target.id)?;
        match fingerprint_regular_file(&destination)? {
            Some(fingerprint) if fingerprint == target.fingerprint => continue,
            Some(_) => {
                return Err(format!(
                    "Recovery snapshot {} changed before its save was finalized.",
                    target.id,
                ));
            }
            None => {}
        }

        let bytes = read_staged_recovery_snapshot(transaction_root, target)?;
        atomic_write(&destination, &bytes)
            .map_err(|error| format!("Could not finalize a recovery snapshot: {error}"))?;
    }

    Ok(())
}

fn read_staged_recovery_snapshot(
    transaction_root: &Path,
    target: &TransactionRecoveryTarget,
) -> Result<Vec<u8>, String> {
    validate_recently_deleted_id(&target.id)?;
    if target.fingerprint.length > MAX_RECENTLY_DELETED_SNAPSHOT_BYTES {
        return Err("A staged recovery snapshot is unexpectedly large.".to_owned());
    }

    let transaction_metadata = fs::symlink_metadata(transaction_root)
        .map_err(|error| format!("Could not inspect a save transaction: {error}"))?;
    if transaction_metadata.file_type().is_symlink() || !transaction_metadata.is_dir() {
        return Err("The save transaction is not a regular folder.".to_owned());
    }

    let recovery_directory = transaction_root.join("recoveries");
    let recovery_metadata = fs::symlink_metadata(&recovery_directory)
        .map_err(|error| format!("Could not inspect staged recovery snapshots: {error}"))?;
    if recovery_metadata.file_type().is_symlink() || !recovery_metadata.is_dir() {
        return Err("The staged recovery snapshot path is not a regular folder.".to_owned());
    }

    let path = transaction_recovery_snapshot_path(transaction_root, &target.id)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Could not inspect a staged recovery snapshot: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("A staged recovery snapshot is not a regular file.".to_owned());
    }
    if metadata.len() != target.fingerprint.length {
        return Err("A staged recovery snapshot does not match its manifest.".to_owned());
    }

    let file = File::open(&path)
        .map_err(|error| format!("Could not open a staged recovery snapshot: {error}"))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect an open recovery snapshot: {error}"))?;
    if !opened_metadata.is_file() || opened_metadata.len() != target.fingerprint.length {
        return Err("A staged recovery snapshot changed while it was being opened.".to_owned());
    }
    let read_limit = target
        .fingerprint
        .length
        .checked_add(1)
        .ok_or_else(|| "A staged recovery snapshot is too large to read safely.".to_owned())?;
    let mut bytes = Vec::with_capacity(target.fingerprint.length as usize);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read a staged recovery snapshot: {error}"))?;
    if bytes.len() as u64 != target.fingerprint.length {
        return Err("A staged recovery snapshot changed while it was being read.".to_owned());
    }
    if fingerprint_bytes(&bytes) != target.fingerprint {
        return Err("A staged recovery snapshot failed its integrity check.".to_owned());
    }

    Ok(bytes)
}

fn verify_applied_recovery_targets(
    root: &Path,
    targets: &[TransactionRecoveryTarget],
) -> Result<(), String> {
    for target in targets {
        let path = recently_deleted_snapshot_path(root, &target.id)?;
        if fingerprint_regular_file(&path)? != Some(target.fingerprint.clone()) {
            return Err(format!(
                "Recovery snapshot {} changed while the note was being archived.",
                target.id,
            ));
        }
    }

    Ok(())
}

fn rollback_recovery_targets(
    root: &Path,
    targets: &[TransactionRecoveryTarget],
    warnings: &mut WarningCollector,
) -> bool {
    let mut recovered = true;
    for target in targets {
        let path = match recently_deleted_snapshot_path(root, &target.id) {
            Ok(path) => path,
            Err(error) => {
                warnings.push(error);
                recovered = false;
                continue;
            }
        };
        match fingerprint_regular_file(&path) {
            Ok(Some(fingerprint)) if fingerprint == target.fingerprint => {
                if let Err(error) = remove_file_durable(&path) {
                    warnings.push(format!(
                        "Could not remove an uncommitted recovery snapshot: {error}"
                    ));
                    recovered = false;
                }
            }
            Ok(Some(_)) => {
                warnings.push(format!(
                    "Did not remove recovery snapshot {} because it changed after the interrupted save.",
                    target.id,
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

    recovered
}

fn resolve_transaction_target_file(
    root: &Path,
    target: &TransactionTarget,
    allow_missing: bool,
) -> Result<PathBuf, String> {
    match target.kind {
        TransactionTargetKind::Markdown => {
            resolve_workspace_file(root, &target.relative_path, allow_missing)
        }
        TransactionTargetKind::Image => {
            resolve_workspace_image_file(root, &target.relative_path, allow_missing)
        }
        TransactionTargetKind::Attachment => {
            resolve_workspace_asset_file(root, &target.relative_path, allow_missing)
        }
    }
}

fn rollback_transaction(
    root: &Path,
    transaction_root: &Path,
    manifest: &TransactionManifest,
    warnings: &mut WarningCollector,
) -> bool {
    let mut recovered = rollback_recovery_targets(root, &manifest.recovery_targets, warnings);
    for target in manifest.targets.iter().rev() {
        if matches!(
            target.kind,
            TransactionTargetKind::Image | TransactionTargetKind::Attachment
        ) {
            match import_image_was_applied(transaction_root, target) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    warnings.push(error);
                    recovered = false;
                    continue;
                }
            }
        }
        let Ok(path) = resolve_transaction_target_file(root, target, true) else {
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
    recover_workspace_transactions_except(root, state, None, warnings)
}

fn recover_workspace_transactions_except(
    root: &Path,
    state: Option<&WorkspaceState>,
    retained_transaction_id: Option<&str>,
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
        if retained_transaction_id == Some(manifest.id.as_str()) {
            continue;
        }
        let committed = state
            .is_some_and(|state| {
                state.last_committed_transaction_id.as_deref() == Some(manifest.id.as_str())
                    || state.last_committed_image_import_id.as_deref()
                        == Some(manifest.id.as_str())
            });
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
            let recovery_targets = manifest
                .recovery_targets
                .iter()
                .filter(|target| {
                    state
                        .and_then(|state| state.recently_deleted_notes.get(&target.id))
                        .is_some_and(|stored| stored.fingerprint == target.fingerprint)
                })
                .cloned()
                .collect::<Vec<_>>();
            finalize_committed_recovery_targets(
                root,
                &transaction_root,
                &recovery_targets,
            )?;
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
            kind: TransactionTargetKind::Markdown,
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

fn editor_positions_revision(fingerprint: &FileFingerprint) -> String {
    format!("{}:{:016x}", fingerprint.length, fingerprint.hash)
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
            location: ImageEmbedLocation::SpecifiedFolderMirrored,
            folder_path: "Images".to_owned(),
        };
        state.attachment_embed_settings = AttachmentEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolderMirrored,
            folder_path: "Files".to_owned(),
        };
        write_workspace_state(&workspace.root, &state)
            .expect("legacy workspace state should be written");

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
    fn mirrored_image_storage_follows_each_notes_folder() {
        const FIRST: &[u8] = b"\x89PNG\r\n\x1a\nfirst-mirrored-image";
        const SECOND: &[u8] = b"\x89PNG\r\n\x1a\nsecond-mirrored-image";
        let workspace = TestWorkspace::new("mirrored-image-locations");
        fs::create_dir(workspace.root.join("test1")).expect("test1 should be created");
        fs::create_dir(workspace.root.join("test2")).expect("test2 should be created");
        fs::write(workspace.root.join("test1/doc1.md"), "# Doc 1")
            .expect("doc1 should be written");
        fs::write(workspace.root.join("test2/doc2.md"), "# Doc 2")
            .expect("doc2 should be written");
        write_workspace_state(&workspace.root, &WorkspaceState::default())
            .expect("workspace state should be written");
        let settings = ImageEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolderMirrored,
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
        .expect("first mirrored image should be embedded");
        let second = embed_workspace_image(
            &workspace.root,
            "test2/doc2.md",
            settings,
            "Second.png",
            SECOND,
            None,
            first.revision,
        )
        .expect("second mirrored image should be embedded");

        assert_eq!(first.image.relative_path, "Images/test1/First.png");
        assert_eq!(second.image.relative_path, "Images/test2/Second.png");
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
            false,
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
            false,
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
            false,
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
            false,
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
    fn mirrored_images_cannot_be_reorganized() {
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\nmanaged-mirror-image";
        let workspace = TestWorkspace::new("managed-mirror-image");
        fs::create_dir_all(workspace.root.join("Images/Notes"))
            .expect("mirrored image folder should be created");
        fs::create_dir(workspace.root.join("Elsewhere"))
            .expect("destination should be created");
        fs::write(workspace.root.join("Images/Notes/Photo.png"), PNG)
            .expect("managed image should be written");
        let mut state = WorkspaceState::default();
        state.image_embed_settings = ImageEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolderMirrored,
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
            "Elsewhere/Photo.png",
            "image-managed",
            &[],
            revision_for_root(&workspace.root).expect("revision should be available"),
            false,
        )
        .expect_err("mirrored images should be managed by note location");

        assert!(error.contains("mirrored image folder"));
        assert!(workspace.root.join("Images/Notes/Photo.png").exists());
        assert!(!workspace.root.join("Elsewhere/Photo.png").exists());
    }

    #[test]
    fn note_moves_carry_mirrored_images_without_allowing_renames() {
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\nmanaged-note-move-image";
        let workspace = TestWorkspace::new("managed-note-move-image");
        fs::create_dir_all(workspace.root.join("Images/Notes"))
            .expect("source mirror folder should be created");
        fs::write(workspace.root.join("Images/Notes/Photo.png"), PNG)
            .expect("managed image should be written");
        let mut state = WorkspaceState::default();
        state.image_embed_settings = ImageEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolderMirrored,
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

        let renamed_error = relocate_workspace_image(
            &workspace.root,
            "Images/Notes/Photo.png",
            "Images/Archive/Renamed.png",
            "image-managed",
            &[],
            revision_for_root(&workspace.root).expect("revision should be available"),
            true,
        )
        .expect_err("a managed note move must not rename its image");
        assert!(renamed_error.contains("without renaming"));

        let moved = relocate_workspace_image(
            &workspace.root,
            "Images/Notes/Photo.png",
            "Images/Archive/Photo.png",
            "image-managed",
            &[],
            revision_for_root(&workspace.root).expect("revision should be available"),
            true,
        )
        .expect("a note move should carry its managed image");

        assert_eq!(moved.image.relative_path, "Images/Archive/Photo.png");
        assert!(!workspace.root.join("Images/Notes/Photo.png").exists());
        assert_eq!(
            fs::read(workspace.root.join("Images/Archive/Photo.png")).unwrap(),
            PNG,
        );
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
    fn attachment_storage_honors_mirrored_locations_and_empty_files() {
        let source = TestWorkspace::new("mirrored-attachment-source");
        let workspace = TestWorkspace::new("mirrored-attachment-target");
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
            location: ImageEmbedLocation::SpecifiedFolderMirrored,
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
        .expect("first mirrored attachment should be embedded");
        let second = embed_workspace_attachment(
            &workspace.root,
            "test2/doc2.md",
            settings.clone(),
            &empty_source,
            None,
            first.revision,
        )
        .expect("empty extensionless attachment should be embedded");

        assert_eq!(first.attachment.relative_path, "Files/test1/First.zip");
        assert_eq!(second.attachment.relative_path, "Files/test2/Empty export");
        assert_eq!(second.attachment.byte_length, 0);
        assert_eq!(second.attachment.media_type, "application/octet-stream");
        assert_eq!(fs::read(workspace.root.join(&second.attachment.relative_path)).unwrap(), b"");
        let loaded = load_workspace(&workspace.root, &empty_vault("Attachments"))
            .expect("mirrored attachments should reload");
        assert_eq!(
            loaded.vault.attachment_embed_settings,
            AttachmentEmbedSettings {
                location: ImageEmbedLocation::SpecifiedFolder,
                folder_path: settings.folder_path,
            },
        );
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
            false,
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
    fn mirrored_attachments_only_move_with_their_note() {
        let workspace = TestWorkspace::new("managed-note-move-attachment");
        let bytes = b"managed attachment";
        fs::create_dir_all(workspace.root.join("Files/Notes"))
            .expect("source mirror folder should be created");
        fs::create_dir(workspace.root.join("Elsewhere"))
            .expect("ordinary destination should be created");
        fs::write(workspace.root.join("Files/Notes/Report.pdf"), bytes)
            .expect("managed attachment should be written");
        let mut state = WorkspaceState::default();
        state.attachment_embed_settings = AttachmentEmbedSettings {
            location: ImageEmbedLocation::SpecifiedFolderMirrored,
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
        write_workspace_state(&workspace.root, &state)
            .expect("workspace state should be written");

        let manual_error = relocate_workspace_attachment(
            &workspace.root,
            "Files/Notes/Report.pdf",
            "Elsewhere/Report.pdf",
            "attachment-managed",
            &[],
            revision_for_root(&workspace.root).unwrap(),
            false,
        )
        .expect_err("mirrored attachments should reject manual moves");
        assert!(manual_error.contains("mirrored attachment folder"));

        let rename_error = relocate_workspace_attachment(
            &workspace.root,
            "Files/Notes/Report.pdf",
            "Files/Archive/Renamed.pdf",
            "attachment-managed",
            &[],
            revision_for_root(&workspace.root).unwrap(),
            true,
        )
        .expect_err("managed note moves must preserve the attachment name");
        assert!(rename_error.contains("without renaming"));

        let moved = relocate_workspace_attachment(
            &workspace.root,
            "Files/Notes/Report.pdf",
            "Files/Archive/Report.pdf",
            "attachment-managed",
            &[],
            revision_for_root(&workspace.root).unwrap(),
            true,
        )
        .expect("a note move should carry its managed attachment");
        assert_eq!(moved.attachment.relative_path, "Files/Archive/Report.pdf");
        assert_eq!(fs::read(workspace.root.join("Files/Archive/Report.pdf")).unwrap(), bytes);
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
