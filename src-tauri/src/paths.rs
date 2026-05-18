use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::{path::BaseDirectory, AppHandle, Manager};

use crate::state::{JobError, JobResult};

pub(crate) fn resolve_output_dir(
    video_path: &Path,
    output_dir: Option<&str>,
) -> JobResult<PathBuf> {
    let dir = output_dir
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| video_path.parent().map(Path::to_path_buf))
        .ok_or_else(|| JobError::failed("无法确定输出目录"))?;
    Ok(dir)
}
pub(crate) fn safe_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(sanitize_file_part)
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "subtitle".to_string())
}
pub(crate) fn sanitize_file_part(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => ch,
        })
        .collect::<String>();
    sanitized.trim().replace(' ', "_")
}

pub(crate) fn locate_binary(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let exe_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    locate_managed_binary(app, &exe_name)
        .or_else(|| locate_resource_binary(app, &exe_name))
        .or_else(|| locate_platform_binary(&exe_name))
        .or_else(|| which::which(&exe_name).ok())
        .or_else(|| which::which(name).ok())
}
fn locate_managed_binary(app: &AppHandle, exe_name: &str) -> Option<PathBuf> {
    sidecars_dir(app)
        .ok()
        .and_then(|dir| find_managed_binary(&dir, exe_name))
}
fn locate_resource_binary(app: &AppHandle, exe_name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for platform_dir in platform_resource_dirs() {
        for relative in [
            format!("bin/{platform_dir}/{exe_name}"),
            format!("resources/bin/{platform_dir}/{exe_name}"),
        ] {
            if let Ok(path) = app.path().resolve(relative, BaseDirectory::Resource) {
                candidates.push(path);
            }
        }
    }
    for relative in [
        format!("bin/{exe_name}"),
        format!("resources/bin/{exe_name}"),
    ] {
        if let Ok(path) = app.path().resolve(relative, BaseDirectory::Resource) {
            candidates.push(path);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for platform_dir in platform_resource_dirs() {
            candidates.push(
                cwd.join("src-tauri")
                    .join("resources")
                    .join("bin")
                    .join(platform_dir)
                    .join(exe_name),
            );
            candidates.push(
                cwd.join("resources")
                    .join("bin")
                    .join(platform_dir)
                    .join(exe_name),
            );
        }
        candidates.push(
            cwd.join("src-tauri")
                .join("resources")
                .join("bin")
                .join(exe_name),
        );
        candidates.push(cwd.join("resources").join("bin").join(exe_name));
    }
    candidates.into_iter().find(|path| path.exists())
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn platform_resource_dirs() -> &'static [&'static str] {
    &["macos-arm64", "darwin-arm64"]
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn platform_resource_dirs() -> &'static [&'static str] {
    &["macos-x64", "darwin-x64"]
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn platform_resource_dirs() -> &'static [&'static str] {
    &["windows-x64", "win32-x64"]
}

#[cfg(all(windows, target_arch = "aarch64"))]
fn platform_resource_dirs() -> &'static [&'static str] {
    &["windows-arm64", "win32-arm64"]
}

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(windows, target_arch = "x86_64"),
    all(windows, target_arch = "aarch64")
)))]
fn platform_resource_dirs() -> &'static [&'static str] {
    &[]
}

#[cfg(target_os = "macos")]
fn locate_platform_binary(exe_name: &str) -> Option<PathBuf> {
    [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/opt/local/bin",
    ]
    .iter()
    .map(Path::new)
    .map(|dir| dir.join(exe_name))
    .find(|path| path.exists())
}

#[cfg(not(target_os = "macos"))]
fn locate_platform_binary(_exe_name: &str) -> Option<PathBuf> {
    None
}
pub(crate) fn sidecars_dir(app: &AppHandle) -> Result<PathBuf, String> {
    managed_dir(app, "sidecars")
}
pub(crate) fn find_managed_binary(root: &Path, exe_name: &str) -> Option<PathBuf> {
    find_expected_managed_binary(root, exe_name).or_else(|| {
        managed_package_dirs(root, exe_name)
            .into_iter()
            .find_map(|dir| find_file_recursive(&dir, exe_name))
    })
}
fn find_expected_managed_binary(root: &Path, exe_name: &str) -> Option<PathBuf> {
    let mut candidates = vec![root.join(exe_name), root.join("bin").join(exe_name)];
    for package_dir in managed_package_dirs(root, exe_name) {
        candidates.push(package_dir.join(exe_name));
        candidates.push(package_dir.join("bin").join(exe_name));

        let Ok(entries) = fs::read_dir(&package_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let child = entry.path();
            candidates.push(child.join(exe_name));
            candidates.push(child.join("bin").join(exe_name));
        }
    }
    candidates.into_iter().find(|path| path.exists())
}
fn managed_package_dirs(root: &Path, exe_name: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let lower = exe_name.to_ascii_lowercase();
    if lower == "ffmpeg" || lower == "ffmpeg.exe" {
        dirs.push(root.join("ffmpeg"));
    } else if lower == "whisper-cli" || lower == "whisper-cli.exe" {
        dirs.push(root.join("whisper.cpp"));
    }
    if let Some(stem) = Path::new(exe_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
    {
        let derived = root.join(stem);
        if !dirs.iter().any(|dir| dir == &derived) {
            dirs.push(derived);
        }
    }
    dirs
}
pub(crate) fn find_file_recursive(root: &Path, file_name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(file_name))
            {
                return Some(path);
            }
        }
    }
    None
}
pub(crate) fn whisper_models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    managed_dir(app, "models")
}
pub(crate) fn managed_dir(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .or_else(|_| app.path().app_config_dir())
        .map_err(|error| error.to_string())?;
    let dir = base.join(name);
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}
pub(crate) fn is_existing_file(path: &Path) -> bool {
    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

pub(crate) fn path_to_string(path: PathBuf) -> String {
    path.to_string_lossy().to_string()
}
pub(crate) fn display_path_to_string(path: PathBuf) -> String {
    display_path_string(&path_to_string(path))
}
fn display_path_string(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::{display_path_string, find_managed_binary};
    use std::{fs, path::PathBuf};

    #[test]
    fn display_path_string_strips_windows_verbatim_drive_prefix() {
        assert_eq!(
            display_path_string(r"\\?\D:\Apps\Luma Subtitle\resources"),
            r"D:\Apps\Luma Subtitle\resources"
        );
    }

    #[test]
    fn display_path_string_strips_windows_verbatim_unc_prefix() {
        assert_eq!(
            display_path_string(r"\\?\UNC\nas\share\Luma Subtitle\resources"),
            r"\\nas\share\Luma Subtitle\resources"
        );
    }

    #[test]
    fn find_managed_binary_prefers_expected_install_locations() {
        let root = temp_test_dir("managed-binary-expected");
        let expected_dir = root.join("ffmpeg").join("ffmpeg-release").join("bin");
        let deep_dir = root.join("downloads").join("very").join("deep");
        fs::create_dir_all(&expected_dir).unwrap();
        fs::create_dir_all(&deep_dir).unwrap();

        let expected = expected_dir.join("ffmpeg.exe");
        let deep = deep_dir.join("ffmpeg.exe");
        fs::write(&expected, b"expected").unwrap();
        fs::write(&deep, b"deep").unwrap();

        assert_eq!(find_managed_binary(&root, "ffmpeg.exe"), Some(expected));

        let _ = fs::remove_dir_all(root);
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("luma-subtitle-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
