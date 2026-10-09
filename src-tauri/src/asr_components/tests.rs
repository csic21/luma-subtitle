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
    let request = ComponentRequest { component_id: "fixture-model".into(), request_id: uuid::Uuid::new_v4().to_string(), plan_sha256: None, acknowledged_terms: Vec::new() };
    let operation = manager.begin(&request, 4).unwrap();
    assert!(manager.begin(&request, 4).is_err());
    operation.cancel.store(true, Ordering::SeqCst);
    assert!(manager.commit_boundary().is_err());
    drop(operation);
    assert!(manager.begin(&request, 4).is_err());
    let new = ComponentRequest { component_id: request.component_id.clone(), request_id: uuid::Uuid::new_v4().to_string(), plan_sha256: None, acknowledged_terms: Vec::new() };
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
            let request = ComponentRequest { component_id: "fixture-model".into(), request_id: uuid::Uuid::new_v4().to_string(), plan_sha256: None, acknowledged_terms: Vec::new() };
            let operation = task_manager.begin(&request, 4).unwrap(); signal.store(true, Ordering::SeqCst);
            while !operation.cancel.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(10)).await; }
        });
        while !started.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(10)).await; }
        manager.shutdown().await; task.await.unwrap();
        assert!(manager.active.lock().is_none());
        assert!(manager.begin(&ComponentRequest { component_id: "fixture-model".into(), request_id: uuid::Uuid::new_v4().to_string(), plan_sha256: None, acknowledged_terms: Vec::new() }, 4).is_err());
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
        let lifetime = setup_process::SetupLifetime::new(&staging, &_setup);
        let files = archive::extract(&archive_path, &staging.payload(), runtime.installed_bytes, runtime.max_files, &AtomicBool::new(false)).unwrap();
        tauri::async_runtime::block_on(self_test(&staging.payload(), runtime, &AtomicBool::new(false), &lifetime)).unwrap();
        store::write_receipt(&staging.payload(), &component, files).unwrap();
        store::validate_files(&staging.payload(), &component, &AtomicBool::new(false)).unwrap();
        let installed = store::commit(&fixture.root, &component, &staging).unwrap();
        let path = PathBuf::from(installed.path.unwrap());
        // Prove private Python and libraries still work after atomic relocation.
        tauri::async_runtime::block_on(self_test(&path, runtime, &AtomicBool::new(false), &lifetime)).unwrap();
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

