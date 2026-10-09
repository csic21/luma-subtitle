use super::*;
use sha2::{Digest, Sha256};
use std::{fs, io::Write};

struct Fixture { base: PathBuf, root: PathBuf }
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("luma-components-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let base = fs::canonicalize(base).unwrap(); let root = base.join("asr-components");
        store::prepare_root(&root).unwrap(); Self { base, root }
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.base); } }
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn model() -> Component {
    Component::Model(catalog::Model {
        id: "fixture-model".into(), version: "a".repeat(40), label: "Fixture model".into(), engine: "whisper-accelerated".into(), backend: "faster-whisper".into(), role: "model".into(), license: "MIT".into(), license_url: "https://huggingface.co/Systran/faster-whisper-tiny".into(), installed_bytes: 4,
        files: vec![catalog::ModelFile { path: "model.bin".into(), url: format!("https://huggingface.co/Systran/faster-whisper-tiny/resolve/{}/model.bin", "a".repeat(40)), bytes: 4, sha256: digest(b"good") }], unavailable_reason: None,
    })
}
fn staged(root: &Path, component: &Component) -> store::Staging {
    let staging = store::Staging::create(root).unwrap();
    fs::write(staging.payload().join("model.bin"), b"good").unwrap();
    store::write_receipt(&staging.payload(), component, vec![store::FileReceipt { path: "model.bin".into(), bytes: 4, sha256: digest(b"good") }]).unwrap();
    store::validate_files(&staging.payload(), component, &AtomicBool::new(false)).unwrap();
    staging
}
fn zip_fixture(root: &Path, names: &[&str], contents: &[u8]) -> PathBuf {
    let path = root.join(format!("{}.zip", uuid::Uuid::new_v4()));
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).unix_permissions(0o644);
    for name in names { zip.start_file(*name, options).unwrap(); zip.write_all(contents).unwrap(); }
    zip.finish().unwrap(); path
}
fn extraction(path: &Path, root: &Path, cap: u64, count: usize) -> Result<Vec<store::FileReceipt>, String> {
    let target = root.join(uuid::Uuid::new_v4().to_string()); fs::create_dir(&target).unwrap();
    archive::extract(path, &target, cap, count, &AtomicBool::new(false))
}
#[test]
fn embedded_catalog_is_valid_and_only_explicit_components_exist() {
    let catalog = catalog::embedded().unwrap();
    assert_eq!(catalog.schema, 1);
    assert!(!catalog.models.is_empty());
    assert!(catalog.components().iter().all(|c| !c.id().contains("firered")));
    assert!(catalog.find("../../models").is_err());
}
#[test]
fn runtime_origins_are_pinned_and_redirects_are_separately_limited() {
    let hash = digest(b"test");
    let valid = "https://github.com/csic21/luma-subtitle/releases/download/asr-components-1.2.0-r1/runtime.zip";
    assert!(catalog::validate_download(valid, 4, &hash, catalog::Source::Runtime).is_ok());
    for url in ["http://github.com/csic21/luma-subtitle/releases/download/asr-components-1.2.0-r1/runtime.zip", "https://evil.example/runtime.zip", "https://github.com/evil/runtime/releases/download/asr-components-1/runtime.zip", "https://github.com/csic21/luma-subtitle/releases/latest/download/runtime.zip", "file:///tmp/runtime.zip"] {
        assert!(catalog::validate_download(url, 4, &hash, catalog::Source::Runtime).is_err(), "{url}");
    }
    for url in ["http://release-assets.githubusercontent.com/a", "https://release-assets.githubusercontent.com.evil.example/a", "https://user:password@github.com/a"] {
        assert!(!catalog::allowed_redirect(&url::Url::parse(url).unwrap(), catalog::Source::Runtime));
    }
    assert!(catalog::allowed_redirect(&url::Url::parse("https://release-assets.githubusercontent.com/a?signature=x").unwrap(), catalog::Source::Runtime));
}
#[test]
fn model_download_requires_official_owner_immutable_revision_and_sha() {
    let Component::Model(model) = model() else { unreachable!() };
    let file = &model.files[0];
    assert!(catalog::validate_download(&file.url, 4, &file.sha256, catalog::Source::Model).is_ok());
    for url in [file.url.replace(&"a".repeat(40), "main"), file.url.replace("Systran", "unknown-owner"), format!("{}?token=secret", file.url)] {
        assert!(catalog::validate_download(&url, 4, &file.sha256, catalog::Source::Model).is_err());
    }
    assert!(catalog::validate_download(&file.url, 0, &file.sha256, catalog::Source::Model).is_err());
    assert!(catalog::validate_download(&file.url, 4, "missing", catalog::Source::Model).is_err());
}
#[test]
fn local_fixture_transport_rejects_truncated_oversized_hash_mismatch_and_cancelled_downloads() {
    let cancel = AtomicBool::new(false); let hash = digest(b"good");
    assert_eq!(download::fixture_download(&[b"go", b"od"], 4, &hash, &cancel).unwrap(), b"good");
    assert!(download::fixture_download(&[b"bad!"], 4, &hash, &cancel).is_err());
    assert!(download::fixture_download(&[b"goo"], 4, &hash, &cancel).is_err());
    assert!(download::fixture_download(&[b"good!"], 4, &hash, &cancel).is_err());
    cancel.store(true, Ordering::SeqCst);
    assert!(download::fixture_download(&[b"good"], 4, &hash, &cancel).is_err());
}
#[test]
fn unsafe_paths_include_traversal_absolute_drive_ads_and_windows_aliases() {
    for name in ["../evil", "a/../../evil", "/absolute", "C:/evil", "C:\\evil", "\\\\server\\share", "ok:stream", "a/./b", "a//b", "AUX.txt", "folder/con", "file.", "folder /file", "a\0b", ""] {
        assert!(archive::relative_path(name).is_err(), "accepted {name:?}");
    }
    assert_eq!(archive::relative_path("lib/python3.12/site-packages/model.py").unwrap(), PathBuf::from("lib/python3.12/site-packages/model.py"));
}
#[test]
fn zip_traversal_never_writes_outside_staging() {
    let fixture = Fixture::new();
    for name in ["../escaped", "/absolute", "C:/escaped", "foo:stream"] {
        let zip = zip_fixture(&fixture.base, &[name], b"bad");
        assert!(extraction(&zip, &fixture.base, 100, 10).is_err());
    }
    assert!(!fixture.base.join("escaped").exists());
}
#[test]
fn zip_rejects_symlink_special_file_and_hardlink_metadata() {
    let fixture = Fixture::new();
    for kind in [0o120777u32, 0o020600, 0o060600] {
        let path = zip_fixture(&fixture.base, &["link"], b"target");
        let mut bytes = fs::read(&path).unwrap();
        let offset = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
        bytes[offset + 5] = 3; // UNIX origin
        bytes[offset + 38..offset + 42].copy_from_slice(&(kind << 16).to_le_bytes());
        fs::write(&path, bytes).unwrap();
        assert!(extraction(&path, &fixture.base, 100, 10).is_err());
    }
    assert!(archive::validate_extra(&[0x0d, 0x00, 0, 0]).is_err()); // PKWARE hard/symbolic link
    assert!(archive::validate_extra(&[0x6e, 0x75, 0, 0]).is_err()); // ASi Unix symlink
    assert!(archive::validate_extra(&[0x55, 0x54, 9, 0]).is_err()); // truncated metadata
}
#[test]
fn zip_rejects_case_aliases_and_declared_limits() {
    let fixture = Fixture::new();
    let path = zip_fixture(&fixture.base, &["AAA", "BBB"], b"good");
    let mut bytes = fs::read(&path).unwrap();
    for index in 0..bytes.len() - 2 { if &bytes[index..index + 3] == b"BBB" { bytes[index..index + 3].copy_from_slice(b"aaa"); } }
    fs::write(&path, bytes).unwrap();
    assert!(extraction(&path, &fixture.base, 100, 10).is_err());
    let path = zip_fixture(&fixture.base, &["one", "two"], b"good");
    assert!(extraction(&path, &fixture.base, 7, 10).is_err());
    assert!(extraction(&path, &fixture.base, 100, 1).is_err());
    assert_eq!(extraction(&path, &fixture.base, 8, 2).unwrap().len(), 2);
}
#[test]
fn extraction_cancel_never_activates_a_component() {
    let fixture = Fixture::new(); let path = zip_fixture(&fixture.base, &["model.bin"], b"good");
    let staging = store::Staging::create(&fixture.root).unwrap();
    assert!(archive::extract(&path, &staging.payload(), 4, 1, &AtomicBool::new(true)).is_err());
    assert_eq!(store::status(&fixture.root, &model()).state, "not_installed");
}
#[test]
fn atomic_activation_retains_previous_version_and_failed_repair_preserves_it() {
    let fixture = Fixture::new(); let component = model();
    let first = store::commit(&fixture.root, &component, &staged(&fixture.root, &component)).unwrap();
    let old = first.path.unwrap();
    let broken = staged(&fixture.root, &component);
    fs::write(broken.payload().join("model.bin"), b"evil").unwrap();
    assert!(store::validate_files(&broken.payload(), &component, &AtomicBool::new(false)).is_err());
    drop(broken);
    assert_eq!(store::status(&fixture.root, &component).path.as_deref(), Some(old.as_str()));
    let second = store::commit(&fixture.root, &component, &staged(&fixture.root, &component)).unwrap();
    assert_ne!(second.path.as_deref(), Some(old.as_str()));
    assert_eq!(fs::read(Path::new(&old).join("model.bin")).unwrap(), b"good");
}
#[test]
fn same_size_corruption_and_receipt_omission_are_damaged() {
    let fixture = Fixture::new(); let component = model();
    let installed = store::commit(&fixture.root, &component, &staged(&fixture.root, &component)).unwrap();
    let directory = PathBuf::from(installed.path.unwrap());
    fs::write(directory.join("model.bin"), b"evil").unwrap();
    assert_eq!(store::status(&fixture.root, &component).state, "damaged");
    fs::write(directory.join("model.bin"), b"good").unwrap();
    let path = directory.join(".luma-receipt.json");
    let mut receipt: store::Receipt = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    receipt.files.clear(); fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert!(store::read_receipt(&directory, &component).is_err());
    assert_eq!(store::status(&fixture.root, &component).state, "damaged");
}
#[test]
fn explicit_staging_recovery_cleans_owned_crash_data_only() {
    let fixture = Fixture::new(); let staging = store::Staging::create(&fixture.root).unwrap();
    let path = staging.directory.clone(); std::mem::forget(staging);
    let unknown = fixture.root.join(".staging").join("user-data"); fs::create_dir(&unknown).unwrap();
    fs::write(unknown.join("keep"), b"user").unwrap();
    store::recover_staging(&fixture.root).unwrap();
    assert!(!path.exists()); assert_eq!(fs::read(unknown.join("keep")).unwrap(), b"user");
}
#[test]
fn remove_does_not_touch_legacy_models_settings_results_or_external_models() {
    let fixture = Fixture::new(); let component = model();
    for name in ["models/ggml-turbo.bin", "sidecars/whisper-cli", "user-model/model.bin", "settings.json", "results.srt"] {
        let path = fixture.base.join(name); fs::create_dir_all(path.parent().unwrap()).unwrap(); fs::write(path, b"untouched").unwrap();
    }
    store::commit(&fixture.root, &component, &staged(&fixture.root, &component)).unwrap();
    store::remove(&fixture.root, &component).unwrap();
    assert_eq!(store::status(&fixture.root, &component).state, "not_installed");
    for name in ["models/ggml-turbo.bin", "sidecars/whisper-cli", "user-model/model.bin", "settings.json", "results.srt"] { assert_eq!(fs::read(fixture.base.join(name)).unwrap(), b"untouched"); }
}
#[cfg(unix)]
#[test]
fn managed_symlink_redirection_is_refused() {
    let fixture = Fixture::new(); let component = model();
    let external = fixture.base.join("user-model"); fs::create_dir(&external).unwrap(); fs::write(external.join("keep"), b"user").unwrap();
    std::os::unix::fs::symlink(&external, fixture.root.join(component.id())).unwrap();
    assert!(store::remove(&fixture.root, &component).is_err());
    assert!(store::prepare_component(&fixture.root, &component).is_err());
    assert_eq!(fs::read(external.join("keep")).unwrap(), b"user");
}
#[test]
fn request_ids_busy_cancel_and_commit_boundary_are_deterministic() {
    let manager = ComponentManager::default();
    let request = ComponentRequest { component_id: "fixture-model".into(), request_id: uuid::Uuid::new_v4().to_string() };
    let operation = manager.begin(&request, 4).unwrap();
    assert!(manager.begin(&request, 4).is_err());
    operation.cancel.store(true, Ordering::SeqCst);
    assert!(manager.commit_boundary().is_err());
    drop(operation);
    assert!(manager.begin(&request, 4).is_err());
    let new = ComponentRequest { component_id: request.component_id.clone(), request_id: uuid::Uuid::new_v4().to_string() };
    let _operation = manager.begin(&new, 4).unwrap();
    assert!(manager.commit_boundary().is_ok());
    assert!(manager.active.lock().as_ref().unwrap().committing);
}
#[test]
fn shutdown_cancels_owned_setup_and_waits_for_operation_cleanup() {
    tauri::async_runtime::block_on(async {
        let manager = Arc::new(ComponentManager::default());
        let task_manager = manager.clone();
        let started = Arc::new(AtomicBool::new(false)); let signal = started.clone();
        let task = tauri::async_runtime::spawn(async move {
            let request = ComponentRequest { component_id: "fixture-model".into(), request_id: uuid::Uuid::new_v4().to_string() };
            let operation = task_manager.begin(&request, 4).unwrap(); signal.store(true, Ordering::SeqCst);
            while !operation.cancel.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(10)).await; }
        });
        while !started.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(10)).await; }
        manager.shutdown().await; task.await.unwrap();
        assert!(manager.active.lock().is_none());
        assert!(manager.begin(&ComponentRequest { component_id: "fixture-model".into(), request_id: uuid::Uuid::new_v4().to_string() }, 4).is_err());
    });
}
#[test]
fn free_space_budget_includes_staging_and_retained_install_and_os_versions_compare_numerically() {
    assert!(required_free_space(100, 200).unwrap() >= 300 + 256 * 1024 * 1024);
    assert!(required_free_space(u64::MAX, 200).is_err());
    assert!(catalog::version_supported("14.0.1", "14.0"));
    assert!(catalog::version_supported("15.1", "14.0"));
    assert!(!catalog::version_supported("13.6", "14.0"));
    assert!(!catalog::version_supported("unknown", "14.0"));
}
#[test]
fn interprocess_setup_lease_prevents_recovery_races_and_recovers_after_drop() {
    let fixture = Fixture::new();
    let first = store::acquire_setup_lease(&fixture.root).unwrap();
    assert!(store::acquire_setup_lease(&fixture.root).is_err());
    drop(first);
    assert!(store::acquire_setup_lease(&fixture.root).is_ok());
}
#[test]
fn managed_worker_shared_leases_prevent_removal_but_allow_new_downloads() {
    let fixture = Fixture::new(); let component = model();
    let status = store::commit(&fixture.root, &component, &staged(&fixture.root, &component)).unwrap();
    let path = status.path.unwrap();
    let worker = acquire_managed_use_leases_at(&fixture.root, &[&path, &path]).unwrap();
    assert_eq!(worker.len(), 1);
    assert!(store::acquire_use_lease(&fixture.root, true).is_err());
    assert!(store::acquire_setup_lease(&fixture.root).is_ok());
    drop(worker);
    let remover = store::acquire_use_lease(&fixture.root, true).unwrap();
    assert!(acquire_managed_use_leases_at(&fixture.root, &[&path]).is_err());
    drop(remover);
    assert!(acquire_managed_use_leases_at(&fixture.root, &[&path]).is_ok());
    let external = fixture.base.join("external-model"); fs::create_dir(&external).unwrap();
    assert!(acquire_managed_use_leases_at(&fixture.root, &[external.to_str().unwrap()]).unwrap().is_empty());
}
#[test]
fn status_snapshot_blocks_removal_without_writing_or_downloading() {
    let fixture = Fixture::new();
    let snapshot = store::snapshot_lease(&fixture.root).unwrap();
    assert!(store::acquire_use_lease(&fixture.root, true).is_err());
    drop(snapshot);
    assert!(store::acquire_use_lease(&fixture.root, true).is_ok());
    assert!(store::snapshot_lease(&fixture.base.join("nonexistent")).unwrap().is_none());
    assert!(!fixture.base.join("nonexistent").exists());
}
#[test]
fn corrupt_local_zip_filename_is_rejected_even_when_central_name_is_safe() {
    let fixture = Fixture::new(); let path = zip_fixture(&fixture.base, &["good"], b"data");
    let mut bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], b"PK\x03\x04");
    bytes[30..34].copy_from_slice(b"../x"); fs::write(&path, bytes).unwrap();
    assert!(extraction(&path, &fixture.base, 100, 10).is_err());
}

