//! Placement is reversible until the final-path checks and activation boundary.
use super::{catalog::Component, setup_process::{self, SetupLifetime}, store, ComponentManager, ComponentRequest};
use std::{fs, path::{Path, PathBuf}, process::Stdio, sync::{atomic::{AtomicBool, AtomicU8, Ordering}, Arc}, time::{Duration, Instant}};

const OLD: &[u8] = b"prior private runtime\n";
const NEW: &[u8] = b"new private runtime\n";
const ENTRYPOINT: &str = "python-fixture.exe";

struct Fixture {
    base: PathBuf,
    root: PathBuf,
    component: Component,
    prior_path: PathBuf,
    prior_journal: Vec<(PathBuf, Vec<u8>)>,
    prior_payload: Vec<(PathBuf, Vec<u8>)>,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("luma-candidate-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&base).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let root = base.join("components");
        store::prepare_root(&root).unwrap();
        let component = Component::Runtime(serde_json::from_value(serde_json::json!({
            "id":"faster-whisper-cpu-windows-x64", "version":"1", "label":"Candidate fixture", "platform":"windows-x64",
            "engine":"whisper-accelerated", "backend":"faster-whisper", "device":"cpu",
            "license":"Fixture", "license_url":"https://example.com/terms", "installed_bytes":1024,
            "max_files":10, "entrypoint":ENTRYPOINT
        })).unwrap());
        let mut fixture = Self { base, root, component, prior_path: PathBuf::new(), prior_journal: Vec::new(), prior_payload: Vec::new() };
        let _lease = store::acquire_setup_lease(&fixture.root).unwrap();
        let staging = fixture.staged(OLD);
        fixture.prior_path = PathBuf::from(store::commit(&fixture.root, &fixture.component, &staging).unwrap().path.unwrap());
        fixture.prior_journal = Self::snapshot(&fixture.journal());
        fixture.prior_payload = Self::snapshot(&fixture.prior_path);
        fixture
    }
    fn staged(&self, bytes: &[u8]) -> store::Staging {
        let staging = store::Staging::create(&self.root).unwrap();
        let executable = staging.payload().join(ENTRYPOINT);
        fs::write(&executable, bytes).unwrap();
        store::write_receipt(&staging.payload(), &self.component, vec![store::FileReceipt {
            path: ENTRYPOINT.into(), bytes: bytes.len() as u64, sha256: store::hash_file(&executable).unwrap(),
        }]).unwrap();
        store::validate_files(&staging.payload(), &self.component, &AtomicBool::new(false)).unwrap();
        staging
    }
    fn journal(&self) -> PathBuf { self.prior_path.parent().unwrap().parent().unwrap().join("activations") }
    fn snapshot(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files: Vec<_> = fs::read_dir(path).unwrap().map(|entry| {
            let entry = entry.unwrap();
            (PathBuf::from(entry.file_name()), fs::read(entry.path()).unwrap())
        }).collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }
    fn assert_prior_unchanged(&self) {
        assert_eq!(Self::snapshot(&self.journal()), self.prior_journal, "the exact prior activation journal must survive");
        assert_eq!(Self::snapshot(&self.prior_path), self.prior_payload, "the exact prior runtime payload must survive");
        let status = store::status(&self.root, &self.component);
        assert_eq!(status.state, "installed", "{:?}", status.error);
        assert_eq!(status.path.as_deref(), self.prior_path.to_str());
    }
    fn request(&self) -> ComponentRequest {
        ComponentRequest { component_id: self.component.id().into(), request_id: uuid::Uuid::new_v4().to_string(), plan_sha256: None, acknowledged_terms: Vec::new() }
    }
    fn setup_lease(&self) -> store::SetupLease {
        // Parallel Unix fork/exec can briefly inherit the released baseline
        // descriptor. Return the successfully acquired lease itself, without
        // creating a second release/reacquire window or changing production.
        let started = Instant::now();
        loop {
            match store::acquire_setup_lease(&self.root) {
                Ok(lease) => return lease,
                Err(error) => assert!(started.elapsed() < Duration::from_secs(10), "baseline setup lease did not release: {error}"),
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn child_task(&self, runtime: &tokio::runtime::Runtime, staging: &store::Staging, candidate: &store::VersionCandidate, lease: &store::SetupLease)
        -> tokio::task::JoinHandle<Result<(bool, Vec<u8>), String>> {
        let lifetime = SetupLifetime::new(staging, lease).with_candidate(candidate);
        let temporary = setup_process::PrivateTemp::create(candidate.path()).unwrap();
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        setup_process::configure(&mut command, candidate.path(), &temporary).unwrap();
        command.args(["--ignored", "--exact", "asr_components::setup_process::tests::native_setup_child", "--nocapture"])
            .env("LUMA_SETUP_CHILD_READY", self.base.join("ready"));
        runtime.spawn(async move {
            setup_process::run_owned(command, "candidate final-path fixture", 30, &AtomicBool::new(false), &lifetime, temporary).await
        })
    }
    async fn ready(&self, task: &mut tokio::task::JoinHandle<Result<(bool, Vec<u8>), String>>) -> Arc<ProcessProbe> {
        let started = Instant::now();
        loop {
            if let Ok(marker) = fs::read_to_string(self.base.join("ready")) {
                if let Ok(pid) = marker.trim().parse() { return Arc::new(ProcessProbe::new(pid)); }
            }
            if task.is_finished() {
                match task.await {
                    Ok(Ok((success, output))) => panic!("candidate child returned before readiness: success={success}; output={}", super::self_test_error_detail(&output)),
                    result => panic!("candidate child returned before readiness: {}", super::self_test_error_detail(format!("{result:?}").as_bytes())),
                }
            }
            assert!(started.elapsed() < Duration::from_secs(10), "candidate child did not become ready; marker={:?}", fs::read_to_string(self.base.join("ready")));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    fn observe_cleanup(&self, staging: &store::Staging, candidate: &store::VersionCandidate, finished: Arc<dyn Fn() -> bool + Send + Sync>) -> Arc<AtomicU8> {
        let observed = Arc::new(AtomicU8::new(0));
        let result = observed.clone(); let complete = finished.clone(); let root = self.root.clone(); let staging_path = staging.directory.clone();
        candidate.observe_cleanup(move || {
            let safe = complete() && staging_path.exists() && store::acquire_setup_lease(&root).is_err();
            result.fetch_or(if safe { 1 } else { 4 }, Ordering::SeqCst);
        });
        let result = observed.clone(); let root = self.root.clone(); let candidate_path = candidate.path().to_path_buf();
        staging.observe_cleanup(move || {
            let safe = finished() && !candidate_path.exists() && store::acquire_setup_lease(&root).is_err();
            result.fetch_or(if safe { 2 } else { 8 }, Ordering::SeqCst);
        });
        observed
    }
    fn await_cleanup(&self, candidate: &Path, staging: &Path) {
        let started = Instant::now();
        loop {
            let lease_error = store::acquire_setup_lease(&self.root).err();
            if !candidate.exists() && !staging.exists() && lease_error.is_none() { break; }
            assert!(started.elapsed() < Duration::from_secs(10),
                "candidate cleanup incomplete: candidate_exists={}; staging_exists={}; lease_error={lease_error:?}", candidate.exists(), staging.exists());
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.base); } }

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}
#[cfg(unix)]
extern "C" { fn kill(pid: i32, signal: i32) -> i32; }
struct ProcessProbe {
    #[cfg(unix)] pid: u32,
    #[cfg(windows)] handle: usize,
}
impl ProcessProbe {
    fn new(pid: u32) -> Self {
        #[cfg(windows)] {
            let handle = unsafe { OpenProcess(0x0010_0000, 0, pid) }; // SYNCHRONIZE only
            assert!(!handle.is_null(), "cannot observe ready native candidate child: {}", std::io::Error::last_os_error());
            Self { handle: handle as usize }
        }
        #[cfg(unix)] { Self { pid } }
    }
    fn exited(&self) -> bool {
        #[cfg(windows)] {
            let state = unsafe { WaitForSingleObject(self.handle as *mut std::ffi::c_void, 0) };
            assert!(state == 0 || state == 258, "native candidate process observation failed: {state}");
            state == 0
        }
        #[cfg(unix)] {
            if unsafe { kill(self.pid.try_into().unwrap(), 0) } == 0 { return false; }
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(3)); // ESRCH: no live process or zombie
            true
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessProbe { fn drop(&mut self) { unsafe { CloseHandle(self.handle as *mut std::ffi::c_void); } } }

#[test]
fn failed_final_validation_keeps_prior_journal_and_payload_identical() {
    let fixture = Fixture::new();
    let lease = fixture.setup_lease();
    let staging = fixture.staged(NEW);
    let candidate = store::VersionCandidate::plan(&fixture.root, &fixture.component, &lease).unwrap();
    let path = candidate.path().to_path_buf();
    candidate.place(&staging).unwrap();
    assert!(!staging.payload().exists());
    fixture.assert_prior_unchanged();
    fs::write(path.join(ENTRYPOINT), b"corrupted final path").unwrap();
    assert_eq!(store::validate_files(&path, &fixture.component, &AtomicBool::new(false)).unwrap_err(),
        "An installed component file is missing or failed SHA-256 verification. Use Repair.");
    let last_owner = candidate.clone(); drop(candidate);
    assert!(path.exists(), "the last candidate owner controls failed-placement cleanup");
    drop(last_owner);
    assert!(!path.exists());
    fixture.assert_prior_unchanged();
}

#[test]
fn cancellation_after_final_validation_keeps_prior_journal_and_payload_identical() {
    let fixture = Fixture::new();
    let lease = fixture.setup_lease();
    let staging = fixture.staged(NEW);
    let candidate = store::VersionCandidate::plan(&fixture.root, &fixture.component, &lease).unwrap();
    let path = candidate.path().to_path_buf();
    candidate.place(&staging).unwrap();
    store::validate_files(&path, &fixture.component, &AtomicBool::new(false)).unwrap();
    let manager = ComponentManager::default();
    let operation = manager.begin(&fixture.request(), 0).unwrap();
    operation.cancel.store(true, Ordering::SeqCst);
    assert_eq!(manager.commit_boundary().unwrap_err(), "Component setup cancelled.");
    drop(candidate);
    assert!(!path.exists());
    fixture.assert_prior_unchanged();
}

#[test]
fn final_path_activation_retains_old_and_new_versions() {
    let fixture = Fixture::new();
    let lease = fixture.setup_lease();
    let staging = fixture.staged(NEW);
    let candidate = store::VersionCandidate::plan(&fixture.root, &fixture.component, &lease).unwrap();
    let path = candidate.path().to_path_buf(); let staging_path = staging.directory.clone();
    assert_ne!(path, fixture.prior_path);
    assert_ne!(path, staging.payload());
    candidate.place(&staging).unwrap();
    assert!(!staging.payload().exists());
    fixture.assert_prior_unchanged();
    assert_eq!(store::validate_files(&path, &fixture.component, &AtomicBool::new(false)).unwrap(), NEW.len() as u64);
    let manager = ComponentManager::default();
    let _operation = manager.begin(&fixture.request(), 0).unwrap();
    manager.commit_boundary().unwrap();
    let status = candidate.activate().unwrap();
    assert_eq!(status.state, "installed");
    assert_eq!(status.path.as_deref(), path.to_str());
    assert_eq!(status.python_path.as_deref(), path.join(ENTRYPOINT).to_str());
    assert_eq!(status.installed_bytes, NEW.len() as u64);
    drop(candidate); drop(staging); drop(lease);
    let started = Instant::now();
    loop {
        let Err(error) = store::acquire_setup_lease(&fixture.root) else { break; };
        assert!(started.elapsed() < Duration::from_secs(10), "successful candidate retained its setup lease: {error}");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!staging_path.exists(), "an activated candidate must release its retained staging owner");
    assert_eq!(fs::read(path.join(ENTRYPOINT)).unwrap(), NEW);
    assert_eq!(Fixture::snapshot(&fixture.prior_path), fixture.prior_payload);
    let journal = Fixture::snapshot(&fixture.journal());
    assert_eq!(journal.len(), fixture.prior_journal.len() + 1);
    assert_eq!(&journal[..fixture.prior_journal.len()], fixture.prior_journal.as_slice());
    assert_eq!(store::status(&fixture.root, &fixture.component).path.as_deref(), path.to_str());
}

#[test]
fn failed_final_self_test_reaps_candidate_and_preserves_prior_journal_and_payload() {
    tauri::async_runtime::block_on(async {
        let fixture = Fixture::new();
        let lease = fixture.setup_lease();
        let staging = fixture.staged(NEW);
        let candidate = store::VersionCandidate::plan(&fixture.root, &fixture.component, &lease).unwrap();
        candidate.place(&staging).unwrap();
        let path = candidate.path().to_path_buf(); let staging_path = staging.directory.clone();
        let lifetime = SetupLifetime::new(&staging, &lease).with_candidate(&candidate);
        let temporary = setup_process::PrivateTemp::create(candidate.path()).unwrap();
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        setup_process::configure(&mut command, candidate.path(), &temporary).unwrap();
        command.args(["--ignored", "--exact", "asr_components::tests::native_self_test_diagnostic_child", "--nocapture"])
            .env("LUMA_SELF_TEST_CHILD_MODE", "failure").current_dir(candidate.path())
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        crate::process_utils::hide_tokio_command_window(&mut command);
        let mut child = setup_process::OwnedChild::spawn(&mut command, &lifetime, temporary).expect("start failing final-path self-test child");
        let process = Arc::new(ProcessProbe::new(child.child_mut().id().expect("unreaped self-test child has a PID")));
        let observed_process = process.clone();
        let observed = fixture.observe_cleanup(&staging, &candidate, Arc::new(move || observed_process.exited()));
        fixture.assert_prior_unchanged();
        drop(lifetime); drop(candidate); drop(staging); drop(lease);
        let error = super::wait_self_test(child, &AtomicBool::new(false), Duration::from_secs(10)).await.unwrap_err();
        assert!(error.contains("native code 9 (0x00000009)"), "{error}");
        assert!(error.ends_with("ImportError: private 模块 é"), "{error}");
        assert!(error.len() < 4600);
        assert!(!error.contains("stdout is not a self-test diagnostic"));
        fixture.await_cleanup(&path, &staging_path);
        assert!(process.exited());
        assert_eq!(observed.load(Ordering::SeqCst), 3, "failed final-path self-test is reaped before candidate and staging cleanup");
        fixture.assert_prior_unchanged();
    });
}

#[test]
fn cancelling_live_final_self_test_reaps_candidate_and_preserves_prior_journal_and_payload() {
    tauri::async_runtime::block_on(async {
        let fixture = Fixture::new();
        let lease = fixture.setup_lease();
        let staging = fixture.staged(NEW);
        let candidate = store::VersionCandidate::plan(&fixture.root, &fixture.component, &lease).unwrap();
        candidate.place(&staging).unwrap();
        let path = candidate.path().to_path_buf(); let staging_path = staging.directory.clone();
        let lifetime = SetupLifetime::new(&staging, &lease).with_candidate(&candidate);
        let temporary = setup_process::PrivateTemp::create(candidate.path()).unwrap();
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        setup_process::configure(&mut command, candidate.path(), &temporary).unwrap();
        command.args(["--ignored", "--exact", "asr_components::setup_process::tests::native_setup_child", "--nocapture"])
            .env("LUMA_SETUP_CHILD_READY", fixture.base.join("ready")).current_dir(candidate.path())
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        crate::process_utils::hide_tokio_command_window(&mut command);
        let child = setup_process::OwnedChild::spawn(&mut command, &lifetime, temporary).expect("start cancellable final-path self-test child");
        let cancel = Arc::new(AtomicBool::new(false)); let task_cancel = cancel.clone();
        let mut task = tokio::spawn(async move {
            super::wait_self_test(child, &task_cancel, Duration::from_secs(10)).await.map(|()| (true, Vec::new()))
        });
        let process = fixture.ready(&mut task).await;
        let observed_process = process.clone();
        let observed = fixture.observe_cleanup(&staging, &candidate, Arc::new(move || observed_process.exited()));
        drop(lifetime); drop(candidate); drop(staging); drop(lease);
        assert!(!process.exited(), "cancellation must be requested while the final-path child is live");
        assert!(path.exists() && staging_path.exists());
        assert!(store::acquire_setup_lease(&fixture.root).is_err());
        fixture.assert_prior_unchanged();
        cancel.store(true, Ordering::SeqCst);
        assert_eq!(task.await.unwrap().unwrap_err(), "Component setup cancelled.");
        fixture.await_cleanup(&path, &staging_path);
        assert!(process.exited());
        assert_eq!(observed.load(Ordering::SeqCst), 3, "live cancellation reaps before candidate cleanup, staging cleanup, and unlock");
        fixture.assert_prior_unchanged();
    });
}

#[test]
fn candidate_future_abort_reaps_before_candidate_staging_cleanup_and_unlock() {
    let fixture = Fixture::new();
    let lease = fixture.setup_lease();
    let staging = fixture.staged(NEW);
    let candidate = store::VersionCandidate::plan(&fixture.root, &fixture.component, &lease).unwrap();
    candidate.place(&staging).unwrap();
    let path = candidate.path().to_path_buf(); let staging_path = staging.directory.clone();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut task = fixture.child_task(&runtime, &staging, &candidate, &lease);
    let process = runtime.block_on(fixture.ready(&mut task));
    let observed_process = process.clone();
    let observed = fixture.observe_cleanup(&staging, &candidate, Arc::new(move || observed_process.exited()));
    drop(candidate); drop(staging); drop(lease);
    assert!(!process.exited());
    assert!(path.exists() && staging_path.exists());
    assert!(store::acquire_setup_lease(&fixture.root).is_err());
    task.abort();
    assert!(runtime.block_on(task).unwrap_err().is_cancelled());
    fixture.await_cleanup(&path, &staging_path);
    assert!(process.exited());
    assert_eq!(observed.load(Ordering::SeqCst), 3, "candidate then staging clean up after reaping, while setup remains locked");
    fixture.assert_prior_unchanged();
}

#[test]
fn candidate_runtime_teardown_reaps_before_candidate_staging_cleanup_and_unlock() {
    let fixture = Fixture::new();
    let lease = fixture.setup_lease();
    let staging = fixture.staged(NEW);
    let candidate = store::VersionCandidate::plan(&fixture.root, &fixture.component, &lease).unwrap();
    candidate.place(&staging).unwrap();
    let path = candidate.path().to_path_buf(); let staging_path = staging.directory.clone();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut task = fixture.child_task(&runtime, &staging, &candidate, &lease);
    let process = runtime.block_on(fixture.ready(&mut task));
    let observed_process = process.clone();
    let observed = fixture.observe_cleanup(&staging, &candidate, Arc::new(move || observed_process.exited()));
    drop(candidate); drop(staging); drop(lease);
    assert!(!process.exited());
    assert!(path.exists() && staging_path.exists());
    assert!(store::acquire_setup_lease(&fixture.root).is_err());
    drop(runtime); drop(task);
    fixture.await_cleanup(&path, &staging_path);
    assert!(process.exited());
    assert_eq!(observed.load(Ordering::SeqCst), 3, "OS reaper retains the final candidate without a Tokio runtime");
    fixture.assert_prior_unchanged();
}

#[test]
fn abort_during_blocked_placement_or_validation_retains_candidate_staging_and_lease() {
    tauri::async_runtime::block_on(async {
        for before_placement in [true, false] {
            let fixture = Fixture::new();
            let lease = fixture.setup_lease();
            let staging = fixture.staged(NEW);
            let candidate = store::VersionCandidate::plan(&fixture.root, &fixture.component, &lease).unwrap();
            let path = candidate.path().to_path_buf(); let staging_path = staging.directory.clone();
            let lifetime = SetupLifetime::new(&staging, &lease).with_candidate(&candidate);
            let finished = Arc::new(AtomicBool::new(false)); let cleanup_finished = finished.clone();
            let observed = fixture.observe_cleanup(&staging, &candidate, Arc::new(move || cleanup_finished.load(Ordering::SeqCst)));
            let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
            let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
            let component = fixture.component.clone(); let work_finished = finished.clone();
            drop(lease);
            let task = tokio::spawn(async move {
                tokio::task::spawn_blocking(move || -> Result<(), String> {
                    let _lifetime = lifetime;
                    let candidate = candidate; let staging = staging;
                    if !before_placement { candidate.place(&staging)?; }
                    started_tx.send(()).map_err(|e| e.to_string())?;
                    release_rx.recv_timeout(Duration::from_secs(10)).map_err(|e| format!("release blocked candidate fixture: {e}"))?;
                    if before_placement { candidate.place(&staging)?; }
                    store::validate_files(candidate.path(), &component, &AtomicBool::new(false))?;
                    work_finished.store(true, Ordering::SeqCst);
                    Ok(())
                }).await.expect("candidate blocking worker panicked")
            });
            let started = Instant::now();
            loop {
                match started_rx.try_recv() {
                    Ok(()) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => (),
                    Err(error) => panic!("candidate worker ended before readiness (before_placement={before_placement}): {error}; result={:?}", task.await),
                }
                if task.is_finished() { panic!("candidate worker returned before readiness (before_placement={before_placement}): {:?}", task.await); }
                assert!(started.elapsed() < Duration::from_secs(10), "candidate worker did not reach gate (before_placement={before_placement})");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            task.abort(); assert!(task.await.unwrap_err().is_cancelled());
            assert!(staging_path.exists());
            assert_eq!(path.exists(), !before_placement);
            assert!(store::acquire_setup_lease(&fixture.root).is_err());
            assert!(!finished.load(Ordering::SeqCst));
            assert_eq!(observed.load(Ordering::SeqCst), 0);
            fixture.assert_prior_unchanged();
            release_tx.send(()).unwrap();
            fixture.await_cleanup(&path, &staging_path);
            assert!(finished.load(Ordering::SeqCst));
            assert_eq!(observed.load(Ordering::SeqCst), 3, "blocking work completes before candidate cleanup, staging cleanup, and unlock");
            fixture.assert_prior_unchanged();
        }
    });
}
