use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct WorkspaceRegistry {
    #[serde(default = "registry_version")]
    pub(super) version: u32,
    #[serde(default)]
    pub(super) active_path: Option<String>,
    #[serde(default)]
    pub(super) recent_vaults: Vec<VaultDescriptor>,
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

pub(super) fn read_registry(app: &AppHandle) -> Result<WorkspaceRegistry, String> {
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

pub(super) fn unique_registry_backup_path(path: &Path) -> PathBuf {
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

pub(super) fn write_registry(app: &AppHandle, registry: &WorkspaceRegistry) -> Result<(), String> {
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

pub(super) fn remember_workspace(registry: &mut WorkspaceRegistry, descriptor: &VaultDescriptor) {
    let descriptor_path = canonical_path_if_available(&descriptor.path);
    registry
        .recent_vaults
        .retain(|existing| canonical_path_if_available(&existing.path) != descriptor_path);
    registry.recent_vaults.push(descriptor.clone());
    registry.active_path = Some(descriptor.path.clone());
    sort_descriptors(&mut registry.recent_vaults);
    registry.recent_vaults.truncate(50);
}

pub(super) fn sort_descriptors(descriptors: &mut [VaultDescriptor]) {
    descriptors.sort_by(|left, right| {
        right
            .last_opened_at
            .cmp(&left.last_opened_at)
            .then_with(|| left.name.cmp(&right.name))
    });
}

pub(super) fn reverse_valid_paths(
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
            warnings.push(format!(
                "Ignored a duplicate {kind} path mapping for {path}."
            ));
        }
    }
    result
}