#[cfg(unix)]
#[test]
fn external_symlink_alias_into_managed_files_still_holds_use_lease() {
    let fixture = Fixture::new(); let component = model();
    let installed = store::commit(&fixture.root, &component, &staged(&fixture.root, &component)).unwrap();
    let alias = fixture.base.join("external-alias");
    std::os::unix::fs::symlink(installed.path.unwrap(), &alias).unwrap();
    let locks = acquire_managed_use_leases_at(&fixture.root, &[alias.to_str().unwrap()]).unwrap();
    assert_eq!(locks.len(), 1);
    assert!(store::acquire_use_lease(&fixture.root, true).is_err());
    drop(locks);
    fs::remove_file(alias).unwrap();
}
#[test]
fn copied_ownership_marker_outside_actual_app_root_does_not_create_a_lock() {
    let fixture = Fixture::new();
    let external = fixture.base.join("manual").join("asr-components");
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join(".owned-by-luma"), b"luma-subtitle-managed-asr-v1\n").unwrap();
    assert!(acquire_managed_use_leases_at(&fixture.root, &[external.to_str().unwrap()]).unwrap().is_empty());
    assert!(!external.join(".use.lock").exists());
    assert!(acquire_managed_use_leases_at(&fixture.root, &[fixture.root.join("removed-model").to_str().unwrap()]).is_err());
}
/// Explicit native-CI integration hook. No production catalog override or local
/// download URL exists. The caller supplies that CI build's archive + manifest.
#[test]
#[ignore = "requires a genuine native runtime pack and explicit local fixture paths"]
fn native_component_archive_installs_repairs_and_removes() {
    let archive_path = PathBuf::from(std::env::var("LUMA_ASR_COMPONENT_ARCHIVE").expect("set LUMA_ASR_COMPONENT_ARCHIVE to the built ZIP"));
    let manifest_path = PathBuf::from(std::env::var("LUMA_ASR_COMPONENT_MANIFEST").expect("set LUMA_ASR_COMPONENT_MANIFEST to the built manifest JSON"));
    assert!(archive_path.is_absolute() && manifest_path.is_absolute());
    let source: serde_json::Value = serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
    let mut runtime = serde_json::Map::new();
    for key in ["id", "version", "label", "platform", "min_os_version", "engine", "backend", "device", "license", "license_url", "installed_bytes", "max_files", "entrypoint", "archive"] {
        if let Some(value) = source.get(key) { runtime.insert(key.into(), value.clone()); }
    }
    let catalog = catalog::parse(&serde_json::json!({"schema": 1, "runtimes": [runtime], "models": []}).to_string()).unwrap();
    let component = catalog.components().remove(0); component.availability().unwrap();
    let Component::Runtime(runtime) = &component else { unreachable!() };
    let archive = runtime.archive.as_ref().unwrap();
    download::verify_digest(fs::metadata(&archive_path).unwrap().len(), &store::hash_file(&archive_path).unwrap(), archive.bytes, &archive.sha256).unwrap();
    let fixture = Fixture::new();
    let _setup = store::acquire_setup_lease(&fixture.root).unwrap();
    let mut installed_paths = Vec::new();
    for _ in 0..2 {
        let staging = store::Staging::create(&fixture.root).unwrap();
        let files = archive::extract(&archive_path, &staging.payload(), runtime.installed_bytes, runtime.max_files, &AtomicBool::new(false)).unwrap();
        tauri::async_runtime::block_on(self_test(&staging.payload(), runtime, &AtomicBool::new(false))).unwrap();
        store::write_receipt(&staging.payload(), &component, files).unwrap();
        store::validate_files(&staging.payload(), &component, &AtomicBool::new(false)).unwrap();
        let installed = store::commit(&fixture.root, &component, &staging).unwrap();
        let path = PathBuf::from(installed.path.unwrap());
        // Prove private Python and libraries still work after atomic relocation.
        tauri::async_runtime::block_on(self_test(&path, runtime, &AtomicBool::new(false))).unwrap();
        assert_eq!(store::status(&fixture.root, &component).state, "installed");
        installed_paths.push(path);
    }
    assert_ne!(installed_paths[0], installed_paths[1]);
    assert!(installed_paths[0].join(&runtime.entrypoint).is_file());
    let cancelled = store::Staging::create(&fixture.root).unwrap();
    assert!(archive::extract(&archive_path, &cancelled.payload(), runtime.installed_bytes, runtime.max_files, &AtomicBool::new(true)).is_err());
    assert_eq!(store::status(&fixture.root, &component).path.as_deref(), Some(installed_paths[1].to_str().unwrap()));
    let _exclusive_use = store::acquire_use_lease(&fixture.root, true).unwrap();
    store::remove(&fixture.root, &component).unwrap();
    assert_eq!(store::status(&fixture.root, &component).state, "not_installed");
    assert!(installed_paths.iter().all(|path| !path.exists()));
    println!("NATIVE_COMPONENT_INSTALLER_OK id={} archive_sha256={} install_repair_cancel_remove=true", runtime.id, archive.sha256);
}

