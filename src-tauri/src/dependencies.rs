use reqwest::{header::RANGE, StatusCode};
use serde::{Deserialize, Serialize};
#[cfg(not(target_os = "macos"))]
use std::{fs, io};
use std::{
    path::Path,
    time::{Duration, Instant},
};
#[cfg(target_os = "macos")]
use std::{path::PathBuf, process::Stdio};
use tauri::{AppHandle, Emitter, Manager, State};
#[cfg(target_os = "macos")]
use tokio::process::Command;
use tokio::{io::AsyncWriteExt, time::sleep};

use crate::{
    paths::{
        find_file_recursive, is_existing_file, locate_binary, path_to_string, sidecars_dir,
        whisper_models_dir,
    },
    state::AppState,
};

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
#[derive(Deserialize)]
pub(crate) struct DownloadWhisperModelRequest {
    preset_id: String,
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
    model: Option<ModelDownloadEvent>,
    dependency: Option<DependencyInstallEvent>,
}
#[derive(Deserialize)]
struct GithubRelease {
    #[cfg(target_os = "macos")]
    tag_name: String,
    #[cfg(target_os = "macos")]
    tarball_url: String,
    #[cfg(not(target_os = "macos"))]
    assets: Vec<GithubAsset>,
}
#[cfg(not(target_os = "macos"))]
#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}
#[derive(Clone, Copy, Default)]
struct DownloadMetrics {
    bytes_per_second: Option<f64>,
    eta_seconds: Option<u64>,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
}
#[derive(Clone, Copy)]
struct DownloadUpdate {
    progress: f32,
    metrics: DownloadMetrics,
    attempt: usize,
    resumed: bool,
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
    Ok(installed)
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

#[cfg(target_os = "macos")]
async fn install_ffmpeg_from_official_source(app: &AppHandle) -> Result<String, String> {
    let result = install_ffmpeg_from_official_source_inner(app).await;
    if let Err(message) = &result {
        emit_dependency_install(
            app,
            "ffmpeg",
            "failed",
            "FFmpeg 安装失败",
            0.0,
            None,
            Some(message.clone()),
        );
    }
    result
}

#[cfg(target_os = "macos")]
async fn install_ffmpeg_from_official_source_inner(app: &AppHandle) -> Result<String, String> {
    ensure_macos_arm64()?;
    ensure_build_tools("FFmpeg", &["clang", "make", "tar", "sh"])?;
    let sidecars_dir = sidecars_dir(app)?;
    let downloads_dir = sidecars_dir.join("downloads");
    tokio::fs::create_dir_all(&downloads_dir)
        .await
        .map_err(|error| format!("创建下载目录失败: {error}"))?;
    let archive_path = downloads_dir.join(FFMPEG_SOURCE_ARCHIVE_NAME);
    download_dependency_archive(app, "FFmpeg 源码", FFMPEG_SOURCE_URL, &archive_path).await?;

    let target_dir = sidecars_dir.join("ffmpeg");
    let staging_dir = target_dir.with_extension("installing");
    let source_dir = staging_dir.join("source");
    let install_dir = staging_dir.join("install");
    let _ = tokio::fs::remove_dir_all(&staging_dir).await;
    tokio::fs::create_dir_all(&source_dir)
        .await
        .map_err(|error| format!("创建 FFmpeg 构建目录失败: {error}"))?;
    tokio::fs::create_dir_all(&install_dir)
        .await
        .map_err(|error| format!("创建 FFmpeg 安装目录失败: {error}"))?;
    extract_tar_archive(app, "FFmpeg 源码", &archive_path, &source_dir, 0.86).await?;
    let configure_path = find_file_recursive(&source_dir, "configure")
        .ok_or_else(|| "FFmpeg 源码包里没有找到 configure".to_string())?;
    let source_root = configure_path
        .parent()
        .ok_or_else(|| "定位 FFmpeg 源码目录失败".to_string())?
        .to_path_buf();
    let jobs = build_parallelism();
    let prefix = install_dir.to_string_lossy().to_string();
    let mut configure = Command::new("./configure");
    configure
        .current_dir(&source_root)
        .arg(format!("--prefix={prefix}"))
        .arg("--arch=arm64")
        .arg("--cc=clang")
        .arg("--disable-doc")
        .arg("--disable-debug")
        .arg("--disable-ffplay")
        .arg("--enable-audiotoolbox")
        .arg("--enable-avfoundation")
        .arg("--enable-videotoolbox");
    run_install_command(app, "ffmpeg", "正在配置官方 FFmpeg 源码", 0.9, configure).await?;

    let mut make = Command::new("make");
    make.current_dir(&source_root)
        .arg("-j")
        .arg(jobs.to_string());
    run_install_command(app, "ffmpeg", "正在编译 FFmpeg，可能需要几分钟", 0.95, make).await?;

    let mut make_install = Command::new("make");
    make_install.current_dir(&source_root).arg("install");
    run_install_command(app, "ffmpeg", "正在安装 FFmpeg", 0.98, make_install).await?;

    let installed_path = install_dir.join("bin").join("ffmpeg");
    ensure_executable(&installed_path).await?;
    let _ = tokio::fs::remove_dir_all(&target_dir).await;
    tokio::fs::rename(&staging_dir, &target_dir)
        .await
        .map_err(|error| format!("保存 FFmpeg 失败: {error}"))?;
    let _ = tokio::fs::remove_file(&archive_path).await;
    let path = path_to_string(target_dir.join("install").join("bin").join("ffmpeg"));
    emit_dependency_install(
        app,
        "ffmpeg",
        "completed",
        "FFmpeg 已从官方源码编译安装",
        1.0,
        Some(path.clone()),
        None,
    );
    Ok(path)
}

#[cfg(target_os = "macos")]
async fn install_whisper_cpp_from_official_source(app: &AppHandle) -> Result<String, String> {
    let result = install_whisper_cpp_from_official_source_inner(app).await;
    if let Err(message) = &result {
        emit_dependency_install(
            app,
            "whisper.cpp",
            "failed",
            "whisper.cpp 安装失败",
            0.0,
            None,
            Some(message.clone()),
        );
    }
    result
}

#[cfg(target_os = "macos")]
async fn install_whisper_cpp_from_official_source_inner(app: &AppHandle) -> Result<String, String> {
    ensure_macos_arm64()?;
    ensure_build_tools("whisper.cpp", &["clang", "cmake", "make", "tar"])?;
    emit_dependency_install(
        app,
        "whisper.cpp",
        "running",
        "正在查询 whisper.cpp 官方发布源码",
        0.0,
        None,
        None,
    );
    let source = latest_whisper_cpp_source().await?;
    let sidecars_dir = sidecars_dir(app)?;
    let downloads_dir = sidecars_dir.join("downloads");
    tokio::fs::create_dir_all(&downloads_dir)
        .await
        .map_err(|error| format!("创建下载目录失败: {error}"))?;
    let archive_path = downloads_dir.join(format!("whisper.cpp-{}.tar.gz", source.tag_name));
    download_dependency_archive(app, "whisper.cpp 源码", &source.tarball_url, &archive_path)
        .await?;

    let target_dir = sidecars_dir.join("whisper.cpp");
    let staging_dir = target_dir.with_extension("installing");
    let source_dir = staging_dir.join("source");
    let build_dir = staging_dir.join("build");
    let _ = tokio::fs::remove_dir_all(&staging_dir).await;
    tokio::fs::create_dir_all(&source_dir)
        .await
        .map_err(|error| format!("创建 whisper.cpp 构建目录失败: {error}"))?;
    extract_tar_archive(app, "whisper.cpp 源码", &archive_path, &source_dir, 0.86).await?;
    let source_root = first_child_dir(&source_dir)
        .await?
        .ok_or_else(|| "whisper.cpp 源码包为空".to_string())?;
    let jobs = build_parallelism();

    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(&source_root)
        .arg("-B")
        .arg(&build_dir)
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg(format!(
            "-DCMAKE_OSX_DEPLOYMENT_TARGET={MACOS_ARM64_DEPLOYMENT_TARGET}"
        ))
        .arg("-DGGML_METAL=ON")
        .arg("-DWHISPER_BUILD_TESTS=OFF")
        .arg("-DWHISPER_BUILD_EXAMPLES=ON")
        .env("MACOSX_DEPLOYMENT_TARGET", MACOS_ARM64_DEPLOYMENT_TARGET);
    run_install_command(
        app,
        "whisper.cpp",
        "正在配置 whisper.cpp Metal 构建",
        0.9,
        configure,
    )
    .await?;

    let mut build = Command::new("cmake");
    build
        .arg("--build")
        .arg(&build_dir)
        .arg("--config")
        .arg("Release")
        .arg("--target")
        .arg("whisper-cli")
        .arg("--parallel")
        .arg(jobs.to_string())
        .env("MACOSX_DEPLOYMENT_TARGET", MACOS_ARM64_DEPLOYMENT_TARGET);
    run_install_command(
        app,
        "whisper.cpp",
        "正在编译 Metal 版 whisper-cli",
        0.97,
        build,
    )
    .await?;

    let built_path = find_file_recursive(&build_dir, "whisper-cli")
        .ok_or_else(|| "whisper.cpp 构建产物里没有找到 whisper-cli".to_string())?;
    ensure_executable(&built_path).await?;
    let relative_path = built_path
        .strip_prefix(&staging_dir)
        .map_err(|error| format!("定位 whisper-cli 失败: {error}"))?
        .to_path_buf();
    let _ = tokio::fs::remove_dir_all(&target_dir).await;
    tokio::fs::rename(&staging_dir, &target_dir)
        .await
        .map_err(|error| format!("保存 whisper.cpp 失败: {error}"))?;
    let _ = tokio::fs::remove_file(&archive_path).await;
    let path = path_to_string(target_dir.join(relative_path));
    emit_dependency_install(
        app,
        "whisper.cpp",
        "completed",
        "whisper-cli 已从官方源码编译安装（Metal）",
        1.0,
        Some(path.clone()),
        None,
    );
    Ok(path)
}

#[cfg(target_os = "macos")]
fn ensure_macos_arm64() -> Result<(), String> {
    if std::env::consts::ARCH != "aarch64" {
        return Err("macOS 版本仅支持 Apple Silicon (arm64)，不支持 Intel Mac".to_string());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_build_tools(item: &str, tools: &[&str]) -> Result<(), String> {
    let missing = tools
        .iter()
        .filter(|tool| which::which(tool).is_err())
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "编译 {item} 需要本机已有构建工具: {}。请先安装 Xcode Command Line Tools 后重试。",
        missing.join(", ")
    ))
}

#[cfg(target_os = "macos")]
struct WhisperCppSource {
    tag_name: String,
    tarball_url: String,
}

#[cfg(target_os = "macos")]
async fn latest_whisper_cpp_source() -> Result<WhisperCppSource, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|error| format!("创建 GitHub 客户端失败: {error}"))?;
    let release = client
        .get(WHISPER_RELEASE_API_URL)
        .send()
        .await
        .map_err(|error| format!("查询 whisper.cpp 发布源码失败: {error}"))?
        .error_for_status()
        .map_err(|error| format!("查询 whisper.cpp 发布源码失败: {error}"))?
        .json::<GithubRelease>()
        .await
        .map_err(|error| format!("解析 whisper.cpp 发布源码失败: {error}"))?;
    Ok(WhisperCppSource {
        tag_name: release.tag_name,
        tarball_url: release.tarball_url,
    })
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
async fn download_dependency_archive(
    app: &AppHandle,
    item: &str,
    url: &str,
    archive_path: &Path,
) -> Result<(), String> {
    emit_dependency_install(
        app,
        item,
        "running",
        format!("开始下载 {item}"),
        0.0,
        None,
        None,
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30 * 60))
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|error| format!("创建下载客户端失败: {error}"))?;
    let result = download_file_with_resume(
        &client,
        url,
        archive_path,
        0.82,
        |update| {
            let metrics = update.metrics;
            let message = download_message(item, update);
            emit_dependency_install_with_metrics(
                app,
                item,
                "running",
                message,
                update.progress,
                None,
                None,
                metrics,
            );
        },
        |attempt, error, downloaded| {
            emit_dependency_install_with_metrics(
                app,
                item,
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
    .await;
    if let Err(message) = &result {
        emit_dependency_install(
            app,
            item,
            "failed",
            message,
            0.0,
            None,
            Some(message.clone()),
        );
    }
    result
}
async fn download_file_with_resume<F, R>(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    progress_scale: f32,
    mut on_update: F,
    mut on_retry: R,
) -> Result<(), String>
where
    F: FnMut(DownloadUpdate),
    R: FnMut(usize, &str, u64),
{
    let mut last_error = String::new();
    for attempt in 1..=DOWNLOAD_MAX_ATTEMPTS {
        let existing_bytes = file_len(path).await.unwrap_or(0);
        let mut request = client.get(url);
        if existing_bytes > 0 {
            request = request.header(RANGE, format!("bytes={existing_bytes}-"));
        }
        let mut response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                last_error = error.to_string();
                retry_download(attempt, &last_error, existing_bytes, &mut on_retry).await;
                continue;
            }
        };
        let status = response.status();
        if status == StatusCode::RANGE_NOT_SATISFIABLE && existing_bytes > 0 {
            return Ok(());
        }
        if !(status.is_success() || status == StatusCode::PARTIAL_CONTENT) {
            last_error = format!("HTTP {status}");
            retry_download(attempt, &last_error, existing_bytes, &mut on_retry).await;
            continue;
        }
        last_error.clear();
        let resumed = existing_bytes > 0 && status == StatusCode::PARTIAL_CONTENT;
        let mut downloaded = if resumed { existing_bytes } else { 0 };
        let total = response.content_length().and_then(|remaining| {
            if resumed {
                existing_bytes.checked_add(remaining)
            } else {
                Some(remaining)
            }
        });
        let mut file = if resumed {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .await
                .map_err(|error| format!("打开续传文件失败: {error}"))?
        } else {
            tokio::fs::File::create(path)
                .await
                .map_err(|error| format!("创建下载文件失败: {error}"))?
        };
        let started_at = Instant::now();
        let started_bytes = downloaded;
        let mut last_emit_progress = -1.0f32;
        let mut last_emit_at = Instant::now();
        loop {
            let chunk = match response.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(error) => {
                    last_error = error.to_string();
                    break;
                }
            };
            file.write_all(&chunk)
                .await
                .map_err(|error| format!("写入下载文件失败: {error}"))?;
            downloaded += chunk.len() as u64;
            let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
            let speed = (downloaded.saturating_sub(started_bytes)) as f64 / elapsed;
            let eta_seconds = total.and_then(|total| {
                (speed > 0.0 && total > downloaded)
                    .then(|| ((total - downloaded) as f64 / speed).ceil() as u64)
            });
            let progress = total
                .map(|total| {
                    (downloaded as f32 / total as f32 * progress_scale).clamp(0.0, progress_scale)
                })
                .unwrap_or(0.0);
            if progress >= progress_scale
                || progress - last_emit_progress >= 0.01
                || last_emit_at.elapsed() >= Duration::from_secs(1)
            {
                last_emit_progress = progress;
                last_emit_at = Instant::now();
                on_update(DownloadUpdate {
                    progress,
                    metrics: DownloadMetrics {
                        bytes_per_second: Some(speed),
                        eta_seconds,
                        downloaded_bytes: Some(downloaded),
                        total_bytes: total,
                    },
                    attempt,
                    resumed,
                });
            }
        }
        file.flush()
            .await
            .map_err(|error| format!("刷新下载文件失败: {error}"))?;
        if last_error.is_empty() {
            return Ok(());
        }
        retry_download(attempt, &last_error, downloaded, &mut on_retry).await;
    }
    Err(format!(
        "下载失败，已重试 {DOWNLOAD_MAX_ATTEMPTS} 次: {last_error}"
    ))
}
async fn retry_download<R>(attempt: usize, error: &str, downloaded: u64, on_retry: &mut R)
where
    R: FnMut(usize, &str, u64),
{
    if attempt < DOWNLOAD_MAX_ATTEMPTS {
        let next_attempt = attempt + 1;
        on_retry(next_attempt, error, downloaded);
        sleep(Duration::from_millis(700 * attempt as u64)).await;
    }
}
async fn file_len(path: &Path) -> Option<u64> {
    tokio::fs::metadata(path)
        .await
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
}
fn download_message(label: &str, update: DownloadUpdate) -> String {
    let downloaded = update
        .metrics
        .downloaded_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "0 KiB".to_string());
    let total = update
        .metrics
        .total_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "未知大小".to_string());
    let prefix = if update.resumed {
        "正在续传"
    } else if update.attempt > 1 {
        "正在重试"
    } else {
        "正在下载"
    };
    format!("{prefix} {label} {downloaded} / {total}")
}
#[cfg(not(target_os = "macos"))]
async fn extract_dependency_archive(
    item: &str,
    exe_name: &str,
    app: &AppHandle,
    archive_path: &Path,
    target_dir: &Path,
) -> Result<String, String> {
    emit_dependency_install(
        app,
        item,
        "running",
        format!("正在解压 {item}"),
        0.9,
        None,
        None,
    );
    let staging_dir = target_dir.with_extension("installing");
    let _ = tokio::fs::remove_dir_all(&staging_dir).await;
    tokio::fs::create_dir_all(&staging_dir)
        .await
        .map_err(|error| format!("创建 {item} 解压目录失败: {error}"))?;
    let archive_path = archive_path.to_path_buf();
    let staging_for_extract = staging_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        extract_zip_archive(&archive_path, &staging_for_extract)
    })
    .await
    .map_err(|error| format!("解压 {item} 任务失败: {error}"))?
    .map_err(|error| format!("解压 {item} 失败: {error}"))?;
    let exe_path = find_file_recursive(&staging_dir, exe_name)
        .ok_or_else(|| format!("{item} 发布包里没有找到 {exe_name}"))?;
    let relative_exe = exe_path
        .strip_prefix(&staging_dir)
        .map_err(|error| format!("定位 {item} 可执行文件失败: {error}"))?
        .to_path_buf();
    let _ = tokio::fs::remove_dir_all(target_dir).await;
    tokio::fs::rename(&staging_dir, target_dir)
        .await
        .map_err(|error| format!("保存 {item} 失败: {error}"))?;
    let installed_path = target_dir.join(relative_exe);
    ensure_executable(&installed_path).await?;
    Ok(path_to_string(installed_path))
}

