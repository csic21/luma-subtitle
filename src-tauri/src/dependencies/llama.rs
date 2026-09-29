use std::path::Path;

use serde::Deserialize;
use tauri::AppHandle;

use crate::paths::{locate_binary, path_to_string, sidecars_dir};
#[cfg(target_os = "macos")]
use crate::paths::find_file_recursive;

use super::{
    download::download_dependency_archive, events::emit_dependency_install, HTTP_USER_AGENT,
};
#[cfg(target_os = "macos")]
use super::install::ensure_executable;
#[cfg(not(target_os = "macos"))]
use super::install::extract_dependency_archive;

#[cfg(target_os = "macos")]
use super::install::extract_tar_archive;

const LLAMA_RELEASE_API_URL: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LlamaCppAssetSelection {
    pub binary_name: String,
    pub cudart_name: Option<String>,
    pub backend: &'static str,
}

#[derive(Deserialize)]
struct GithubRelease {
    assets: Vec<GithubAsset>,
}

#[derive(Clone, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub(super) async fn install_llama_cpp(app: AppHandle) -> Result<String, String> {
    if let Some(path) = locate_binary(&app, "llama-server") {
        let path = path_to_string(path);
        emit_dependency_install(
            &app,
            "llama.cpp",
            "completed",
            "llama.cpp 已可用",
            1.0,
            Some(path.clone()),
            None,
        );
        return Ok(path);
    }

    let result = install_llama_cpp_inner(&app).await;
    if let Err(message) = &result {
        emit_dependency_install(
            &app,
            "llama.cpp",
            "failed",
            "llama.cpp 安装失败",
            0.0,
            None,
            Some(message.clone()),
        );
    }
    result
}

pub(crate) fn llama_backend_label(server: &Path) -> Option<String> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let _ = server;
        Some("Metal".to_string())
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let dir = server.parent()?;
        if file_exists(dir, "ggml-cuda.dll")
            || file_exists(dir, "ggml-cuda.dylib")
            || file_exists(dir, "ggml-cuda.so")
        {
            return Some("CUDA".to_string());
        }
        if file_exists(dir, "ggml-vulkan.dll") || file_exists(dir, "ggml-vulkan.so") {
            return Some("Vulkan".to_string());
        }
        if cfg!(target_os = "macos") {
            Some("Metal".to_string())
        } else {
            Some("CPU".to_string())
        }
    }
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn file_exists(dir: &Path, name: &str) -> bool {
    dir.join(name).exists()
}

async fn install_llama_cpp_inner(app: &AppHandle) -> Result<String, String> {
    emit_dependency_install(
        app,
        "llama.cpp",
        "running",
        "正在查询 llama.cpp 官方发布包",
        0.0,
        None,
        None,
    );
    let release = latest_llama_cpp_release().await?;
    let asset_names = release
        .assets
        .iter()
        .map(|asset| asset.name.as_str())
        .collect::<Vec<_>>();
    let selection = select_llama_cpp_assets(
        &asset_names,
        current_install_os(),
        std::env::consts::ARCH,
        super::has_nvidia_gpu_hint(),
    )
    .ok_or_else(|| {
        format!(
            "当前系统没有匹配的 llama.cpp 发布包（{} / {}）",
            current_install_os(),
            std::env::consts::ARCH
        )
    })?;
    let binary = find_asset(&release, &selection.binary_name)
        .ok_or_else(|| format!("未找到 llama.cpp 包 {}", selection.binary_name))?;

    let sidecars_dir = sidecars_dir(app)?;
    let downloads_dir = sidecars_dir.join("downloads");
    tokio::fs::create_dir_all(&downloads_dir)
        .await
        .map_err(|error| format!("创建下载目录失败: {error}"))?;
    let archive_path = downloads_dir.join(&binary.name);
    download_dependency_archive(app, "llama.cpp", &binary.browser_download_url, &archive_path)
        .await?;

    let target_dir = sidecars_dir.join("llama.cpp");
    let path = extract_llama_archive(app, &binary.name, &archive_path, &target_dir).await?;

    if let Some(cudart_name) = selection.cudart_name {
        if let Some(cudart) = find_asset(&release, &cudart_name) {
            let cudart_path = downloads_dir.join(&cudart.name);
            download_dependency_archive(
                app,
                "CUDA Runtime",
                &cudart.browser_download_url,
                &cudart_path,
            )
            .await?;
            extract_zip_into_existing(app, "CUDA Runtime", &cudart_path, &target_dir).await?;
            let _ = tokio::fs::remove_file(&cudart_path).await;
        }
    }

    let _ = tokio::fs::remove_file(&archive_path).await;
    emit_dependency_install(
        app,
        "llama.cpp",
        "completed",
        format!("llama.cpp 已安装（{}）", selection.backend),
        1.0,
        Some(path.clone()),
        None,
    );
    Ok(path)
}

