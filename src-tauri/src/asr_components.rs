//! User-triggered, app-owned optional components. No startup downloads, resolvers,
//! global Python changes, legacy model changes, or remote trust manifests.
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, future::Future, path::{Path, PathBuf}, process::Stdio, sync::{atomic::{AtomicBool, Ordering}, Arc, OnceLock}, time::{Duration, Instant}};
use tauri::{AppHandle, Emitter, Manager};

mod archive;
mod assembly;
mod catalog;
mod download;
mod direct_crt;
#[cfg(windows)] pub(crate) use direct_crt::signature_helper_from_args;
mod recipe;
mod tar_bootstrap;
mod setup_process;
mod wheel_preflight;
mod store;
#[cfg(test)] mod tests;
use catalog::Component;
const POLL: Duration = Duration::from_millis(100);
static MANAGED_ROOT: OnceLock<PathBuf> = OnceLock::new();
pub(crate) fn initialize(app: &AppHandle) -> Result<(), String> {
    let root = root_path(app, false)?;
    if let Some(existing) = MANAGED_ROOT.get() {
        if existing != &root { return Err("Managed component root was already initialized for a different application.".into()); }
        return Ok(());
    }
    MANAGED_ROOT.set(root).map_err(|_| "Managed component root initialization raced".into())
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ComponentStatus {
    id: String, kind: String, state: String, version: Option<String>, path: Option<String>,
    python_path: Option<String>, installed_bytes: u64, cached_bytes: u64, error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Progress {
    request_id: String, component_id: String, phase: String,
    downloaded_bytes: u64, total_bytes: u64, message: String,
}
#[derive(Serialize)]
pub(crate) struct Status { components: Vec<ComponentStatus>, operation: Option<Progress>, consents: Vec<recipe::Consent> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ComponentRequest {
    component_id: String, request_id: String,
    #[serde(default)] plan_sha256: Option<String>,
    #[serde(default)] acknowledged_terms: Vec<recipe::TermAcknowledgement>,
}
struct Operation { progress: Progress, cancel: Arc<AtomicBool>, committing: bool }
#[derive(Default)]
pub(crate) struct ComponentManager {
    active: Mutex<Option<Operation>>,
    used_request_ids: Mutex<HashSet<String>>,
    stopping: AtomicBool,
    store_gate: tokio::sync::RwLock<()>,
}
impl ComponentManager {
    pub(crate) async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(operation) = self.active.lock().as_ref() { operation.cancel.store(true, Ordering::SeqCst); }
        // Download requests poll cancellation; extraction checks every chunk;
        // the self-test is killed and explicitly reaped before its future ends.
        while self.active.lock().is_some() { tokio::time::sleep(POLL).await; }
    }
    fn begin<'a>(&'a self, request: &ComponentRequest, total_bytes: u64) -> Result<OperationGuard<'a>, String> {
        if self.stopping.load(Ordering::SeqCst) { return Err("The app is closing; component setup cannot start.".into()); }
        if uuid::Uuid::parse_str(&request.request_id).is_err() { return Err("Component setup requires a fresh UUID request ID.".into()); }
        let mut active = self.active.lock();
        // Pair with shutdown while holding the same active-operation lock.
        if self.stopping.load(Ordering::SeqCst) { return Err("The app is closing; component setup cannot start.".into()); }
        if active.is_some() { return Err("Another engine component is being changed. Wait or cancel it first.".into()); }
        let mut ids = self.used_request_ids.lock();
        if ids.contains(&request.request_id) { return Err("This component setup request was already used. Retry with a fresh request ID.".into()); }
        // Keep request IDs for this entire app session, bounding malicious callers.
        if ids.len() >= 10_000 { return Err("Too many component setup requests in this app session. Restart the app to continue.".into()); }
        ids.insert(request.request_id.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        *active = Some(Operation { progress: Progress { request_id: request.request_id.clone(), component_id: request.component_id.clone(), phase: "preparing".into(), downloaded_bytes: 0, total_bytes, message: "Preparing private engine component storage".into() }, cancel: cancel.clone(), committing: false });
        Ok(OperationGuard { manager: self, cancel })
    }
    fn report(&self, app: &AppHandle, phase: &str, bytes: u64, message: &str) {
        let payload = {
            let mut active = self.active.lock();
            let Some(operation) = active.as_mut() else { return; };
            operation.progress.phase = phase.into();
            operation.progress.downloaded_bytes = bytes;
            operation.progress.message = message.into();
            operation.progress.clone()
        };
        let _ = app.emit("asr-component-progress", payload);
    }
    fn commit_boundary(&self) -> Result<(), String> {
        let mut active = self.active.lock();
        let operation = active.as_mut().ok_or("Component operation is no longer active")?;
        archive::cancelled(&operation.cancel)?;
        if self.stopping.load(Ordering::SeqCst) { return Err("Component setup cancelled.".into()); }
        operation.committing = true;
        Ok(())
    }
}
struct OperationGuard<'a> { manager: &'a ComponentManager, cancel: Arc<AtomicBool> }
impl Drop for OperationGuard<'_> { fn drop(&mut self) { self.manager.active.lock().take(); } }

