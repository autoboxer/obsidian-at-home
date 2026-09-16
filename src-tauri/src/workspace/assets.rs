use super::*;

mod deletion;
mod embedding;
mod external_upload;
pub(crate) mod files;
mod imports;
mod inventory;
mod relocation;

pub(in crate::workspace) use deletion::*;
pub(in crate::workspace) use embedding::*;
pub(in crate::workspace) use external_upload::*;
pub(in crate::workspace) use files::*;
pub(in crate::workspace) use imports::*;
pub(in crate::workspace) use inventory::*;
pub(in crate::workspace) use relocation::*;

pub(in crate::workspace) const MAX_VAULT_ASSETS: usize = 100_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(in crate::workspace) enum VaultAssetKind {
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
pub(in crate::workspace) struct StoredVaultAsset {
    #[serde(default)]
    pub(in crate::workspace) kind: VaultAssetKind,
    pub(in crate::workspace) relative_path: String,
    pub(in crate::workspace) media_type: String,
    pub(in crate::workspace) fingerprint: FileFingerprint,
    #[serde(default)]
    pub(in crate::workspace) modified_nanos: u64,
}

#[derive(Debug)]
pub(in crate::workspace) struct PreparedAssetNoteUpdate {
    path: PathBuf,
    expected_content: Vec<u8>,
    content: Vec<u8>,
}

pub(in crate::workspace) fn prepare_asset_note_updates(
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
            return Err(format!(
                "{} is not a regular Markdown file.",
                update.relative_path
            ));
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

pub(in crate::workspace) fn workspace_asset_limit_reached(
    image_count: usize,
    attachment_count: usize,
    limit: usize,
) -> bool {
    image_count.saturating_add(attachment_count) >= limit
}
