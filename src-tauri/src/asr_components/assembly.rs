//! One reviewed offline pip primitive, embedded in the app and shared with CI.
use super::{archive, catalog::{Archive, Runtime, Source}, download, recipe::Recipe, store::{self, FileReceipt}, ComponentManager, POLL};
use serde_json::json;
use std::{collections::VecDeque, fs::{self, OpenOptions}, io::{Read, Write}, path::{Path, PathBuf}, process::Stdio, sync::{atomic::{AtomicBool, Ordering}, Arc}, time::{Duration, Instant}};
use tauri::AppHandle;
use tokio::io::AsyncReadExt;
pub(super) const SCRIPT: &str = include_str!("../../../scripts/asr-components/assemble.py");

pub(super) async fn prepare(app: &AppHandle, manager: &ComponentManager, root: &Path, staging: &store::Staging, runtime: &Runtime, recipe: &Recipe, cancel: &Arc<AtomicBool>, lifetime: &super::setup_process::SetupLifetime) -> Result<Vec<FileReceipt>, String> {
    let python = Archive { url: recipe.python.url.clone(), bytes: recipe.python.bytes, sha256: recipe.python.sha256.clone() };
    let python_archive = download::cached(root, &runtime.id, &python, Source::Python, cancel, |bytes| manager.report(app, "downloading", bytes, "Downloading verified private Python from its official source")).await?;
    let wheelhouse = staging.directory.join("wheelhouse"); store::create_private_dir(&wheelhouse)?;
    let mut completed = python.bytes;
    let crt_package = if recipe.windows_crt.is_some() {
        let artifact = super::direct_crt::artifact();
        let package = download::cached(root, &runtime.id, &artifact, Source::MicrosoftCrt, cancel, |bytes| manager.report(app, "downloading", completed + bytes, "Downloading the fixed Microsoft runtime package")).await?;
        completed += artifact.bytes; Some(package)
    } else { None };
    for wheel in &recipe.wheels {
        archive::cancelled(cancel)?;
        let artifact = Archive { url: wheel.url.clone(), bytes: wheel.bytes, sha256: wheel.sha256.clone() };
        let mut high_water = 0;
        let source = download::cached(root, &runtime.id, &artifact, Source::Wheel, cancel, |bytes| {
            high_water = high_water.max(bytes);
            manager.report(app, "downloading", completed + high_water, "Downloading exact, hash-verified upstream wheels");
        }).await?;
        let destination = wheelhouse.join(&wheel.filename); let cap = wheel.installed_bytes; let files = wheel.max_files; let cancellation = cancel.clone();
        let retained = lifetime.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _lifetime = retained;
            archive::inspect(&source, cap, files, &cancellation)?;
            copy_local(&source, &destination, &cancellation)
        }).await.map_err(|e| e.to_string())??;
        completed += wheel.bytes;
    }
    manager.report(app, "extracting", completed, "Safely preparing the pinned private Python runtime");
    let payload = staging.payload(); let byte_cap = recipe.python.installed_bytes; let file_cap = recipe.python.max_files; let cancellation = cancel.clone();
    let retained = lifetime.clone();
    let mut python_files = tauri::async_runtime::spawn_blocking(move || {
        let _lifetime = retained;
        super::tar_bootstrap::extract(&python_archive, &payload, byte_cap, file_cap, &cancellation)
    }).await.map_err(|e| e.to_string())??;
    archive::cancelled(cancel)?;
    if let Some(package) = crt_package {
        manager.report(app, "verifying", completed, "Verifying Microsoft signatures and preparing private runtime files");
        super::direct_crt::install(&package, &staging.directory, &staging.payload(), cancel, super::direct_crt::TrustMode::Online, lifetime).await?;
    }
    let crt_receipts = if recipe.windows_crt.is_some() { super::direct_crt::verified_files(&staging.payload(), cancel)? } else { Vec::new() };
    for receipt in &crt_receipts { if !python_files.iter().any(|f| f.path == receipt.path) { python_files.push(receipt.clone()); } }
    let inspected_wheelhouse = wheelhouse.clone(); let inspected_recipe = recipe.clone(); let cancellation = cancel.clone();
    let retained = lifetime.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _lifetime = retained;
        super::wheel_preflight::validate(&inspected_wheelhouse, &inspected_recipe, &python_files, &cancellation)
    }).await.map_err(|e| e.to_string())??;
    let lock = json!({"schema":1,"platform":runtime.platform,"python":"3.12","wheels":recipe.wheels});
    let lock_path = staging.directory.join("wheel-lock.json"); write_new(&lock_path, &serde_json::to_vec(&lock).map_err(|e| e.to_string())?)?;
    let script = staging.directory.join("offline-assemble.py"); write_new(&script, SCRIPT.as_bytes())?;
    manager.report(app, "assembling", completed, "Installing the fixed local wheel set without network access or dependency resolution");
    run(&staging.payload(), &script, &lock_path, &wheelhouse, runtime, cancel, lifetime).await?;
    verify_report(&staging.payload(), runtime, recipe)?;
    if recipe.windows_crt.is_some() {
        let retained = super::direct_crt::verified_files(&staging.payload(), cancel)?;
        if retained.iter().zip(&crt_receipts).any(|(a,b)| a.path != b.path || a.bytes != b.bytes || a.sha256 != b.sha256) || retained.len() != crt_receipts.len() { return Err("Offline assembly changed a verified Microsoft runtime file or notice.".into()); }
    }
    manager.report(app, "verifying", completed, "Verifying the complete assembled private component");
    let payload = staging.payload(); let byte_cap = runtime.installed_bytes; let file_cap = runtime.max_files; let cancellation = cancel.clone();
    let retained = lifetime.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _lifetime = retained;
        inventory(&payload, byte_cap, file_cap, &cancellation)
    }).await.map_err(|e| e.to_string())?
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new(); options.create_new(true).write(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())
}
fn copy_local(source: &Path, destination: &Path, cancel: &AtomicBool) -> Result<(), String> {
    store::regular_file(source)?;
    let mut input = fs::File::open(source).map_err(|e| e.to_string())?;
    let mut options = OpenOptions::new(); options.create_new(true).write(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut output = options.open(destination).map_err(|e| e.to_string())?;
    let mut buffer = [0u8; 256 * 1024];
    loop { archive::cancelled(cancel)?; let n = input.read(&mut buffer).map_err(|e| e.to_string())?; if n == 0 { break; } output.write_all(&buffer[..n]).map_err(|e| e.to_string())?; }
    output.sync_all().map_err(|e| e.to_string())
}
pub(super) async fn run(root: &Path, script: &Path, lock: &Path, wheelhouse: &Path, runtime: &Runtime, cancel: &AtomicBool, lifetime: &super::setup_process::SetupLifetime) -> Result<(), String> {
    archive::cancelled(cancel)?;
    let executable = store::checked_path(root, &runtime.entrypoint)?; store::regular_file(&executable)?;
    let mut command = tokio::process::Command::new(executable);
    let temporary = super::setup_process::PrivateTemp::create(root)?;
    super::setup_process::configure(&mut command, root, &temporary)?;
    command.args(["-I", "-S", "-B", "-u", "-X", "utf8"]).arg(script).arg("--runtime-root").arg(root).arg("--wheel-lock").arg(lock).arg("--wheelhouse").arg(wheelhouse)
        .current_dir(root).env_remove("PYTHONPATH").env_remove("PYTHONHOME")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    crate::process_utils::hide_tokio_command_window(&mut command);
    let mut child = super::setup_process::OwnedChild::spawn(&mut command, lifetime, temporary).map_err(|e| format!("Cannot start private offline assembly: {e}"))?;
    let mut stdout = child.child_mut().stdout.take().ok_or("Private assembly stdout is missing")?;
    let mut stderr = child.child_mut().stderr.take().ok_or("Private assembly stderr is missing")?;
    let mut out_done = false; let mut err_done = false; let mut status = None; let mut bytes = 0usize;
    let mut tail = VecDeque::new(); let mut buffer = [0u8; 4096]; let started = Instant::now();
    let result = loop {
        if cancel.load(Ordering::SeqCst) { break Err("Component setup cancelled.".into()); }
        if started.elapsed() > Duration::from_secs(600) { break Err("Private offline assembly timed out. The previous component is unchanged.".into()); }
        if status.is_none() { match child.child_mut().try_wait() { Ok(value) => status = value, Err(e) => break Err(format!("Cannot inspect private assembly process: {e}")) } }
        // Drain both pipes in this owned future. No detached reader task can
        // outlive cancellation or keep the child blocked on a full pipe.
        for stream in 0..2 {
            let read = if stream == 0 {
                if out_done { continue; }
                tokio::time::timeout(Duration::from_millis(20), stdout.read(&mut buffer)).await
            } else {
                if err_done { continue; }
                tokio::time::timeout(Duration::from_millis(20), stderr.read(&mut buffer)).await
            };
            match read {
                Ok(Ok(0)) => { if stream == 0 { out_done = true; } else { err_done = true; } },
                Ok(Ok(n)) => { bytes += n; tail.extend(&buffer[..n]); let excess = tail.len().saturating_sub(4096); tail.drain(..excess); },
                Ok(Err(e)) => { child.finish(true).await; return Err(format!("Cannot read private assembly diagnostics: {e}")); },
                Err(_) => (),
            }
        }
        if bytes > 8 * 1024 * 1024 { break Err("Private assembly produced excessive output and was stopped.".into()); }
        if let Some(exit) = status {
            if out_done && err_done {
                if exit.success() { break Ok(()); }
                let tail: Vec<u8> = tail.into_iter().collect();
                let diagnostic: String = String::from_utf8_lossy(&tail).chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect();
                break Err(format!("{}\n{}", super::self_test_failure(exit), diagnostic));
            }
        }
        tokio::time::sleep(POLL).await;
    };
    // Always await termination/reaping, including cancellation and time limits.
    child.finish(result.is_err()).await;
    result
}
pub(super) fn verify_report(root: &Path, runtime: &Runtime, recipe: &Recipe) -> Result<(), String> {
    let path = root.join("ASSEMBLY.json");
    if store::regular_file(&path)?.len() > 1024 * 1024 { return Err("Oversized private assembly report.".into()); }
    let report: serde_json::Value = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if report["schema"] != 1 || report["method"] != "private-offline-pip" || report["python"] != "3.12.15" || report["pip"] != recipe.python.pip_version || report["wheels"].as_u64() != Some(recipe.wheels.len() as u64) {
        return Err("Private assembly report did not match the embedded recipe.".into());
    }
    for field in ["no_index", "no_dependencies", "binary_only", "require_hashes", "pip_config_disabled", "network_guard", "subprocess_guard", "generated_launchers_removed", "installer_removed"] {
        if report[field] != true { return Err(format!("Private assembly did not confirm required invariant: {field}")); }
    }
    let lock = json!({"schema":1,"platform":runtime.platform,"python":"3.12","wheels":recipe.wheels});
    let lock_bytes = serde_json::to_vec(&lock).map_err(|e| e.to_string())?;
    use sha2::{Digest, Sha256};
    let expected_wheels: Vec<_> = recipe.wheels.iter().map(|w| json!({"name":w.name,"version":w.version,"filename":w.filename,"url":w.url,"bytes":w.bytes,"sha256":w.sha256})).collect();
    if report["wheel_lock_sha256"] != format!("{:x}", Sha256::digest(lock_bytes)) || report["upstream_wheels"] != json!(expected_wheels) || report["installation_scheme"] != "private-prefix" {
        return Err("Private assembly provenance or destination scheme differs from the exact recipe.".into());
    }
    if report["source_builds"] != false || report["direct_url_metadata"] != false || report["startup_site_hooks"] != false { return Err("Private assembly used unsupported build or provenance behavior.".into()); }
    Ok(())
}
pub(super) fn inventory(root: &Path, byte_cap: u64, file_cap: usize, cancel: &AtomicBool) -> Result<Vec<FileReceipt>, String> {
    store::ensure_directory(root)?;
    let mut stack = vec![root.to_path_buf()]; let mut receipts = Vec::new(); let mut total = 0u64; let mut entries = 0usize;
    while let Some(directory) = stack.pop() {
        archive::cancelled(cancel)?; store::ensure_directory(&directory)?;
        for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
            archive::cancelled(cancel)?;
            let path = entry.map_err(|e| e.to_string())?.path();
            entries += 1; if entries > file_cap.saturating_mul(4) { return Err("Assembled component contains too many filesystem entries.".into()); }
            let relative = path.strip_prefix(root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/"); archive::relative_path(&relative)?;
            if relative.split('/').next().is_some_and(|p| p.starts_with(".assembly-")) { return Err("Private assembly left an incomplete temporary directory.".into()); }
            if fs::symlink_metadata(&path).map_err(|e| e.to_string())?.is_dir() { store::ensure_directory(&path)?; stack.push(path); continue; }
            let bytes = store::regular_file(&path)?.len(); total = total.checked_add(bytes).ok_or("Assembled component size overflow")?;
            if total > byte_cap || receipts.len() >= file_cap { return Err("Assembled component exceeds its reviewed final-tree bounds.".into()); }
            let sha256 = store::hash_file_checked(&path, cancel)?;
            receipts.push(FileReceipt { path: relative, bytes, sha256 });
        }
    }
    receipts.sort_by(|a, b| a.path.cmp(&b.path));
    if receipts.is_empty() { return Err("Assembled component is empty.".into()); }
    Ok(receipts)
}