#[tauri::command]
pub(crate) fn asr_component_catalog() -> Result<catalog::Catalog, String> { catalog::embedded() }
fn root_path(app: &AppHandle, create: bool) -> Result<PathBuf, String> {
    // Only this app-owned sibling is touched. Never use python_path/model_path,
    // the legacy sidecars directory, settings, results, or user model folders.
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    if create { std::fs::create_dir_all(&base).map_err(|e| e.to_string())?; }
    if !base.exists() { return Ok(base.join("asr-components")); }
    let base = std::fs::canonicalize(base).map_err(|e| e.to_string())?;
    let root = base.join("asr-components");
    if create { store::prepare_root(&root)?; }
    Ok(root)
}
#[tauri::command]
pub(crate) async fn asr_component_status(app: AppHandle) -> Result<Status, String> {
    let catalog = catalog::embedded()?; let root = root_path(&app, false)?;
    let manager = app.state::<ComponentManager>();
    let _snapshot = manager.store_gate.read().await;
    let _cross_process_snapshot = store::snapshot_lease(&root)?;
    let (components, consents) = tauri::async_runtime::spawn_blocking(move || {
        let _cross_process_snapshot = _cross_process_snapshot;
        let components = catalog.components().iter().map(|c| store::status(&root, c)).collect();
        let consents = store::read_consents(&root, &catalog)?;
        Ok::<_, String>((components, consents))
    }).await.map_err(|e| e.to_string())??;
    let operation = manager.active.lock().as_ref().map(|o| o.progress.clone());
    Ok(Status { components, operation, consents })
}
#[tauri::command]
pub(crate) fn cancel_asr_component(app: AppHandle, request_id: String) -> bool {
    let manager = app.state::<ComponentManager>();
    let active = manager.active.lock();
    match active.as_ref() {
        Some(operation) if operation.progress.request_id == request_id && !operation.committing => { operation.cancel.store(true, Ordering::SeqCst); true }
        _ => false,
    }
}
#[tauri::command]
pub(crate) async fn install_asr_component(app: AppHandle, request: ComponentRequest) -> Result<ComponentStatus, String> { install(app, request, false).await }
#[tauri::command]
pub(crate) async fn repair_asr_component(app: AppHandle, request: ComponentRequest) -> Result<ComponentStatus, String> { install(app, request, true).await }

