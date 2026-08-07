use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemFont {
    family: String,
    monospaced: bool,
}

#[tauri::command]
pub async fn list_system_fonts() -> Result<Vec<SystemFont>, String> {
    tauri::async_runtime::spawn_blocking(scan_system_fonts)
        .await
        .map_err(|error| error.to_string())
}

fn scan_system_fonts() -> Vec<SystemFont> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();

    let mut families: BTreeMap<String, SystemFont> = BTreeMap::new();

    for face in database.faces() {
        let Some((family, _language)) = face.families.first() else {
            continue;
        };
        let family = family.trim();

        if family.is_empty() || family.len() > 256 || family.contains('\0') {
            continue;
        }

        let normalized = family.to_lowercase();
        let entry = families.entry(normalized).or_insert_with(|| SystemFont {
            family: family.to_owned(),
            monospaced: false,
        });

        entry.monospaced |= face.monospaced;
    }

    families.into_values().collect()
}
