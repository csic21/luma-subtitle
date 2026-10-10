#[cfg(any(target_os = "macos", not(target_os = "macos")))]
use std::fs;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

#[cfg(target_os = "macos")]
use std::process::Command;
use tauri::AppHandle;

#[cfg(not(target_os = "macos"))]
use crate::paths::{find_file_recursive, path_to_string};

pub(super) fn extract_zip_into_dir(
    archive_path: &Path,
    output_dir: &Path,
    artifact: &super::trust::Artifact,
) -> Result<(), String> {
    crate::asr_components::extract_legacy_zip(
        archive_path,
        output_dir,
        artifact.unpacked_bytes,
        artifact.file_limit,
    )
}

#[derive(Clone)]
pub(super) struct StagedDirectory(pub std::path::PathBuf, std::sync::Arc<StagingCleanup>);
struct StagingCleanup(std::path::PathBuf);
impl StagedDirectory {
    pub(super) fn new(target: &Path) -> Result<Self, String> {
        let path = target.with_extension(format!("{}.installing", uuid::Uuid::new_v4()));
        crate::asr_components::legacy_private_dir(&path)?;
        Ok(Self(
            path.clone(),
            std::sync::Arc::new(StagingCleanup(path)),
        ))
    }
}
impl Drop for StagingCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
/// Preserve a previous install on all pre-activation failures and roll back if
/// the final rename fails. Callers serialize installers and finish all payloads first.
pub(super) fn activate_directory(staged: &Path, target: &Path) -> Result<(), String> {
    let backup = target.with_extension(format!("{}.previous", uuid::Uuid::new_v4()));
    let existed = match fs::symlink_metadata(target) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => true,
        Ok(_) => return Err("Refusing a non-directory dependency target".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.to_string()),
    };
    if existed {
        fs::rename(target, &backup).map_err(|e| e.to_string())?;
    }
    if let Err(error) = fs::rename(staged, target) {
        if existed {
            fs::rename(&backup, target).map_err(|rollback| {
                format!(
                    "Activation failed ({error}); restore {} manually: {rollback}",
                    backup.display()
                )
            })?;
        }
        return Err(format!(
            "Dependency activation failed; previous install preserved: {error}"
        ));
    }
    if existed {
        let _ = fs::remove_dir_all(backup);
    }
    Ok(())
}

use super::events::emit_dependency_install;

#[cfg(not(target_os = "macos"))]
pub(super) async fn extract_dependency_archive(
    item: &str,
    exe_name: &str,
    app: &AppHandle,
    archive_path: &Path,
    artifact: &super::trust::Artifact,
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
    let staging = StagedDirectory::new(target_dir)?;
    let staging_dir = &staging.0;
    let artifact = artifact.clone();
    let archive_path = archive_path.to_path_buf();
    let staging_for_extract = staging.clone();
    tauri::async_runtime::spawn_blocking(move || {
        extract_zip_into_dir(&archive_path, &staging_for_extract.0, &artifact)
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
    ensure_executable(&exe_path).await?;
    activate_directory(staging_dir, target_dir)?;
    let installed_path = target_dir.join(relative_exe);
    Ok(path_to_string(installed_path))
}

#[cfg(target_os = "macos")]
pub(super) async fn extract_tar_archive(
    app: &AppHandle,
    item: &str,
    archive_path: &Path,
    output_dir: &Path,
    artifact: &super::trust::Artifact,
    lifetime: &StagedDirectory,
    progress: f32,
) -> Result<(), String> {
    emit_dependency_install(
        app,
        item,
        "running",
        format!("正在解包 {item}"),
        progress,
        None,
        None,
    );
    let source = archive_path.to_path_buf();
    let destination = output_dir.to_path_buf();
    let artifact = artifact.clone();
    let lifetime = lifetime.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _owner = lifetime;
        super::archive::extract_tar(&source, &destination, &artifact)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(target_os = "macos")]
pub(super) async fn run_install_command(
    app: &AppHandle,
    item: &str,
    message: impl Into<String>,
    progress: f32,
    command: Command,
    lifetime: &StagedDirectory,
) -> Result<(), String> {
    let message = message.into();
    emit_dependency_install(app, item, "running", message, progress, None, None);
    let output = bounded_install_output(command, lifetime, Duration::from_secs(45 * 60)).await?;
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
    lines[start..]
        .iter()
        .map(|line| line.chars().take(512).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(target_os = "macos")]
pub(super) async fn fix_whisper_macos_rpaths(
    target_dir: &Path,
    staging_dir: &Path,
    lifetime: &StagedDirectory,
) -> Result<(), String> {
    let staging_prefix = staging_dir.to_string_lossy().to_string();
    let target_prefix = target_dir.to_string_lossy().to_string();
    for path in collect_whisper_macos_binaries(staging_dir)? {
        let rpaths = read_macos_rpaths(&path, lifetime).await?;
        for old_rpath in rpaths {
            if !old_rpath.starts_with(&staging_prefix) {
                continue;
            }
            let new_rpath = old_rpath.replacen(&staging_prefix, &target_prefix, 1);
            change_macos_rpath(&path, &old_rpath, &new_rpath, lifetime).await?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn collect_whisper_macos_binaries(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut binaries = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|error| format!("读取 whisper.cpp 安装目录失败: {error}"))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if file_name == "whisper-cli" || file_name.ends_with(".dylib") {
                binaries.push(path);
            }
        }
    }
    Ok(binaries)
}

#[cfg(target_os = "macos")]
async fn read_macos_rpaths(path: &Path, lifetime: &StagedDirectory) -> Result<Vec<String>, String> {
    let mut command = Command::new("otool");
    command.arg("-l").arg(path);
    let output = bounded_install_output(command, lifetime, Duration::from_secs(60)).await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "读取 Mach-O rpath 失败: {}{}",
            output.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!("\n{}", trim_command_output(&stderr))
            }
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("path "))
        .filter_map(|line| line.split(" (offset").next())
        .map(str::to_string)
        .collect())
}

