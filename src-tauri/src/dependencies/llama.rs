use std::path::Path;

use tauri::AppHandle;

use crate::paths::find_file_recursive;
use crate::paths::{locate_binary, path_to_string, sidecars_dir};

use super::{
    download::{download_dependency_archive, PartialFile},
    events::emit_dependency_install,
    install::{activate_directory, ensure_executable, extract_zip_into_dir, StagedDirectory},
    trust,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LlamaCppAssetSelection {
    pub binary_name: String,
    pub cudart_name: Option<String>,
    pub backend: &'static str,
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
        "正在准备已固定版本的 llama.cpp 官方发布包",
        0.0,
        None,
        None,
    );
    let artifacts = trust::catalog()?
        .into_iter()
        .filter(|item| item.id.starts_with("llama-"))
        .collect::<Vec<_>>();
    let asset_names = artifacts
        .iter()
        .map(|item| item.file_name.as_str())
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
    let binary = artifacts
        .iter()
        .find(|item| item.file_name == selection.binary_name)
        .ok_or_else(|| format!("未找到 llama.cpp 包 {}", selection.binary_name))?;

    let sidecars_dir = sidecars_dir(app)?;
    let downloads_dir = sidecars_dir.join("downloads");
    tokio::fs::create_dir_all(&downloads_dir)
        .await
        .map_err(|error| format!("创建下载目录失败: {error}"))?;
    let archive_path = downloads_dir.join(format!(
        "{}.{}.part",
        binary.file_name,
        uuid::Uuid::new_v4()
    ));
    let _cleanup = PartialFile(archive_path.clone());
    download_dependency_archive(app, "llama.cpp", binary, &archive_path).await?;

    // Verify both CUDA payloads before extracting or replacing any active files.
    let cudart = selection
        .cudart_name
        .as_ref()
        .map(|name| {
            artifacts
                .iter()
                .find(|item| &item.file_name == name)
                .ok_or("Missing reviewed CUDA runtime")
        })
        .transpose()?;
    let cudart_download = if let Some(cudart) = cudart {
        let path = downloads_dir.join(format!(
            "{}.{}.part",
            cudart.file_name,
            uuid::Uuid::new_v4()
        ));
        let cleanup = PartialFile(path.clone());
        download_dependency_archive(app, "CUDA Runtime", cudart, &path).await?;
        Some((cleanup, cudart.clone()))
    } else {
        None
    };
    let target_dir = sidecars_dir.join("llama.cpp");
    let staging = StagedDirectory::new(&target_dir)?;
    let stage_owner = staging.clone();
    let source = archive_path.clone();
    let artifact = binary.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if artifact.file_name.ends_with(".tar.gz") {
            super::archive::extract_tar(&source, &stage_owner.0, &artifact)
        } else {
            extract_zip_into_dir(&source, &stage_owner.0, &artifact)
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Some((path, artifact)) = cudart_download {
        let stage_owner = staging.clone();
        let source = path.0.clone();
        tauri::async_runtime::spawn_blocking(move || {
            extract_zip_into_dir(&source, &stage_owner.0, &artifact)
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    let executable = if cfg!(target_os = "macos") {
        "llama-server"
    } else {
        "llama-server.exe"
    };
    let staged_exe = find_file_recursive(&staging.0, executable)
        .ok_or("Reviewed llama.cpp archive is missing llama-server")?;
    ensure_executable(&staged_exe).await?;
    let relative = staged_exe
        .strip_prefix(&staging.0)
        .map_err(|e| e.to_string())?
        .to_path_buf();
    activate_directory(&staging.0, &target_dir)?;
    let path = path_to_string(target_dir.join(relative));
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
        ("macos", "aarch64") => {
            first_matching(available, macos_arm64_binary_suffixes()).map(|binary_name| {
                LlamaCppAssetSelection {
                    binary_name,
                    cudart_name: None,
                    backend: "Metal",
                }
            })
        }
        ("windows", "x86_64") => select_windows_x64_assets(available, has_nvidia),
        ("windows", "aarch64") => {
            first_matching(available, &["bin-win-cpu-arm64.zip"]).map(|binary_name| {
                LlamaCppAssetSelection {
                    binary_name,
                    cudart_name: None,
                    backend: "CPU",
                }
            })
        }
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
