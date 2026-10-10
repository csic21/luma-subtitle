//! Bounded CLI subprocesses. Each launch owns its process group / Windows Job;
//! cancellation, timeout, overflow and abandoned futures all terminate and reap it.
use std::{io::Read, process::{Child, Command, Output, Stdio}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use crate::state::{JobError, JobResult};

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop { fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); } }

pub(crate) async fn output(command: Command, cancel: Arc<AtomicBool>, timeout: Duration, max_bytes: usize) -> JobResult<Output> {
    output_with_lifetime(command, cancel, timeout, max_bytes, Arc::new(())).await
}
pub(crate) async fn output_with_lifetime<T: Send + Sync + 'static>(mut command: Command, cancel: Arc<AtomicBool>, timeout: Duration, max_bytes: usize, lifetime: Arc<T>) -> JobResult<Output> {
    let abandoned = Arc::new(AtomicBool::new(false));
    let guard = CancelOnDrop(abandoned.clone());
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _lifetime = lifetime;
        run(&mut command, &cancel, &abandoned, timeout, max_bytes)
    }).await.map_err(|e| JobError::failed(format!("CLI worker failed: {e}")))?;
    drop(guard);
    result
}

fn reader(mut pipe: impl Read + Send + 'static, cap: usize, overflow: Arc<AtomicBool>) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
    std::thread::spawn(move || {
        let mut output = Vec::new(); let mut chunk = [0u8; 8192];
        loop {
            let count = pipe.read(&mut chunk)?;
            if count == 0 { return Ok(output); }
            if count > cap.saturating_sub(output.len()) {
                overflow.store(true, Ordering::SeqCst);
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "CLI output limit exceeded"));
            }
            output.extend_from_slice(&chunk[..count]);
        }
    })
}

fn run(command: &mut Command, cancel: &AtomicBool, abandoned: &AtomicBool, timeout: Duration, max_bytes: usize) -> JobResult<Output> {
    if cancel.load(Ordering::SeqCst) || abandoned.load(Ordering::SeqCst) { return Err(JobError::Cancelled); }
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    platform::configure(command);
    let child = command.spawn().map_err(|e| JobError::failed(format!("Cannot launch CLI: {e}")))?;
    let mut owned = Owned { child, tree: None };
    owned.tree = Some(platform::Tree::attach(&owned.child).map_err(|e| JobError::failed(format!("Cannot own CLI process tree: {e}")))?);
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = reader(owned.child.stdout.take().expect("stdout pipe"), max_bytes, overflow.clone());
    let stderr = reader(owned.child.stderr.take().expect("stderr pipe"), max_bytes, overflow.clone());
    let started = Instant::now();
    let status = loop {
        if cancel.load(Ordering::SeqCst) || abandoned.load(Ordering::SeqCst) { break Err(JobError::Cancelled); }
        if overflow.load(Ordering::SeqCst) { break Err(JobError::failed("CLI output limit exceeded")); }
        if started.elapsed() >= timeout { break Err(JobError::failed(format!("CLI timed out after {} seconds", timeout.as_secs()))); }
        match owned.child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => break Err(JobError::failed(format!("Cannot wait for CLI: {e}"))),
        }
    };
    // Descendants may outlive a successful parent or retain its pipes.
    owned.terminate();
    let out = stdout.join(); let err = stderr.join();
    let status = status?;
    let stdout = out.map_err(|_| JobError::failed("CLI stdout reader failed"))?.map_err(|e| JobError::failed(e.to_string()))?;
    let stderr = err.map_err(|_| JobError::failed("CLI stderr reader failed"))?.map_err(|e| JobError::failed(e.to_string()))?;
    Ok(Output { status, stdout, stderr })
}

struct Owned { child: Child, tree: Option<platform::Tree> }
impl Owned { fn terminate(&mut self) { if let Some(tree) = self.tree.take() { tree.terminate(); } let _ = self.child.kill(); let _ = self.child.wait(); } }
impl Drop for Owned { fn drop(&mut self) { self.terminate(); } }

