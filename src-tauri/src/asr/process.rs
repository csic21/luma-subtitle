//! One owned worker, serialized across optional-engine jobs. No daemon or network port.
use super::AsrConfig;
use crate::{
    process_utils::hide_tokio_command_window,
    state::{JobError, JobResult},
};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex as AsyncMutex,
};

const POLL: Duration = Duration::from_millis(100);
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 4096;
const WORKER_SOURCE: &str = include_str!("worker.py");
const NAGISA_COMPAT_SOURCE: &str = include_str!("nagisa_compat.py");
const NAGISA_COMPAT_MARKER: &str = "# LUMA_NAGISA_COMPAT_SOURCE";
const CT2_CPU_POLICY_MARKER: &str = "# LUMA_MANAGED_CT2_CPU_POLICY";
const CT2_CPU_POLICY_SOURCE: &str = "LUMA_MANAGED_CT2_CPU_POLICY = \"luma-cpu-seq-1\"";

fn prepared_worker_source(source: &str, managed_windows_qwen: bool, managed_windows_ct2_cpu: bool) -> Result<String, String> {
    if managed_windows_qwen && managed_windows_ct2_cpu {
        return Err("A managed worker cannot select two runtime policies.".into());
    }
    if managed_windows_ct2_cpu {
        if source.matches(CT2_CPU_POLICY_MARKER).count() != 1 {
            return Err("Managed CPU runtime policy marker is missing or ambiguous.".into());
        }
        return Ok(source.replacen(CT2_CPU_POLICY_MARKER, CT2_CPU_POLICY_SOURCE, 1));
    }
    if !managed_windows_qwen { return Ok(source.to_owned()); }
    if source.matches(NAGISA_COMPAT_MARKER).count() != 1 {
        return Err("Managed Qwen compatibility source marker is missing or ambiguous.".into());
    }
    Ok(source.replacen(NAGISA_COMPAT_MARKER,
        &format!("{NAGISA_COMPAT_SOURCE}\nLUMA_MANAGED_QWEN_RUNTIME = True\n"), 1))
}

// Called only after the active managed-runtime receipt check. This sets the
// child's cwd, never the application's cwd, so Numba cannot adopt a launcher's
// unrelated .numba_config.yaml. External runtimes keep their existing behavior.
fn managed_worker_directory(python: &str, managed_windows_qwen: bool) -> JobResult<Option<PathBuf>> {
    if !managed_windows_qwen {
        return Ok(None);
    }
    let path = Path::new(python);
    if !path.is_absolute() {
        return Err(JobError::failed("The verified managed interpreter must have an absolute path."));
    }
    let executable = std::fs::canonicalize(path)
        .map_err(|e| JobError::failed(format!("Cannot resolve the managed runtime directory: {e}")))?;
    if !executable.is_file() {
        return Err(JobError::failed("The verified managed interpreter is not a regular file."));
    }
    let directory = executable.parent()
        .ok_or_else(|| JobError::failed("The managed interpreter has no runtime directory."))?;
    Ok(Some(directory.to_path_buf()))
}

#[derive(Default)]
pub(crate) struct AsrRuntime {
    worker: AsyncMutex<Option<Worker>>,
    stopping: AtomicBool,
    #[cfg(test)]
    ct2_receipt_proof: Option<crate::asr_components::ManagedCt2RuntimeProof>,
}

/// Excludes transcription/probes while a managed component is validated or
/// activated. The installer must never hold this guard during a download.
pub(crate) struct ComponentMaintenanceGuard<'a> {
    _slot: tokio::sync::MutexGuard<'a, Option<Worker>>,
}