fn recipe_runtime() -> catalog::Runtime {
    let term = "Fixture upstream terms\n";
    let runtime: catalog::Runtime = serde_json::from_value(serde_json::json!({
        "id":"fixture-runtime", "version":"1.0.0", "label":"Fixture private runtime", "platform":"windows-x64", "engine":"whisper-accelerated", "backend":"faster-whisper", "device":"cpu", "license":"Fixture", "license_url":"https://example.com/terms", "installed_bytes":1024*1024, "max_files":100, "entrypoint":"python.exe",
        "recipe": { "schema":1,
            "python":{"url":"https://github.com/astral-sh/python-build-standalone/releases/download/20261003/cpython-3.12.15%2B20261003-x86_64-pc-windows-msvc-install_only_stripped.tar.gz", "bytes":4,"sha256":digest(b"good"),"installed_bytes":100000,"max_files":100,"entrypoint":"python.exe","site_packages":"Lib/site-packages","pip_version":"26.2.1"},
            "wheels":[{"name":"example","version":"1.0","filename":"example-1.0-py3-none-any.whl","url":format!("https://files.pythonhosted.org/packages/aa/bb/{}/example-1.0-py3-none-any.whl", "a".repeat(64)),"bytes":4,"sha256":digest(b"good"),"installed_bytes":1000,"max_files":20}],
            "terms":[{"id":"fixture-terms","version":"1","url":"https://example.com/terms","text":term,"sha256":digest(term.as_bytes())}]
        }
    })).unwrap();
    runtime.recipe.as_ref().unwrap().validate(&runtime).unwrap(); runtime
}
#[test]
fn managed_ct2_cpu_policy_requires_exact_recipe_and_active_receipt() {
    let fixture = Fixture::new();
    let mut runtime = recipe_runtime(); runtime.id = "faster-whisper-cpu-windows-x64".into();
    let wheel = &mut runtime.recipe.as_mut().unwrap().wheels[0];
    wheel.name = "ctranslate2".into(); wheel.version = "4.8.2".into();
    wheel.filename = catalog::CPU_WHEEL_FILENAME.into(); wheel.url = catalog::CPU_WHEEL_URL.into();
    assert!(reviewed_windows_ct2_cpu_recipe(&runtime));
    for field in ["id", "platform", "engine", "backend", "device", "recipe", "name", "version", "filename", "url", "sha256", "bytes"] {
        let mut other = runtime.clone();
        match field {
            "id" => other.id = "other".into(), "platform" => other.platform = "macos-arm64".into(),
            "engine" => other.engine = "qwen3-asr".into(), "backend" => other.backend = "qwen3-asr".into(),
            "device" => other.device = "cuda".into(), "recipe" => other.recipe = None,
            key => { let wheel = &mut other.recipe.as_mut().unwrap().wheels[0]; match key {
                "name" => wheel.name = "CTranslate2".into(), "version" => wheel.version = "4.8.3".into(),
                "filename" => wheel.filename = "ctranslate2-4.8.2-cp312-cp312-win_amd64.whl".into(),
                "url" => wheel.url = "https://files.pythonhosted.org/upstream.whl".into(),
                "sha256" => wheel.sha256 = "invalid".into(), "bytes" => wheel.bytes = 0, _ => unreachable!(),
            }}
        }
        assert!(!reviewed_windows_ct2_cpu_recipe(&other), "accepted {field}");
    }
    let catalog = catalog::Catalog { schema:1, platform:"windows-x64".into(), runtimes:vec![runtime.clone()], models:vec![] };
    let component = Component::Runtime(runtime);
    let stage = store::Staging::create(&fixture.root).unwrap();
    fs::write(stage.payload().join("python.exe"), b"good").unwrap();
    store::write_receipt(&stage.payload(), &component, vec![store::FileReceipt { path:"python.exe".into(), bytes:4, sha256:digest(b"good") }]).unwrap();
    let python = store::commit(&fixture.root, &component, &stage).unwrap().python_path.unwrap();
    let proof = ManagedCt2RuntimeProof::new(&fixture.root, &catalog, &python).unwrap();
    assert!(!proof.acquire(&python).unwrap().is_empty());
    let external = fixture.base.join("manual-python.exe"); fs::write(&external, b"good").unwrap();
    assert!(!verified_managed_windows_ct2_cpu_runtime_at(&fixture.root, external.to_str().unwrap(), &catalog).unwrap());
    assert!(proof.acquire(external.to_str().unwrap()).is_err());
    let mut changed = catalog.clone(); changed.runtimes[0].recipe.as_mut().unwrap().wheels[0].sha256 = digest(b"different wheel");
    assert!(verified_managed_windows_ct2_cpu_runtime_at(&fixture.root, &python, &changed).is_err());
    fs::write(&python, b"evil").unwrap(); assert!(proof.acquire(&python).is_err());
    fs::write(&python, b"good").unwrap();
    let receipt = Path::new(&python).parent().unwrap().join(".luma-receipt.json");
    let original = fs::read(&receipt).unwrap(); fs::write(&receipt, b"{}").unwrap();
    assert!(proof.acquire(&python).is_err()); fs::write(&receipt, original).unwrap();
    let next = store::Staging::create(&fixture.root).unwrap();
    fs::write(next.payload().join("python.exe"), b"good").unwrap();
    store::write_receipt(&next.payload(), &component, vec![store::FileReceipt { path:"python.exe".into(), bytes:4, sha256:digest(b"good") }]).unwrap();
    store::commit(&fixture.root, &component, &next).unwrap(); assert!(proof.acquire(&python).is_err());
}
#[test]
fn managed_qwen_compatibility_requires_active_interpreter_identity_not_model_lease() {
    let fixture = Fixture::new();
    let mut runtime = recipe_runtime();
    runtime.engine = "qwen3-asr".into(); runtime.backend = "qwen3-asr".into();
    let catalog = catalog::Catalog { schema: 1, platform: "windows-x64".into(), runtimes: vec![runtime.clone()], models: Vec::new() };
    let component = Component::Runtime(runtime);
    let stage = store::Staging::create(&fixture.root).unwrap();
    fs::write(stage.payload().join("python.exe"), b"good").unwrap();
    store::write_receipt(&stage.payload(), &component, vec![store::FileReceipt { path: "python.exe".into(), bytes: 4, sha256: digest(b"good") }]).unwrap();
    let installed = store::commit(&fixture.root, &component, &stage).unwrap();
    let python = installed.python_path.unwrap();
    let _lease = acquire_managed_use_leases_at(&fixture.root, &[&python]).unwrap();
    assert!(verified_managed_windows_qwen_runtime_at(&fixture.root, &python, &catalog).unwrap());

    let external = fixture.base.join("manual-python.exe"); fs::write(&external, b"good").unwrap();
    let managed_model = model();
    let managed_model = store::commit(&fixture.root, &managed_model, &staged(&fixture.root, &managed_model)).unwrap().path.unwrap();
    let mixed_leases = acquire_managed_use_leases_at(&fixture.root, &[external.to_str().unwrap(), &managed_model]).unwrap();
    assert!(!mixed_leases.is_empty());
    assert!(!verified_managed_windows_qwen_runtime_at(&fixture.root, external.to_str().unwrap(), &catalog).unwrap());
    assert!(verified_managed_windows_qwen_runtime_at(&fixture.root, &managed_model, &catalog).is_err());
    drop(mixed_leases);

    fs::write(&python, b"evil").unwrap();
    assert!(verified_managed_windows_qwen_runtime_at(&fixture.root, &python, &catalog).is_err());
    fs::write(&python, b"good").unwrap();
    let next = store::Staging::create(&fixture.root).unwrap();
    fs::write(next.payload().join("python.exe"), b"good").unwrap();
    store::write_receipt(&next.payload(), &component, vec![store::FileReceipt { path: "python.exe".into(), bytes: 4, sha256: digest(b"good") }]).unwrap();
    store::commit(&fixture.root, &component, &next).unwrap();
    assert!(verified_managed_windows_qwen_runtime_at(&fixture.root, &python, &catalog).is_err());
}
#[test]
fn managed_qwen_self_test_embeds_shared_compatibility_only_on_windows() {
    let qwen = self_test_script("qwen3-asr").unwrap();
    assert_eq!(qwen.contains("def luma_prepare_nagisa()"), cfg!(windows));
    if cfg!(windows) {
        assert!(qwen.find("sys.addaudithook(offline)").unwrap() < qwen.find("def luma_prepare_nagisa()").unwrap());
        let configure = qwen.find("\nluma_configure_numba_workqueue()\n").unwrap();
        let nagisa = qwen.find("\nluma_prepare_nagisa()\n").unwrap();
        let probe = qwen.find("\nluma_probe_numba_workqueue()\n").unwrap();
        let imports = qwen.find("from qwen_asr import").unwrap();
        let checked = qwen.find("\nluma_check_numba_workqueue(require_initialized=True)\n").unwrap();
        assert!(configure < nagisa && nagisa < probe && probe < imports && imports < checked);
    }
    assert!(!self_test_script("faster-whisper").unwrap().contains("luma_prepare_nagisa"));
    assert!(!self_test_script("mlx-whisper").unwrap().contains("luma_prepare_nagisa"));
}
#[test]
fn recipe_rejects_remote_trust_drift_missing_bounds_and_changed_displayed_terms() {
    let runtime = recipe_runtime(); let valid = runtime.recipe.as_ref().unwrap();
    let mut changed = valid.clone(); changed.python.pip_version = "latest".into(); assert!(changed.validate(&runtime).is_err());
    let mut changed = valid.clone(); changed.wheels[0].url = "https://evil.example/package.whl".into(); assert!(changed.validate(&runtime).is_err());
    let mut changed = valid.clone(); changed.wheels[0].max_files = 0; assert!(changed.validate(&runtime).is_err());
    let mut changed = valid.clone(); changed.terms[0].text.push('!'); assert!(changed.validate(&runtime).is_err());
    let mut changed = valid.clone(); changed.wheels.push(changed.wheels[0].clone()); assert!(changed.validate(&runtime).is_err());
    let mut changed = valid.clone(); changed.terms[0].raw_sha256 = Some(digest(b"original legacy bytes")); changed.terms[0].source_encoding = Some("cp1252".into()); assert!(changed.validate(&runtime).is_ok());
}
#[test]
fn exact_plan_and_every_term_are_required_before_setup_can_begin() {
    let runtime = recipe_runtime(); let required = runtime.recipe.as_ref().unwrap().acknowledgements();
    let mut request = ComponentRequest { component_id: runtime.id.clone(), request_id: uuid::Uuid::new_v4().to_string(), plan_sha256: None, acknowledged_terms: Vec::new() };
    assert!(recipe::validate_acknowledgement(&runtime, &request).is_err());
    request.plan_sha256 = Some(recipe::plan_hash(&runtime).unwrap());
    assert!(recipe::validate_acknowledgement(&runtime, &request).is_err());
    request.acknowledged_terms = required.clone();
    assert!(recipe::validate_acknowledgement(&runtime, &request).unwrap().is_some());
    request.acknowledged_terms.push(required[0].clone()); assert!(recipe::validate_acknowledgement(&runtime, &request).is_err());
    request.acknowledged_terms = required; request.plan_sha256 = Some("0".repeat(64)); assert!(recipe::validate_acknowledgement(&runtime, &request).is_err());
    // Pure acknowledgement validation cannot create state or fetch a source.
    let manager = ComponentManager::default(); assert!(manager.active.lock().is_none());
}
#[test]
fn consent_receipts_store_tuples_only_and_never_approve_a_changed_plan() {
    let fixture = Fixture::new(); let runtime = recipe_runtime();
    let consent = recipe::Consent { component_id:runtime.id.clone(), plan_sha256:recipe::plan_hash(&runtime).unwrap(), terms:runtime.recipe.as_ref().unwrap().acknowledgements() };
    let mut catalog = catalog::Catalog { schema:1, platform:"unsupported".into(), runtimes:vec![runtime], models:Vec::new() };
    assert!(store::read_consents(&fixture.root, &catalog).unwrap().is_empty());
    store::record_consent(&fixture.root, &consent).unwrap();
    assert_eq!(store::read_consents(&fixture.root, &catalog).unwrap(), vec![consent]);
    let directory = fixture.root.join(".consents/fixture-runtime");
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let text = fs::read_to_string(path).unwrap(); assert!(!text.contains("Fixture upstream terms")); assert!(!text.contains("credential"));
        }
    }
    let term = &mut catalog.runtimes[0].recipe.as_mut().unwrap().terms[0];
    term.text.push_str("Changed\n"); term.sha256 = digest(term.text.as_bytes()); term.version = "2".into();
    assert!(store::read_consents(&fixture.root, &catalog).unwrap().is_empty());
}
fn tar_fixture(base: &Path, members: &[(&str, u8, &[u8], &str)]) -> PathBuf {
    let path = base.join(format!("{}.tar.gz", uuid::Uuid::new_v4()));
    let gzip = flate2::write::GzEncoder::new(fs::File::create(&path).unwrap(), flate2::Compression::default());
    let mut builder = tar::Builder::new(gzip);
    for (name, kind, bytes, link) in members {
        let mut header = tar::Header::new_gnu(); header.set_mode(0o755); header.set_size(bytes.len() as u64);
        header.set_entry_type(tar::EntryType::new(*kind));
        header.as_mut_bytes()[..100].fill(0); header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        if !link.is_empty() { header.as_mut_bytes()[157..257].fill(0); header.as_mut_bytes()[157..157+link.len()].copy_from_slice(link.as_bytes()); }
        header.set_cksum(); builder.append(&header, *bytes).unwrap();
    }
    builder.finish().unwrap(); builder.into_inner().unwrap().finish().unwrap(); path
}
fn unpack_tar(fixture: &Fixture, archive: &Path, cap: u64, files: usize, cancelled: bool) -> Result<Vec<store::FileReceipt>, String> {
    let root = fixture.base.join(uuid::Uuid::new_v4().to_string()); fs::create_dir(&root).unwrap();
    let result = tar_bootstrap::extract(archive, &root, cap, files, &AtomicBool::new(cancelled));
    if result.is_ok() { for entry in fs::read_dir(&root).unwrap() { assert!(!entry.unwrap().file_type().unwrap().is_symlink()); } }
    result
}
#[test]
fn pinned_python_internal_links_are_copied_as_regular_files_with_expansion_caps() {
    let fixture = Fixture::new();
    let archive = tar_fixture(&fixture.base, &[("python/",b'5',b"",""),("python/bin/",b'5',b"",""),("python/bin/python3.12",b'0',b"good",""),("python/bin/python3",b'2',b"","python3.12"),("python/copy",b'1',b"","python/bin/python3.12")]);
    let files = unpack_tar(&fixture, &archive,12,3,false).unwrap();
    assert_eq!(files.len(),3); assert!(files.iter().all(|f| f.sha256 == digest(b"good")));
    assert!(unpack_tar(&fixture,&archive,8,3,false).is_err());
    assert!(unpack_tar(&fixture,&archive,12,2,false).is_err());
    assert!(unpack_tar(&fixture,&archive,12,3,true).is_err());
}
#[test]
fn python_tar_rejects_escape_cycle_alias_sparse_device_and_missing_targets() {
    let fixture = Fixture::new();
    for members in [
        vec![("python/../escape",b'0',&b"bad"[..],"")],
        vec![("other/file",b'0',&b"bad"[..],"")],
        vec![("python/file:stream",b'0',&b"bad"[..],"")],
        vec![("python/link",b'2',&b""[..],"../../escape")],
        vec![("python/link",b'2',&b""[..],"/absolute")],
        vec![("python/link",b'1',&b""[..],"../escape")],
        vec![("python/a",b'2',&b""[..],"b"),("python/b",b'2',&b""[..],"a")],
        vec![("python/missing",b'2',&b""[..],"not-present")],
        vec![("python/device",b'3',&b""[..],"")],
        vec![("python/sparse",b'S',&b""[..],"")],
        vec![("python/A",b'0',&b"x"[..],""),("python/a",b'0',&b"x"[..],"")],
    ] {
        let archive = tar_fixture(&fixture.base,&members);
        assert!(unpack_tar(&fixture,&archive,100,20,false).is_err(),"{members:?}");
    }
    assert!(!fixture.base.join("escape").exists());
}
#[test]
fn component_cache_cleanup_and_removal_are_scoped_to_one_owned_engine() {
    let fixture = Fixture::new(); let component = model();
    let ours = store::cache_directory(&fixture.root,component.id()).unwrap();
    let other = store::cache_directory(&fixture.root,"other-engine").unwrap();
    let hash = digest(b"good"); fs::write(ours.join(&hash),b"good").unwrap(); fs::write(other.join(&hash),b"good").unwrap();
    let partial = ours.join(format!("{}.{}.partial",hash,uuid::Uuid::new_v4())); fs::write(&partial,b"unfinished").unwrap();
    fs::write(ours.join("unrecognized.txt"),b"keep until explicit removal").unwrap();
    store::cache_directory(&fixture.root,component.id()).unwrap(); assert!(!partial.exists()); assert!(ours.join("unrecognized.txt").exists());
    let status = store::status(&fixture.root,&component);
    assert_eq!(status.state,"not_installed"); assert_eq!(status.installed_bytes,0); assert_eq!(status.cached_bytes,4);
    assert!(!fixture.root.join(component.id()).exists());
    store::remove(&fixture.root,&component).unwrap(); assert!(!ours.exists()); assert_eq!(fs::read(other.join(hash)).unwrap(),b"good");
    assert_eq!(store::status(&fixture.root,&component).cached_bytes,0);
}
#[test]
fn required_windows_environment_keys_are_preserved_case_insensitively() {
    for key in ["SystemRoot","SYSTEMROOT","windir","COMSPEC","PROCESSOR_ARCHITECTURE","Processor_ArchiteW6432","NUMBER_OF_PROCESSORS"] { assert!(setup_process::preserved_os_key(key),"{key}"); }
    for key in ["PATH","PYTHONPATH","PYTHONHOME","PIP_INDEX_URL","HTTP_PROXY","AWS_SECRET_ACCESS_KEY","HF_TOKEN"] { assert!(!setup_process::preserved_os_key(key),"{key}"); }
}
#[cfg(unix)]
#[test]
fn owned_assembly_cancellation_kills_and_reaps_without_detached_readers() {
    use std::os::unix::fs::PermissionsExt;
    tauri::async_runtime::block_on(async {
        let fixture = Fixture::new(); let staging = store::Staging::create(&fixture.root).unwrap();
        let lease = store::acquire_setup_lease(&fixture.root).unwrap();
        let lifetime = setup_process::SetupLifetime::new(&staging, &lease);
        let mut runtime = recipe_runtime(); runtime.entrypoint = "python-fixture".into();
        let executable = staging.payload().join(&runtime.entrypoint);
        fs::write(&executable,b"#!/bin/sh\necho $$ > child.pid\nwhile :; do :; done\n").unwrap(); fs::set_permissions(&executable,fs::Permissions::from_mode(0o700)).unwrap();
        let cancel = Arc::new(AtomicBool::new(false)); let flag = cancel.clone();
        let task = tauri::async_runtime::spawn(async move { tokio::time::sleep(Duration::from_millis(150)).await; flag.store(true,Ordering::SeqCst); });
        let started = Instant::now();
        let result = assembly::run(&staging.payload(),&staging.directory.join("unused.py"),&staging.directory.join("unused.json"),&staging.directory.join("unused-wheels"),&runtime,&cancel,&lifetime).await;
        task.await.unwrap(); assert!(result.unwrap_err().contains("cancelled")); assert!(started.elapsed() < Duration::from_secs(5));
        let pid = fs::read_to_string(staging.payload().join("child.pid")).unwrap();
        assert!(!std::process::Command::new("/bin/kill").args(["-0",pid.trim()]).status().unwrap().success());
    });
}

