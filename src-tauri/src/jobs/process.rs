use std::{
    path::Path,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use tauri::AppHandle;
use tokio::{io::AsyncReadExt, process::Command, time::sleep};

use crate::{
    dependencies::ensure_whisper_vad_model,
    paths::locate_binary,
    process_utils::hide_tokio_command_window,
    settings::normalize_language,
    state::{JobError, JobResult},
};

#[derive(Clone, Copy)]
pub(super) enum TranscriptionMode {
    Standard,
    ConservativeRetry,
}

pub(super) async fn prepare_audio(
    app: &AppHandle,
    input_path: &Path,
    audio_path: &Path,
    cancel: Arc<AtomicBool>,
) -> JobResult<()> {
    let ffmpeg = locate_binary(app, "ffmpeg")
        .ok_or_else(|| JobError::failed(missing_binary_message("ffmpeg")))?;
    let mut command = Command::new(ffmpeg);
    command
        .arg("-y")
        .arg("-i")
        .arg(input_path)
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg(audio_path);
    run_process(command, cancel, "FFmpeg 抽音频失败").await
}

pub(super) async fn transcribe_audio(
    app: &AppHandle,
    model_path: &Path,
    audio_path: &Path,
    output_base: &Path,
    language: &str,
    mode: TranscriptionMode,
    cancel: Arc<AtomicBool>,
) -> JobResult<()> {
    if !model_path.exists() {
        return Err(JobError::failed("Whisper 模型文件不存在"));
    }
    let whisper = locate_binary(app, "whisper-cli")
        .ok_or_else(|| JobError::failed(missing_binary_message("whisper-cli")))?;
    let threads = std::thread::available_parallelism()
        .map(|count| count.get().saturating_sub(1).clamp(4, 12).to_string())
        .unwrap_or_else(|_| "8".to_string());
    let mut command = Command::new(whisper);
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    command.env("WHISPER_ARG_DEVICE", "0");
    command
        .arg("-m")
        .arg(model_path)
        .arg("-f")
        .arg(audio_path)
        .arg("-l")
        .arg(normalize_language(language))
        .arg("-t")
        .arg(threads)
        .arg("--max-context")
        .arg("0")
        .arg("-oj")
        .arg("-of")
        .arg(output_base);

    if matches!(mode, TranscriptionMode::ConservativeRetry) {
        let vad_model = ensure_whisper_vad_model(app)
            .await
            .map_err(|error| JobError::failed(format!("准备 VAD 模型失败: {error}")))?;
        command
            .arg("--vad")
            .arg("--vad-model")
            .arg(vad_model)
            .arg("--vad-threshold")
            .arg("0.35")
            .arg("--vad-min-speech-duration-ms")
            .arg("150")
            .arg("--vad-min-silence-duration-ms")
            .arg("500")
            .arg("--vad-max-speech-duration-s")
            .arg("30")
            .arg("--vad-speech-pad-ms")
            .arg("200")
            .arg("--suppress-nst")
            .arg("--entropy-thold")
            .arg("2.0")
            .arg("--logprob-thold")
            .arg("-0.5")
            .arg("--no-speech-thold")
            .arg("0.7");
    }

    run_process(command, cancel, "whisper.cpp 转写失败").await
}

fn missing_binary_message(name: &str) -> String {
    let binary_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let resource_dir = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "src-tauri/resources/bin/macos-arm64"
    } else {
        "src-tauri/resources/bin"
    };
    format!("未找到 {binary_name}，请放入 {resource_dir} 或加入 PATH")
}

async fn run_process(
    mut command: Command,
    cancel: Arc<AtomicBool>,
    failure_context: &str,
) -> JobResult<()> {
    hide_tokio_command_window(&mut command);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| JobError::failed(format!("{failure_context}: {error}")))?;
    let mut stderr = child.stderr.take();
    let stderr_reader = tauri::async_runtime::spawn(async move {
        let mut buffer = Vec::new();
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.read_to_end(&mut buffer).await;
        }
        String::from_utf8_lossy(&buffer).to_string()
    });
    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            return Err(JobError::Cancelled);
        }
        match child
            .try_wait()
            .map_err(|error| JobError::failed(format!("{failure_context}: {error}")))?
        {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                let stderr = stderr_reader.await.unwrap_or_default();
                let detail = process_error_detail(&stderr);
                return Err(JobError::failed(format!(
                    "{failure_context}，退出码: {}{}",
                    status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    detail
                )));
            }
            None => sleep(Duration::from_millis(200)).await,
        }
    }
}

fn process_error_detail(stderr: &str) -> String {
    let lines = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    let detail = lines
        .iter()
        .rev()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    format!("\n{detail}")
}

#[cfg(all(test, unix))]
mod tests {
    use super::{run_process, JobError};
    use std::{
        fs, process,
        sync::{atomic::AtomicBool, Arc},
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use tokio::process::Command;

    #[test]
    fn cancellation_kills_running_child() {
        tauri::async_runtime::block_on(async {
            let marker = std::env::temp_dir().join(format!(
                "luma-process-cancel-{}-{}",
                process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system time should be after epoch")
                    .as_nanos()
            ));
            let mut command = Command::new("sh");
            command
                .arg("-c")
                .arg("sleep 1; touch \"$1\"")
                .arg("luma-process-test")
                .arg(&marker);

            let result = run_process(
                command,
                Arc::new(AtomicBool::new(true)),
                "test process failed",
            )
            .await;

            assert!(matches!(result, Err(JobError::Cancelled)));
            thread::sleep(Duration::from_millis(1_200));
            let marker_exists = marker.exists();
            let _ = fs::remove_file(&marker);
            assert!(
                !marker_exists,
                "cancelled child should not finish its script"
            );
        });
    }
}
