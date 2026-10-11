use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{
    dependencies::{
        downloaded_translation_model_files, downloaded_whisper_model_files, llama_backend_label,
    },
    paths::{display_path_to_string, locate_binary, managed_dir},
};

#[derive(Serialize)]
pub(crate) struct EnvironmentResponse {
    ffmpeg_path: Option<String>,
    whisper_path: Option<String>,
    llama_path: Option<String>,
    llama_backend: Option<String>,
    gpu_name: Option<String>,
    cuda_driver: Option<String>,
    resource_dir: String,
    config_dir: String,
    sidecar_dir: String,
    model_dir: String,
    downloaded_model_files: Vec<String>,
    downloaded_translation_model_files: Vec<String>,
}

#[tauri::command]
pub(crate) async fn check_environment(app: AppHandle) -> Result<EnvironmentResponse, String> {
    let gpu = gpu_info().await;
    tauri::async_runtime::spawn_blocking(move || check_environment_inner(app, gpu))
        .await
        .map_err(|error| format!("检查环境任务失败: {error}"))
}

fn check_environment_inner(
    app: AppHandle,
    (gpu_name, cuda_driver): (Option<String>, Option<String>),
) -> EnvironmentResponse {
    let llama_path = locate_binary(&app, "llama-server");
    let llama_backend = llama_path
        .as_ref()
        .and_then(|path| llama_backend_label(path));
    EnvironmentResponse {
        ffmpeg_path: locate_binary(&app, "ffmpeg").map(display_path_to_string),
        whisper_path: locate_binary(&app, "whisper-cli").map(display_path_to_string),
        llama_path: llama_path.map(display_path_to_string),
        llama_backend,
        gpu_name,
        cuda_driver,
        resource_dir: app
            .path()
            .resource_dir()
            .map(display_path_to_string)
            .unwrap_or_else(|_| "开发模式资源目录尚未生成".to_string()),
        config_dir: app
            .path()
            .app_config_dir()
            .map(display_path_to_string)
            .unwrap_or_else(|_| "配置目录不可用".to_string()),
        sidecar_dir: managed_dir(&app, "sidecars")
            .map(display_path_to_string)
            .unwrap_or_else(|_| "依赖目录不可用".to_string()),
        model_dir: managed_dir(&app, "models")
            .map(display_path_to_string)
            .unwrap_or_else(|_| "模型目录不可用".to_string()),
        downloaded_model_files: downloaded_whisper_model_files(&app),
        downloaded_translation_model_files: downloaded_translation_model_files(&app),
    }
}

#[cfg(target_os = "macos")]
async fn gpu_info() -> (Option<String>, Option<String>) {
    if std::env::consts::ARCH != "aarch64" {
        return (Some("Intel Mac".to_string()), None);
    }
    let mut command = std::process::Command::new("sysctl");
    command.args(["-n", "machdep.cpu.brand_string"]);
    let chip = crate::owned_process::output(
        command,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        std::time::Duration::from_secs(3),
        16 * 1024,
    )
    .await
    .ok()
    .filter(|output| output.status.success())
    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    .filter(|value| !value.is_empty())
    .unwrap_or_else(|| "Apple Silicon".to_string());
    // A chip name is not evidence that the selected whisper binary uses Metal.
    // Actual backend selection is reported from each transcription process.
    (Some(chip), None)
}

#[cfg(not(target_os = "macos"))]
async fn gpu_info() -> (Option<String>, Option<String>) {
    match crate::process_utils::probe_nvidia_gpu().await {
        Some(gpu) => (Some(gpu.name), gpu.driver_version),
        None => (None, None),
    }
}
