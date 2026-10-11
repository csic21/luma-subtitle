#[cfg(any(target_os = "windows", test))]
pub(crate) const WINDOWS_CREATE_NO_WINDOW_FLAG: u32 = 0x08000000;

#[cfg(test)]
pub(crate) fn windows_create_no_window_flag() -> u32 {
    WINDOWS_CREATE_NO_WINDOW_FLAG
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn hide_std_command_window(command: &mut std::process::Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(WINDOWS_CREATE_NO_WINDOW_FLAG);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

pub(crate) fn hide_tokio_command_window(command: &mut tokio::process::Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(WINDOWS_CREATE_NO_WINDOW_FLAG);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NvidiaGpuInfo {
    pub(crate) name: String,
    pub(crate) driver_version: Option<String>,
}

/// Driver probes must never block an async executor, wait indefinitely for a
/// broken driver, or accumulate unlimited subprocess output. The owned worker
/// also kills/reaps the process tree if this future is abandoned.
#[cfg(not(target_os = "macos"))]
pub(crate) async fn probe_nvidia_gpu() -> Option<NvidiaGpuInfo> {
    let mut command = std::process::Command::new("nvidia-smi");
    hide_std_command_window(&mut command);
    command.args([
        "--query-gpu=name,driver_version",
        "--format=csv,noheader,nounits",
    ]);
    probe_nvidia_gpu_command(command, std::time::Duration::from_secs(3)).await
}

#[cfg(any(not(target_os = "macos"), test))]
async fn probe_nvidia_gpu_command(
    command: std::process::Command,
    timeout: std::time::Duration,
) -> Option<NvidiaGpuInfo> {
    let output = crate::owned_process::output(
        command,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        timeout,
        16 * 1024,
    )
    .await
    .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_nvidia_gpu(&output.stdout)
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_nvidia_gpu(stdout: &[u8]) -> Option<NvidiaGpuInfo> {
    let stdout = String::from_utf8_lossy(stdout);
    // Both requested fields are CSV columns. Ignore unrelated banners instead
    // of treating arbitrary stdout as evidence of a usable NVIDIA driver.
    let (name, driver) = stdout.lines().find_map(|line| line.split_once(','))?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let driver_version = driver
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Some(NvidiaGpuInfo {
        name: name.to_string(),
        driver_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_first_gpu_and_optional_driver() {
        assert_eq!(
            parse_nvidia_gpu(b" NVIDIA RTX 4090, 551.86\r\nNVIDIA RTX 3080, 551.86\r\n"),
            Some(NvidiaGpuInfo {
                name: "NVIDIA RTX 4090".to_string(),
                driver_version: Some("551.86".to_string()),
            })
        );
        assert_eq!(
            parse_nvidia_gpu(b"\nNVIDIA RTX 4090, \n").unwrap().driver_version,
            None
        );
        assert!(parse_nvidia_gpu(b" \r\n").is_none());
        assert!(parse_nvidia_gpu(b", 551.86\n").is_none());
        assert!(parse_nvidia_gpu(b"Unexpected driver diagnostic\n").is_none());
    }

    #[test]
    fn a_missing_gpu_probe_remains_an_unavailable_hint() {
        tauri::async_runtime::block_on(async {
            let command = std::process::Command::new(
                std::env::temp_dir().join(format!("missing-nvidia-{}", uuid::Uuid::new_v4())),
            );
            assert!(probe_nvidia_gpu_command(command, std::time::Duration::from_secs(1))
                .await
                .is_none());
        });
    }

    #[test]
    fn gpu_probe_child_fixture() {
        use std::io::Write;
        let Ok(mode) = std::env::var("LUMA_GPU_PROBE_TEST_MODE") else {
            return;
        };
        let marker = std::path::PathBuf::from(
            std::env::var_os("LUMA_GPU_PROBE_TEST_MARKER").unwrap(),
        );
        std::fs::write(&marker, std::process::id().to_string()).unwrap();
        match mode.as_str() {
            "timeout" => std::thread::sleep(std::time::Duration::from_secs(30)),
            "stdout-flood" | "stderr-flood" => {
                let flood = vec![b'x'; 262_144];
                if mode == "stdout-flood" {
                    let _ = std::io::stdout().lock().write_all(&flood);
                } else {
                    let _ = std::io::stderr().lock().write_all(&flood);
                }
                // Only the output cap should end these fixtures, before the
                // probe timeout. Do not let an ordinary exit hide a bad cap.
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
            "failed" | "success" => {
                let mut stdout = std::io::stdout().lock();
                writeln!(stdout, "NVIDIA Fixture GPU, 551.86").unwrap();
                stdout.flush().unwrap();
                std::process::exit(if mode == "failed" { 7 } else { 0 });
            }
            _ => panic!("unknown GPU probe fixture mode"),
        }
        std::fs::write(marker.with_extension("leaked"), "child outlived probe").unwrap();
    }

    fn probe_fixture(mode: &str, marker: &std::path::Path) -> std::process::Command {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "process_utils::tests::gpu_probe_child_fixture",
                "--nocapture",
                "--quiet",
                "--test-threads=2",
            ])
            .env("LUMA_GPU_PROBE_TEST_MODE", mode)
            .env("LUMA_GPU_PROBE_TEST_MARKER", marker);
        command
    }

    #[cfg(unix)]
    fn fixture_is_alive(pid: u32) -> bool {
        extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        // kill(pid, 0) also sees unreaped zombies. No signal is sent.
        unsafe { kill(pid as i32, 0) == 0 }
    }

    #[cfg(windows)]
    fn fixture_is_alive(pid: u32) -> bool {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        };
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                // ERROR_INVALID_PARAMETER means the PID no longer exists;
                // access-denied or other inspection failures are not proof.
                assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(87));
                return false;
            }
            let mut code = 0;
            let result = GetExitCodeProcess(process, &mut code);
            CloseHandle(process);
            assert_ne!(result, 0, "could not inspect owned GPU fixture");
            code == 259 // STILL_ACTIVE
        }
    }

    #[test]
    fn gpu_probe_bounds_timeout_both_streams_and_reaps_every_fixture() {
        tauri::async_runtime::block_on(async {
            let root = std::env::temp_dir().join(format!("luma-gpu-probe-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            for mode in ["timeout", "stdout-flood", "stderr-flood", "failed", "success"] {
                let marker = root.join(mode);
                let command = probe_fixture(mode, &marker);
                let started = std::time::Instant::now();
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(8),
                    probe_nvidia_gpu_command(command, std::time::Duration::from_secs(3)),
                ).await.expect("GPU probe worker failed to honor its bound");
                if mode == "success" {
                    assert_eq!(result, Some(NvidiaGpuInfo {
                        name: "NVIDIA Fixture GPU".to_string(),
                        driver_version: Some("551.86".to_string()),
                    }));
                } else {
                    assert!(result.is_none(), "{mode} must not become a GPU hint");
                }
                assert!(started.elapsed() < std::time::Duration::from_secs(5));
                if mode.ends_with("flood") {
                    assert!(started.elapsed() < std::time::Duration::from_secs(3),
                        "output cap must stop {mode} before the timeout");
                }
                let pid: u32 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
                assert!(!fixture_is_alive(pid), "GPU fixture {mode} was not killed/reaped");
                assert!(!marker.with_extension("leaked").exists());
                std::fs::remove_file(marker).unwrap();
            }
            std::fs::remove_dir(root).unwrap();
        });
    }
}
