use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{AppHandle, State};

#[cfg(not(target_os = "macos"))]
use crate::paths::sidecars_dir;
use crate::{
    paths::{is_existing_file, locate_binary, path_to_string, whisper_models_dir},
    state::AppState,
};

mod download;
mod events;
mod install;
#[cfg(target_os = "macos")]
mod source_build;

use download::download_dependency_archive;
use download::{download_file_with_resume, download_message};
use events::{
    emit_dependency_install, emit_model_download, emit_model_download_with_metrics, format_bytes,
    DownloadMetrics,
};
pub(crate) use events::{DependencyInstallEvent, DownloadStatus, ModelDownloadEvent};
#[cfg(not(target_os = "macos"))]
use install::{ensure_executable, extract_dependency_archive};
#[cfg(target_os = "macos")]
use source_build::{install_ffmpeg_from_official_source, install_whisper_cpp_from_official_source};

#[cfg(not(target_os = "macos"))]
const FFMPEG_DOWNLOAD_URL: &str =
    "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";
#[cfg(target_os = "macos")]
const FFMPEG_SOURCE_URL: &str = "https://ffmpeg.org/releases/ffmpeg-8.1.1.tar.xz";
#[cfg(target_os = "macos")]
const FFMPEG_SOURCE_ARCHIVE_NAME: &str = "ffmpeg-8.1.1.tar.xz";
#[cfg(target_os = "macos")]
const MACOS_ARM64_DEPLOYMENT_TARGET: &str = "11.0";
const WHISPER_RELEASE_API_URL: &str =
    "https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest";
const WHISPER_VAD_MODEL_FILE_NAME: &str = "ggml-silero-v6.2.0.bin";
const WHISPER_VAD_MODEL_URL: &str =
    "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v6.2.0.bin";
const HTTP_USER_AGENT: &str = "Luma Subtitle dependency installer";
const DOWNLOAD_MAX_ATTEMPTS: usize = 4;
#[cfg(not(target_os = "macos"))]
const WHISPER_CPP_ASSET_CANDIDATES: &[&str] = &[
    "whisper-cublas-12.4.0-bin-x64.zip",
    "whisper-cublas-11.8.0-bin-x64.zip",
    "whisper-bin-x64.zip",
];
const WHISPER_MODEL_PRESETS: &[WhisperModelPreset] = &[
    WhisperModelPreset {
        id: "tiny",
        file_name: "ggml-tiny.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
    },
    WhisperModelPreset {
        id: "base",
        file_name: "ggml-base.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
    },
    WhisperModelPreset {
        id: "small",
        file_name: "ggml-small.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
    },
    WhisperModelPreset {
        id: "large-v3-turbo-q5_0",
        file_name: "ggml-large-v3-turbo-q5_0.bin",
        url:
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
    },
];
#[derive(Clone, Copy)]
struct WhisperModelPreset {
    id: &'static str,
    file_name: &'static str,
    url: &'static str,
}

#[derive(Deserialize)]
pub(crate) struct DownloadWhisperModelRequest {
    preset_id: String,
}
#[cfg(not(target_os = "macos"))]
#[derive(Deserialize)]
struct GithubRelease {
    assets: Vec<GithubAsset>,
}
#[cfg(not(target_os = "macos"))]
#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[tauri::command]
pub(crate) fn download_status(state: State<'_, AppState>) -> DownloadStatus {
    DownloadStatus {
        model: state.model_download.lock().clone(),
        dependency: state.dependency_install.lock().clone(),
    }
}
#[tauri::command]
pub(crate) async fn install_dependencies(app: AppHandle) -> Result<Vec<String>, String> {
    let mut installed = Vec::new();
    installed.push(install_ffmpeg(&app).await?);
    installed.push(install_whisper_cpp(&app).await?);
    installed.push(path_to_string(ensure_whisper_vad_model(&app).await?));
    Ok(installed)
}