#[cfg(target_os = "macos")]
async fn extract_tar_archive(
    app: &AppHandle,
    item: &str,
    archive_path: &Path,
    output_dir: &Path,
    progress: f32,
) -> Result<(), String> {
    let mut command = Command::new("tar");
    command
        .arg("-xf")
        .arg(archive_path)
        .arg("-C")
        .arg(output_dir);
    run_install_command(app, item, format!("正在解包 {item}"), progress, command).await
}

#[cfg(target_os = "macos")]
async fn run_install_command(
    app: &AppHandle,
    item: &str,
    message: impl Into<String>,
    progress: f32,
    mut command: Command,
) -> Result<(), String> {
    let message = message.into();
    emit_dependency_install(app, item, "running", message, progress, None, None);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let output = command
        .output()
        .await
        .map_err(|error| format!("执行 {item} 安装命令失败: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        trim_command_output(&stderr)
    } else {
        trim_command_output(&stdout)
    };
    Err(format!(
        "{item} 安装命令退出失败: {}{}",
        output.status,
        if detail.is_empty() {
            String::new()
        } else {
            format!("\n{detail}")
        }
    ))
}

#[cfg(target_os = "macos")]
fn trim_command_output(output: &str) -> String {
    let lines = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let start = lines.len().saturating_sub(24);
    lines[start..].join("\n")
}

#[cfg(target_os = "macos")]
fn build_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get().clamp(2, 8))
        .unwrap_or(4)
}