#[cfg(unix)]
mod platform {
    use super::*;
    use std::os::unix::process::CommandExt;
    extern "C" { fn kill(pid: i32, signal: i32) -> i32; }
    pub(super) fn configure(command: &mut Command) { command.process_group(0); }
    pub(super) struct Tree(i32);
    impl Tree {
        pub(super) fn attach(child: &Child) -> std::io::Result<Self> { Ok(Self(child.id() as i32)) }
        pub(super) fn terminate(self) { unsafe { kill(-self.0, 9); } }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::{io::AsRawHandle, process::CommandExt};
    use windows_sys::Win32::{Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE}, System::{JobObjects::{CreateJobObjectW, SetInformationJobObject, AssignProcessToJobObject, TerminateJobObject, JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE}, Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, THREADENTRY32, TH32CS_SNAPTHREAD}, Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME, CREATE_SUSPENDED}}};
    pub(super) fn configure(command: &mut Command) { command.creation_flags(CREATE_SUSPENDED | crate::process_utils::WINDOWS_CREATE_NO_WINDOW_FLAG); }
    pub(super) struct Tree(HANDLE);
    impl Tree {
        pub(super) fn attach(child: &Child) -> std::io::Result<Self> { unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() { return Err(std::io::Error::last_os_error()); }
            let tree = Self(job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits as *const _ as *const _, std::mem::size_of_val(&limits) as u32) == 0
                || AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) == 0 { return Err(std::io::Error::last_os_error()); }
            // The child cannot execute or spawn before assignment. Resume only
            // threads belonging to this exact suspended child PID.
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE { return Err(std::io::Error::last_os_error()); }
            let mut entry: THREADENTRY32 = std::mem::zeroed(); entry.dwSize = std::mem::size_of_val(&entry) as u32;
            let mut more = Thread32First(snapshot, &mut entry); let mut resumed = false;
            while more != 0 {
                if entry.th32OwnerProcessID == child.id() {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() { resumed |= ResumeThread(thread) != u32::MAX; CloseHandle(thread); }
                }
                more = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            if !resumed { return Err(std::io::Error::new(std::io::ErrorKind::Other, "Cannot resume owned CLI")); }
            Ok(tree)
        } }
        pub(super) fn terminate(self) { unsafe { TerminateJobObject(self.0, 1); } }
    }
    impl Drop for Tree { fn drop(&mut self) { unsafe { CloseHandle(self.0); } } }
}

#[cfg(not(any(unix, windows)))]
compile_error!("CLI process ownership must be implemented for this platform");

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(mode: &str, marker: &std::path::Path) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "owned_process::tests::child_fixture", "--nocapture"])
            .env("LUMA_OWNED_TEST_MODE", mode).env("LUMA_OWNED_TEST_MARKER", marker);
        command
    }
    #[test]
    fn child_fixture() {
        let Ok(mode) = std::env::var("LUMA_OWNED_TEST_MODE") else { return; };
        let path = std::path::PathBuf::from(std::env::var_os("LUMA_OWNED_TEST_MARKER").unwrap());
        if mode == "descendant" { std::thread::sleep(Duration::from_millis(900)); std::fs::write(path, "leaked").unwrap(); return; }
        if mode == "flood" { use std::io::Write; let _ = std::io::stdout().write_all(&vec![b'x'; 200_000]); return; }
        let _child = fixture("descendant", &path).spawn().unwrap();
        std::thread::sleep(Duration::from_secs(20));
    }
    #[test]
    fn probes_bound_time_output_and_reap_descendants() {
        let root = std::env::temp_dir().join(format!("luma-cli-proof-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        for mode in ["sleep", "flood"] {
            let marker = root.join(mode);
            let start = Instant::now();
            let result = run(&mut fixture(mode, &marker), &AtomicBool::new(false), &AtomicBool::new(false), Duration::from_millis(250), 8192);
            assert!(result.is_err()); assert!(start.elapsed() < Duration::from_secs(5));
        }
        std::thread::sleep(Duration::from_millis(1100));
        assert!(!root.join("sleep").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancellation_and_dropped_futures_reap_owned_children() {
        tauri::async_runtime::block_on(async {
            let root = std::env::temp_dir().join(format!("luma-cli-cancel-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            let trigger = cancel.clone();
            std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(150)); trigger.store(true, Ordering::SeqCst); });
            assert!(matches!(output(fixture("sleep", &root.join("cancel")), cancel, Duration::from_secs(10), 8192).await, Err(JobError::Cancelled)));
            let future = output(fixture("sleep", &root.join("drop")), Arc::new(AtomicBool::new(false)), Duration::from_secs(10), 8192);
            assert!(tokio::time::timeout(Duration::from_millis(150), future).await.is_err());
            tokio::time::sleep(Duration::from_millis(1100)).await;
            assert!(!root.join("cancel").exists()); assert!(!root.join("drop").exists());
            std::fs::remove_dir_all(root).unwrap();
        });
    }
}
