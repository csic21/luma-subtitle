use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

use super::WhisperModelPreset;

#[derive(Clone, Serialize)]
pub(crate) struct DependencyInstallEvent {
    item: String,
    status: String,
    message: String,
    progress: f32,
    path: Option<String>,
    error: Option<String>,
    bytes_per_second: Option<f64>,
    eta_seconds: Option<u64>,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
}

#[derive(Clone, Serialize)]
pub(crate) struct ModelDownloadEvent {
    preset_id: String,
    file_name: String,
    status: String,
    message: String,
    progress: f32,
    path: Option<String>,
    error: Option<String>,
    bytes_per_second: Option<f64>,
    eta_seconds: Option<u64>,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
}

#[derive(Serialize)]
pub(crate) struct DownloadStatus {
    pub(crate) model: Option<ModelDownloadEvent>,
    pub(crate) dependency: Option<DependencyInstallEvent>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct DownloadMetrics {
    pub(super) bytes_per_second: Option<f64>,
    pub(super) eta_seconds: Option<u64>,
    pub(super) downloaded_bytes: Option<u64>,
    pub(super) total_bytes: Option<u64>,
}

#[derive(Clone, Copy)]
pub(super) struct DownloadUpdate {
    pub(super) progress: f32,
    pub(super) metrics: DownloadMetrics,
    pub(super) attempt: usize,
    pub(super) resumed: bool,
}

pub(super) fn emit_dependency_install(
    app: &AppHandle,
    item: impl Into<String>,
    status: impl Into<String>,
    message: impl Into<String>,
    progress: f32,
    path: Option<String>,
    error: Option<String>,
) {
    emit_dependency_install_with_metrics(
        app,
        item,
        status,
        message,
        progress,
        path,
        error,
        DownloadMetrics::default(),
    );
}

pub(super) fn emit_dependency_install_with_metrics(
    app: &AppHandle,
    item: impl Into<String>,
    status: impl Into<String>,
    message: impl Into<String>,
    progress: f32,
    path: Option<String>,
    error: Option<String>,
    metrics: DownloadMetrics,
) {
    let event = DependencyInstallEvent {
        item: item.into(),
        status: status.into(),
        message: message.into(),
        progress,
        path,
        error,
        bytes_per_second: metrics.bytes_per_second,
        eta_seconds: metrics.eta_seconds,
        downloaded_bytes: metrics.downloaded_bytes,
        total_bytes: metrics.total_bytes,
    };
    *app.state::<AppState>().dependency_install.lock() = Some(event.clone());
    let _ = app.emit("dependency-install-event", event);
}

pub(super) fn emit_model_download(
    app: &AppHandle,
    preset: WhisperModelPreset,
    status: impl Into<String>,
    message: impl Into<String>,
    progress: f32,
    path: Option<String>,
    error: Option<String>,
) {
    emit_model_download_with_metrics(
        app,
        preset,
        status,
        message,
        progress,
        path,
        error,
        DownloadMetrics::default(),
    );
}

pub(super) fn emit_model_download_with_metrics(
    app: &AppHandle,
    preset: WhisperModelPreset,
    status: impl Into<String>,
    message: impl Into<String>,
    progress: f32,
    path: Option<String>,
    error: Option<String>,
    metrics: DownloadMetrics,
) {
    let event = ModelDownloadEvent {
        preset_id: preset.id.to_string(),
        file_name: preset.file_name.to_string(),
        status: status.into(),
        message: message.into(),
        progress,
        path,
        error,
        bytes_per_second: metrics.bytes_per_second,
        eta_seconds: metrics.eta_seconds,
        downloaded_bytes: metrics.downloaded_bytes,
        total_bytes: metrics.total_bytes,
    };
    *app.state::<AppState>().model_download.lock() = Some(event.clone());
    let _ = app.emit("model-download-event", event);
}

pub(super) fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes < 1024 * 1024 {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB)
    }
}