pub(crate) async fn ensure_whisper_vad_model(app: &AppHandle) -> Result<PathBuf, String> {
    let models_dir = whisper_models_dir(app)?;
    tokio::fs::create_dir_all(&models_dir)
        .await
        .map_err(|error| format!("创建 VAD 模型目录失败: {error}"))?;
    let model_path = models_dir.join(WHISPER_VAD_MODEL_FILE_NAME);
    if is_existing_file(&model_path) {
        emit_dependency_install(
            app,
            "Silero VAD",
            "completed",
            "VAD 模型已可用",
            1.0,
            Some(path_to_string(model_path.clone())),
            None,
        );
        return Ok(model_path);
    }

    let partial_path = models_dir.join(format!("{WHISPER_VAD_MODEL_FILE_NAME}.part"));
    let _ = tokio::fs::remove_file(&partial_path).await;
    download_dependency_archive(app, "Silero VAD", WHISPER_VAD_MODEL_URL, &partial_path).await?;
    if model_path.exists() {
        tokio::fs::remove_file(&model_path)
            .await
            .map_err(|error| format!("替换旧 VAD 模型失败: {error}"))?;
    }
    tokio::fs::rename(&partial_path, &model_path)
        .await
        .map_err(|error| format!("保存 VAD 模型失败: {error}"))?;
    emit_dependency_install(
        app,
        "Silero VAD",
        "completed",
        "VAD 模型已安装",
        1.0,
        Some(path_to_string(model_path.clone())),
        None,
    );
    Ok(model_path)
}

pub(crate) fn downloaded_whisper_model_files(app: &AppHandle) -> Vec<String> {
    let Ok(models_dir) = whisper_models_dir(app) else {
        return Vec::new();
    };
    WHISPER_MODEL_PRESETS
        .iter()
        .filter(|preset| is_existing_file(&models_dir.join(preset.file_name)))
        .map(|preset| preset.file_name.to_string())
        .collect()
}

#[tauri::command]
pub(crate) async fn download_whisper_model(
    app: AppHandle,
    request: DownloadWhisperModelRequest,
) -> Result<String, String> {
    let preset = find_whisper_model_preset(&request.preset_id)
        .ok_or_else(|| "未知 Whisper 模型预设".to_string())?;
    let models_dir = whisper_models_dir(&app)?;
    tokio::fs::create_dir_all(&models_dir)
        .await
        .map_err(|error| format!("创建模型目录失败: {error}"))?;
    let model_path = models_dir.join(preset.file_name);
    if is_existing_file(&model_path) {
        let path = path_to_string(model_path);
        emit_model_download(
            &app,
            preset,
            "completed",
            "模型已存在",
            1.0,
            Some(path.clone()),
            None,
        );
        return Ok(path);
    }
    let partial_path = models_dir.join(format!("{}.part", preset.file_name));
    let _ = tokio::fs::remove_file(&partial_path).await;
    emit_model_download(&app, preset, "running", "开始下载模型", 0.0, None, None);
    let result = download_whisper_model_to_path(&app, preset, &partial_path).await;
    match result {
        Ok(()) => {
            if model_path.exists() {
                tokio::fs::remove_file(&model_path)
                    .await
                    .map_err(|error| format!("替换旧模型失败: {error}"))?;
            }
            tokio::fs::rename(&partial_path, &model_path)
                .await
                .map_err(|error| format!("保存模型失败: {error}"))?;
            let path = path_to_string(model_path);
            emit_model_download(
                &app,
                preset,
                "completed",
                "模型已下载",
                1.0,
                Some(path.clone()),
                None,
            );
            Ok(path)
        }
        Err(message) => {
            emit_model_download(
                &app,
                preset,
                "failed",
                "模型下载失败",
                0.0,
                None,
                Some(message.clone()),
            );
            Err(message)
        }
    }
}

