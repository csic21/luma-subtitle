//! Local corruption/version evidence, not a new trust root. A receipt is written
//! only in private staging after every archive passed the embedded download pins
//! and bounded extraction. It does not authenticate files against a hostile user
//! who can replace both this receipt and the installed files.
use super::trust::{self, Artifact};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const RECEIPT_NAME: &str = ".luma-llama-receipt.json";
const MAX_RECEIPT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u32,
    os: String,
    arch: String,
    backend: String,
    artifacts: Vec<ArtifactIdentity>,
    executable: String,
    directories: Vec<String>,
    files: Vec<FileRecord>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ArtifactIdentity {
    id: String,
    version: String,
    file_name: String,
    bytes: u64,
    sha256: String,
}
impl From<&Artifact> for ArtifactIdentity {
    fn from(item: &Artifact) -> Self {
        Self {
            id: item.id.clone(),
            version: item.version.clone(),
            file_name: item.file_name.clone(),
            bytes: item.bytes,
            sha256: item.sha256.to_ascii_lowercase(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FileRecord {
    path: String,
    bytes: u64,
    sha256: String,
}

pub(super) struct VerifiedBundle {
    pub executable: PathBuf,
    pub backend: &'static str,
}

struct Limits {
    files: usize,
    entries: usize,
    bytes: u64,
}

/// Call only after the pinned downloads and extraction have succeeded. This and
/// validation do blocking filesystem work and belong on spawn_blocking workers.
pub(super) fn write_verified_receipt(
    root: &Path,
    artifacts: &[Artifact],
    os: &str,
    arch: &str,
) -> Result<VerifiedBundle, String> {
    let (backend, limits) = bundle_spec(artifacts, os, arch)?;
    let (directories, files) = scan_bundle(root, &limits, false)?;
    let executable = executable_path(&files, os)?;
    let receipt = Receipt {
        schema: 1,
        os: os.to_owned(),
        arch: arch.to_owned(),
        backend: backend.to_owned(),
        artifacts: artifacts.iter().map(ArtifactIdentity::from).collect(),
        executable: executable.clone(),
        directories,
        files,
    };
    let bytes = serde_json::to_vec(&receipt).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err("llama.cpp receipt exceeded its byte limit".into());
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(root.join(RECEIPT_NAME)).map_err(|e| e.to_string())?;
    file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
    Ok(VerifiedBundle { executable: root.join(executable), backend })
}

/// A missing, stale, malformed or damaged bundle must be repaired from reviewed
/// archives. Never follow receipt paths to hash files; inventory the bounded tree
/// independently, then require an exact match including every DLL and directory.
pub(super) fn validate_installed(
    root: &Path,
    catalog: &[Artifact],
    os: &str,
    arch: &str,
) -> Result<VerifiedBundle, String> {
    checked_metadata(root, true)?;
    let receipt_path = root.join(RECEIPT_NAME);
    let metadata = checked_metadata(&receipt_path, false)?;
    if metadata.len() == 0 || metadata.len() > MAX_RECEIPT_BYTES {
        return Err("Invalid llama.cpp receipt size".into());
    }
    let mut bytes = Vec::new();
    File::open(receipt_path)
        .map_err(|e| e.to_string())?
        .take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 != metadata.len() || bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err("llama.cpp receipt changed or exceeded its limit".into());
    }
    let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if receipt.schema != 1 || receipt.os != os || receipt.arch != arch
        || receipt.artifacts.is_empty() || receipt.artifacts.len() > 2
    {
        return Err("Unsupported llama.cpp receipt/platform".into());
    }
    let artifacts = receipt.artifacts.iter().map(|identity| {
        let item = catalog.iter().find(|item| item.id == identity.id)
            .ok_or("llama.cpp receipt artifact is no longer pinned")?;
        if ArtifactIdentity::from(item) != *identity {
            return Err("llama.cpp receipt does not match the current pinned version/hash");
        }
        Ok(item.clone())
    }).collect::<Result<Vec<_>, _>>()?;
    let (backend, limits) = bundle_spec(&artifacts, os, arch)?;
    if receipt.backend != backend || receipt.files.is_empty()
        || receipt.files.len() > limits.files
        || receipt.directories.len().saturating_add(receipt.files.len()) > limits.entries
    {
        return Err("Invalid llama.cpp receipt inventory/backend".into());
    }
    let mut paths = HashSet::new();
    let mut total = 0u64;
    for directory in &receipt.directories {
        validate_record_path(directory, &mut paths)?;
    }
    for file in &receipt.files {
        validate_record_path(&file.path, &mut paths)?;
        if file.sha256.len() != 64 || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid llama.cpp file digest".into());
        }
        total = total.checked_add(file.bytes).ok_or("llama.cpp receipt size overflow")?;
        if total > limits.bytes {
            return Err("llama.cpp receipt exceeds reviewed bundle bounds".into());
        }
    }
    let executable = executable_path(&receipt.files, os)?;
    if receipt.executable != executable {
        return Err("Invalid llama.cpp executable receipt path".into());
    }
    let (directories, files) = scan_bundle(root, &limits, true)?;
    if receipt.directories != directories || receipt.files != files {
        return Err("llama.cpp bundle is missing, changed, or contains extra files".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if checked_metadata(&root.join(&executable), false)?.permissions().mode() & 0o111 == 0 {
            return Err("llama.cpp server is no longer executable".into());
        }
    }
    Ok(VerifiedBundle { executable: root.join(executable), backend })
}

fn bundle_spec(artifacts: &[Artifact], os: &str, arch: &str) -> Result<(&'static str, Limits), String> {
    let first = artifacts.first().ok_or("Missing llama.cpp binary artifact")?;
    let backend = match (os, arch, first.id.as_str()) {
        ("macos", "aarch64", "llama-metal") => "Metal",
        ("windows", "aarch64", "llama-cpu-arm64") => "CPU",
        ("windows", "x86_64", "llama-cpu") => "CPU",
        // Preserve a fully verified existing backend even if a later GPU probe
        // fails. Fresh non-NVIDIA installs choose CPU in the asset selector.
        ("windows", "x86_64", "llama-vulkan") => "Vulkan",
        ("windows", "x86_64", "llama-cuda") => "CUDA",
        _ => return Err("Unsupported llama.cpp receipt artifact/platform".into()),
    };
    if backend == "CUDA" {
        if artifacts.len() != 2 || artifacts[1].id != "llama-cudart"
            || artifacts[1].version != first.version
            || super::llama::matching_cudart(&[artifacts[1].file_name.as_str()], &first.file_name).is_none()
        {
            return Err("llama.cpp CUDA bundle requires the matching pinned runtime".into());
        }
    } else if artifacts.len() != 1 {
        return Err("Unexpected llama.cpp receipt artifact".into());
    }
    let mut limits = Limits { files: 0, entries: 0, bytes: 0 };
    for artifact in artifacts {
        trust::validate_artifact(artifact)?;
        if artifact.file_limit == 0 || artifact.unpacked_bytes == 0 {
            return Err("Missing llama.cpp extraction bounds".into());
        }
        limits.files = limits.files.checked_add(artifact.file_limit).ok_or("llama.cpp file limit overflow")?;
        limits.bytes = limits.bytes.checked_add(artifact.unpacked_bytes).ok_or("llama.cpp byte limit overflow")?;
    }
    limits.entries = limits.files.checked_mul(4).and_then(|n| n.checked_add(1024))
        .ok_or("llama.cpp entry limit overflow")?;
    Ok((backend, limits))
}

fn validate_record_path(path: &str, seen: &mut HashSet<String>) -> Result<(), String> {
    crate::asr_components::legacy_relative_path(path)?;
    if path.ends_with('/') || path.eq_ignore_ascii_case(RECEIPT_NAME)
        || !seen.insert(path.to_lowercase())
    {
        return Err("Duplicate, reserved or ambiguous llama.cpp receipt path".into());
    }
    Ok(())
}

fn checked_metadata(path: &Path, directory: bool) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let wrong_type = if directory { !metadata.is_dir() } else { !metadata.is_file() };
    if metadata.file_type().is_symlink() || wrong_type {
        return Err("llama.cpp bundle contains a link or unexpected file type".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("llama.cpp bundle contains a Windows reparse point".into());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !directory && metadata.nlink() != 1 {
            return Err("llama.cpp bundle contains a hard-linked file".into());
        }
    }
    Ok(metadata)
}

fn scan_bundle(root: &Path, limits: &Limits, allow_receipt: bool) -> Result<(Vec<String>, Vec<FileRecord>), String> {
    checked_metadata(root, true)?;
    let mut stack = vec![root.to_path_buf()];
    let mut seen = HashSet::new();
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut entries = 0usize;
    let mut total = 0u64;
    // Heap-backed even though this function is blocking: never inflate a native
    // installer command future or a platform worker's stack with a large array.
    let mut buffer = vec![0u8; 64 * 1024];
    while let Some(directory) = stack.pop() {
        checked_metadata(&directory, true)?;
        for entry in fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            entries = entries.checked_add(1).ok_or("llama.cpp entry count overflow")?;
            if entries > limits.entries + allow_receipt as usize {
                return Err("llama.cpp bundle exceeds reviewed entry bounds".into());
            }
            let path = entry.path();
            let relative = path.strip_prefix(root).map_err(|e| e.to_string())?
                .iter().map(|part| part.to_str().ok_or("llama.cpp bundle paths must be UTF-8"))
                .collect::<Result<Vec<_>, _>>()?.join("/");
            if relative == RECEIPT_NAME && allow_receipt {
                checked_metadata(&path, false)?;
                continue;
            }
            validate_record_path(&relative, &mut seen)?;
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if metadata.is_dir() {
                checked_metadata(&path, true)?;
                directories.push(relative);
                stack.push(path);
                continue;
            }
            let metadata = checked_metadata(&path, false)?;
            if files.len() >= limits.files {
                return Err("llama.cpp bundle exceeds reviewed file bounds".into());
            }
            total = total.checked_add(metadata.len()).ok_or("llama.cpp bundle size overflow")?;
            if total > limits.bytes {
                return Err("llama.cpp bundle exceeds reviewed byte bounds".into());
            }
            let mut input = File::open(&path).map_err(|e| e.to_string())?;
            let mut bytes = 0u64;
            let mut digest = Sha256::new();
            loop {
                let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
                if count == 0 { break; }
                bytes = bytes.checked_add(count as u64).ok_or("llama.cpp file size overflow")?;
                if bytes > metadata.len() {
                    return Err("llama.cpp file changed while hashing".into());
                }
                digest.update(&buffer[..count]);
            }
            if bytes != metadata.len() {
                return Err("llama.cpp file changed while hashing".into());
            }
            files.push(FileRecord { path: relative, bytes, sha256: format!("{:x}", digest.finalize()) });
        }
    }
    directories.sort();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((directories, files))
}

fn executable_path(files: &[FileRecord], os: &str) -> Result<String, String> {
    let name = if os == "macos" { "llama-server" } else { "llama-server.exe" };
    let mut matches = files.iter().filter(|file| file.path.rsplit('/').next() == Some(name));
    let executable = matches.next().ok_or("Reviewed llama.cpp bundle is missing llama-server")?;
    if matches.next().is_some() || executable.bytes == 0 {
        return Err("llama.cpp bundle has an ambiguous or empty server".into());
    }
    Ok(executable.path.clone())
}

#[cfg(all(test, not(feature = "native-install-regression")))]
mod tests {
    use super::*;
    use super::super::install::{activate_directory_preserving_previous, StagedDirectory};

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("luma-llama-receipt-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }
    fn artifact(id: &str) -> Artifact {
        let mut artifact = trust::artifact(id).unwrap();
        artifact.unpacked_bytes = 1024;
        artifact.file_limit = 8;
        artifact
    }
    fn create_bundle(root: &Path) {
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("bin/llama-server.exe"), b"fake server; never execute").unwrap();
        fs::write(root.join("bin/ggml-base.dll"), b"fake DLL").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.join("bin/llama-server.exe"), fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    fn healthy() -> (Temp, Vec<Artifact>) {
        let temp = Temp::new();
        create_bundle(&temp.0);
        let artifacts = vec![artifact("llama-cpu")];
        write_verified_receipt(&temp.0, &artifacts, "windows", "x86_64").unwrap();
        (temp, artifacts)
    }
    fn change_receipt(root: &Path, change: impl FnOnce(&mut Receipt)) {
        let path = root.join(RECEIPT_NAME);
        let mut receipt: Receipt = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        change(&mut receipt);
        fs::write(path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    }
    fn invalid(root: &Path, artifacts: &[Artifact]) -> bool {
        validate_installed(root, artifacts, "windows", "x86_64").is_err()
    }

    #[test]
    fn healthy_current_bundle_passes_complete_file_hash_validation() {
        let (temp, artifacts) = healthy();
        let bundle = validate_installed(&temp.0, &artifacts, "windows", "x86_64").unwrap();
        assert_eq!(bundle.backend, "CPU");
        assert_eq!(bundle.executable, temp.0.join("bin/llama-server.exe"));
        assert!(validate_installed(&temp.0, &artifacts, "windows", "aarch64").is_err());
    }

    #[test]
    fn stale_version_and_changed_archive_pin_require_repair() {
        let (temp, artifacts) = healthy();
        change_receipt(&temp.0, |r| r.artifacts[0].version = "b1".into());
        assert!(invalid(&temp.0, &artifacts));
        let (temp, mut artifacts) = healthy();
        artifacts[0].sha256 = "0".repeat(64);
        assert!(invalid(&temp.0, &artifacts));
    }

    #[test]
    fn corrupt_server_corrupt_dll_and_missing_dll_require_repair() {
        for file in ["bin/llama-server.exe", "bin/ggml-base.dll"] {
            let (temp, artifacts) = healthy();
            let bytes = fs::read(temp.0.join(file)).unwrap();
            fs::write(temp.0.join(file), vec![b'x'; bytes.len()]).unwrap();
            assert!(invalid(&temp.0, &artifacts));
        }
        let (temp, artifacts) = healthy();
        fs::remove_file(temp.0.join("bin/ggml-base.dll")).unwrap();
        assert!(invalid(&temp.0, &artifacts));
    }

    #[test]
    fn unreceipted_bundle_and_extra_files_or_directories_require_repair() {
        let temp = Temp::new();
        create_bundle(&temp.0);
        assert!(invalid(&temp.0, &[artifact("llama-cpu")]));
        let (temp, artifacts) = healthy();
        fs::write(temp.0.join("bin/unexpected.dll"), b"extra").unwrap();
        assert!(invalid(&temp.0, &artifacts));
        let (temp, artifacts) = healthy();
        fs::create_dir(temp.0.join("unexpected")).unwrap();
        assert!(invalid(&temp.0, &artifacts));
    }

    #[test]
    fn arbitrary_alias_and_duplicate_receipt_paths_are_rejected() {
        for path in ["../outside.dll", "/outside.dll", "C:/outside.dll", "bin\\outside.dll",
            "bin/../outside.dll", "bin/file.", "bin/NUL", RECEIPT_NAME, "bin/ggml-base.dll/"]
        {
            let (temp, artifacts) = healthy();
            change_receipt(&temp.0, |r| r.files[0].path = path.into());
            assert!(invalid(&temp.0, &artifacts), "accepted {path}");
        }
        let (temp, artifacts) = healthy();
        change_receipt(&temp.0, |r| r.files[0].path = r.files[1].path.to_uppercase());
        assert!(invalid(&temp.0, &artifacts));
        let (temp, artifacts) = healthy();
        change_receipt(&temp.0, |r| r.executable = "../llama-server.exe".into());
        assert!(invalid(&temp.0, &artifacts));
    }

    #[test]
    fn receipt_parser_and_tree_have_fixed_and_reviewed_bounds() {
        let (temp, artifacts) = healthy();
        let file = OpenOptions::new().write(true).open(temp.0.join(RECEIPT_NAME)).unwrap();
        file.set_len(MAX_RECEIPT_BYTES + 1).unwrap();
        assert!(invalid(&temp.0, &artifacts));
        let temp = Temp::new();
        create_bundle(&temp.0);
        let mut pin = artifact("llama-cpu");
        pin.file_limit = 1;
        assert!(write_verified_receipt(&temp.0, &[pin], "windows", "x86_64").is_err());
        let mut pin = artifact("llama-cpu");
        pin.unpacked_bytes = 1;
        assert!(write_verified_receipt(&temp.0, &[pin], "windows", "x86_64").is_err());
    }

    #[test]
    fn receipt_cannot_change_backend_or_omit_cuda_runtime_pin() {
        let (temp, artifacts) = healthy();
        change_receipt(&temp.0, |r| r.backend = "CUDA".into());
        assert!(invalid(&temp.0, &artifacts));
        let temp = Temp::new();
        create_bundle(&temp.0);
        assert!(write_verified_receipt(&temp.0, &[artifact("llama-cuda")], "windows", "x86_64").is_err());
    }

    #[test]
    fn verified_cuda_and_vulkan_receipts_preserve_backend_without_gpu_probe() {
        for (backend, artifacts) in [
            ("CUDA", vec![artifact("llama-cuda"), artifact("llama-cudart")]),
            ("Vulkan", vec![artifact("llama-vulkan")]),
        ] {
            let temp = Temp::new();
            create_bundle(&temp.0);
            if backend == "CUDA" {
                fs::write(temp.0.join("bin/cudart64_12.dll"), b"fake runtime").unwrap();
            }
            write_verified_receipt(&temp.0, &artifacts, "windows", "x86_64").unwrap();
            assert_eq!(validate_installed(&temp.0, &artifacts, "windows", "x86_64").unwrap().backend, backend);
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_server_directory_receipt_and_hardlinks_are_rejected() {
        use std::os::unix::fs::symlink;
        let external = Temp::new();
        create_bundle(&external.0);
        for file in ["bin/llama-server.exe", "bin/ggml-base.dll", RECEIPT_NAME] {
            let (temp, artifacts) = healthy();
            fs::remove_file(temp.0.join(file)).unwrap();
            symlink(external.0.join("bin/ggml-base.dll"), temp.0.join(file)).unwrap();
            assert!(invalid(&temp.0, &artifacts));
        }
        let (temp, artifacts) = healthy();
        fs::remove_dir_all(temp.0.join("bin")).unwrap();
        symlink(external.0.join("bin"), temp.0.join("bin")).unwrap();
        assert!(invalid(&temp.0, &artifacts));
        let (temp, artifacts) = healthy();
        fs::remove_file(temp.0.join("bin/ggml-base.dll")).unwrap();
        fs::hard_link(external.0.join("bin/ggml-base.dll"), temp.0.join("bin/ggml-base.dll")).unwrap();
        assert!(invalid(&temp.0, &artifacts));
    }

    #[cfg(windows)]
    #[test]
    fn junction_in_bundle_is_rejected_without_following_it() {
        let external = Temp::new();
        create_bundle(&external.0);
        let (temp, artifacts) = healthy();
        fs::remove_dir_all(temp.0.join("bin")).unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(temp.0.join("bin")).arg(external.0.join("bin"))
            .output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(invalid(&temp.0, &artifacts));
        fs::remove_dir(temp.0.join("bin")).unwrap();
    }

    #[test]
    fn blocking_receipt_worker_keeps_staging_alive_if_installer_is_dropped() {
        let temp = Temp::new();
        let staging = StagedDirectory::new(&temp.0.join("llama.cpp")).unwrap();
        create_bundle(&staging.0);
        let path = staging.0.clone();
        let stage_owner = staging.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let stage_owner = stage_owner;
            receive.recv().unwrap();
            write_verified_receipt(&stage_owner.0, &[artifact("llama-cpu")], "windows", "x86_64")
        });
        drop(staging);
        assert!(path.is_dir());
        send.send(()).unwrap();
        assert!(worker.join().unwrap().is_ok());
        assert!(!path.exists());
    }

    #[test]
    fn failed_staging_and_failed_activation_preserve_original_bytes() {
        let temp = Temp::new();
        let target = temp.0.join("llama.cpp");
        create_bundle(&target);
        fs::write(target.join("custom-model.gguf"), b"user model").unwrap();
        let staging = StagedDirectory::new(&target).unwrap();
        fs::write(staging.0.join("ggml-base.dll"), b"incomplete").unwrap();
        assert!(write_verified_receipt(&staging.0, &[artifact("llama-cpu")], "windows", "x86_64").is_err());
        drop(staging);
        assert_eq!(fs::read(target.join("custom-model.gguf")).unwrap(), b"user model");
        assert!(activate_directory_preserving_previous(&temp.0.join("missing-stage"), &target).is_err());
        assert_eq!(fs::read(target.join("custom-model.gguf")).unwrap(), b"user model");
        assert_eq!(fs::read(target.join("bin/llama-server.exe")).unwrap(), b"fake server; never execute");
    }

    #[test]
    fn successful_repair_retains_user_data_in_previous_tree() {
        let temp = Temp::new();
        let target = temp.0.join("llama.cpp");
        create_bundle(&target);
        fs::write(target.join("custom-model.gguf"), b"user model").unwrap();
        fs::write(target.join("settings.json"), b"user config").unwrap();
        let artifacts = vec![artifact("llama-cpu")];
        let staging = StagedDirectory::new(&target).unwrap();
        create_bundle(&staging.0);
        write_verified_receipt(&staging.0, &artifacts, "windows", "x86_64").unwrap();
        activate_directory_preserving_previous(&staging.0, &target).unwrap();
        validate_installed(&target, &artifacts, "windows", "x86_64").unwrap();
        let previous = fs::read_dir(&temp.0).unwrap().map(|e| e.unwrap().path())
            .find(|path| path.extension().and_then(|v| v.to_str()) == Some("previous")).unwrap();
        assert_eq!(fs::read(previous.join("custom-model.gguf")).unwrap(), b"user model");
        assert_eq!(fs::read(previous.join("settings.json")).unwrap(), b"user config");
    }
}