impl AsrRuntime {
    fn spawn_worker(&self, config: &AsrConfig) -> JobResult<Worker> {
        #[cfg(test)]
        if let Some(proof) = &self.ct2_receipt_proof {
            return Worker::spawn_for_receipt_proof(config, WORKER_SOURCE, proof);
        }
        Worker::spawn(config, WORKER_SOURCE)
    }
    pub(crate) async fn begin_component_maintenance(
        &self,
    ) -> Result<ComponentMaintenanceGuard<'_>, String> {
        if self.stopping.load(Ordering::SeqCst) {
            return Err("The application is closing; component maintenance was cancelled.".into());
        }
        let mut slot = self.worker.try_lock().map_err(|_| {
            "ASR is running. Cancel or wait for the current task before installing, repairing or removing its components.".to_string()
        })?;
        if self.stopping.load(Ordering::SeqCst) {
            return Err("The application is closing; component maintenance was cancelled.".into());
        }
        if let Some(mut worker) = slot.take() {
            worker.stop().await;
        }
        Ok(ComponentMaintenanceGuard { _slot: slot })
    }

    pub(crate) async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(mut worker) = self.worker.lock().await.take() {
            worker.stop().await;
        }
    }
    pub(crate) async fn release_idle(&self) -> Result<bool, String> {
        let mut slot = self.worker.try_lock().map_err(|_| {
            "ASR is running. Cancel or wait for the current task before releasing its model."
                .to_string()
        })?;
        if let Some(mut worker) = slot.take() {
            worker.stop().await;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    pub(crate) async fn release_for_legacy(&self, cancel: &AtomicBool) -> JobResult<()> {
        loop {
            check_cancel(cancel, &self.stopping)?;
            if let Ok(mut slot) = self.worker.try_lock() {
                if let Some(mut worker) = slot.take() {
                    worker.stop().await;
                }
                return Ok(());
            }
            tokio::time::sleep(POLL).await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn request(
        &self,
        config: &AsrConfig,
        operation: &str,
        audio: Option<&Path>,
        language: Option<&str>,
        cancel: Arc<AtomicBool>,
        limit: Option<Duration>,
        mut progress: impl FnMut(&Value),
    ) -> JobResult<Value> {
        let mut slot = loop {
            check_cancel(&cancel, &self.stopping)?;
            if let Ok(slot) = self.worker.try_lock() {
                break slot;
            }
            if operation == "probe" {
                return Err(JobError::failed("ASR is running. Wait or cancel the current transcription before checking another backend."));
            }
            tokio::time::sleep(POLL).await;
        };
        // Move the child into this future. Dropping an interrupted future kills it
        // instead of returning an in-flight worker with unread responses to the pool.
        let mut worker = if let Some(mut worker) = slot.take() {
            if worker.config == *config
                && worker
                    .child
                    .as_mut()
                    .expect("worker owns its child")
                    .try_wait()
                    .map_err(|e| JobError::failed(e.to_string()))?
                    .is_none()
            {
                worker
            } else {
                worker.stop().await;
                self.spawn_worker(config)?
            }
        } else {
            self.spawn_worker(config)?
        };
        let id = uuid::Uuid::new_v4().to_string();
        let payload = json!({"id":id,"op":operation,"engine":config.engine,"model_path":config.model_path,"aligner_path":config.aligner_path,"device":config.device,"language":language.unwrap_or("auto"),"audio_path":audio.map(|p| p.to_string_lossy().to_string())});
        let result = worker
            .exchange(&id, &payload, &cancel, &self.stopping, limit, &mut progress)
            .await;
        match result {
            Ok(value) => {
                *slot = Some(worker);
                Ok(value)
            }
            Err(error) => {
                worker.stop().await;
                Err(error)
            }
        }
    }
}

struct Worker {
    script_path: PathBuf,
    config: AsrConfig,
    child: Option<Child>,
    managed_use_leases: Vec<std::fs::File>,
    stdin: ChildStdin,
    stdout: ChildStdout,
    pending: Vec<u8>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    stderr_reader: tokio::task::JoinHandle<()>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Keep cross-process storage leases until the OS has reaped the child,
        // even if an async request is dropped or the Tokio runtime is closing.
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let leases = Arc::new(std::mem::take(&mut self.managed_use_leases));
                let retained = leases.clone();
                let reaper = std::thread::Builder::new().name("luma-asr-reaper".into()).spawn(move || {
                    let _leases = retained;
                    loop {
                        match child.try_wait() {
                            Ok(Some(_)) => break,
                            _ => { let _ = child.start_kill(); std::thread::sleep(POLL); }
                        }
                    }
                });
                if reaper.is_err() {
                    // Conservatively retain the lock until app exit if an OS
                    // thread cannot be created. Never unlock a live worker.
                    std::mem::forget(leases);
                }
            }
        }
        self.stderr_reader.abort();
        let _ = std::fs::remove_file(&self.script_path);
    }
}
impl Worker {
    fn spawn(config: &AsrConfig, source: &str) -> JobResult<Self> {
        let managed_use_leases = crate::asr_components::acquire_managed_use_leases(
            &managed_config_paths(config),
        ).map_err(JobError::failed)?;
        let managed_windows_qwen = cfg!(windows) && config.engine == "qwen3-asr"
            && crate::asr_components::verified_managed_windows_qwen_runtime(&config.python_path)
                .map_err(JobError::failed)?;
        let managed_windows_ct2_cpu = cfg!(windows) && config.engine == "whisper-accelerated"
            && matches!(config.device.as_str(), "cpu" | "auto")
            && crate::asr_components::verified_managed_windows_ct2_cpu_runtime(&config.python_path)
                .map_err(JobError::failed)?;
        let source = prepared_worker_source(source, managed_windows_qwen, managed_windows_ct2_cpu).map_err(JobError::failed)?;
        let working_directory = managed_worker_directory(&config.python_path, managed_windows_qwen)?;
        Self::spawn_prepared(config, &source, managed_use_leases, working_directory)
    }
    #[cfg(test)]
    fn spawn_for_receipt_proof(config: &AsrConfig, source: &str, proof: &crate::asr_components::ManagedCt2RuntimeProof) -> JobResult<Self> {
        if !cfg!(windows) || config.engine != "whisper-accelerated" || !matches!(config.device.as_str(), "cpu" | "auto") {
            return Err(JobError::failed("The test-owned receipt proof requires the managed Windows CPU recipe."));
        }
        let leases = proof.acquire(&config.python_path).map_err(JobError::failed)?;
        let source = prepared_worker_source(source, false, true).map_err(JobError::failed)?;
        Self::spawn_prepared(config, &source, leases, None)
    }
    fn spawn_prepared(config: &AsrConfig, source: &str, managed_use_leases: Vec<std::fs::File>, working_directory: Option<PathBuf>) -> JobResult<Self> {
        // A file avoids Windows' 32K command-line limit. It contains only the
        // embedded application code, never user media, settings or credentials.
        let script_path =
            std::env::temp_dir().join(format!("luma-asr-{}.py", uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let written = options
            .open(&script_path)
            .and_then(|mut file| file.write_all(source.as_bytes()));
        if let Err(error) = written {
            let _ = std::fs::remove_file(&script_path);
            return Err(JobError::failed(format!(
                "Cannot prepare embedded ASR worker: {error}"
            )));
        }
        let mut command = Command::new(&config.python_path);
        command
            .args(["-I", "-B", "-u", "-X", "utf8"])
            .arg(&script_path)
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("HF_HUB_DISABLE_TELEMETRY", "1")
            .env("DO_NOT_TRACK", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(directory) = working_directory {
            command.current_dir(directory);
        }
        hide_tokio_command_window(&mut command);
        let mut child = command.spawn().map_err(|e| {
            let _ = std::fs::remove_file(&script_path);
            JobError::failed(format!("Cannot start the optional ASR engine component: {e}. Install or repair its component in ASR settings, check an advanced external runtime if configured, or select whisper.cpp. Operating-system security restrictions are not bypassed."))
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| JobError::failed("ASR worker stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| JobError::failed("ASR worker stdout unavailable"))?;
        let mut pipe = child
            .stderr
            .take()
            .ok_or_else(|| JobError::failed("ASR worker stderr unavailable"))?;
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let captured = stderr.clone();
        let stderr_reader = tokio::spawn(async move {
            let mut bytes = [0u8; 2048];
            while let Ok(count) = pipe.read(&mut bytes).await {
                if count == 0 {
                    break;
                }
                let mut tail = captured.lock();
                tail.extend(&bytes[..count]);
                let excess = tail.len().saturating_sub(MAX_STDERR_BYTES);
                tail.drain(..excess);
            }
        });
        Ok(Self {
            script_path,
            config: config.clone(),
            child: Some(child),
            managed_use_leases,
            stdin,
            stdout,
            pending: Vec::new(),
            stderr,
            stderr_reader,
        })
    }
    async fn stop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        self.stderr_reader.abort();
    }
    fn detail(&self) -> String {
        let bytes: Vec<u8> = self.stderr.lock().iter().copied().collect();
        let text = String::from_utf8_lossy(&bytes);
        if text.trim().is_empty() {
            String::new()
        } else {
            format!("\n{}", text.trim())
        }
    }
    async fn exchange(
        &mut self,
        id: &str,
        payload: &Value,
        cancel: &AtomicBool,
        stopping: &AtomicBool,
        limit: Option<Duration>,
        progress: &mut impl FnMut(&Value),
    ) -> JobResult<Value> {
        check_cancel(cancel, stopping)?;
        let mut body = serde_json::to_vec(payload).map_err(|e| JobError::failed(e.to_string()))?;
        body.push(b'\n');
        tokio::time::timeout(Duration::from_secs(5), self.stdin.write_all(&body))
            .await
            .map_err(|_| {
                JobError::failed(
                    "ASR worker did not accept the request. Retry to start a fresh worker.",
                )
            })?
            .map_err(|e| {
                JobError::failed(format!("ASR worker pipe failed: {e}{}", self.detail()))
            })?;
        let started = Instant::now();
        loop {
            check_cancel(cancel, stopping)?;
            if limit.is_some_and(|limit| started.elapsed() >= limit) {
                return Err(JobError::failed("ASR environment check timed out. Verify the selected Python environment and retry."));
            }
            if let Some(newline) = self.pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=newline).collect();
                let event: Value = serde_json::from_slice(&line).map_err(|_| JobError::failed(format!("ASR worker returned invalid protocol output. Check the optional runtime installation.{}", self.detail())))?;
                if event.get("id").and_then(Value::as_str) != Some(id) {
                    return Err(JobError::failed(
                        "ASR response belongs to another request; worker has been reset.",
                    ));
                }
                match event.get("event").and_then(Value::as_str) {
                    Some("progress") => progress(&event),
                    Some("probe" | "result") => {
                        check_cancel(cancel, stopping)?;
                        return Ok(event);
                    }
                    Some("error") => {
                        return Err(JobError::failed(
                            event
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("Optional ASR worker failed")
                                .to_string(),
                        ))
                    }
                    _ => return Err(JobError::failed("ASR worker returned an unknown event.")),
                }
                continue;
            }
            let mut bytes = [0u8; 8192];
            match tokio::time::timeout(POLL, self.stdout.read(&mut bytes)).await {
                Err(_) => continue,
                Ok(Err(error)) => return Err(JobError::failed(format!("ASR worker output failed: {error}{}", self.detail()))),
                Ok(Ok(0)) => return Err(JobError::failed(format!("Optional ASR worker exited before returning subtitles. Retry after checking runtime/model compatibility.{}", self.detail()))),
                Ok(Ok(count)) => {
                    if self.pending.len().saturating_add(count) > MAX_RESPONSE_BYTES { return Err(JobError::failed("ASR response exceeded the safe size limit.")); }
                    self.pending.extend_from_slice(&bytes[..count]);
                }
            }
        }
    }
}
fn managed_config_paths(config: &AsrConfig) -> Vec<&str> {
    let mut paths = vec![config.python_path.as_str(), config.model_path.as_str()];
    // Settings retain engine-specific values when switching engines. Whisper
    // must not resolve an irrelevant, possibly removed Qwen aligner path.
    if config.engine == "qwen3-asr" {
        paths.push(config.aligner_path.as_str());
    }
    paths
}
fn check_cancel(cancel: &AtomicBool, stopping: &AtomicBool) -> JobResult<()> {
    if cancel.load(Ordering::SeqCst) || stopping.load(Ordering::SeqCst) {
        Err(JobError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod real_worker_lifecycle {
    use super::*;

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
        pid: u32,
        #[cfg(windows)] handle: usize,
    }
    impl ProcessProbe {
        fn new(pid: u32) -> Self {
            #[cfg(windows)] {
                let handle = unsafe { OpenProcess(0x0010_0000, 0, pid) }; // SYNCHRONIZE only
                assert!(!handle.is_null(), "open the live ASR worker for exit observation");
                Self { pid, handle: handle as usize }
            }
            #[cfg(unix)] { Self { pid } }
        }
        fn exited(&self) -> bool {
            #[cfg(windows)] {
                let state = unsafe { WaitForSingleObject(self.handle as *mut std::ffi::c_void, 0) };
                assert!(state == 0 || state == 258, "native ASR process observation failed");
                state == 0
            }
            #[cfg(unix)] {
                if unsafe { kill(self.pid.try_into().unwrap(), 0) } == 0 { return false; }
                assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(3)); // ESRCH, no zombie
                true
            }
        }
    }
    #[cfg(windows)]
    impl Drop for ProcessProbe {
        fn drop(&mut self) { unsafe { CloseHandle(self.handle as *mut std::ffi::c_void); } }
    }

    pub(super) struct LeaseObservation {
        process: Arc<ProcessProbe>,
        script: PathBuf,
        lease_path: PathBuf,
        cancel: Arc<AtomicBool>,
        observer: Option<std::thread::JoinHandle<Result<Value, String>>>,
    }
    impl LeaseObservation {
        fn start(pid: u32, script: PathBuf, lease_path: PathBuf, label: &'static str) -> Self {
            let process = Arc::new(ProcessProbe::new(pid));
            assert!(!process.exited(), "observe a live loaded-model worker");
            let file = std::fs::OpenOptions::new().read(true).write(true).open(&lease_path).unwrap();
            assert!(fs2::FileExt::try_lock_exclusive(&file).is_err(), "worker must retain the test lease");
            let cancel = Arc::new(AtomicBool::new(false));
            let stop = cancel.clone(); let child = process.clone();
            let observer = std::thread::Builder::new().name("luma-asr-test-lease-observer".into()).spawn(move || {
                let started = Instant::now();
                loop {
                    if stop.load(Ordering::SeqCst) { return Err("lease observer cancelled during fixture cleanup".into()); }
                    match fs2::FileExt::try_lock_exclusive(&file) {
                        Ok(()) => {
                            if !child.exited() { return Err(format!("{label}: lease became available while PID {pid} was alive")); }
                            return Ok(json!({"operation":label,"pid":pid,"process_exited_when_lease_available":true,
                                "lease_wait_ms":started.elapsed().as_millis(),"lease_kind":"injected test-owned shared use lease"}));
                        }
                        Err(error) if started.elapsed() >= Duration::from_secs(30) =>
                            return Err(format!("{label}: lease observation timed out: {error}")),
                        Err(_) => std::thread::sleep(Duration::from_millis(2)),
                    }
                }
            }).unwrap();
            Self { process, script, lease_path, cancel, observer:Some(observer) }
        }
        pub(super) fn pid(&self) -> u32 { self.process.pid }
        fn result(&mut self) -> Result<Value,String> {
            self.observer.take().unwrap().join().map_err(|_| "lease observer panicked".to_owned())?
        }
        pub(super) fn finish(mut self) -> Value {
            let mut result = self.result().unwrap();
            assert!(self.process.exited(), "worker must be reaped before successful transition");
            assert!(!self.script.exists(), "stopped worker script must be removed");
            result["script_removed"] = json!(true);
            result
        }
    }
    impl Drop for LeaseObservation {
        fn drop(&mut self) {
            self.cancel.store(true,Ordering::SeqCst);
            if let Some(observer) = self.observer.take() { let _ = observer.join(); }
            let _ = std::fs::remove_file(&self.lease_path);
        }
    }

    pub(super) async fn observe(runtime: &AsrRuntime, output: &Path, label: &'static str) -> LeaseObservation {
        let mut slot = runtime.worker.lock().await;
        let worker = slot.as_mut().expect("actual model must be loaded before observing a lifecycle transition");
        let child = worker.child.as_mut().unwrap();
        assert!(child.try_wait().unwrap().is_none());
        let pid = child.id().unwrap();
        let path = output.join(format!("{label}-{}.lease",uuid::Uuid::new_v4()));
        let lease = std::fs::OpenOptions::new().create_new(true).read(true).write(true).open(&path).unwrap();
        fs2::FileExt::lock_shared(&lease).unwrap();
        // Exercise the real Worker's retention/drop path. This does not claim
        // that the fixture's paths were registered in global MANAGED_ROOT.
        worker.managed_use_leases.push(lease);
        LeaseObservation::start(pid,worker.script_path.clone(),path,label)
    }

    pub(super) async fn pid(runtime: &AsrRuntime) -> u32 {
        runtime.worker.lock().await.as_ref().unwrap().child.as_ref().unwrap().id().unwrap()
    }

    pub(super) fn verify_file(path: &Path, bytes: u64, digest: &str) {
        use sha2::Digest;
        use std::io::Read;
        assert!(std::fs::symlink_metadata(path).unwrap().is_file(),"fixture must be a regular file");
        assert_eq!(std::fs::metadata(path).unwrap().len(),bytes);
        let mut file = std::fs::File::open(path).unwrap();
        let mut hash = sha2::Sha256::new(); let mut buffer = [0u8;65536];
        loop { let count = file.read(&mut buffer).unwrap(); if count == 0 { break; } hash.update(&buffer[..count]); }
        assert_eq!(format!("{:x}",hash.finalize()),digest,"fixture bytes must match the pinned public source");
    }
    pub(super) fn checkpoint(output: &Path, report: &Value) {
        std::fs::write(output.join("lifecycle.json"),serde_json::to_vec_pretty(report).unwrap()).unwrap();
    }
    pub(super) fn transition(output: &Path, report: &mut Value, result: Value) {
        println!("REAL_OPTIONAL_WORKER_TRANSITION {result}");
        report["transitions"].as_array_mut().unwrap().push(result);
        checkpoint(output,report);
    }
    pub(super) async fn cold_transcription(runtime: &AsrRuntime, config: &AsrConfig, audio: &Path, duration: u64) -> Value {
        let result = tokio::time::timeout(Duration::from_secs(180),runtime.request(config,"transcribe",Some(audio),Some("en"),
            Arc::new(AtomicBool::new(false)),Some(Duration::from_secs(180)),|_|{})).await.unwrap().unwrap();
        assert_eq!(result["reused"],false); assert_eq!(result["backend"],"faster-whisper"); assert_eq!(result["device"],"cpu");
        let segments = super::super::validate_segments(&result,duration).unwrap();
        assert!(crate::subtitles::render_srt(&segments,None).to_lowercase().contains("country"));
        result
    }

    #[test]
    fn lease_observer_rejects_unlock_while_process_is_alive() {
        let path = std::env::temp_dir().join(format!("luma-early-unlock-{}.lease",uuid::Uuid::new_v4()));
        let lease = std::fs::OpenOptions::new().create_new(true).read(true).write(true).open(&path).unwrap();
        fs2::FileExt::lock_shared(&lease).unwrap();
        let mut observer = LeaseObservation::start(std::process::id(),PathBuf::new(),path,"early-unlock-negative");
        drop(lease);
        assert!(observer.result().unwrap_err().contains("was alive"));
    }

    #[test]
    fn real_worker_release_observer_uses_private_lease_and_native_process_exit() {
        tauri::async_runtime::block_on(async {
            let output = std::env::temp_dir().join(format!("luma-worker-observer-{}",uuid::Uuid::new_v4()));
            std::fs::create_dir(&output).unwrap();
            let output = std::fs::canonicalize(output).unwrap();
            let config = AsrConfig { python_path:which::which("python3").or_else(|_|which::which("python")).unwrap().to_string_lossy().into(),..AsrConfig::default() };
            let runtime = AsrRuntime::default();
            *runtime.worker.lock().await = Some(Worker::spawn(&config,"import time\ntime.sleep(30)").unwrap());
            let observer = observe(&runtime,&output,"lightweight-release").await;
            assert!(tokio::time::timeout(Duration::from_secs(15),runtime.release_idle()).await.unwrap().unwrap());
            let result = observer.finish();
            assert_eq!(result["process_exited_when_lease_available"],true);
            std::fs::remove_dir(output).unwrap();
        });
    }
}

#[cfg(test)]
pub(crate) async fn run_real_optional_worker_fixture(
    config: &AsrConfig,
    replacement_model: &Path,
    audio: &Path,
    long_audio: &Path,
    output: &Path,
    ct2_receipt_proof: Option<crate::asr_components::ManagedCt2RuntimeProof>,
) {
    // Shared by the opt-in environment wrapper and the real managed installer
    // fixture. The assertions exercise the same production worker lifecycle.
    config.validate().unwrap();
    std::fs::create_dir_all(output).unwrap();
    let output = std::fs::canonicalize(output).unwrap();
    let model = std::fs::canonicalize(&config.model_path).unwrap();
    let replacement_model = std::fs::canonicalize(replacement_model).unwrap();
    assert_ne!(model,replacement_model,"replacement must select a genuinely distinct model directory");
    let pins: Value = serde_json::from_str(include_str!("../../../scripts/asr-components/fixtures.json")).unwrap();
    for directory in [&model,&replacement_model] {
        for pin in pins["faster_whisper_tiny"]["files"].as_array().unwrap() {
            real_worker_lifecycle::verify_file(&directory.join(pin["path"].as_str().unwrap()),pin["bytes"].as_u64().unwrap(),pin["sha256"].as_str().unwrap());
        }
    }
    real_worker_lifecycle::verify_file(audio,pins["audio"]["bytes"].as_u64().unwrap(),pins["audio"]["sha256"].as_str().unwrap());
    assert!(std::fs::metadata(long_audio).unwrap().len() <= 36*1024*1024);
    assert_eq!(super::wav_duration_ms(long_audio).unwrap(),1_100_000);
    use sha2::Digest;
    let source = std::env::var("LUMA_ASR_TEST_VERIFIER_SOURCE_SHA").ok();
    if let Some(source) = &source { assert!(source.len()==40 && source.bytes().all(|b|b.is_ascii_hexdigit())); }
    let mut report = json!({"schema":1,"passed":false,"verifier_source_sha":source,
        "embedded_worker_sha256":format!("{:x}",sha2::Sha256::digest(WORKER_SOURCE.as_bytes())),
        "model_revision":pins["faster_whisper_tiny"]["version"],"audio_sha256":pins["audio"]["sha256"],
        "cold_warm_srt_export_tested":false,"active_cancellation_recovery_tested":false,
        "injected_lease_retention_tested":false,"global_managed_path_selection_tested":false,
        "managed_receipt_policy_selection_tested":ct2_receipt_proof.is_some(),
        "cpu_thread_policy":if ct2_receipt_proof.is_some() { Some("luma-cpu-seq-1:cpu_threads=1") } else { None },
        "stop_policy":"production kill-and-reap","graceful_eof_tested":false,"transitions":[]});
    real_worker_lifecycle::checkpoint(&output,&report);
    let runtime = AsrRuntime { ct2_receipt_proof, ..Default::default() };
    let cancel = Arc::new(AtomicBool::new(false));
    let probe = runtime
        .request(
            config,
            "probe",
            None,
            None,
            cancel.clone(),
            Some(Duration::from_secs(60)),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(probe["ready"], true, "{probe}");
    assert_eq!(probe["device"], "cpu");
    if runtime.ct2_receipt_proof.is_some() {
        assert!(probe["warnings"].as_array().unwrap().iter().any(|warning| warning.as_str().is_some_and(|text| text.contains("cpu_threads=1"))));
    }
    let initial_pid = real_worker_lifecycle::pid(&runtime).await;
    let duration = super::wav_duration_ms(audio).unwrap();
    for (id, reused) in [("cold", false), ("warm", true)] {
        let result = runtime
            .request(
                config,
                "transcribe",
                Some(audio),
                Some("en"),
                cancel.clone(),
                Some(Duration::from_secs(180)),
                |_| {},
            )
            .await
            .unwrap();
        assert_eq!(result["reused"], reused);
        assert_eq!(result["backend"], "faster-whisper");
        assert_eq!(result["device"], "cpu");
        let segments = super::validate_segments(&result, duration).unwrap();
        let rendered = crate::subtitles::render_srt(&segments, None);
        let source_path = output.join(format!("{id}.source.srt"));
        crate::subtitles::write_srt_text(&source_path, &rendered)
            .await
            .unwrap();
        let imported = crate::subtitles::parse_srt_file(&source_path).unwrap();
        let export_path = output.join(format!("{id}.export.srt"));
        crate::subtitles::write_srt_text(
            &export_path,
            &crate::subtitles::render_srt(&imported, None),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read_to_string(export_path).unwrap(), rendered);
        assert_eq!(imported.len(), segments.len());
        assert!(
            rendered.to_lowercase().contains("country"),
            "public JFK fixture should contain known speech"
        );
        std::fs::write(
            output.join(format!("{id}.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(real_worker_lifecycle::pid(&runtime).await,initial_pid,"cold and warm inference must reuse one process");
    report["cold_warm_srt_export_tested"] = json!(true);
    real_worker_lifecycle::checkpoint(&output,&report);
    let replacement = AsrConfig { model_path:replacement_model.to_string_lossy().into_owned(),..config.clone() };
    replacement.validate().unwrap();
    println!("REAL_OPTIONAL_WORKER_STAGE model-replacement");
    let prior = real_worker_lifecycle::observe(&runtime,&output,"model-replacement").await;
    let replaced = real_worker_lifecycle::cold_transcription(&runtime,&replacement,audio,duration).await;
    assert_ne!(real_worker_lifecycle::pid(&runtime).await,prior.pid());
    real_worker_lifecycle::transition(&output,&mut report,prior.finish());
    std::fs::write(output.join("replacement.json"),serde_json::to_vec_pretty(&replaced).unwrap()).unwrap();

    println!("REAL_OPTIONAL_WORKER_STAGE release-idle");
    let prior = real_worker_lifecycle::observe(&runtime,&output,"release-idle").await;
    assert!(tokio::time::timeout(Duration::from_secs(15),runtime.release_idle()).await.unwrap().unwrap());
    assert!(runtime.worker.lock().await.is_none());
    real_worker_lifecycle::transition(&output,&mut report,prior.finish());
    real_worker_lifecycle::cold_transcription(&runtime,&replacement,audio,duration).await;

    println!("REAL_OPTIONAL_WORKER_STAGE legacy-release");
    let prior = real_worker_lifecycle::observe(&runtime,&output,"legacy-release").await;
    tokio::time::timeout(Duration::from_secs(15),runtime.release_for_legacy(&AtomicBool::new(false))).await.unwrap().unwrap();
    assert!(runtime.worker.lock().await.is_none());
    real_worker_lifecycle::transition(&output,&mut report,prior.finish());
    real_worker_lifecycle::cold_transcription(&runtime,&replacement,audio,duration).await;

    println!("REAL_OPTIONAL_WORKER_STAGE active-cancellation");
    let prior = real_worker_lifecycle::observe(&runtime,&output,"active-cancellation").await;
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let mut armed = false;
    let start = Instant::now();
    let result = runtime
        .request(
            &replacement,
            "transcribe",
            Some(long_audio),
            Some("en"),
            cancelled,
            Some(Duration::from_secs(60)),
            |event| {
                if !armed
                    && event["message"]
                        .as_str()
                        .unwrap_or("")
                        .starts_with("Transcribing")
                {
                    armed = true;
                    let flag = flag.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        flag.store(true, Ordering::SeqCst);
                    });
                }
            },
        )
        .await;
    assert!(armed, "cancel after the actual model begins inference");
    assert!(matches!(result, Err(JobError::Cancelled)), "{result:?}");
    assert!(start.elapsed() < Duration::from_secs(15));
    assert!(runtime.worker.lock().await.is_none());
    real_worker_lifecycle::transition(&output,&mut report,prior.finish());
    let recovered = runtime
        .request(
            &replacement,
            "transcribe",
            Some(audio),
            Some("en"),
            cancel,
            Some(Duration::from_secs(180)),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(recovered["reused"], false);
    super::validate_segments(&recovered, duration).unwrap();
    std::fs::write(
        output.join("recovered.json"),
        serde_json::to_vec_pretty(&recovered).unwrap(),
    )
    .unwrap();
    report["active_cancellation_recovery_tested"] = json!(true);
    real_worker_lifecycle::checkpoint(&output,&report);
    println!("REAL_OPTIONAL_WORKER_STAGE shutdown");
    let prior = real_worker_lifecycle::observe(&runtime,&output,"shutdown").await;
    tokio::time::timeout(Duration::from_secs(15),runtime.shutdown()).await.unwrap();
    assert!(runtime.worker.lock().await.is_none());
    real_worker_lifecycle::transition(&output,&mut report,prior.finish());
    let stopped = runtime.request(&replacement,"probe",None,None,Arc::new(AtomicBool::new(false)),Some(Duration::from_secs(1)),|_|{}).await;
    assert!(matches!(stopped,Err(JobError::Cancelled)));
    report["injected_lease_retention_tested"] = json!(true);
    report["passed"] = json!(true);
    real_worker_lifecycle::checkpoint(&output,&report);
    println!("REAL_OPTIONAL_WORKER_LIFECYCLE {report}");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_source_requires_explicit_managed_gate_and_unique_marker() {
        assert_eq!(prepared_worker_source(WORKER_SOURCE, false, false).unwrap(), WORKER_SOURCE);
        let prepared = prepared_worker_source(WORKER_SOURCE, true, false).unwrap();
        assert!(prepared.contains("LUMA_MANAGED_QWEN_RUNTIME = True"));
        assert!(prepared.contains("def luma_prepare_nagisa()"));
        assert!(!prepared.contains(NAGISA_COMPAT_MARKER));
        assert!(prepared.find("from __future__ import annotations").unwrap() < prepared.find("def luma_prepare_nagisa()").unwrap());
        assert!(prepared_worker_source("print('manual')", true, false).is_err());
        assert!(prepared_worker_source(&format!("{NAGISA_COMPAT_MARKER}\n{NAGISA_COMPAT_MARKER}"), true, false).is_err());
    }
    #[test]
    fn ct2_policy_source_is_explicit_unique_and_separate_from_qwen() {
        let source = prepared_worker_source(WORKER_SOURCE, false, true).unwrap();
        assert_eq!(source.matches(CT2_CPU_POLICY_SOURCE).count(), 1);
        assert!(!source.contains(CT2_CPU_POLICY_MARKER));
        assert!(source.contains(NAGISA_COMPAT_MARKER));
        assert!(!source.contains("LUMA_MANAGED_QWEN_RUNTIME = True"));
        assert!(source.find("from __future__ import annotations").unwrap() < source.find(CT2_CPU_POLICY_SOURCE).unwrap());
        assert!(prepared_worker_source(WORKER_SOURCE, true, true).is_err());
        assert!(prepared_worker_source("print('manual')", false, true).is_err());
        assert!(prepared_worker_source(&format!("{CT2_CPU_POLICY_MARKER}\n{CT2_CPU_POLICY_MARKER}"), false, true).is_err());
        assert!(!prepared_worker_source(WORKER_SOURCE, true, false).unwrap().contains(CT2_CPU_POLICY_SOURCE));
    }
    #[test]
    fn managed_worker_directory_is_canonical_and_does_not_change_app_cwd() {
        let base = std::env::temp_dir().join(format!("luma-asr-cwd-é-测试-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let executable = base.join("python.exe");
        std::fs::write(&executable, b"owned fixture, never executed").unwrap();
        let before = std::env::current_dir().unwrap();
        let selected = managed_worker_directory(executable.to_str().unwrap(), true).unwrap().unwrap();
        assert_eq!(selected, base);
        let mut command = Command::new(&executable);
        command.current_dir(&selected);
        assert_eq!(command.as_std().get_current_dir(), Some(base.as_path()));
        assert_eq!(std::env::current_dir().unwrap(), before);
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn external_worker_directory_is_untouched_and_managed_paths_fail_closed() {
        assert!(managed_worker_directory("unresolved-external-python", false).unwrap().is_none());
        assert!(managed_worker_directory("relative-managed-python", true).is_err());
        let missing = std::env::temp_dir().join(format!("luma-missing-asr-{}", uuid::Uuid::new_v4()));
        assert!(managed_worker_directory(missing.to_str().unwrap(), true).is_err());
    }
    fn python_config() -> AsrConfig {
        AsrConfig {
            python_path: which::which("python3")
                .or_else(|_| which::which("python"))
                .expect("python3 is required for lightweight worker protocol tests")
                .to_string_lossy()
                .to_string(),
            ..AsrConfig::default()
        }
    }
    #[test]
    fn whisper_lease_paths_ignore_stale_qwen_aligner() {
        let mut config = python_config();
        config.engine = "whisper-accelerated".into();
        config.aligner_path = "/missing/previous-qwen-aligner".into();
        assert_eq!(managed_config_paths(&config).len(), 2);
        assert!(!managed_config_paths(&config).contains(&config.aligner_path.as_str()));
        config.engine = "qwen3-asr".into();
        assert!(managed_config_paths(&config).contains(&config.aligner_path.as_str()));
    }
    #[test]
    fn component_maintenance_stops_idle_worker_and_excludes_new_requests() {
        tauri::async_runtime::block_on(async {
            let runtime = AsrRuntime::default();
            let config = python_config();
            let worker = Worker::spawn(&config, "import time\ntime.sleep(30)").unwrap();
            let script = worker.script_path.clone();
            *runtime.worker.lock().await = Some(worker);
            let guard = runtime.begin_component_maintenance().await.unwrap();
            assert!(!script.exists());
            assert!(runtime.worker.try_lock().is_err());
            assert!(runtime
                .request(
                    &config,
                    "probe",
                    None,
                    None,
                    Arc::new(AtomicBool::new(false)),
                    Some(Duration::from_secs(1)),
                    |_| {},
                )
                .await
                .is_err());
            drop(guard);
            assert!(runtime.worker.lock().await.is_none());
        });
    }
    #[test]
    fn component_maintenance_refuses_active_or_stopping_runtime() {
        tauri::async_runtime::block_on(async {
            let runtime = AsrRuntime::default();
            let active_request = runtime.worker.lock().await;
            assert!(runtime.begin_component_maintenance().await.is_err());
            drop(active_request);
            runtime.shutdown().await;
            assert!(runtime.begin_component_maintenance().await.is_err());
        });
    }
    #[test]
    fn worker_reuses_process_for_multiple_requests_and_returns_real_progress() {
        tauri::async_runtime::block_on(async {
            let script = "import json,sys\nn=0\nfor line in sys.stdin:\n r=json.loads(line); n+=1\n print(json.dumps({'id':r['id'],'event':'progress','progress':0.5,'message':'working'}),flush=True)\n print(json.dumps({'id':r['id'],'event':'result','count':n}),flush=True)";
            let mut worker = Worker::spawn(&python_config(), script).unwrap();
            let cancel = AtomicBool::new(false);
            let stopping = AtomicBool::new(false);
            for count in 1..=2 {
                let mut progress = Vec::new();
                let result = worker
                    .exchange(
                        "test",
                        &json!({"id":"test"}),
                        &cancel,
                        &stopping,
                        None,
                        &mut |event| progress.push(event.clone()),
                    )
                    .await
                    .unwrap();
                assert_eq!(result["count"], count);
                assert_eq!(progress.len(), 1);
            }
            worker.stop().await;
            assert!(worker.child.as_mut().unwrap().try_wait().unwrap().is_some());
        });
    }
    #[test]
    fn unicode_paths_and_text_round_trip_through_worker_stdio() {
        tauri::async_runtime::block_on(async {
            let mut worker = Worker::spawn(&python_config(), "import json,sys\nr=json.loads(input());print(json.dumps({'id':r['id'],'event':'result','path':r['path'],'encoding':sys.stdin.encoding}),flush=True)").unwrap();
            let value = worker
                .exchange(
                    "unicode",
                    &json!({"id":"unicode","path":"C:/字幕/日本語/音声.wav"}),
                    &AtomicBool::new(false),
                    &AtomicBool::new(false),
                    Some(Duration::from_secs(2)),
                    &mut |_| {},
                )
                .await
                .unwrap();
            assert_eq!(value["path"], "C:/字幕/日本語/音声.wav");
            assert_eq!(value["encoding"], "utf-8");
            worker.stop().await;
        });
    }

    #[test]
    fn cancellation_stops_and_reaps_worker_and_a_new_one_recovers() {
        tauri::async_runtime::block_on(async {
            let mut worker =
                Worker::spawn(&python_config(), "import time\ntime.sleep(30)").unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let trigger = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                flag.store(true, Ordering::SeqCst);
            });
            let start = Instant::now();
            let result = worker
                .exchange(
                    "test",
                    &json!({"id":"test"}),
                    &cancel,
                    &AtomicBool::new(false),
                    None,
                    &mut |_| {},
                )
                .await;
            trigger.await.unwrap();
            assert!(matches!(result, Err(JobError::Cancelled)));
            worker.stop().await;
            assert!(start.elapsed() < Duration::from_secs(2));
            assert!(worker.child.as_mut().unwrap().try_wait().unwrap().is_some());
            let mut replacement = Worker::spawn(&python_config(), "import sys,json\nr=json.loads(input());print(json.dumps({'id':r['id'],'event':'result'}),flush=True)").unwrap();
            let result = replacement
                .exchange(
                    "next",
                    &json!({"id":"next"}),
                    &AtomicBool::new(false),
                    &AtomicBool::new(false),
                    Some(Duration::from_secs(2)),
                    &mut |_| {},
                )
                .await;
            assert!(result.is_ok());
            replacement.stop().await;
        });
    }
    #[test]
    fn mismatched_responses_and_crashed_workers_fail_cleanly() {
        tauri::async_runtime::block_on(async {
            for script in ["import sys,json\ninput(); print(json.dumps({'id':'stale','event':'result'}),flush=True)", "import sys\ninput();sys.exit(7)"] {
                let mut worker = Worker::spawn(&python_config(), script).unwrap();
                assert!(worker.exchange("current", &json!({"id":"current"}), &AtomicBool::new(false), &AtomicBool::new(false), Some(Duration::from_secs(2)), &mut |_| {}).await.is_err());
                worker.stop().await;
            }
        });
    }
    #[test]
    fn legacy_transition_releases_optional_model_without_starting_python() {
        tauri::async_runtime::block_on(async {
            let runtime = AsrRuntime::default();
            runtime
                .release_for_legacy(&AtomicBool::new(false))
                .await
                .unwrap();
            assert!(runtime.worker.lock().await.is_none());
            let worker = Worker::spawn(&python_config(), "import time\ntime.sleep(30)").unwrap();
            let script = worker.script_path.clone();
            *runtime.worker.lock().await = Some(worker);
            runtime
                .release_for_legacy(&AtomicBool::new(false))
                .await
                .unwrap();
            assert!(runtime.worker.lock().await.is_none());
            assert!(!script.exists());
        });
    }

    #[test]
    fn managed_runtime_shutdown_interrupts_active_request_and_removes_script() {
        tauri::async_runtime::block_on(async {
            let runtime = Arc::new(AsrRuntime::default());
            let config = python_config();
            let worker = Worker::spawn(&config, "import time\ntime.sleep(30)").unwrap();
            let script = worker.script_path.clone();
            *runtime.worker.lock().await = Some(worker);
            let running = runtime.clone();
            let request = tokio::spawn(async move {
                running
                    .request(
                        &config,
                        "transcribe",
                        None,
                        None,
                        Arc::new(AtomicBool::new(false)),
                        None,
                        |_| {},
                    )
                    .await
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(runtime.release_idle().await.is_err());
            let start = Instant::now();
            runtime.shutdown().await;
            assert!(matches!(request.await.unwrap(), Err(JobError::Cancelled)));
            assert!(start.elapsed() < Duration::from_secs(2));
            assert!(!script.exists());
            assert!(runtime.worker.lock().await.is_none());
        });
    }

    #[test]
    fn aborting_request_future_drops_worker_instead_of_reusing_unread_results() {
        tauri::async_runtime::block_on(async {
            let runtime = Arc::new(AsrRuntime::default());
            let config = python_config();
            let worker = Worker::spawn(&config, "import time\ntime.sleep(30)").unwrap();
            let script = worker.script_path.clone();
            let pid = worker.child.as_ref().unwrap().id().unwrap();
            *runtime.worker.lock().await = Some(worker);
            let running = runtime.clone();
            let request = tokio::spawn(async move {
                running
                    .request(
                        &config,
                        "transcribe",
                        None,
                        None,
                        Arc::new(AtomicBool::new(false)),
                        None,
                        |_| {},
                    )
                    .await
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
            request.abort();
            let _ = request.await;
            assert!(!script.exists());
            assert!(runtime.worker.lock().await.is_none());
            // Linux exposes zombie/running state; kill_on_drop is reaped by Tokio.
            #[cfg(target_os = "linux")]
            {
                for _ in 0..20 {
                    if !Path::new(&format!("/proc/{pid}")).exists() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                assert!(
                    !Path::new(&format!("/proc/{pid}")).exists(),
                    "aborted worker must be killed and reaped"
                );
            }
            #[cfg(not(target_os = "linux"))]
            let _ = pid;
        });
    }

    #[test]
    #[ignore = "Requires explicitly prepared optional runtime, local model and public WAV fixtures; never downloads"]
    fn real_optional_worker_transcribes_exports_reuses_and_cancels() {
        tauri::async_runtime::block_on(async {
            let config = AsrConfig {
                engine: "whisper-accelerated".into(),
                device: "cpu".into(),
                python_path: std::env::var("LUMA_ASR_TEST_PYTHON")
                    .expect("set the isolated Python executable"),
                model_path: std::env::var("LUMA_ASR_TEST_MODEL")
                    .expect("set an existing local CTranslate2 model"),
                ..AsrConfig::default()
            };
            let audio = std::path::PathBuf::from(
                std::env::var("LUMA_ASR_TEST_AUDIO").expect("set a public test WAV"),
            );
            let replacement_model = std::path::PathBuf::from(
                std::env::var("LUMA_ASR_TEST_REPLACEMENT_MODEL").expect("set the second verified Tiny fixture directory"),
            );
            let long_audio = std::path::PathBuf::from(
                std::env::var("LUMA_ASR_TEST_LONG_AUDIO").expect("set the cancellation fixture"),
            );
            let output = std::path::PathBuf::from(
                std::env::var("LUMA_ASR_TEST_OUTPUT").expect("set an isolated output directory"),
            );
            run_real_optional_worker_fixture(&config, &replacement_model, &audio, &long_audio, &output, None).await;
        });
    }

    #[test]
    fn shutdown_flag_cancels_before_launch_and_legacy_requires_no_python() {
        tauri::async_runtime::block_on(async {
            let runtime = AsrRuntime::default();
            runtime.shutdown().await;
            let result = runtime
                .request(
                    &AsrConfig::default(),
                    "probe",
                    None,
                    None,
                    Arc::new(AtomicBool::new(false)),
                    None,
                    |_| {},
                )
                .await;
            assert!(matches!(result, Err(JobError::Cancelled)));
        });
        assert!(AsrConfig::default().validate().is_ok());
    }
}
