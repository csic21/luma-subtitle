//! Private setup processes receive only required OS state and app-owned paths.
use super::store;
use std::{path::{Path, PathBuf}, ffi::OsString};

#[cfg(any(windows, test))]
const PYTHON_PATH_ERROR: &str = "Private Python requires a shorter absolute local-drive path with ordinary file names. UNC, device, ambiguous, and launch paths of 260 UTF-16 units or longer are unsupported. No files were moved or Windows settings changed.";
#[cfg(any(windows, test))]
fn ordinary_python_spelling(value: &str) -> Result<String, String> {
    let ordinary = value.strip_prefix(r"\\?\").unwrap_or(value);
    let bytes = ordinary.as_bytes();
    if bytes.len() < 4 || !bytes[0].is_ascii_alphabetic() || &bytes[1..3] != b":\\" {
        return Err(PYTHON_PATH_ERROR.into());
    }
    for part in ordinary[3..].split('\\') {
        let stem = part.split('.').next().unwrap_or("").to_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$")
            || ["COM", "LPT"].iter().any(|prefix| stem.strip_prefix(*prefix).is_some_and(|digit| matches!(digit, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")));
        if part.is_empty() || part.encode_utf16().count() > 255 || matches!(part, "." | "..") || part.ends_with('.') || part.ends_with(' ') || reserved
            || part.chars().any(|c| c.is_control() || "<>:\"/|?*".contains(c)) {
            return Err(PYTHON_PATH_ERROR.into());
        }
    }
    if ordinary.encode_utf16().count() >= 260 { return Err(PYTHON_PATH_ERROR.into()); }
    Ok(ordinary.to_owned())
}
#[cfg(any(windows, test))]
fn check_python_launch_identity(canonical: &Path, roundtrip: &Path) -> Result<(), String> {
    if canonical != roundtrip { return Err("The private Python launch path resolves to a different file or directory. Setup stopped without changing the active component.".into()); }
    Ok(())
}
/// Adapt only a verified private Python process boundary. Store/trust/cache and
/// configuration identities remain canonical. Ordinary Win32 parsing is needed
/// by pip and native packages that join paths with '/' or parent components.
pub(crate) fn private_python_launch_path(path: &Path) -> Result<PathBuf, String> {
    #[cfg(not(windows))] { Ok(path.to_path_buf()) }
    #[cfg(windows)] {
        // Reject unsafe namespaces/spellings before even resolving them.
        ordinary_python_spelling(path.to_str().ok_or(PYTHON_PATH_ERROR)?)?;
        let canonical = std::fs::canonicalize(path).map_err(|e| format!("Cannot resolve private Python launch input: {e}"))?;
        let ordinary = PathBuf::from(ordinary_python_spelling(canonical.to_str().ok_or(PYTHON_PATH_ERROR)?)?);
        let roundtrip = std::fs::canonicalize(&ordinary).map_err(|e| format!("Cannot verify ordinary private Python launch path: {e}"))?;
        check_python_launch_identity(&canonical, &roundtrip)?;
        Ok(ordinary)
    }
}

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

/// Every setup child retains the staging tree and its cross-process lock. Field
/// order is deliberate: cleanup completes before the setup lock is released.
#[derive(Clone)]
pub(super) struct SetupLifetime {
    _staging: store::Staging,
    _lease: store::SetupLease,
}
impl SetupLifetime {
    pub(super) fn new(staging: &store::Staging, lease: &store::SetupLease) -> Self {
        Self { _staging: staging.clone(), _lease: lease.clone() }
    }
}
struct ChildResources {
    _temporary: PrivateTemp,
    _lifetime: SetupLifetime,
}
/// Mirrors the ASR worker's OS-thread reaper so future abort and runtime teardown
/// cannot remove files or unlock setup while an owned process is still alive.
pub(super) struct OwnedChild {
    child: Option<tokio::process::Child>,
    resources: Option<std::sync::Arc<ChildResources>>,
}
impl OwnedChild {
    pub(super) fn spawn(command: &mut tokio::process::Command, lifetime: &SetupLifetime, temporary: PrivateTemp) -> std::io::Result<Self> {
        let resources = std::sync::Arc::new(ChildResources { _temporary: temporary, _lifetime: lifetime.clone() });
        let child = command.kill_on_drop(true).spawn()?;
        Ok(Self { child: Some(child), resources: Some(resources) })
    }
    pub(super) fn child_mut(&mut self) -> &mut tokio::process::Child {
        self.child.as_mut().expect("setup child is owned until reaped")
    }
    pub(super) async fn finish(&mut self, kill: bool) {
        if let Some(child) = self.child.as_mut() {
            if kill { let _ = child.kill().await; }
            if child.wait().await.is_ok() { self.child.take(); }
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else { return; };
        let _ = child.start_kill();
        if matches!(child.try_wait(), Ok(Some(_))) { return; }
        let resources = self.resources.take().expect("live setup child retains its resources");
        let retained = resources.clone();
        let reaper = std::thread::Builder::new().name("luma-setup-reaper".into()).spawn(move || {
            let _resources = retained;
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    _ => { let _ = child.start_kill(); std::thread::sleep(super::POLL); }
                }
            }
        });
        if reaper.is_err() {
            // An exhausted OS cannot justify releasing a live child's staging or
            // lock. Retain them until process exit, as the ASR worker does.
            std::mem::forget(resources);
        }
    }
}

/// All short-lived setup helpers stay owned until both termination and pipe
/// draining finish. Cancellation/timeout never leaves a background writer.
pub(super) async fn run_owned(command: tokio::process::Command, label: &str, seconds: u64, cancel: &std::sync::atomic::AtomicBool, lifetime: &SetupLifetime, temporary: PrivateTemp) -> Result<(bool, Vec<u8>), String> {
    let (status, output) = run_owned_status(command, label, seconds, cancel, lifetime, temporary).await?;
    Ok((status.success(), output))
}
/// Preserve native exit diagnostics without changing setup ownership or cleanup.
pub(super) async fn run_owned_status(mut command: tokio::process::Command, label: &str, seconds: u64, cancel: &std::sync::atomic::AtomicBool, lifetime: &SetupLifetime, temporary: PrivateTemp) -> Result<(std::process::ExitStatus, Vec<u8>), String> {
    use std::{process::Stdio, sync::atomic::Ordering, time::{Duration, Instant}};
    use tokio::io::AsyncReadExt;
    super::archive::cancelled(cancel)?;
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    crate::process_utils::hide_tokio_command_window(&mut command);
    let mut child = OwnedChild::spawn(&mut command, lifetime, temporary).map_err(|e| format!("Cannot start {label}: {e}"))?;
    let mut stdout = child.child_mut().stdout.take().expect("configured stdout pipe");
    let mut stderr = child.child_mut().stderr.take().expect("configured stderr pipe");
    let mut ended = [false, false]; let mut status = None;
    let mut output = Vec::new(); let mut buffer = [0u8; 2048]; let started = Instant::now();
    let result = loop {
        if cancel.load(Ordering::SeqCst) { break Err("Component setup cancelled.".into()); }
        if started.elapsed() > Duration::from_secs(seconds) { break Err(format!("{label} timed out. Check Windows security policy and connectivity, then retry. The previous component is unchanged.")); }
        if status.is_none() { match child.child_mut().try_wait() { Ok(value) => status = value, Err(e) => break Err(format!("Cannot inspect {label}: {e}")) } }
        let mut failure = None;
        for stream in 0..2 {
            if ended[stream] { continue; }
            let read = if stream == 0 { tokio::time::timeout(Duration::from_millis(20), stdout.read(&mut buffer)).await }
                else { tokio::time::timeout(Duration::from_millis(20), stderr.read(&mut buffer)).await };
            match read {
                Ok(Ok(0)) => ended[stream] = true,
                Ok(Ok(n)) => { output.extend_from_slice(&buffer[..n]); if output.len() > 64 * 1024 { failure = Some(format!("{label} produced excessive diagnostics and was stopped.")); break; } },
                Ok(Err(e)) => { failure = Some(format!("Cannot read {label}: {e}")); break; },
                Err(_) => (),
            }
        }
        if let Some(error) = failure { break Err(error); }
        if ended.iter().all(|v| *v) { if let Some(exit) = status { break Ok((exit, output)); } }
        tokio::time::sleep(super::POLL).await;
    };
    child.finish(result.is_err()).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, sync::{atomic::{AtomicBool, AtomicU8, Ordering}, Arc}, time::{Duration, Instant}};

    #[test]
    fn private_python_spelling_preserves_unicode_and_rejects_unsafe_namespaces() {
        let ordinary = r"C:\private Python é 测试\python.exe";
        assert_eq!(ordinary_python_spelling(ordinary).unwrap(), ordinary);
        assert_eq!(ordinary_python_spelling(&format!(r"\\?\{ordinary}")).unwrap(), ordinary);
        for value in [r"\\server\share\python.exe", r"\\?\UNC\server\share\python.exe", r"\\.\C:\python.exe", r"\\?\Volume{fixture}\python.exe",
            r"C:python.exe", r"relative\python.exe", r"C:\runtime.\python.exe", r"C:\runtime \python.exe", r"C:\one\..\python.exe",
            r"C:\one\.\python.exe", r"C:\one/python.exe", r"C:\one\\python.exe", r"C:\python.exe:stream", r"C:\NUL.txt", r"C:\COM¹\python.exe",
            "C:\\runtime\u{0}\\python.exe"] {
            assert!(ordinary_python_spelling(value).is_err(), "accepted {value}");
        }
        assert!(check_python_launch_identity(Path::new("canonical-one"), Path::new("canonical-two")).is_err());
        check_python_launch_identity(Path::new("canonical-one"), Path::new("canonical-one")).unwrap();
    }
    #[test]
    fn private_python_spelling_counts_utf16_launch_path_limits() {
        let prefix = r"C:\runtime\";
        let count = 259 - prefix.encode_utf16().count();
        let allowed = format!("{prefix}{}", "a".repeat(count));
        assert!(ordinary_python_spelling(&allowed).is_ok());
        assert!(ordinary_python_spelling(&format!("{allowed}a")).unwrap_err().contains("260 UTF-16"));
        assert!(ordinary_python_spelling(&format!("{prefix}{}😀", "a".repeat(count - 2))).is_ok());
        assert!(ordinary_python_spelling(&format!("{prefix}{}😀", "a".repeat(count - 1))).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn native_private_python_launch_spelling_roundtrips_exact_unicode_files() {
        let base = std::env::temp_dir().join(format!("luma-python-é-测试-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&base).unwrap(); let canonical = fs::canonicalize(&base).unwrap();
        let interpreter = canonical.join("python.exe"); fs::write(&interpreter, b"not executed").unwrap();
        let before = std::env::current_dir().unwrap();
        for path in [&canonical, &interpreter] {
            let ordinary = private_python_launch_path(path).unwrap();
            assert!(!ordinary.to_str().unwrap().starts_with(r"\\?\"));
            assert_eq!(fs::canonicalize(&ordinary).unwrap(), fs::canonicalize(path).unwrap());
            assert_eq!(private_python_launch_path(&ordinary).unwrap(), ordinary);
        }
        assert_eq!(std::env::current_dir().unwrap(), before);
        assert_eq!(fs::read(&interpreter).unwrap(), b"not executed");
        fs::remove_dir_all(base).unwrap();
    }

    struct Fixture { base: PathBuf, root: PathBuf }
    impl Fixture {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!("luma-setup-abort-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&base).unwrap();
            let base = fs::canonicalize(base).unwrap();
            let root = base.join("components");
            store::prepare_root(&root).unwrap();
            Self { base, root }
        }
        fn command(&self, root: &Path, temporary: &PrivateTemp) -> tokio::process::Command {
            let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
            configure(&mut command, root, temporary).unwrap();
            command.args(["--ignored", "--exact", "asr_components::setup_process::tests::native_setup_child", "--nocapture"])
                .env("LUMA_SETUP_CHILD_READY", self.base.join("ready"));
            command
        }
        fn child_pid(&self) -> Option<u32> {
            fs::read_to_string(self.base.join("ready")).ok()?.trim().parse().ok()
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.base); } }

    // The existing test executable supplies a native, dependency-free child on
    // both Windows and Unix. It neither downloads nor invokes another process.
    #[test]
    #[ignore = "spawned only by the setup lifetime tests"]
    fn native_setup_child() {
        let ready = PathBuf::from(std::env::var_os("LUMA_SETUP_CHILD_READY").expect("setup child marker"));
        fs::write(ready, std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(60));
    }

    #[cfg(windows)]
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    #[cfg(unix)]
    extern "C" { fn kill(pid: i32, signal: i32) -> i32; }
    #[derive(Debug)]
    struct ProcessProbe {
        pid: u32,
        #[cfg(windows)] handle: usize,
    }
    impl ProcessProbe {
        fn new(pid: u32) -> Self {
            #[cfg(windows)] {
                let handle = unsafe { OpenProcess(0x0010_0000, 0, pid) }; // SYNCHRONIZE only
                assert!(!handle.is_null(), "open the live native setup child");
                Self { pid, handle: handle as usize }
            }
            #[cfg(unix)] { Self { pid } }
        }
        fn exited(&self) -> bool {
            #[cfg(windows)] {
                let state = unsafe { WaitForSingleObject(self.handle as *mut std::ffi::c_void, 0) };
                assert!(state == 0 || state == 258, "native process observation failed");
                state == 0
            }
            #[cfg(unix)] {
                if unsafe { kill(self.pid.try_into().unwrap(), 0) } == 0 { return false; }
                assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(3)); // ESRCH, including no zombie
                true
            }
        }
    }
    #[cfg(windows)]
    impl Drop for ProcessProbe { fn drop(&mut self) { unsafe { CloseHandle(self.handle as *mut std::ffi::c_void); } } }

    async fn ready(fixture: &Fixture) -> Arc<ProcessProbe> {
        let started = Instant::now();
        loop {
            if let Some(pid) = fixture.child_pid() { return Arc::new(ProcessProbe::new(pid)); }
            assert!(started.elapsed() < Duration::from_secs(10), "native setup child did not start");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    fn observe_cleanup(fixture: &Fixture, staging: &store::Staging, process: &Arc<ProcessProbe>) -> Arc<AtomicU8> {
        let observed = Arc::new(AtomicU8::new(0)); let result = observed.clone();
        let process = process.clone(); let root = fixture.root.clone();
        staging.observe_cleanup(move || {
            // Observe at the cleanup boundary, not only after polling catches
            // up: the process must already have exited and setup must be locked.
            let safe = process.exited() && store::acquire_setup_lease(&root).is_err();
            result.store(if safe { 1 } else { 2 }, Ordering::SeqCst);
        });
        observed
    }
    fn cleanup_finished(fixture: &Fixture, staging: &Path, temporary: &Path, process: &ProcessProbe) -> bool {
        let staging_removed = !staging.exists();
        let temporary_removed = !temporary.exists();
        let unlocked = store::acquire_setup_lease(&fixture.root).is_ok();
        if staging_removed || temporary_removed || unlocked {
            assert!(process.exited(), "setup files and lease must outlive the native child");
        }
        staging_removed && temporary_removed && unlocked
    }
    fn lease_diagnostic(root: &Path) -> String {
        // Tests retain the raw OS error that the user-facing lease message
        // intentionally abstracts. Open only the existing private lock file.
        match fs::OpenOptions::new().read(true).write(true).open(root.join(".setup.lock")) {
            Ok(file) => match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => "raw OS lease probe available".into(),
                Err(error) => format!("raw OS lease probe: {error}; code={:?}; kind={:?}", error.raw_os_error(), error.kind()),
            },
            Err(error) => format!("raw OS lease open: {error}"),
        }
    }
    async fn await_cleanup(fixture: &Fixture, staging: &Path, temporary: &Path, process: &ProcessProbe) {
        let started = Instant::now();
        while !cleanup_finished(fixture, staging, temporary, process) {
            assert!(started.elapsed() < Duration::from_secs(10),
                "native setup cleanup incomplete: {process:?}; exited={}; staging_exists={}; temp_exists={}; lease_error={:?}; {}",
                process.exited(), staging.exists(), temporary.exists(), store::acquire_setup_lease(&fixture.root).err(), lease_diagnostic(&fixture.root));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[test]
    fn owned_setup_future_abort_reaps_before_staging_cleanup_and_unlock() {
        tauri::async_runtime::block_on(async {
            let fixture = Fixture::new();
            let lease = store::acquire_setup_lease(&fixture.root).unwrap();
            let staging = store::Staging::create(&fixture.root).unwrap();
            let lifetime = SetupLifetime::new(&staging, &lease);
            let staging_path = staging.directory.clone();
            let temporary = PrivateTemp::create(&staging.payload()).unwrap();
            let temp_path = temporary.path.clone();
            let command = fixture.command(&staging.payload(), &temporary);
            drop(lease);
            let task = tokio::spawn(async move {
                run_owned(command, "abort fixture", 30, &AtomicBool::new(false), &lifetime, temporary).await
            });
            let process = ready(&fixture).await;
            let cleanup_observed = observe_cleanup(&fixture, &staging, &process); drop(staging);
            assert!(!process.exited());
            assert!(staging_path.exists() && temp_path.exists());
            assert!(store::acquire_setup_lease(&fixture.root).is_err());
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            await_cleanup(&fixture, &staging_path, &temp_path, &process).await;
            assert_eq!(cleanup_observed.load(Ordering::SeqCst), 1, "cleanup observed a reaped child while setup was still locked");
        });
    }

    #[test]
    fn owned_setup_runtime_shutdown_reaps_before_staging_cleanup_and_unlock() {
        let fixture = Fixture::new();
        let lease = store::acquire_setup_lease(&fixture.root).unwrap();
        let staging = store::Staging::create(&fixture.root).unwrap();
        let lifetime = SetupLifetime::new(&staging, &lease);
        let staging_path = staging.directory.clone();
        let temporary = PrivateTemp::create(&staging.payload()).unwrap();
        let temp_path = temporary.path.clone();
        let command = fixture.command(&staging.payload(), &temporary);
        drop(lease);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let task = runtime.spawn(async move {
            run_owned(command, "runtime shutdown fixture", 30, &AtomicBool::new(false), &lifetime, temporary).await
        });
        let process = runtime.block_on(ready(&fixture));
        let cleanup_observed = observe_cleanup(&fixture, &staging, &process); drop(staging);
        assert!(!process.exited());
        assert!(store::acquire_setup_lease(&fixture.root).is_err());
        drop(runtime); drop(task);
        let started = Instant::now();
        while !cleanup_finished(&fixture, &staging_path, &temp_path, &process) {
            assert!(started.elapsed() < Duration::from_secs(10), "OS reaper must work without a Tokio runtime");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(cleanup_observed.load(Ordering::SeqCst), 1);
    }

    #[cfg(unix)]
    #[test]
    fn assembly_and_self_test_abort_retain_staging_and_setup_lease() {
        use std::os::unix::fs::PermissionsExt;
        tauri::async_runtime::block_on(async {
            for assembly in [true, false] {
                let fixture = Fixture::new();
                let lease = store::acquire_setup_lease(&fixture.root).unwrap();
                let staging = store::Staging::create(&fixture.root).unwrap();
                let lifetime = SetupLifetime::new(&staging, &lease);
                let staging_path = staging.directory.clone(); let root = staging.payload();
                let executable = root.join("python-fixture");
                let ready_path = fixture.base.join("ready").to_string_lossy().replace('\'', "'\"'\"'");
                fs::write(&executable, format!("#!/bin/sh\nprintf '%s\\n' \"$$\" > '{ready_path}'\nwhile :; do :; done\n")).unwrap();
                fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
                let runtime: super::super::catalog::Runtime = serde_json::from_value(serde_json::json!({
                    "id":"abort-runtime", "version":"1", "label":"Abort fixture", "platform":"fixture", "engine":"whisper-accelerated",
                    "backend":"faster-whisper", "device":"cpu", "license":"Fixture", "license_url":"https://example.com/terms",
                    "installed_bytes":1024, "max_files":10, "entrypoint":"python-fixture"
                })).unwrap();
                drop(lease);
                let task = tokio::spawn(async move {
                    if assembly {
                        super::super::assembly::run(&root, &root.join("unused.py"), &root.join("unused.json"), &root.join("unused-wheels"), &runtime, &AtomicBool::new(false), &lifetime).await
                    } else {
                        super::super::self_test(&root, &runtime, &AtomicBool::new(false), &lifetime).await
                    }
                });
                let process = ready(&fixture).await;
                let cleanup_observed = observe_cleanup(&fixture, &staging, &process); drop(staging);
                let temps: Vec<_> = fs::read_dir(&staging_path).unwrap().map(|entry| entry.unwrap().path())
                    .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with(".setup-temp-")).collect();
                assert_eq!(temps.len(), 1, "setup entry point creates one private temp root");
                assert!(!process.exited());
                assert!(store::acquire_setup_lease(&fixture.root).is_err());
                task.abort(); assert!(task.await.unwrap_err().is_cancelled());
                await_cleanup(&fixture, &staging_path, &temps[0], &process).await;
                assert_eq!(cleanup_observed.load(Ordering::SeqCst), 1);
            }
        });
    }

    #[test]
    fn cloned_setup_resources_release_only_after_the_final_owner() {
        let fixture = Fixture::new();
        let lease = store::acquire_setup_lease(&fixture.root).unwrap();
        let staging = store::Staging::create(&fixture.root).unwrap();
        let staging_path = staging.directory.clone();
        let first = SetupLifetime::new(&staging, &lease); let retained = first.clone();
        drop(staging); drop(lease); drop(first);
        assert!(staging_path.exists());
        assert!(store::acquire_setup_lease(&fixture.root).is_err());
        drop(retained);
        assert!(!staging_path.exists());
        // Observe release within a bound instead of assuming an immediate OS
        // lock probe. Other parallel Unix launches can briefly inherit a
        // CLOEXEC descriptor between fork and exec; no owner may remain here.
        let started = Instant::now();
        loop {
            let Err(error) = store::acquire_setup_lease(&fixture.root) else { break; };
            assert!(started.elapsed() < Duration::from_secs(10), "final setup lease did not release: {error}; staging_exists={}; {}", staging_path.exists(), lease_diagnostic(&fixture.root));
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn blocking_setup_abort_retains_staging_until_work_finishes() {
        tauri::async_runtime::block_on(async {
            let fixture = Fixture::new();
            let lease = store::acquire_setup_lease(&fixture.root).unwrap();
            let staging = store::Staging::create(&fixture.root).unwrap();
            let lifetime = SetupLifetime::new(&staging, &lease);
            let staging_path = staging.directory.clone(); let payload = staging.payload();
            let finished = Arc::new(AtomicBool::new(false));
            let observed = Arc::new(AtomicU8::new(0));
            let cleanup_finished = finished.clone(); let cleanup_observed = observed.clone(); let root = fixture.root.clone();
            staging.observe_cleanup(move || {
                let safe = cleanup_finished.load(Ordering::SeqCst) && store::acquire_setup_lease(&root).is_err();
                cleanup_observed.store(if safe { 1 } else { 2 }, Ordering::SeqCst);
            });
            let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
            let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
            let work_finished = finished.clone();
            drop(staging); drop(lease);
            let task = tokio::spawn(async move {
                let retained = lifetime.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let _lifetime = retained;
                    started_tx.send(()).unwrap();
                    // Bounded even if the test fails before releasing the gate.
                    release_rx.recv_timeout(Duration::from_secs(10)).expect("release blocking setup fixture");
                    fs::write(payload.join("after-abort.bin"), b"retained").unwrap();
                    let files = super::super::assembly::inventory(&payload, 1024, 10, &AtomicBool::new(false)).unwrap();
                    assert_eq!(files.len(), 1);
                    assert_eq!(files[0].path, "after-abort.bin");
                    work_finished.store(true, Ordering::SeqCst);
                }).await.unwrap();
            });
            let started = Instant::now();
            loop {
                match started_rx.try_recv() {
                    Ok(()) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => (),
                    Err(error) => panic!("blocking setup fixture did not start: {error}"),
                }
                assert!(started.elapsed() < Duration::from_secs(10));
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            task.abort(); assert!(task.await.unwrap_err().is_cancelled());
            assert!(staging_path.exists(), "aborting the waiter must not delete a live writer's staging");
            assert!(store::acquire_setup_lease(&fixture.root).is_err());
            assert!(!finished.load(Ordering::SeqCst)); assert_eq!(observed.load(Ordering::SeqCst), 0);
            release_tx.send(()).unwrap();
            let started = Instant::now();
            loop {
                let removed = !staging_path.exists(); let unlocked = store::acquire_setup_lease(&fixture.root).is_ok();
                if removed || unlocked { assert!(finished.load(Ordering::SeqCst)); }
                if removed && unlocked { break; }
                assert!(started.elapsed() < Duration::from_secs(10), "blocking writer cleanup did not finish");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(observed.load(Ordering::SeqCst), 1, "blocking work finishes before staging cleanup and unlock");
        });
    }

    #[test]
    fn owned_setup_cancel_and_timeout_await_reaping() {
        tauri::async_runtime::block_on(async {
            for cancel_requested in [true, false] {
                let fixture = Fixture::new();
                let lease = store::acquire_setup_lease(&fixture.root).unwrap();
                let staging = store::Staging::create(&fixture.root).unwrap();
                let lifetime = SetupLifetime::new(&staging, &lease);
                let staging_path = staging.directory.clone();
                let temporary = PrivateTemp::create(&staging.payload()).unwrap();
                let temp_path = temporary.path.clone();
                let command = fixture.command(&staging.payload(), &temporary);
                let cancel = Arc::new(AtomicBool::new(false)); let flag = cancel.clone();
                drop(lease);
                let task = tokio::spawn(async move {
                    run_owned(command, "cancellation fixture", if cancel_requested { 30 } else { 2 }, &flag, &lifetime, temporary).await
                });
                let process = ready(&fixture).await;
                let cleanup_observed = observe_cleanup(&fixture, &staging, &process); drop(staging);
                if cancel_requested { cancel.store(true, Ordering::SeqCst); }
                let error = task.await.unwrap().unwrap_err();
                assert!(error.contains(if cancel_requested { "cancelled" } else { "timed out" }));
                assert!(process.exited(), "ordinary cancellation/timeout must await kill and reap");
                await_cleanup(&fixture, &staging_path, &temp_path, &process).await;
                assert_eq!(cleanup_observed.load(Ordering::SeqCst), 1);
            }
        });
    }
}
