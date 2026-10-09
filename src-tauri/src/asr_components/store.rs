use super::{archive::relative_path, catalog::Component, ComponentStatus};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::{self, OpenOptions}, io::{Read, Write}, path::{Path, PathBuf}, sync::atomic::AtomicBool};
use uuid::Uuid;

const OWNER: &str = "luma-subtitle-managed-asr-v1\n";
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileReceipt { pub path: String, pub bytes: u64, pub sha256: String }
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    pub schema: u32, pub id: String, pub version: String, pub kind: String,
    pub fingerprint: String, pub files: Vec<FileReceipt>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Activation { schema: u32, id: String, directory: Option<String>, receipt_sha256: Option<String> }

pub(super) fn create_private_dir(path: &Path) -> Result<(), String> {
    if path.exists() || fs::symlink_metadata(path).is_ok() { return ensure_directory(path); }
    if let Some(parent) = path.parent() { create_private_dir(parent)?; }
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
    match builder.create(path) { Ok(()) => Ok(()), Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ensure_directory(path), Err(e) => Err(e.to_string()) }
}
pub(super) fn ensure_directory(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() { return Err("Managed component directory was replaced by a link or non-directory.".into()); }
    reject_reparse(&meta)
}
fn reject_reparse(meta: &fs::Metadata) -> Result<(), String> {
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 { return Err("Managed component paths may not use Windows reparse points.".into()); }
    }
    let _ = meta;
    Ok(())
}
pub(super) fn regular_file(path: &Path) -> Result<fs::Metadata, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() { return Err("Managed component file was replaced by a link or special file.".into()); }
    reject_reparse(&meta)?;
    Ok(meta)
}
pub(super) fn checked_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = relative_path(relative)?;
    let mut current = root.to_path_buf(); ensure_directory(&current)?;
    let parts: Vec<_> = relative.iter().collect();
    for (index, part) in parts.iter().enumerate() {
        current.push(part);
        if index + 1 != parts.len() { ensure_directory(&current)?; }
    }
    Ok(current)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new(); options.create_new(true).write(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())
}
fn owned_dir(path: &Path) -> Result<(), String> {
    create_private_dir(path)?;
    let marker = path.join(".owned-by-luma");
    if marker.exists() { regular_file(&marker)?; if fs::read_to_string(&marker).map_err(|e| e.to_string())? != OWNER { return Err("Managed component ownership marker does not match.".into()); } }
    else { write_new(&marker, OWNER.as_bytes())?; }
    Ok(())
}
fn verify_owned(path: &Path) -> Result<(), String> {
    ensure_directory(path)?;
    let marker = path.join(".owned-by-luma"); regular_file(&marker)?;
    if fs::read_to_string(marker).map_err(|e| e.to_string())? != OWNER { return Err("Managed component ownership marker does not match.".into()); }
    Ok(())
}
/// OS-backed lock is released even after a process crash. It prevents two app
/// instances from recovering/deleting one another's active staging directories.
pub(super) fn acquire_setup_lease(root: &Path) -> Result<fs::File, String> {
    verify_owned(root)?;
    let path = root.join(".setup.lock");
    if fs::symlink_metadata(&path).is_ok() { regular_file(&path)?; }
    let mut options = OpenOptions::new(); options.create(true).read(true).write(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let file = options.open(path).map_err(|e| e.to_string())?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| "Another app instance is changing engine components. Wait for it to finish or close it, then retry.")?;
    Ok(file)
}
pub(super) fn acquire_use_lease(root: &Path, exclusive: bool) -> Result<fs::File, String> {
    verify_owned(root)?;
    let path = root.join(".use.lock");
    if fs::symlink_metadata(&path).is_ok() { regular_file(&path)?; }
    let mut options = OpenOptions::new(); options.create(true).read(true).write(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let file = options.open(path).map_err(|e| e.to_string())?;
    let result = if exclusive { fs2::FileExt::try_lock_exclusive(&file) } else { fs2::FileExt::try_lock_shared(&file) };
    result.map_err(|_| "Managed engine components are in use or being removed by another app instance. Release its model or close that instance, then retry.")?;
    Ok(file)
}
pub(super) fn uses_managed_root(root: &Path, paths: &[&str]) -> Result<bool, String> {
    let canonical_root = if root.exists() { Some(fs::canonicalize(root).map_err(|e| e.to_string())?) } else { None };
    let mut managed = false;
    for value in paths.iter().filter(|v| !v.is_empty()) {
        let path = Path::new(value);
        // A missing path can be a just-removed component or a broken symlink
        // alias. Never silently downgrade it to a manual, unleased environment.
        let canonical = fs::canonicalize(path).map_err(|e| format!("Cannot resolve optional ASR path before acquiring its use lease: {e}"))?;
        let in_root = path.starts_with(root) || canonical_root.as_ref().is_some_and(|root| canonical.starts_with(root));
        if in_root {
            verify_owned(root)?;
            let canonical_root = canonical_root.as_ref().ok_or("The managed component root has disappeared")?;
            let relative = canonical.strip_prefix(canonical_root).map_err(|_| "Managed component path resolves outside the app-owned root")?;
            checked_path(root, &relative.to_string_lossy().replace('\\', "/"))?;
            managed = true;
        }
    }
    Ok(managed)
}
pub(super) fn prepare_root(root: &Path) -> Result<(), String> {
    owned_dir(root)?;
    let path = root.join(".use.lock");
    if path.exists() { regular_file(&path)?; }
    else { match write_new(&path, b"") { Ok(()) => (), Err(error) => { if regular_file(&path).is_err() { return Err(error); } } } }
    Ok(())
}
pub(super) fn snapshot_lease(root: &Path) -> Result<Option<fs::File>, String> {
    if !root.exists() { return Ok(None); }
    verify_owned(root)?;
    let path = root.join(".use.lock");
    if !path.exists() { return Ok(None); } // New, unactivated root only.
    regular_file(&path)?;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    fs2::FileExt::try_lock_shared(&file).map_err(|_| "Another app instance is removing managed components. Wait for it to finish, then refresh.")?;
    Ok(Some(file))
}
pub(super) fn prepare_component(root: &Path, component: &Component) -> Result<PathBuf, String> {
    verify_owned(root)?;
    let directory = root.join(component.id()); owned_dir(&directory)?;
    create_private_dir(&directory.join("versions"))?;
    create_private_dir(&directory.join("activations"))?;
    Ok(directory)
}
pub(super) struct Staging { pub directory: PathBuf }
impl Staging {
    pub(super) fn create(root: &Path) -> Result<Self, String> {
        verify_owned(root)?;
        let staging = root.join(".staging"); owned_dir(&staging)?;
        let directory = staging.join(Uuid::new_v4().to_string()); owned_dir(&directory)?;
        create_private_dir(&directory.join("payload"))?;
        Ok(Self { directory })
    }
    pub(super) fn payload(&self) -> PathBuf { self.directory.join("payload") }
}
impl Drop for Staging {
    fn drop(&mut self) { if verify_owned(&self.directory).is_ok() { let _ = fs::remove_dir_all(&self.directory); } }
}
pub(super) fn recover_staging(root: &Path) -> Result<(), String> {
    let staging = root.join(".staging");
    if !staging.exists() { return Ok(()); }
    verify_owned(root)?; verify_owned(&staging)?;
    for entry in fs::read_dir(staging).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if Uuid::parse_str(&entry.file_name().to_string_lossy()).is_ok() && verify_owned(&entry.path()).is_ok() {
            fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
pub(super) fn fingerprint(component: &Component) -> Result<String, String> {
    let json = match component { Component::Runtime(v) => serde_json::to_vec(v), Component::Model(v) => serde_json::to_vec(v) }.map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(json)))
}
pub(super) fn write_receipt(payload: &Path, component: &Component, files: Vec<FileReceipt>) -> Result<(), String> {
    let receipt = Receipt { schema: 1, id: component.id().into(), version: component.version().into(), kind: component.kind().into(), fingerprint: fingerprint(component)?, files };
    let bytes = serde_json::to_vec(&receipt).map_err(|e| e.to_string())?;
    write_new(&payload.join(".luma-receipt.json"), &bytes)
}
fn activation_files(component_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let root = component_dir.join("activations"); ensure_directory(&root)?;
    let mut files = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.len() == 62 && name.ends_with(".json") && name.as_bytes()[..20].iter().all(|b| b.is_ascii_digit()) && name.as_bytes()[20] == b'-' && Uuid::parse_str(&name[21..57]).is_ok() {
            regular_file(&entry.path())?; files.push(entry.path());
        }
    }
    files.sort(); Ok(files)
}
fn activate(component_dir: &Path, component: &Component, directory: Option<String>) -> Result<(), String> {
    let files = activation_files(component_dir)?;
    let previous = files.last().and_then(|p| p.file_name()).and_then(|p| p.to_str()).and_then(|s| s[..20].parse::<u64>().ok()).unwrap_or(0);
    let sequence = previous.checked_add(1).ok_or("Component activation sequence overflow")?;
    let receipt_sha256 = match &directory {
        Some(name) => Some(hash_file(&component_dir.join("versions").join(name).join(".luma-receipt.json"))?),
        None => None,
    };
    let activation = Activation { schema: 1, id: component.id().into(), directory, receipt_sha256 };
    let name = format!("{sequence:020}-{}", Uuid::new_v4());
    let pending = component_dir.join("activations").join(format!("{name}.pending"));
    write_new(&pending, &serde_json::to_vec(&activation).map_err(|e| e.to_string())?)?;
    // The new, unique filename is never an overwrite (also atomic on Windows).
    fs::rename(&pending, pending.with_extension("json")).map_err(|e| e.to_string())?;
    sync_dir(&component_dir.join("activations"));
    Ok(())
}
fn sync_dir(path: &Path) { #[cfg(unix)] { if let Ok(file) = fs::File::open(path) { let _ = file.sync_all(); } } let _ = path; }
/// The caller must finish validate_files and its self-test before crossing the
/// cancellation boundary. This final, short commit does not rehash large files.
pub(super) fn commit(root: &Path, component: &Component, staging: &Staging) -> Result<ComponentStatus, String> {
    let receipt = read_receipt(&staging.payload(), component)?;
    let total: u64 = receipt.files.iter().map(|f| f.bytes).sum();
    let cached_bytes = cache_bytes(root, component.id())?;
    let component_dir = prepare_component(root, component)?;
    let name = format!("{}--{}", component.version(), Uuid::new_v4());
    let destination = component_dir.join("versions").join(&name);
    // This rename and the receipt activation happen on the same filesystem.
    // Until the journal record commits, the previous activation remains active.
    fs::rename(staging.payload(), &destination).map_err(|e| e.to_string())?;
    sync_dir(&component_dir.join("versions"));
    activate(&component_dir, component, Some(name))?;
    Ok(ComponentStatus {
        id: component.id().into(), kind: component.kind().into(), state: "installed".into(), version: Some(component.version().into()),
        python_path: match component { Component::Runtime(runtime) => Some(destination.join(&runtime.entrypoint).to_string_lossy().into_owned()), Component::Model(_) => None },
        path: Some(destination.to_string_lossy().into_owned()), installed_bytes: total, cached_bytes, error: None,
    })
}
fn current(root: &Path, component: &Component) -> Result<Option<PathBuf>, String> {
    verify_owned(root)?;
    let directory = root.join(component.id());
    if !directory.exists() { return Ok(None); }
    verify_owned(&directory)?;
    let files = activation_files(&directory)?;
    let Some(last) = files.last() else { return Ok(None); };
    if regular_file(last)?.len() > 8192 { return Err("Oversized component activation receipt.".into()); }
    let bytes = fs::read(last).map_err(|e| e.to_string())?;
    let activation: Activation = serde_json::from_slice(&bytes).map_err(|_| "Damaged component activation receipt")?;
    if activation.schema != 1 || activation.id != component.id() { return Err("Component activation identity mismatch.".into()); }
    let Some(name) = activation.directory else { return Ok(None); };
    let Some((version, uuid)) = name.rsplit_once("--") else { return Err("Invalid component activation directory.".into()); };
    if version != component.version() || Uuid::parse_str(uuid).is_err() || relative_path(&name)?.components().count() != 1 { return Err("The installed component version differs from this app's verified catalog. Repair it to install the matching version.".into()); }
    let versions = directory.join("versions"); ensure_directory(&versions)?;
    let path = versions.join(name); ensure_directory(&path)?;
    let receipt = path.join(".luma-receipt.json");
    if regular_file(&receipt)?.len() > 32 * 1024 * 1024 || activation.receipt_sha256.as_deref() != Some(hash_file(&receipt)?.as_str()) { return Err("Component file receipt has changed since activation. Use Repair.".into()); }
    Ok(Some(path))
}
pub(super) fn read_receipt(path: &Path, component: &Component) -> Result<Receipt, String> {
    let receipt_path = path.join(".luma-receipt.json");
    if regular_file(&receipt_path)?.len() > 32 * 1024 * 1024 { return Err("Oversized component file receipt.".into()); }
    let receipt: Receipt = serde_json::from_slice(&fs::read(receipt_path).map_err(|e| e.to_string())?).map_err(|_| "Damaged component file receipt")?;
    if receipt.schema != 1 || receipt.id != component.id() || receipt.version != component.version() || receipt.kind != component.kind() || receipt.fingerprint != fingerprint(component)? || receipt.files.is_empty() || receipt.files.len() > 100_000 {
        return Err("Component receipt does not match the embedded catalog.".into());
    }
    if let Component::Model(model) = component {
        if receipt.files.len() != model.files.len() { return Err("Model receipt does not contain every pinned file.".into()); }
        for (actual, expected) in receipt.files.iter().zip(&model.files) {
            if actual.path != expected.path || actual.bytes != expected.bytes || !actual.sha256.eq_ignore_ascii_case(&expected.sha256) { return Err("Model receipt does not match the pinned model file list.".into()); }
        }
    }
    let mut paths = std::collections::HashSet::new();
    if receipt.files.iter().any(|file| !paths.insert(file.path.to_ascii_lowercase()) || file.sha256.len() != 64 || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())) { return Err("Invalid component file receipt entries.".into()); }
    Ok(receipt)
}
pub(super) fn hash_file(path: &Path) -> Result<String, String> {
    hash_file_checked(path, &AtomicBool::new(false))
}
pub(super) fn hash_file_checked(path: &Path, cancel: &AtomicBool) -> Result<String, String> {
    regular_file(path)?;
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new(); let mut buffer = [0u8; 256 * 1024];
    loop {
        super::archive::cancelled(cancel)?;
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
pub(super) fn validate_files(path: &Path, component: &Component, cancel: &AtomicBool) -> Result<u64, String> {
    let receipt = read_receipt(path, component)?;
    let mut total = 0u64;
    for file in &receipt.files {
        super::archive::cancelled(cancel)?;
        let file_path = checked_path(path, &file.path)?;
        if regular_file(&file_path)?.len() != file.bytes || !hash_file_checked(&file_path, cancel)?.eq_ignore_ascii_case(&file.sha256) { return Err("An installed component file is missing or failed SHA-256 verification. Use Repair.".into()); }
        total = total.checked_add(file.bytes).ok_or("Component size overflow")?;
        if total > component.installed_bytes() { return Err("Component receipt exceeds its size limit.".into()); }
    }
    if let Component::Runtime(runtime) = component { regular_file(&checked_path(path, &runtime.entrypoint)?)?; }
    Ok(total)
}
pub(super) fn status(root: &Path, component: &Component) -> ComponentStatus {
    status_with_cancel(root, component, &AtomicBool::new(false))
}
pub(super) fn status_with_cancel(root: &Path, component: &Component, cancel: &AtomicBool) -> ComponentStatus {
    let mut status = ComponentStatus { id: component.id().into(), kind: component.kind().into(), state: "not_installed".into(), version: None, path: None, python_path: None, installed_bytes: 0, cached_bytes: 0, error: None };
    if !root.exists() { return status; }
    let result: Result<Option<(PathBuf, u64)>, String> = (|| {
        status.cached_bytes = cache_bytes(root, component.id())?;
        let Some(path) = current(root, component)? else { return Ok(None); };
        if let Component::Runtime(runtime) = component { super::catalog::check_os(runtime.min_os_version.as_deref())?; }
        let total = validate_files(&path, component, cancel)?;
        Ok(Some((path, total)))
    })();
    match result {
        Ok(Some((path, bytes))) => {
            status.state = "installed".into(); status.version = Some(component.version().into()); status.installed_bytes = bytes;
            if let Component::Runtime(runtime) = component { status.python_path = Some(path.join(&runtime.entrypoint).to_string_lossy().into_owned()); }
            status.path = Some(path.to_string_lossy().into_owned());
        }
        Ok(None) => (),
        Err(error) => { status.state = "damaged".into(); status.error = Some(error); }
    }
    status
}
pub(super) fn remove(root: &Path, component: &Component) -> Result<(), String> {
    verify_owned(root)?;
    let directory = root.join(component.id());
    if !directory.exists() { return remove_component_cache(root, component.id()); }
    verify_owned(&directory)?;
    // An atomic tombstone prevents old receipts becoming active after removal,
    // even if the process exits during deletion. No external model paths enter here.
    activate(&directory, component, None)?;
    let versions = directory.join("versions"); ensure_directory(&versions)?;
    for entry in fs::read_dir(&versions).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some((_, uuid)) = name.rsplit_once("--") {
            if Uuid::parse_str(uuid).is_ok() && ensure_directory(&entry.path()).is_ok() {
                // Receipt verifies ownership even for retained previous versions.
                let receipt_path = entry.path().join(".luma-receipt.json");
                if regular_file(&receipt_path).is_ok_and(|meta| meta.len() <= 32 * 1024 * 1024) {
                    let bytes = fs::read(receipt_path).map_err(|e| e.to_string())?;
                    if let Ok(receipt) = serde_json::from_slice::<Receipt>(&bytes) {
                        if receipt.schema == 1 && receipt.id == component.id() && receipt.kind == component.kind() {
                            fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
        }
    }
    remove_component_cache(root, component.id())
}

pub(super) fn record_consent(root: &Path, consent: &super::recipe::Consent) -> Result<(), String> {
    verify_owned(root)?;
    if !super::catalog::safe_id(&consent.component_id) || !super::recipe::valid_hash(&consent.plan_sha256) { return Err("Invalid consent record identity.".into()); }
    let consent_root = root.join(".consents"); owned_dir(&consent_root)?;
    let directory = consent_root.join(&consent.component_id); owned_dir(&directory)?;
    if find_consent(&directory, consent)?.as_ref() == Some(consent) { return Ok(()); }
    let name = format!("{}-{}", consent.plan_sha256, Uuid::new_v4());
    let pending = directory.join(format!("{name}.pending"));
    write_new(&pending, &serde_json::to_vec(consent).map_err(|e| e.to_string())?)?;
    fs::rename(&pending, pending.with_extension("json")).map_err(|e| e.to_string())?;
    sync_dir(&directory); Ok(())
}
fn find_consent(directory: &Path, expected: &super::recipe::Consent) -> Result<Option<super::recipe::Consent>, String> {
    if !directory.exists() { return Ok(None); }
    let plan = &expected.plan_sha256;
    verify_owned(directory)?;
    let mut count = 0;
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        count += 1; if count > 10_000 { return Err("Too many local component consent records.".into()); }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&format!("{plan}-")) || !name.ends_with(".json") { continue; }
        let path = entry.path();
        if !regular_file(&path).is_ok_and(|meta| meta.len() <= 512 * 1024) { continue; }
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        if let Ok(consent) = serde_json::from_slice::<super::recipe::Consent>(&bytes) {
            if &consent == expected { return Ok(Some(consent)); }
        }
    }
    Ok(None)
}
pub(super) fn read_consents(root: &Path, catalog: &super::catalog::Catalog) -> Result<Vec<super::recipe::Consent>, String> {
    if !root.exists() || !root.join(".consents").exists() { return Ok(Vec::new()); }
    verify_owned(root)?; verify_owned(&root.join(".consents"))?;
    let mut result = Vec::new();
    for runtime in &catalog.runtimes {
        let Some(recipe) = &runtime.recipe else { continue; };
        let plan = super::recipe::plan_hash(runtime)?;
        let expected = super::recipe::Consent { component_id: runtime.id.clone(), plan_sha256: plan.clone(), terms: recipe.acknowledgements() };
        if let Some(consent) = find_consent(&root.join(".consents").join(&runtime.id), &expected)? {
            if consent.component_id == runtime.id && consent.terms == recipe.acknowledgements() { result.push(consent); }
        }
    }
    Ok(result)
}

pub(super) fn cache_directory(root: &Path, component_id: &str) -> Result<PathBuf, String> {
    verify_owned(root)?;
    if !super::catalog::safe_id(component_id) { return Err("Invalid managed cache owner.".into()); }
    let cache_root = root.join(".downloads"); owned_dir(&cache_root)?;
    // Per-component cache ownership keeps Remove precise and avoids deleting
    // artifacts needed by another engine's active installation or retry.
    let directory = cache_root.join(component_id); owned_dir(&directory)?;
    // Clean only recognized incomplete cache filenames after obtaining the
    // cross-process setup lease; valid completed hashes remain reusable.
    for entry in fs::read_dir(&directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let parts: Vec<_> = name.split('.').collect();
        if parts.len() == 3 && super::recipe::valid_hash(parts[0]) && Uuid::parse_str(parts[1]).is_ok() && parts[2] == "partial" {
            regular_file(&entry.path())?; fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
        }
    }
    Ok(directory)
}

/// Completed cache size is storage disclosure, not a reuse-integrity claim.
/// Every artifact is rehashed against the embedded recipe before reuse.
pub(super) fn cache_bytes(root: &Path, component_id: &str) -> Result<u64, String> {
    if !super::catalog::safe_id(component_id) { return Err("Invalid managed cache owner.".into()); }
    if !root.exists() { return Ok(0); }
    verify_owned(root)?;
    let cache_root = root.join(".downloads");
    if !cache_root.exists() { return Ok(0); }
    verify_owned(&cache_root)?;
    let directory = cache_root.join(component_id);
    if !directory.exists() { return Ok(0); }
    verify_owned(&directory)?;
    let mut bytes = 0u64;
    for (index, entry) in fs::read_dir(directory).map_err(|e| e.to_string())?.enumerate() {
        if index >= 10_000 { return Err("Too many managed component cache entries.".into()); }
        let entry = entry.map_err(|e| e.to_string())?;
        if super::recipe::valid_hash(&entry.file_name().to_string_lossy()) {
            bytes = bytes.checked_add(regular_file(&entry.path())?.len()).ok_or("Managed component cache size overflow")?;
        }
    }
    Ok(bytes)
}

fn remove_component_cache(root: &Path, component_id: &str) -> Result<(), String> {
    let cache_root = root.join(".downloads");
    if !cache_root.exists() { return Ok(()); }
    verify_owned(&cache_root)?;
    let directory = cache_root.join(component_id);
    if directory.exists() { verify_owned(&directory)?; fs::remove_dir_all(directory).map_err(|e| e.to_string())?; }
    Ok(())
}
