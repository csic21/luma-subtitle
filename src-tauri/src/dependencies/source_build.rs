use std::process::Command;
use tauri::AppHandle;

use crate::paths::{find_file_recursive, path_to_string, sidecars_dir};

use super::{
    download::download_dependency_archive,
    events::emit_dependency_install,
    install::{
        activate_directory, build_parallelism, ensure_executable, extract_tar_archive,
        first_child_dir, fix_whisper_macos_rpaths, run_install_command, StagedDirectory,
    },
    trust, MACOS_ARM64_DEPLOYMENT_TARGET,
};

pub(super) async fn install_ffmpeg_from_official_source(app: &AppHandle) -> Result<String, String> {
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

async fn install_ffmpeg_from_official_source_inner(app: &AppHandle) -> Result<String, String> {
    ensure_macos_arm64()?;
    ensure_build_tools("FFmpeg", &["clang", "make", "tar", "sh"])?;
    let sidecars_dir = sidecars_dir(app)?;
    let downloads_dir = sidecars_dir.join("downloads");
    tokio::fs::create_dir_all(&downloads_dir)
        .await
        .map_err(|error| format!("创建下载目录失败: {error}"))?;
    let artifact = trust::artifact("ffmpeg-source")?;
    let archive_path = downloads_dir.join(format!(
        "{}.{}.part",
        artifact.file_name,
        uuid::Uuid::new_v4()
    ));
    let _cleanup = super::download::PartialFile(archive_path.clone());
    download_dependency_archive(app, "FFmpeg 源码", &artifact, &archive_path).await?;

    let target_dir = sidecars_dir.join("ffmpeg");
    let staging = StagedDirectory::new(&target_dir)?;
    let staging_dir = &staging.0;
    let source_dir = staging_dir.join("source");
    let install_dir = staging_dir.join("install");
    tokio::fs::create_dir_all(&source_dir)
        .await
        .map_err(|error| format!("创建 FFmpeg 构建目录失败: {error}"))?;
    tokio::fs::create_dir_all(&install_dir)
        .await
        .map_err(|error| format!("创建 FFmpeg 安装目录失败: {error}"))?;
    extract_tar_archive(
        app,
        "FFmpeg 源码",
        &archive_path,
        &source_dir,
        &artifact,
        &staging,
        0.86,
    )
    .await?;
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
    run_install_command(
        app,
        "ffmpeg",
        "正在配置官方 FFmpeg 源码",
        0.9,
        configure,
        &staging,
    )
    .await?;

    let mut make = Command::new("make");
    make.current_dir(&source_root)
        .arg("-j")
        .arg(jobs.to_string());
    run_install_command(
        app,
        "ffmpeg",
        "正在编译 FFmpeg，可能需要几分钟",
        0.95,
        make,
        &staging,
    )
    .await?;

    let mut make_install = Command::new("make");
    make_install.current_dir(&source_root).arg("install");
    run_install_command(
        app,
        "ffmpeg",
        "正在安装 FFmpeg",
        0.98,
        make_install,
        &staging,
    )
    .await?;

    let installed_path = install_dir.join("bin").join("ffmpeg");
    ensure_executable(&installed_path).await?;
    activate_directory(staging_dir, &target_dir)?;
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

pub(super) async fn install_whisper_cpp_from_official_source(
    app: &AppHandle,
) -> Result<String, String> {
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

async fn install_whisper_cpp_from_official_source_inner(app: &AppHandle) -> Result<String, String> {
    ensure_macos_arm64()?;
    ensure_build_tools(
        "whisper.cpp",
        &[
            "clang",
            "cmake",
            "make",
            "tar",
            "otool",
            "install_name_tool",
        ],
    )?;
    emit_dependency_install(
        app,
        "whisper.cpp",
        "running",
        "正在准备已固定版本的 whisper.cpp 官方源码",
        0.0,
        None,
        None,
    );
    let artifact = trust::artifact("whisper-source")?;
    let sidecars_dir = sidecars_dir(app)?;
    let downloads_dir = sidecars_dir.join("downloads");
    tokio::fs::create_dir_all(&downloads_dir)
        .await
        .map_err(|error| format!("创建下载目录失败: {error}"))?;
    let archive_path = downloads_dir.join(format!(
        "{}.{}.part",
        artifact.file_name,
        uuid::Uuid::new_v4()
    ));
    let _cleanup = super::download::PartialFile(archive_path.clone());
    download_dependency_archive(app, "whisper.cpp 源码", &artifact, &archive_path).await?;

    let target_dir = sidecars_dir.join("whisper.cpp");
    let staging = StagedDirectory::new(&target_dir)?;
    let staging_dir = &staging.0;
    let source_dir = staging_dir.join("source");
    let build_dir = staging_dir.join("build");
    tokio::fs::create_dir_all(&source_dir)
        .await
        .map_err(|error| format!("创建 whisper.cpp 构建目录失败: {error}"))?;
    extract_tar_archive(
        app,
        "whisper.cpp 源码",
        &archive_path,
        &source_dir,
        &artifact,
        &staging,
        0.86,
    )
    .await?;
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
        &staging,
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
        &staging,
    )
    .await?;

    let built_path = find_file_recursive(&build_dir, "whisper-cli")
        .ok_or_else(|| "whisper.cpp 构建产物里没有找到 whisper-cli".to_string())?;
    ensure_executable(&built_path).await?;
    let relative_path = built_path
        .strip_prefix(&staging_dir)
        .map_err(|error| format!("定位 whisper-cli 失败: {error}"))?
        .to_path_buf();
    fix_whisper_macos_rpaths(&target_dir, staging_dir, &staging).await?;
    activate_directory(staging_dir, &target_dir)?;
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

fn ensure_macos_arm64() -> Result<(), String> {
    if std::env::consts::ARCH != "aarch64" {
        return Err("macOS 版本仅支持 Apple Silicon (arm64)，不支持 Intel Mac".to_string());
    }
    Ok(())
}

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