#[cfg(not(target_os = "macos"))]
async fn extract_llama_archive(
    app: &AppHandle,
    _archive_name: &str,
    archive_path: &Path,
    target_dir: &Path,
) -> Result<String, String> {
    extract_dependency_archive(
        "llama.cpp",
        "llama-server.exe",
        app,
        archive_path,
        target_dir,
    )
    .await
}

#[cfg(target_os = "macos")]
async fn extract_llama_archive(
    app: &AppHandle,
    archive_name: &str,
    archive_path: &Path,
    target_dir: &Path,
) -> Result<String, String> {
    if std::env::consts::ARCH != "aarch64" {
        return Err("macOS 版本仅支持 Apple Silicon (arm64)，不支持 Intel Mac".to_string());
    }
    if archive_name.ends_with(".zip") {
        return Err("macOS 需要 tar.gz 格式的官方 llama.cpp 包".to_string());
    }
    let staging_dir = target_dir.with_extension("installing");
    let _ = tokio::fs::remove_dir_all(&staging_dir).await;
    tokio::fs::create_dir_all(&staging_dir)
        .await
        .map_err(|error| format!("创建 llama.cpp 解压目录失败: {error}"))?;
    extract_tar_archive(app, "llama.cpp", archive_path, &staging_dir, 0.9).await?;
    let exe_path = find_file_recursive(&staging_dir, "llama-server")
        .ok_or_else(|| "llama.cpp 发布包里没有找到 llama-server".to_string())?;
    ensure_executable(&exe_path).await?;
    let relative_exe = exe_path
        .strip_prefix(&staging_dir)
        .map_err(|error| format!("定位 llama-server 失败: {error}"))?
        .to_path_buf();
    let _ = tokio::fs::remove_dir_all(target_dir).await;
    tokio::fs::rename(&staging_dir, target_dir)
        .await
        .map_err(|error| format!("保存 llama.cpp 失败: {error}"))?;
    Ok(path_to_string(target_dir.join(relative_exe)))
}

#[cfg(not(target_os = "macos"))]
async fn extract_zip_into_existing(
    app: &AppHandle,
    item: &str,
    archive_path: &Path,
    target_dir: &Path,
) -> Result<(), String> {
    use super::install::extract_zip_into_dir;
    emit_dependency_install(
        app,
        item,
        "running",
        format!("正在解压 {item}"),
        0.94,
        None,
        None,
    );
    let archive_path = archive_path.to_path_buf();
    let target_dir = target_dir.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || extract_zip_into_dir(&archive_path, &target_dir))
        .await
        .map_err(|error| format!("解压 {item} 任务失败: {error}"))?
        .map_err(|error| format!("解压 {item} 失败: {error}"))
}

#[cfg(target_os = "macos")]
async fn extract_zip_into_existing(
    _app: &AppHandle,
    _item: &str,
    _archive_path: &Path,
    _target_dir: &Path,
) -> Result<(), String> {
    Ok(())
}

async fn latest_llama_cpp_release() -> Result<GithubRelease, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|error| format!("创建 GitHub 客户端失败: {error}"))?;
    let releases = client
        .get(LLAMA_RELEASE_API_URL)
        .query(&[("per_page", "8")])
        .send()
        .await
        .map_err(|error| format!("查询 llama.cpp 发布包失败: {error}"))?
        .error_for_status()
        .map_err(|error| format!("查询 llama.cpp 发布包失败: {error}"))?
        .json::<Vec<GithubRelease>>()
        .await
        .map_err(|error| format!("解析 llama.cpp 发布包失败: {error}"))?;
    releases
        .into_iter()
        .find(|release| !release.assets.is_empty())
        .ok_or_else(|| "未找到包含安装包的 llama.cpp 发布".to_string())
}

fn find_asset<'a>(release: &'a GithubRelease, name: &str) -> Option<&'a GithubAsset> {
    release.assets.iter().find(|asset| asset.name == name)
}

fn current_install_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

pub(super) fn select_llama_cpp_assets(
    available: &[&str],
    os: &str,
    arch: &str,
    has_nvidia: bool,
) -> Option<LlamaCppAssetSelection> {
    match (os, arch) {
        ("macos", "aarch64") => first_matching(available, macos_arm64_binary_suffixes()).map(
            |binary_name| LlamaCppAssetSelection {
                binary_name,
                cudart_name: None,
                backend: "Metal",
            },
        ),
        ("windows", "x86_64") => select_windows_x64_assets(available, has_nvidia),
        ("windows", "aarch64") => first_matching(available, &["bin-win-cpu-arm64.zip"]).map(
            |binary_name| LlamaCppAssetSelection {
                binary_name,
                cudart_name: None,
                backend: "CPU",
            },
        ),
        _ => None,
    }
}

