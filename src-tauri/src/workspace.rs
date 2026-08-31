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
pub(crate) mod commands;
mod filesystem;
mod persistence;
mod registry;
mod revision;

#[cfg(test)]
mod tests;

pub(crate) use assets::files::attachments::copy_attachment_file_for_transfer_impl as copy_attachment_file_for_transfer;
pub(crate) use assets::files::images::{
    is_supported_image_path_impl as is_supported_image_path,
    validate_image_bytes_impl as validate_image_bytes,
};
use assets::*;
pub use commands::*;
use filesystem::*;
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
