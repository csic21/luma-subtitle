#[cfg(not(target_os = "macos"))]
use std::io;
#[cfg(any(target_os = "macos", not(target_os = "macos")))]
use std::fs;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::{path::PathBuf, process::Stdio};

use tauri::AppHandle;
#[cfg(target_os = "macos")]
use tokio::process::Command;

#[cfg(not(target_os = "macos"))]
use crate::paths::{find_file_recursive, path_to_string};

use super::events::emit_dependency_install;

#[cfg(not(target_os = "macos"))]
pub(super) async fn extract_dependency_archive(
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
pub(super) async fn extract_tar_archive(
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
pub(super) async fn run_install_command(
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
pub(super) async fn fix_whisper_macos_rpaths(
    target_dir: &Path,
    staging_dir: &Path,
) -> Result<(), String> {
    let staging_prefix = staging_dir.to_string_lossy().to_string();
    let target_prefix = target_dir.to_string_lossy().to_string();
    for path in collect_whisper_macos_binaries(target_dir)? {
        let rpaths = read_macos_rpaths(&path).await?;
        for old_rpath in rpaths {
            if !old_rpath.starts_with(&staging_prefix) {
                continue;
            }
            let new_rpath = old_rpath.replacen(&staging_prefix, &target_prefix, 1);
            change_macos_rpath(&path, &old_rpath, &new_rpath).await?;
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
async fn read_macos_rpaths(path: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("otool")
        .arg("-l")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| format!("读取 Mach-O rpath 失败: {error}"))?;
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
async fn change_macos_rpath(path: &Path, old_rpath: &str, new_rpath: &str) -> Result<(), String> {
    let output = Command::new("install_name_tool")
        .arg("-rpath")
        .arg(old_rpath)
        .arg(new_rpath)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| format!("修复 whisper.cpp 动态库路径失败: {error}"))?;
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