fn select_windows_x64_assets(
    available: &[&str],
    has_nvidia: bool,
) -> Option<LlamaCppAssetSelection> {
    if has_nvidia {
        if let Some(binary_name) = first_matching(available, windows_cuda_binary_suffixes()) {
            let cudart_name = matching_cudart(available, &binary_name);
            return Some(LlamaCppAssetSelection {
                binary_name,
                cudart_name,
                backend: "CUDA",
            });
        }
    }
    if let Some(binary_name) = first_matching(available, &["bin-win-vulkan-x64.zip"]) {
        return Some(LlamaCppAssetSelection {
            binary_name,
            cudart_name: None,
            backend: "Vulkan",
        });
    }
    first_matching(available, &["bin-win-cpu-x64.zip"]).map(|binary_name| LlamaCppAssetSelection {
        binary_name,
        cudart_name: None,
        backend: "CPU",
    })
}

fn windows_cuda_binary_suffixes() -> &'static [&'static str] {
    &[
        "bin-win-cuda-12.4-x64.zip",
        "bin-win-cuda-13.3-x64.zip",
        "bin-win-cuda-13.4-x64.zip",
        "bin-win-cuda-12-x64.zip",
    ]
}

fn macos_arm64_binary_suffixes() -> &'static [&'static str] {
    &["bin-macos-arm64.tar.gz"]
}

fn first_matching(available: &[&str], suffixes: &[&str]) -> Option<String> {
    suffixes.iter().find_map(|suffix| {
        available
            .iter()
            .copied()
            .find(|name| is_llama_binary_asset(name, suffix))
            .map(str::to_string)
    })
}

fn is_llama_binary_asset(name: &str, suffix: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(&suffix.to_ascii_lowercase())
        && !lower.contains("cudart")
        && !lower.contains("kleidi")
}

fn matching_cudart(available: &[&str], binary_name: &str) -> Option<String> {
    let marker = binary_name
        .split("bin-win-")
        .nth(1)
        .map(|value| format!("cudart-llama-bin-win-{value}"))?;
    available
        .iter()
        .copied()
        .find(|name| name.eq_ignore_ascii_case(&marker))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::select_llama_cpp_assets;

    const SAMPLE_ASSETS: &[&str] = &[
        "cudart-llama-bin-win-cuda-12.4-x64.zip",
        "cudart-llama-bin-win-cuda-13.3-x64.zip",
        "llama-b11243-bin-macos-arm64.tar.gz",
        "llama-b11243-bin-macos-arm64-kleidiai.tar.gz",
        "llama-b11243-bin-macos-x64.tar.gz",
        "llama-b11243-bin-win-cpu-arm64.zip",
        "llama-b11243-bin-win-cpu-x64.zip",
        "llama-b11243-bin-win-cuda-12.4-x64.zip",
        "llama-b11243-bin-win-cuda-13.3-x64.zip",
        "llama-b11243-bin-win-vulkan-x64.zip",
    ];

    #[test]
    fn selects_cuda_12_and_matching_cudart_on_nvidia_windows() {
        let selected = select_llama_cpp_assets(SAMPLE_ASSETS, "windows", "x86_64", true)
            .expect("windows cuda asset");
        assert_eq!(selected.backend, "CUDA");
        assert_eq!(
            selected.binary_name,
            "llama-b11243-bin-win-cuda-12.4-x64.zip"
        );
        assert_eq!(
            selected.cudart_name.as_deref(),
            Some("cudart-llama-bin-win-cuda-12.4-x64.zip")
        );
    }

    #[test]
    fn selects_vulkan_then_cpu_when_windows_has_no_nvidia() {
        let selected = select_llama_cpp_assets(SAMPLE_ASSETS, "windows", "x86_64", false)
            .expect("windows vulkan asset");
        assert_eq!(selected.backend, "Vulkan");
        assert_eq!(selected.binary_name, "llama-b11243-bin-win-vulkan-x64.zip");
        assert!(selected.cudart_name.is_none());
    }

    #[test]
    fn selects_cpu_when_windows_has_neither_cuda_nor_vulkan() {
        let assets = ["llama-b1-bin-win-cpu-x64.zip"];
        let selected =
            select_llama_cpp_assets(&assets, "windows", "x86_64", true).expect("cpu fallback");
        assert_eq!(selected.backend, "CPU");
        assert_eq!(selected.binary_name, "llama-b1-bin-win-cpu-x64.zip");
    }

    #[test]
    fn selects_official_metal_tarball_on_apple_silicon() {
        let selected = select_llama_cpp_assets(SAMPLE_ASSETS, "macos", "aarch64", false)
            .expect("macos metal asset");
        assert_eq!(selected.backend, "Metal");
        assert_eq!(selected.binary_name, "llama-b11243-bin-macos-arm64.tar.gz");
        assert!(selected.cudart_name.is_none());
    }

    #[test]
    fn rejects_intel_mac_and_linux_in_this_app() {
        assert!(select_llama_cpp_assets(SAMPLE_ASSETS, "macos", "x86_64", false).is_none());
        assert!(select_llama_cpp_assets(SAMPLE_ASSETS, "linux", "x86_64", true).is_none());
    }
}