async fn install(app: AppHandle, request: ComponentRequest, repair: bool) -> Result<ComponentStatus, String> {
    let component = catalog::embedded()?.find(&request.component_id)?;
    component.availability()?;
    // Verify explicit exact-plan assent before beginning an operation, creating
    // storage, downloading, or executing any upstream code.
    let consent = match &component { Component::Runtime(runtime) => recipe::validate_acknowledgement(runtime, &request)?, Component::Model(_) => None };
    let manager = app.state::<ComponentManager>();
    let operation = manager.begin(&request, component.download_bytes())?;
    manager.report(&app, "preparing", 0, "Preparing private engine component storage");
    let result = install_inner(&app, &manager, &component, operation.cancel.clone(), repair, consent).await;
    match &result {
        Ok(_) => manager.report(&app, "complete", component.download_bytes(), "Engine component is ready. Select it to use it."),
        Err(error) => {
            let bytes = manager.active.lock().as_ref().map_or(0, |o| o.progress.downloaded_bytes);
            manager.report(&app, if operation.cancel.load(Ordering::SeqCst) { "cancelled" } else { "error" }, bytes, error);
        },
    }
    result
}
async fn install_inner(app: &AppHandle, manager: &ComponentManager, component: &Component, cancel: Arc<AtomicBool>, repair: bool, consent: Option<recipe::Consent>) -> Result<ComponentStatus, String> {
    let root = root_path(app, true)?;
    let _lease = store::acquire_setup_lease(&root)?;
    if let Some(consent) = consent { store::record_consent(&root, &consent)?; }
    if !repair {
        let check_root = root.clone(); let check_component = component.clone(); let cancellation = cancel.clone();
        let retained_lease = _lease.clone();
        let previous = tauri::async_runtime::spawn_blocking(move || {
            let _lease = retained_lease;
            store::status_with_cancel(&check_root, &check_component, &cancellation)
        }).await.map_err(|e| e.to_string())?;
        archive::cancelled(&cancel)?;
        if previous.state == "installed" { return Ok(previous); }
    }
    let recovery_root = root.clone(); let retained_lease = _lease.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = retained_lease;
        store::recover_staging(&recovery_root)
    }).await.map_err(|e| e.to_string())??;
    archive::cancelled(&cancel)?;
    let multiplier = if matches!(component, Component::Runtime(runtime) if runtime.recipe.is_some()) { 2 } else { 1 };
    let needed = required_free_space(component.download_bytes().checked_mul(multiplier).ok_or("Download budget overflow")?, component.installed_bytes().checked_mul(multiplier).ok_or("Staging budget overflow")?)?;
    let available = fs2::available_space(&root).map_err(|e| format!("Cannot check free space for private components: {e}"))?;
    if available < needed { return Err(format!("Not enough free disk space for a safe staged install. Need {needed} bytes free; {available} bytes are available. Existing components and models were preserved.")); }
    let staging = store::Staging::create(&root)?;
    let lifetime = setup_process::SetupLifetime::new(&staging, &_lease);
    let files = match component {
        Component::Runtime(runtime) => {
            if let Some(recipe) = &runtime.recipe {
                assembly::prepare(app, manager, &root, &staging, runtime, recipe, &cancel, &lifetime).await?
            } else {
            let archive = runtime.archive.as_ref().ok_or("Runtime package has not been published")?;
            let download_path = staging.directory.join("runtime.zip");
            download::fetch(&archive.url, archive.bytes, &archive.sha256, catalog::Source::Runtime, &download_path, &cancel, |bytes| manager.report(app, "downloading", bytes, "Downloading the verified engine package")).await?;
            manager.report(app, "verifying", archive.bytes, "Engine download size and SHA-256 verified");
            manager.report(app, "extracting", archive.bytes, "Safely extracting the private engine package");
            let payload = staging.payload(); let max_bytes = runtime.installed_bytes; let max_files = runtime.max_files; let cancellation = cancel.clone();
            // Await normal cancellation; retained ownership also protects the
            // tree if this awaiting future is aborted during blocking work.
            let retained = lifetime.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _lifetime = retained;
                archive::extract(&download_path, &payload, max_bytes, max_files, &cancellation)
            }).await.map_err(|e| e.to_string())??
            }
        }
        Component::Model(model) => {
            let mut completed = 0u64; let mut files = Vec::new();
            for file in &model.files {
                archive::cancelled(&cancel)?;
                let relative = archive::relative_path(&file.path)?;
                let target = staging.payload().join(relative);
                if let Some(parent) = target.parent() { store::create_private_dir(parent)?; }
                download::fetch(&file.url, file.bytes, &file.sha256, catalog::Source::Model, &target, &cancel, |bytes| manager.report(app, "downloading", completed + bytes, "Downloading verified model files")).await?;
                completed += file.bytes;
                files.push(store::FileReceipt { path: file.path.clone(), bytes: file.bytes, sha256: file.sha256.clone() });
            }
            manager.report(app, "verifying", completed, "All model file sizes and SHA-256 hashes verified");
            files
        }
    };
    archive::cancelled(&cancel)?;
    // Serialize against optional inference and warm loaded models only now,
    // after network I/O. The worker slot remains locked through activation.
    let runtime = app.state::<crate::asr::AsrRuntime>();
    let _maintenance = runtime.begin_component_maintenance().await?;
    archive::cancelled(&cancel)?;
    if let Component::Runtime(runtime) = component {
        manager.report(app, "testing", component.download_bytes(), "Checking the private engine before activation");
        self_test(&staging.payload(), runtime, &cancel, &lifetime).await?;
    }
    archive::cancelled(&cancel)?;
    store::write_receipt(&staging.payload(), component, files)?;
    let payload = staging.payload(); let check_component = component.clone(); let cancellation = cancel.clone();
    let retained = lifetime.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _lifetime = retained;
        store::validate_files(&payload, &check_component, &cancellation)
    }).await.map_err(|e| e.to_string())??;
    let _store_write = cancellable(manager.store_gate.write(), &cancel).await?;
    manager.commit_boundary()?;
    manager.report(app, "activating", component.download_bytes(), "Activating the verified component; keeping the previous version");
    let check_component = component.clone(); let retained = lifetime.clone();
    let status = tauri::async_runtime::spawn_blocking(move || {
        let _lifetime = retained;
        // Drop this extra staging owner before the retained setup lock, also on
        // unwinding, so last-owner cleanup remains inside the lease lifetime.
        let staging = staging;
        store::commit(&root, &check_component, &staging)
    }).await.map_err(|e| e.to_string())??;
    if status.state != "installed" { return Err(status.error.unwrap_or_else(|| "Component activation could not be verified.".into())); }
    Ok(status)
}
#[tauri::command]
pub(crate) async fn remove_asr_component(app: AppHandle, request: ComponentRequest) -> Result<ComponentStatus, String> {
    let component = catalog::embedded()?.find(&request.component_id)?;
    let manager = app.state::<ComponentManager>(); let operation = manager.begin(&request, 0)?;
    let result: Result<ComponentStatus, String> = async {
        let root = root_path(&app, false)?;
        if !root.exists() { return Ok(store::status(&root, &component)); }
        let _lease = store::acquire_setup_lease(&root)?;
        let _store_write = cancellable(manager.store_gate.write(), &operation.cancel).await?;
        let runtime = app.state::<crate::asr::AsrRuntime>();
        let _maintenance = runtime.begin_component_maintenance().await?;
        let _use_lease = store::acquire_use_lease(&root, true)?;
        archive::cancelled(&operation.cancel)?;
        manager.commit_boundary()?;
        manager.report(&app, "removing", 0, "Removing only this app-owned component");
        let removing_component = component.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _lease = _lease;
            let _use_lease = _use_lease;
            store::remove(&root, &removing_component)?;
            Ok::<_, String>(store::status(&root, &removing_component))
        }).await.map_err(|e| e.to_string())?
    }.await;
    match &result {
        Ok(_) => manager.report(&app, "complete", 0, "Managed component removed. Other engines and user models were preserved."),
        Err(error) => manager.report(&app, if operation.cancel.load(Ordering::SeqCst) { "cancelled" } else { "error" }, 0, error),
    }
    result
}
fn self_test_script(backend: &str) -> Result<String, String> {
    let code = match backend {
        "faster-whisper" => "import faster_whisper, ctranslate2; assert ctranslate2.get_supported_compute_types('cpu')",
        // Installation checks that the packages load and a tensor executes.
        // A hosted runner may lack Metal. This fallback does not change the
        // actual worker's selected-Metal capability check or claim GPU coverage.
        "mlx-whisper" => "import mlx_whisper; import mlx.core as mx; mx.set_default_device(mx.gpu if mx.metal.is_available() else mx.cpu); mx.eval(mx.ones((1,)))",
        "qwen3-asr" => "from qwen_asr import Qwen3ASRModel, Qwen3ForcedAligner; import torch; assert (torch.tensor([1.0])+1).item()==2",
        _ => return Err("Unsupported component self-test backend.".into()),
    };
    let compatibility = if cfg!(windows) && backend == "qwen3-asr" {
        format!("{}\nluma_configure_numba_workqueue()\nluma_prepare_nagisa()\nluma_probe_numba_workqueue()\n", include_str!("asr/nagisa_compat.py"))
    } else { String::new() };
    let threading_check = if cfg!(windows) && backend == "qwen3-asr" {
        "luma_check_numba_workqueue(require_initialized=True)\n"
    } else { "" };
    Ok(format!("import sys, os\nsys.modules['vllm'] = None\ndef offline(event, args):\n    if event in ('socket.connect','socket.getaddrinfo','socket.bind','subprocess.Popen','os.system','os.posix_spawn','os.fork'):\n        raise RuntimeError('Private engine self-test is offline and cannot start child processes')\nsys.addaudithook(offline)\n{compatibility}{code}\n{threading_check}"))
}
fn self_test_failure(status: std::process::ExitStatus) -> String {
    let detail = match status.code() {
        Some(code) => format!("{status}; native code {code} (0x{:08x})", code as u32),
        None => status.to_string(),
    };
    format!("Private engine self-test failed ({detail}). The previous component is unchanged. Try Repair. If the OS reports a security block, use its supported review or recovery flow, or ask your administrator; do not disable protections.")
}
async fn self_test(root: &Path, runtime: &catalog::Runtime, cancel: &AtomicBool, lifetime: &setup_process::SetupLifetime) -> Result<(), String> {
    let executable = store::checked_path(root, &runtime.entrypoint)?; store::regular_file(&executable)?;
    let code = self_test_script(&runtime.backend)?;
    let mut command = tokio::process::Command::new(&executable);
    let temporary = setup_process::PrivateTemp::create(root)?;
    setup_process::configure(&mut command, root, &temporary)?;
    command.args(["-I", "-B", "-u", "-X", "utf8", "-c"]).arg(code).current_dir(root)
        .env_remove("PYTHONPATH").env_remove("PYTHONHOME")
        .env("HF_HUB_OFFLINE", "1").env("TRANSFORMERS_OFFLINE", "1").env("HF_DATASETS_OFFLINE", "1").env("HF_HUB_DISABLE_IMPLICIT_TOKEN", "1")
        .env("HF_HUB_DISABLE_TELEMETRY", "1").env("DO_NOT_TRACK", "1")
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
    crate::process_utils::hide_tokio_command_window(&mut command);
    let mut child = setup_process::OwnedChild::spawn(&mut command, lifetime, temporary).map_err(|e| format!("Cannot start private engine self-test: {e}"))?;
    let started = Instant::now();
    let result = loop {
        if cancel.load(Ordering::SeqCst) || started.elapsed() > Duration::from_secs(120) {
            break Err(if cancel.load(Ordering::SeqCst) { "Component setup cancelled." } else { "Private engine self-test timed out. The previous component is still installed." }.into());
        }
        match child.child_mut().try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(status)) => break Err(self_test_failure(status)),
            Ok(None) => tokio::time::sleep(POLL).await,
            Err(error) => break Err(format!("Cannot inspect private engine self-test: {error}")),
        }
    };
    child.finish(result.is_err()).await;
    result
}
pub(super) async fn cancellable<F: Future>(future: F, cancel: &AtomicBool) -> Result<F::Output, String> {
    let mut future = Box::pin(future);
    loop {
        archive::cancelled(cancel)?;
        if let Ok(result) = tokio::time::timeout(POLL, &mut future).await { return Ok(result); }
    }
}

