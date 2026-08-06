mod vault;
mod workspace;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            vault::pick_folder,
            vault::import_obsidian_vault,
            vault::export_obsidian_vault,
            workspace::workspace_bootstrap,
            workspace::workspace_open,
            workspace::workspace_create,
            workspace::workspace_save,
            workspace::workspace_forget,
            workspace::workspace_revision,
        ])
        .run(tauri::generate_context!())
        .expect("Obsidian At Home failed to start");
}