fn wheel_fixture(wheelhouse:&Path,name:&str,files:&[(&str,&[u8])])->recipe::Wheel {
    wheel_fixture_version(wheelhouse,name,"1.0",0o644,files)
}
fn wheel_fixture_version(wheelhouse:&Path,name:&str,version:&str,mode:u32,files:&[(&str,&[u8])])->recipe::Wheel {
    let normalized = name.replace('-',"_");
    let filename=format!("{normalized}-{version}-py3-none-any.whl"); let path=wheelhouse.join(&filename);
    let mut zip=zip::ZipWriter::new(fs::File::create(&path).unwrap()); let options=zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).unix_permissions(mode);
    for (suffix,contents) in [("WHEEL","Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n".to_string()),("METADATA",format!("Name: {name}\nVersion: {version}\n")),("RECORD",String::new())] {
        zip.start_file(format!("{normalized}-{version}.dist-info/{suffix}"),options).unwrap();zip.write_all(contents.as_bytes()).unwrap();
    }
    for (path,bytes) in files {zip.start_file(*path,options).unwrap();zip.write_all(bytes).unwrap();}
    zip.finish().unwrap();
    recipe::Wheel{name:name.into(),version:version.into(),filename:filename.clone(),url:format!("https://files.pythonhosted.org/packages/aa/bb/{}/{filename}","a".repeat(64)),bytes:fs::metadata(&path).unwrap().len(),sha256:store::hash_file(&path).unwrap(),installed_bytes:100000,max_files:100}
}
#[test]
fn wheel_destination_mapping_preserves_standard_private_prefix_data_layout() {
    assert_eq!(wheel_preflight::destination_for_test("example-1.0.data/data/Library/bin/library.dll","example-1.0.data","Lib/site-packages").unwrap(),"Library/bin/library.dll");
    assert_eq!(wheel_preflight::destination_for_test("example-1.0.data/purelib/example.py","example-1.0.data","Lib/site-packages").unwrap(),"Lib/site-packages/example.py");
    assert!(wheel_preflight::destination_for_test("example-1.0.data/data/../../python.exe","example-1.0.data","Lib/site-packages").is_err());
    assert!(wheel_preflight::destination_for_test("other-1.0.data/data/file","example-1.0.data","Lib/site-packages").is_err());
    assert!(wheel_preflight::destination_for_test("example-1.0.data/headers/header.h","example-1.0.data","Lib/site-packages").is_err());
}
#[test]
fn malicious_wheel_mapping_hooks_and_cross_wheel_collisions_fail_before_pip() {
    let fixture=Fixture::new(); let wheelhouse=fixture.base.join("wheels");fs::create_dir(&wheelhouse).unwrap();
    let mut recipe=recipe_runtime().recipe.unwrap();
    let python=vec![store::FileReceipt{path:"python.exe".into(),bytes:4,sha256:digest(b"good")}];
    for files in [
        vec![("example-1.0.data/data/python.exe",&b"evil"[..])],
        vec![("example.py",&b"one"[..]),("example-1.0.data/purelib/example.py",&b"two"[..])],
        vec![("subdir",&b"one"[..]),("subdir/child.py",&b"two"[..])],
        vec![("sitecustomize.py",&b"import socket"[..])],
        vec![("SITECUSTOMIZE.py",&b"import socket"[..])],
        vec![("example-1.0.data/data/lib/site-packages/sitecustomize.py",&b"import socket"[..])],
        vec![("PIP.py",&b"import socket"[..])],
        vec![("evil.PTH",&b"import socket"[..])],
        vec![("evil.pth",&b"import socket; socket.socket()"[..])],
    ] {
        recipe.wheels=vec![wheel_fixture(&wheelhouse,"example",&files)];
        assert!(wheel_preflight::validate(&wheelhouse,&recipe,&python,&AtomicBool::new(false)).is_err(),"{files:?}");
        assert!(!fixture.base.join("ASSEMBLY.json").exists());
    }
    recipe.wheels=vec![wheel_fixture(&wheelhouse,"example",&[("namespace/common.py",b"first")]),wheel_fixture(&wheelhouse,"another",&[("namespace/common.py",b"second")])];
    assert!(wheel_preflight::validate(&wheelhouse,&recipe,&python,&AtomicBool::new(false)).is_err());
    recipe.wheels=vec![wheel_fixture(&wheelhouse,"example",&[("namespace/first.py",b"first")]),wheel_fixture(&wheelhouse,"another",&[("namespace/second.py",b"second")])];
    assert!(wheel_preflight::validate(&wheelhouse,&recipe,&python,&AtomicBool::new(false)).is_ok());
}

