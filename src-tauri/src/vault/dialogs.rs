use super::*;

/// Opens the system's native directory picker and returns a normal filesystem path.
#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let selected = app.dialog().file().blocking_pick_folder();
    selected
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| {
                    format!("The selected folder is not a local filesystem path: {error}")
                })
        })
        .transpose()
}

/// Opens the native picker for a raster image that can be copied into a vault.
#[tauri::command]
pub async fn pick_image_file(app: AppHandle) -> Result<Option<String>, String> {
    let selected = app
        .dialog()
        .file()
        .add_filter(
            "Images",
            &["png", "jpg", "jpeg", "gif", "webp", "bmp", "avif"],
        )
        .blocking_pick_file();
    selected
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| {
                    format!("The selected image is not a local filesystem path: {error}")
                })
        })
        .transpose()
}

/// Opens the native picker for a regular file that can be copied into a vault.
#[tauri::command]
pub async fn pick_attachment_file(app: AppHandle) -> Result<Option<String>, String> {
    let selected = app.dialog().file().blocking_pick_file();
    selected
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| {
                    format!("The selected attachment is not a local filesystem path: {error}")
                })
        })
        .transpose()
}