async fn download_whisper_model_to_path(
    app: &AppHandle,
    preset: WhisperModelPreset,
    partial_path: &Path,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30 * 60))
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|error| format!("创建下载客户端失败: {error}"))?;
    download_file_with_resume(
        &client,
        preset.url,
        partial_path,
        1.0,
        |update| {
            let metrics = update.metrics;
            let message = download_message("模型", update);
            emit_model_download_with_metrics(
                app,
                preset,
                "running",
                message,
                update.progress,
                None,
                None,
                metrics,
            );
        },
        |attempt, error, downloaded| {
            emit_model_download_with_metrics(
                app,
                preset,
                "running",
                format!(
                    "下载中断，保留 {}，正在重试 {}/{}: {}",
                    format_bytes(downloaded),
                    attempt,
                    DOWNLOAD_MAX_ATTEMPTS,
                    error
                ),
                0.0,
                None,
                None,
                DownloadMetrics {
                    downloaded_bytes: Some(downloaded),
                    ..DownloadMetrics::default()
                },
            );
        },
    )
    .await
}
async fn install_ffmpeg(app: &AppHandle) -> Result<String, String> {
    if let Some(path) = locate_binary(app, "ffmpeg") {
        let path = path_to_string(path);
        emit_dependency_install(
            app,
            "ffmpeg",
            "completed",
            "FFmpeg 已可用",
            1.0,
            Some(path.clone()),
            None,
        );
        return Ok(path);
    }
    #[cfg(target_os = "macos")]
    {
        install_ffmpeg_from_official_source(app).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let sidecars_dir = sidecars_dir(app)?;
        let downloads_dir = sidecars_dir.join("downloads");
        tokio::fs::create_dir_all(&downloads_dir)
            .await
            .map_err(|error| format!("创建下载目录失败: {error}"))?;
        let archive_path = downloads_dir.join("ffmpeg-release-essentials.zip");
        download_dependency_archive(app, "ffmpeg", FFMPEG_DOWNLOAD_URL, &archive_path).await?;
        let path = extract_dependency_archive(
            "ffmpeg",
            "ffmpeg.exe",
            app,
            &archive_path,
            &sidecars_dir.join("ffmpeg"),
        )
        .await?;
        let _ = tokio::fs::remove_file(&archive_path).await;
        emit_dependency_install(
            app,
            "ffmpeg",
            "completed",
            "FFmpeg 已安装",
            1.0,
            Some(path.clone()),
            None,
        );
        Ok(path)
    }
}
async fn install_whisper_cpp(app: &AppHandle) -> Result<String, String> {
    if let Some(path) = locate_binary(app, "whisper-cli") {
        let path = path_to_string(path);
        emit_dependency_install(
            app,
            "whisper.cpp",
            "completed",
            "whisper.cpp 已可用",
            1.0,
            Some(path.clone()),
            None,
        );
        return Ok(path);
    }
    #[cfg(target_os = "macos")]
    {
        install_whisper_cpp_from_official_source(app).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        emit_dependency_install(
            app,
            "whisper.cpp",
            "running",
            "正在查询 whisper.cpp 发布包",
            0.0,
            None,
            None,
        );
        let asset = latest_whisper_cpp_asset().await?;
        let sidecars_dir = sidecars_dir(app)?;
        let downloads_dir = sidecars_dir.join("downloads");
        tokio::fs::create_dir_all(&downloads_dir)
            .await
            .map_err(|error| format!("创建下载目录失败: {error}"))?;
        let archive_path = downloads_dir.join(&asset.name);
        download_dependency_archive(
            app,
            "whisper.cpp",
            &asset.browser_download_url,
            &archive_path,
        )
        .await?;
        let path = extract_dependency_archive(
            "whisper.cpp",
            "whisper-cli.exe",
            app,
            &archive_path,
            &sidecars_dir.join("whisper.cpp"),
        )
        .await?;
        let _ = tokio::fs::remove_file(&archive_path).await;
        emit_dependency_install(
            app,
            "whisper.cpp",
            "completed",
            "whisper.cpp 已安装",
            1.0,
            Some(path.clone()),
            None,
        );
        Ok(path)
    }
}

#[cfg(not(target_os = "macos"))]
async fn latest_whisper_cpp_asset() -> Result<GithubAsset, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|error| format!("创建 GitHub 客户端失败: {error}"))?;
    let release = client
        .get(WHISPER_RELEASE_API_URL)
        .send()
        .await
        .map_err(|error| format!("查询 whisper.cpp 发布包失败: {error}"))?
        .error_for_status()
        .map_err(|error| format!("查询 whisper.cpp 发布包失败: {error}"))?
        .json::<GithubRelease>()
        .await
        .map_err(|error| format!("解析 whisper.cpp 发布包失败: {error}"))?;
    WHISPER_CPP_ASSET_CANDIDATES
        .iter()
        .find_map(|name| {
            release
                .assets
                .iter()
                .find(|asset| asset.name == *name)
                .map(|asset| GithubAsset {
                    name: asset.name.clone(),
                    browser_download_url: asset.browser_download_url.clone(),
                })
        })
        .ok_or_else(|| "未找到可用的 whisper.cpp Windows x64 发布包".to_string())
}
fn find_whisper_model_preset(id: &str) -> Option<WhisperModelPreset> {
    WHISPER_MODEL_PRESETS
        .iter()
        .copied()
        .find(|preset| preset.id == id)
}