#[test]
fn tar_extension_metadata_is_rejected_before_unbounded_preprocessing() {
    let fixture = Fixture::new();
    for kind in [b'L', b'K', b'x', b'g', b'S'] {
        let path = fixture.base.join(format!("metadata-{kind}.tar.gz"));
        let mut gzip = flate2::write::GzEncoder::new(fs::File::create(&path).unwrap(), flate2::Compression::default());
        let mut header = tar::Header::new_gnu(); header.set_path("python/metadata").unwrap();
        header.set_mode(0o600); header.set_size(1024 * 1024 * 1024); header.set_entry_type(tar::EntryType::new(kind)); header.set_cksum();
        gzip.write_all(header.as_bytes()).unwrap(); gzip.finish().unwrap();
        let error = unpack_tar(&fixture,&path,100,20,false).unwrap_err();
        assert!(error.contains("unsupported extension or sparse metadata"),"kind={kind}: {error}");
    }
}

/// Native proof only: local inputs must still match every immutable recipe hash.
/// No environment override or fixture transport is compiled into production.
#[test]
#[ignore = "requires native private Python and exact local wheel inputs"]
fn native_direct_recipe_installs_repairs_and_removes() {
    let runtime_path = PathBuf::from(std::env::var_os("LUMA_ASR_RECIPE_RUNTIME").expect("LUMA_ASR_RECIPE_RUNTIME"));
    let inputs = fs::canonicalize(PathBuf::from(std::env::var_os("LUMA_ASR_RECIPE_INPUTS").expect("LUMA_ASR_RECIPE_INPUTS"))).unwrap();
    let source: serde_json::Value = serde_json::from_slice(&fs::read(runtime_path).unwrap()).unwrap();
    let catalog = catalog::parse(&serde_json::json!({"schema":1,"runtimes":[source],"models":[]}).to_string()).unwrap();
    let component = catalog.components().remove(0);
    let Component::Runtime(runtime) = &component else { unreachable!() };
    assert_eq!(runtime.platform,catalog::platform()); catalog::check_os(runtime.min_os_version.as_deref()).unwrap();
    // Candidate unavailability is deliberately preserved: this test exercises
    // internal assembly without making an unreviewed production entry usable.
    let recipe = runtime.recipe.as_ref().expect("exact recipe required");
    let python_archive = inputs.join(&recipe.python.sha256);
    download::verify_digest(store::regular_file(&python_archive).unwrap().len(),&store::hash_file(&python_archive).unwrap(),recipe.python.bytes,&recipe.python.sha256).unwrap();
    for wheel in &recipe.wheels {
        let path = inputs.join(&wheel.filename);
        download::verify_digest(store::regular_file(&path).unwrap().len(),&store::hash_file(&path).unwrap(),wheel.bytes,&wheel.sha256).unwrap();
    }
    let crt_package = recipe.windows_crt.as_ref().map(|_| {
        let artifact = direct_crt::artifact(); let path = inputs.join(&artifact.sha256);
        download::verify_digest(store::regular_file(&path).unwrap().len(),&store::hash_file(&path).unwrap(),artifact.bytes,&artifact.sha256).unwrap();
        path
    });
    let mut fixture = Fixture::new();
    let unicode_root = fixture.base.join("Managed runtime é 测试");
    fs::rename(&fixture.root,&unicode_root).unwrap(); fixture.root = unicode_root;
    let _setup = store::acquire_setup_lease(&fixture.root).unwrap(); let cancel = AtomicBool::new(false);
    let mut installed_paths = Vec::new();
    for _ in 0..2 {
        let staging = store::Staging::create(&fixture.root).unwrap();
        let lifetime = setup_process::SetupLifetime::new(&staging, &_setup);
        let mut files = tar_bootstrap::extract(&python_archive,&staging.payload(),recipe.python.installed_bytes,recipe.python.max_files,&cancel).unwrap();
        if let Some(package) = &crt_package {
            tauri::async_runtime::block_on(direct_crt::install(package,&staging.directory,&staging.payload(),&cancel,direct_crt::TrustMode::Online,&lifetime)).unwrap();
        }
        let crt_receipts = if crt_package.is_some() { direct_crt::verified_files(&staging.payload(),&cancel).unwrap() } else { Vec::new() };
        for receipt in &crt_receipts {
            if let Some(existing) = files.iter().find(|entry| entry.path == receipt.path) {
                assert_eq!(existing.bytes,receipt.bytes); assert_eq!(existing.sha256,receipt.sha256);
            } else { files.push(receipt.clone()); }
        }
        let wheelhouse = staging.directory.join("wheelhouse"); fs::create_dir(&wheelhouse).unwrap();
        for wheel in &recipe.wheels { fs::copy(inputs.join(&wheel.filename),wheelhouse.join(&wheel.filename)).unwrap(); }
        wheel_preflight::validate(&wheelhouse,recipe,&files,&cancel).unwrap();
        let script = staging.directory.join("offline-assemble.py"); fs::write(&script,assembly::SCRIPT).unwrap();
        let lock = serde_json::json!({"schema":1,"platform":runtime.platform,"python":"3.12","wheels":recipe.wheels});
        let lock_path = staging.directory.join("wheel-lock.json"); fs::write(&lock_path,serde_json::to_vec(&lock).unwrap()).unwrap();
        tauri::async_runtime::block_on(assembly::run(&staging.payload(),&script,&lock_path,&wheelhouse,runtime,&cancel,&lifetime)).unwrap();
        assembly::verify_report(&staging.payload(),runtime,recipe).unwrap();
        if crt_package.is_some() {
            let after_pip = direct_crt::verified_files(&staging.payload(),&cancel).unwrap();
            assert_eq!(serde_json::to_value(&after_pip).unwrap(),serde_json::to_value(&crt_receipts).unwrap(),"Offline pip changed the fixed CRT receipts");
        }
        let files = assembly::inventory(&staging.payload(),runtime.installed_bytes,runtime.max_files,&cancel).unwrap();
        tauri::async_runtime::block_on(self_test(&staging.payload(),runtime,&cancel,&lifetime)).unwrap();
        store::write_receipt(&staging.payload(),&component,files).unwrap(); store::validate_files(&staging.payload(),&component,&cancel).unwrap();
        let installed = store::commit(&fixture.root,&component,&staging).unwrap(); let path = PathBuf::from(installed.path.unwrap());
        tauri::async_runtime::block_on(self_test(&path,runtime,&cancel,&lifetime)).unwrap();
        if crt_package.is_some() {
            let relocated_crt = direct_crt::verified_files(&path,&cancel).unwrap();
            assert_eq!(serde_json::to_value(&relocated_crt).unwrap(),serde_json::to_value(&crt_receipts).unwrap(),"Activation changed the fixed CRT receipts");
        }
        assert_eq!(store::status(&fixture.root,&component).state,"installed"); installed_paths.push(path);
    }
    assert_ne!(installed_paths[0],installed_paths[1]); assert!(installed_paths[0].join(&runtime.entrypoint).is_file());
    let failed = store::Staging::create(&fixture.root).unwrap();
    assert!(tar_bootstrap::extract(&python_archive,&failed.payload(),recipe.python.installed_bytes,recipe.python.max_files,&AtomicBool::new(true)).is_err());
    assert_eq!(store::status(&fixture.root,&component).path.as_deref(),Some(installed_paths[1].to_str().unwrap()));
    // This gate cannot select the old PyPI/cuDNN reference wheel. UI availability
    // remains unchanged: the worker uses this test-owned successful activation.
    if runtime.id == "faster-whisper-cpu-windows-x64" {
        let filename = "ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl";
        let url = format!("https://github.com/csic21/luma-subtitle/releases/download/asr-ct2-cpu-4.8.2-1/{filename}");
        let cpu = recipe.wheels.iter().find(|wheel| wheel.name == "ctranslate2").unwrap();
        assert_eq!(cpu.filename,filename); assert_eq!(cpu.url,url); assert_eq!(cpu.version,"4.8.2");
        assert_eq!(recipe.windows_crt.as_deref(),Some("msvc-14.44.35211-x64"));
        let lock: serde_json::Value = serde_json::from_str(include_str!("../../../scripts/asr-components/locks/faster-whisper-cpu-windows-x64.json")).unwrap();
        let pin = lock["wheels"].as_array().unwrap().iter().find(|wheel| wheel["name"] == "ctranslate2").unwrap();
        assert_eq!(pin["filename"].as_str(),Some(cpu.filename.as_str())); assert_eq!(pin["url"].as_str(),Some(cpu.url.as_str()));
        assert_eq!(pin["sha256"].as_str(),Some(cpu.sha256.as_str())); assert_eq!(pin["bytes"].as_u64(),Some(cpu.bytes));
        let model = fs::canonicalize(std::env::var_os("LUMA_ASR_TEST_MODEL").expect("pinned Tiny fixture required")).unwrap();
        let replacement_model = fs::canonicalize(std::env::var_os("LUMA_ASR_TEST_REPLACEMENT_MODEL").expect("second pinned Tiny fixture required")).unwrap();
        let audio = fs::canonicalize(std::env::var_os("LUMA_ASR_TEST_AUDIO").expect("pinned JFK fixture required")).unwrap();
        let long_audio = fs::canonicalize(std::env::var_os("LUMA_ASR_TEST_LONG_AUDIO").expect("bounded cancellation fixture required")).unwrap();
        let output = PathBuf::from(std::env::var_os("LUMA_ASR_TEST_OUTPUT").expect("isolated worker output required"));
        let pins: serde_json::Value = serde_json::from_str(include_str!("../../../scripts/asr-components/fixtures.json")).unwrap();
        for directory in [&model,&replacement_model] {
            for pin in pins["faster_whisper_tiny"]["files"].as_array().unwrap() {
                let file = directory.join(pin["path"].as_str().unwrap());
                download::verify_digest(store::regular_file(&file).unwrap().len(),&store::hash_file(&file).unwrap(),pin["bytes"].as_u64().unwrap(),pin["sha256"].as_str().unwrap()).unwrap();
            }
        }
        download::verify_digest(store::regular_file(&audio).unwrap().len(),&store::hash_file(&audio).unwrap(),pins["audio"]["bytes"].as_u64().unwrap(),pins["audio"]["sha256"].as_str().unwrap()).unwrap();
        assert!(store::regular_file(&long_audio).unwrap().len() <= 36*1024*1024);
        let config = crate::asr::AsrConfig { engine:"whisper-accelerated".into(),device:"cpu".into(),
            python_path:installed_paths[1].join(&runtime.entrypoint).to_string_lossy().into_owned(),
            model_path:model.to_string_lossy().into_owned(),..Default::default() };
        let receipt_proof = ManagedCt2RuntimeProof::new(&fixture.root,&catalog,&config.python_path).unwrap();
        tauri::async_runtime::block_on(crate::asr::run_real_optional_worker_fixture(&config,&replacement_model,&audio,&long_audio,&output,Some(receipt_proof)));
        store::validate_files(&installed_paths[1],&component,&cancel).unwrap();
        println!("NATIVE_DIRECT_WORKER_LIFECYCLE_OK cold_warm_srt_export_cancel_recovery=true");
    }
    let _exclusive = store::acquire_use_lease(&fixture.root,true).unwrap(); store::remove(&fixture.root,&component).unwrap();
    assert_eq!(store::status(&fixture.root,&component).state,"not_installed"); assert!(installed_paths.iter().all(|p| !p.exists()));
    println!("NATIVE_DIRECT_RECIPE_INSTALLER_OK id={} plan_sha256={} install_repair_cancel_remove=true",runtime.id,recipe::plan_hash(runtime).unwrap());
}

