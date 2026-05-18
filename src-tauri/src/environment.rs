use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{
    dependencies::downloaded_whisper_model_files,
    paths::{locate_binary, managed_dir, path_to_string},
};

#[derive(Serialize)]
pub(crate) struct EnvironmentResponse {
    ffmpeg_path: Option<String>,
    whisper_path: Option<String>,
    gpu_name: Option<String>,
    cuda_driver: Option<String>,
    resource_dir: String,
    config_dir: String,
    sidecar_dir: String,
    model_dir: String,
    downloaded_model_files: Vec<String>,
}

#[tauri::command]
pub(crate) fn check_environment(app: AppHandle) -> EnvironmentResponse {
    let (gpu_name, cuda_driver) = gpu_info();
    EnvironmentResponse {
        ffmpeg_path: locate_binary(&app, "ffmpeg").map(path_to_string),
        whisper_path: locate_binary(&app, "whisper-cli").map(path_to_string),
        gpu_name,
        cuda_driver,
        resource_dir: app
            .path()
            .resource_dir()
            .map(path_to_string)
            .unwrap_or_else(|_| "开发模式资源目录尚未生成".to_string()),
        config_dir: app
            .path()
            .app_config_dir()
            .map(path_to_string)
            .unwrap_or_else(|_| "配置目录不可用".to_string()),
        sidecar_dir: managed_dir(&app, "sidecars")
            .map(path_to_string)
            .unwrap_or_else(|_| "依赖目录不可用".to_string()),
        model_dir: managed_dir(&app, "models")
            .map(path_to_string)
            .unwrap_or_else(|_| "模型目录不可用".to_string()),
        downloaded_model_files: downloaded_whisper_model_files(&app),
    }
}

#[cfg(target_os = "macos")]
fn gpu_info() -> (Option<String>, Option<String>) {
    if std::env::consts::ARCH != "aarch64" {
        return (
            Some("Intel Mac".to_string()),
            Some("Metal 未启用".to_string()),
        );
    }
    let chip = std::process::Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Apple Silicon".to_string());
    (Some(chip), Some("Metal".to_string()))
}

#[cfg(not(target_os = "macos"))]
fn gpu_info() -> (Option<String>, Option<String>) {
    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,driver_version",
            "--format=csv,noheader,nounits",
        ])
        .output();
    let Ok(output) = output else {
        return (None, None);
    };
    if !output.status.success() {
        return (None, None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut fields = stdout.lines().next().unwrap_or_default().split(',');
    let name = fields
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let driver = fields
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    (name.map(str::to_string), driver.map(str::to_string))
}