#[test]
fn installation_mlx_self_test_chooses_available_device_without_changing_worker_contract() {
    let script = self_test_script("mlx-whisper").unwrap();
    assert!(script.contains("mx.set_default_device(mx.gpu if mx.metal.is_available() else mx.cpu)"));
    assert!(script.find("sys.addaudithook(offline)").unwrap() < script.find("import mlx_whisper").unwrap());
    assert!(script.find("mx.set_default_device(").unwrap() < script.find("mx.eval(").unwrap());
    assert!(!self_test_script("faster-whisper").unwrap().contains("mx.cpu"));
    assert!(self_test_script("unknown").is_err());
}
#[cfg(unix)]
#[test]
fn self_test_errors_preserve_exit_code_or_signal_and_supported_recovery() {
    use std::os::unix::process::ExitStatusExt;
    let exited = self_test_failure(std::process::ExitStatus::from_raw(23 << 8));
    assert!(exited.contains("native code 23 (0x00000017)"));
    assert!(exited.contains("previous component is unchanged"));
    assert!(exited.contains("supported review or recovery"));
    let signalled = std::process::ExitStatus::from_raw(9);
    assert!(self_test_failure(signalled).contains(&signalled.to_string()));
}
#[cfg(windows)]
#[test]
fn self_test_errors_preserve_windows_native_failure_code() {
    use std::os::windows::process::ExitStatusExt;
    let status = std::process::ExitStatus::from_raw(0xc0000135);
    let message = self_test_failure(status);
    assert!(message.contains("0xc0000135"));
    assert!(message.contains("supported review or recovery"));
}