#[test]
fn tar_root_directory_cannot_hide_a_data_body() {
    let fixture = Fixture::new();
    let archive = tar_fixture(&fixture.base,&[("python/",b'5',b"unexpected","")]);
    assert!(unpack_tar(&fixture,&archive,100,20,false).unwrap_err().contains("root directory contains unexpected data"));
}

#[test]
fn reviewed_mlx_overlap_requires_exact_owners_version_path_bytes_and_mode() {
    let fixture = Fixture::new(); let wheelhouse = fixture.base.join("mlx-wheels"); fs::create_dir(&wheelhouse).unwrap();
    let mut recipe = recipe_runtime().recipe.unwrap(); let cancel = AtomicBool::new(false);
    for (second,version,mode,path,content,allowed) in [
        ("mlx-metal","0.29.3",0o644,"mlx/utils.py",&b"same"[..],true),
        ("mlx-metal","0.29.3",0o644,"mlx/utils.py",&b"evil"[..],false),
        ("mlx-metal","0.29.3",0o755,"mlx/utils.py",&b"same"[..],false),
        ("mlx-metal","0.29.4",0o644,"mlx/utils.py",&b"same"[..],false),
        ("another","0.29.3",0o644,"mlx/utils.py",&b"same"[..],false),
        ("mlx-metal","0.29.3",0o644,"mlx/unreviewed.py",&b"same"[..],false),
        ("mlx-metal","0.29.3",0o644,"mlx/UTILS.py",&b"same"[..],false),
    ] {
        let first_path = if path == "mlx/UTILS.py" { "mlx/utils.py" } else { path };
        recipe.wheels = vec![wheel_fixture_version(&wheelhouse,"mlx","0.29.3",0o644,&[(first_path,b"same")]),wheel_fixture_version(&wheelhouse,second,version,mode,&[(path,content)])];
        assert_eq!(wheel_preflight::validate(&wheelhouse,&recipe,&[],&cancel).is_ok(),allowed,"owner={second} version={version} mode={mode} path={path}");
    }
}