fn required_free_space(download_bytes: u64, installed_bytes: u64) -> Result<u64, String> {
    let overhead = (installed_bytes / 20).clamp(256 * 1024 * 1024, 512 * 1024 * 1024);
    download_bytes.checked_add(installed_bytes).and_then(|b| b.checked_add(overhead)).ok_or_else(|| "Component disk-space estimate overflow".into())
}

/// Keep these shared leases for the entire managed worker lifetime, including
/// its warm/idle model cache. External/manual environments require no lease.
pub(crate) fn acquire_managed_use_leases(paths: &[&str]) -> Result<Vec<std::fs::File>, String> {
    let Some(root) = MANAGED_ROOT.get() else { return Ok(Vec::new()); };
    acquire_managed_use_leases_at(root, paths)
}
/// Called while the worker owns its shared use lease. A managed model alone is
/// not authority to alter an external/manual Python environment.
pub(crate) fn verified_managed_windows_qwen_runtime(python: &str) -> Result<bool, String> {
    let Some(root) = MANAGED_ROOT.get() else { return Ok(false); };
    verified_managed_windows_qwen_runtime_at(root, python, &catalog::embedded()?)
}
fn verified_managed_windows_qwen_runtime_at(root: &Path, python: &str, catalog: &catalog::Catalog) -> Result<bool, String> {
    if !store::uses_managed_root(root, &[python])? { return Ok(false); }
    let selected = std::fs::canonicalize(python).map_err(|e| e.to_string())?;
    for runtime in &catalog.runtimes {
        if runtime.platform == "windows-x64" && runtime.engine == "qwen3-asr" && runtime.backend == "qwen3-asr"
            && store::matches_active_interpreter(root, runtime, &selected)? {
            return Ok(true);
        }
    }
    Err("The selected managed Qwen interpreter is not an active verified runtime. Select the installed component or use Repair.".into())
}
/// Apply the cleanup reliability policy only to the owned CPU wheel recipe.
/// Receipt fingerprinting binds the wheel's final published hash and size, not
/// just its filename. A model lease or manual interpreter never selects this.
pub(crate) fn verified_managed_windows_ct2_cpu_runtime(python: &str) -> Result<bool, String> {
    let Some(root) = MANAGED_ROOT.get() else { return Ok(false); };
    verified_managed_windows_ct2_cpu_runtime_at(root, python, &catalog::embedded()?)
}
fn reviewed_windows_ct2_cpu_recipe(runtime: &catalog::Runtime) -> bool {
    if runtime.id != "faster-whisper-cpu-windows-x64" || runtime.platform != "windows-x64"
        || runtime.engine != "whisper-accelerated" || runtime.backend != "faster-whisper"
        || runtime.device != "cpu" || runtime.archive.is_some() { return false; }
    let Some(recipe) = &runtime.recipe else { return false; };
    let wheels: Vec<_> = recipe.wheels.iter().filter(|wheel| wheel.name.eq_ignore_ascii_case("ctranslate2")).collect();
    wheels.len() == 1 && wheels[0].name == "ctranslate2" && wheels[0].version == "4.8.2"
        && wheels[0].filename == catalog::CPU_WHEEL_FILENAME && wheels[0].url == catalog::CPU_WHEEL_URL
        && recipe.validate(runtime).is_ok()
}
fn verified_managed_windows_ct2_cpu_runtime_at(root: &Path, python: &str, catalog: &catalog::Catalog) -> Result<bool, String> {
    if !store::uses_managed_root(root, &[python])? { return Ok(false); }
    let selected = std::fs::canonicalize(python).map_err(|e| e.to_string())?;
    for runtime in &catalog.runtimes {
        if reviewed_windows_ct2_cpu_recipe(runtime)
            && store::matches_active_interpreter(root, runtime, &selected)? { return Ok(true); }
    }
    Err("The selected managed CPU interpreter is not the active verified CTranslate2 recipe. Select the installed component or use Repair.".into())
}
/// Native installer proof context. It cannot exist in a release build and does
/// not replace the production root/catalog. Every spawn holds a real use lease
/// and runs the same active-receipt/interpreter/recipe matcher as production.
#[cfg(test)]
pub(crate) struct ManagedCt2RuntimeProof { root: PathBuf, catalog: catalog::Catalog }
#[cfg(test)]
impl ManagedCt2RuntimeProof {
    fn new(root: &Path, catalog: &catalog::Catalog, python: &str) -> Result<Self, String> {
        let proof = Self { root: root.to_owned(), catalog: catalog.clone() };
        let _leases = proof.acquire(python)?;
        Ok(proof)
    }
    pub(crate) fn acquire(&self, python: &str) -> Result<Vec<std::fs::File>, String> {
        let leases = acquire_managed_use_leases_at(&self.root, &[python])?;
        if leases.is_empty() || !verified_managed_windows_ct2_cpu_runtime_at(&self.root, python, &self.catalog)? {
            return Err("The native CPU proof interpreter has no matching active managed receipt.".into());
        }
        Ok(leases)
    }
}
fn acquire_managed_use_leases_at(root: &Path, paths: &[&str]) -> Result<Vec<std::fs::File>, String> {
    if store::uses_managed_root(root, paths)? { Ok(vec![store::acquire_use_lease(root, false)?]) } else { Ok(Vec::new()) }
}