#[cfg(target_os = "macos")]
async fn first_child_dir(path: &Path) -> Result<Option<PathBuf>, String> {
    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|error| format!("读取源码目录失败: {error}"))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("读取源码目录失败: {error}"))?
    {
        let file_type = entry
            .file_type()
            .await
            .map_err(|error| format!("读取源码目录失败: {error}"))?;
        if file_type.is_dir() {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

#[cfg(unix)]
async fn ensure_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| format!("读取可执行文件权限失败: {error}"))?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    tokio::fs::set_permissions(path, permissions)
        .await
        .map_err(|error| format!("设置可执行权限失败: {error}"))
}

#[cfg(not(unix))]
async fn ensure_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn extract_zip_archive(archive_path: &Path, output_dir: &Path) -> Result<(), String> {
    let file = fs::File::open(archive_path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| error.to_string())?;
        let Some(enclosed_name) = file.enclosed_name() else {
            continue;
        };
        let output_path = output_dir.join(enclosed_name);
        if file.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = fs::File::create(&output_path).map_err(|error| error.to_string())?;
        io::copy(&mut file, &mut output).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn find_whisper_model_preset(id: &str) -> Option<WhisperModelPreset> {
    WHISPER_MODEL_PRESETS
        .iter()
        .copied()
        .find(|preset| preset.id == id)
}

fn emit_dependency_install(
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
fn emit_dependency_install_with_metrics(
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
fn emit_model_download(
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
fn emit_model_download_with_metrics(
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

fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes < 1024 * 1024 {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB)
    }
}