#[cfg(target_os = "macos")]
async fn change_macos_rpath(
    path: &Path,
    old_rpath: &str,
    new_rpath: &str,
    lifetime: &StagedDirectory,
) -> Result<(), String> {
    let mut command = Command::new("install_name_tool");
    command
        .arg("-rpath")
        .arg(old_rpath)
        .arg(new_rpath)
        .arg(path);
    let output = bounded_install_output(command, lifetime, Duration::from_secs(60)).await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "修复 whisper.cpp 动态库路径失败: {}{}",
        output.status,
        if stderr.trim().is_empty() {
            String::new()
        } else {
            format!("\n{}", trim_command_output(&stderr))
        }
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn build_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get().clamp(2, 8))
        .unwrap_or(4)
}

#[cfg(target_os = "macos")]
pub(super) async fn first_child_dir(path: &Path) -> Result<Option<PathBuf>, String> {
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
pub(super) async fn ensure_executable(path: &Path) -> Result<(), String> {
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
pub(super) async fn ensure_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(super) fn activate_model(staged: &Path, target: &Path) -> Result<(), String> {
    let backup = target.with_extension(format!("{}.previous", uuid::Uuid::new_v4()));
    let existed = match fs::symlink_metadata(target) {
        Ok(_) => {
            crate::asr_components::legacy_regular_file(target)?;
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.to_string()),
    };
    if existed {
        fs::rename(target, &backup).map_err(|e| e.to_string())?;
    }
    if let Err(error) = fs::rename(staged, target) {
        if existed {
            fs::rename(&backup, target).map_err(|rollback| {
                format!(
                    "Model activation failed ({error}); restore {} manually: {rollback}",
                    backup.display()
                )
            })?;
        }
        return Err(format!(
            "Model activation failed; previous file preserved: {error}"
        ));
    }
    if existed {
        let _ = fs::remove_file(backup);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn bounded_install_output(
    command: Command,
    lifetime: &StagedDirectory,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    crate::owned_process::output_with_lifetime(command, Arc::new(AtomicBool::new(false)), timeout, 8 * 1024 * 1024, Arc::new(lifetime.clone())).await.map_err(|error| match error {
        crate::state::JobError::Cancelled => "Dependency installation was cancelled; the previous install is unchanged.".to_string(),
        crate::state::JobError::Failed(_) => "Dependency build tool failed, timed out, or exceeded its output limit; the previous install is unchanged.".to_string(),
    })
}
