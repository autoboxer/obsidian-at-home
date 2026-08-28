mod appearance;
mod vault;
mod workspace;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            appearance::list_system_fonts,
            vault::pick_folder,
            vault::pick_image_file,
            vault::pick_attachment_file,
            vault::import_obsidian_vault,
            vault::export_obsidian_vault,
            workspace::workspace_bootstrap,
            workspace::workspace_open,
            workspace::workspace_create,
            workspace::workspace_save,
            workspace::workspace_save_with_image_import,
            workspace::workspace_archive_note,
            workspace::workspace_restore_recently_deleted_note,
            workspace::workspace_delete_recently_deleted_notes,
            workspace::workspace_prune_recently_deleted_notes,
            workspace::workspace_save_editor_positions,
            workspace::workspace_forget,
            workspace::workspace_revision,
            workspace::workspace_embed_image_file,
            workspace::workspace_embed_vault_image,
            workspace::workspace_relocate_image,
            workspace::workspace_relocate_attachment,
            workspace::workspace_open_attachment,
            workspace::workspace_save_attachment_copy,
            workspace::workspace_embed_image_bytes,
            workspace::workspace_embed_attachment_file,
            workspace::workspace_embed_vault_attachment,
            workspace::workspace_read_image,
            workspace::workspace_import_images,
        ])
        .run(tauri::generate_context!())
        .expect("Obsidian At Home failed to start");
}
