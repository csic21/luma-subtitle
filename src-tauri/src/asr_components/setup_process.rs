//! Private setup processes receive only required OS state and app-owned paths.
use super::store;
use std::{path::{Path, PathBuf}, ffi::OsString};

pub(super) struct PrivateTemp { path: PathBuf }
impl PrivateTemp {
    pub(super) fn create(root: &Path) -> Result<Self, String> {
        let parent = root.parent().ok_or("Private setup root has no owned parent")?;
        store::ensure_directory(parent)?;
        let path = parent.join(format!(".setup-temp-{}", uuid::Uuid::new_v4()));
        store::create_private_dir(&path)?; Ok(Self { path })
    }
}
impl Drop for PrivateTemp {
    fn drop(&mut self) { if store::ensure_directory(&self.path).is_ok() { let _ = std::fs::remove_dir_all(&self.path); } }
}
pub(super) fn preserved_os_key(key: &str) -> bool {
    matches!(key.to_ascii_uppercase().as_str(), "SYSTEMROOT" | "WINDIR" | "COMSPEC" | "SYSTEMDRIVE" | "PROCESSOR_ARCHITECTURE" | "PROCESSOR_ARCHITEW6432" | "NUMBER_OF_PROCESSORS")
}
pub(super) fn configure(command: &mut tokio::process::Command, root: &Path, temporary: &PrivateTemp) -> Result<(), String> {
    command.env_clear();
    let mut system_root: Option<OsString> = None;
    if cfg!(windows) {
        for (key, value) in std::env::vars_os() {
            if preserved_os_key(&key.to_string_lossy()) {
                if key.to_string_lossy().eq_ignore_ascii_case("SYSTEMROOT") { system_root = Some(value.clone()); }
                command.env(key, value);
            }
        }
    }
    let mut path = vec![root.to_path_buf(), root.join("bin")];
    if let Some(system_root) = system_root { path.push(PathBuf::from(system_root).join("System32")); }
    command.env("PATH", std::env::join_paths(path).map_err(|e| e.to_string())?)
        .env("HOME", &temporary.path).env("USERPROFILE", &temporary.path)
        .env("APPDATA", &temporary.path).env("LOCALAPPDATA", &temporary.path)
        .env("TMP", &temporary.path).env("TEMP", &temporary.path).env("TMPDIR", &temporary.path)
        .env("HF_HUB_OFFLINE", "1").env("TRANSFORMERS_OFFLINE", "1").env("HF_DATASETS_OFFLINE", "1")
        .env("HF_HUB_DISABLE_IMPLICIT_TOKEN", "1").env("HF_HUB_DISABLE_TELEMETRY", "1")
        .env("DO_NOT_TRACK", "1").env("LANG", "C.UTF-8");
    Ok(())
}
