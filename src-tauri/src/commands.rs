use crate::paths::path_to_string;

#[tauri::command]
pub(crate) async fn select_video() -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter("Video", &["mp4", "mkv", "mov", "avi", "webm", "m4v"])
            .pick_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(picked.map(path_to_string))
}
#[tauri::command]
pub(crate) async fn select_output_dir() -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(|| rfd::FileDialog::new().pick_folder())
        .await
        .map_err(|error| error.to_string())?;
    Ok(picked.map(path_to_string))
}
#[tauri::command]
pub(crate) async fn select_whisper_model() -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter("whisper.cpp model", &["bin", "gguf"])
            .pick_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(picked.map(path_to_string))
}

#[tauri::command]
pub(crate) async fn select_srt() -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter("SubRip subtitle", &["srt"])
            .pick_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(picked.map(path_to_string))
}

#[tauri::command]
pub(crate) fn open_path(path: String) -> Result<(), String> {
    opener::open(path).map_err(|error| error.to_string())
}
