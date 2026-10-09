use std::{
    path::Path,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use tauri::AppHandle;
use tokio::{io::AsyncReadExt, process::Command, time::sleep};

use crate::{
    dependencies::ensure_whisper_vad_model,
    job_events::{publish_job_event, JobEventDraft},
    paths::locate_binary,
    process_utils::hide_tokio_command_window,
    settings::normalize_language,
    state::{ensure_not_cancelled, JobError, JobResult},
};

#[path = "process_output.rs"]
mod output;
use output::{transcription_threads, ProcessOutput};

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
    run_process(command, cancel, "FFmpeg 抽音频失败", None)
        .await
        .map(|_| ())
}

pub(super) struct TranscriptionPaths<'a> {
    pub(super) model: &'a Path,
    pub(super) audio: &'a Path,
    pub(super) output: &'a Path,
}

pub(super) async fn transcribe_audio(
    app: &AppHandle,
    job_id: &str,
    paths: TranscriptionPaths<'_>,
    language: &str,
    mode: TranscriptionMode,
    cancel: Arc<AtomicBool>,
) -> JobResult<()> {
    let TranscriptionPaths {
        model: model_path,
        audio: audio_path,
        output: output_base,
    } = paths;
    ensure_not_cancelled(&cancel)?;
    if !model_path.exists() {
        return Err(JobError::failed("Whisper 模型文件不存在"));
    }
    let whisper = locate_binary(app, "whisper-cli")
        .ok_or_else(|| JobError::failed(missing_binary_message("whisper-cli")))?;
    let threads = std::thread::available_parallelism()
        .map(|count| transcription_threads(count.get()))
        .unwrap_or(4);
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
        .arg(threads.to_string())
        .arg("--max-context")
        .arg("0")
        .arg("--print-progress")
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

    ensure_not_cancelled(&cancel)?;
    // Older whisper-cli versions can return success without writing JSON. Never
    // let a previous attempt's output be mistaken for a fresh transcription.
    let output_json = output_base.with_extension("json");
    match tokio::fs::remove_file(&output_json).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(JobError::failed(format!("清理旧转写结果失败: {error}"))),
    }
    let progress = TranscriptionProgress::new(app, job_id, mode);
    let started = Instant::now();
    let output = run_process(command, cancel, "whisper.cpp 转写失败", Some(progress)).await?;
    let (stage, _, end) = mode.progress_range();
    publish_job_event(
        app,
        JobEventDraft::running(
            job_id,
            stage,
            format!(
                "本次转写耗时 {:.1} 秒 · 线程 {threads} · {}",
                started.elapsed().as_secs_f64(),
                output.diagnostic_summary()
            ),
            end,
        ),
    );
    Ok(())
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

impl TranscriptionMode {
    fn progress_range(self) -> (&'static str, f32, f32) {
        match self {
            Self::Standard => ("transcribing", 0.26, 0.73),
            Self::ConservativeRetry => ("transcribing-retry", 0.74, 0.89),
        }
    }
}

struct TranscriptionProgress {
    app: AppHandle,
    job_id: String,
    mode: TranscriptionMode,
    reported: u8,
    last_report: Instant,
    started: Instant,
}

impl TranscriptionProgress {
    fn new(app: &AppHandle, job_id: &str, mode: TranscriptionMode) -> Self {
        Self {
            app: app.clone(),
            job_id: job_id.to_string(),
            mode,
            reported: 0,
            last_report: Instant::now(),
            started: Instant::now(),
        }
    }

    fn report(&mut self, output: &ProcessOutput) {
        let Some(percent) = output.progress else {
            return;
        };
        // Bound database writes and UI events even for very noisy sidecars.
        if percent < self.reported.saturating_add(5)
            || self.last_report.elapsed() < Duration::from_secs(1)
        {
            return;
        }
        let (stage, start, end) = self.mode.progress_range();
        publish_job_event(
            &self.app,
            JobEventDraft::running(
                &self.job_id,
                stage,
                format!(
                    "本次转写 {percent}% · 已用 {:.0} 秒 · 引擎报告后端：{}",
                    self.started.elapsed().as_secs_f64(),
                    output.backend.as_deref().unwrap_or("未报告")
                ),
                start + (end - start) * f32::from(percent) / 100.0,
            ),
        );
        self.reported = percent;
        self.last_report = Instant::now();
    }
}

async fn run_process(
    mut command: Command,
    cancel: Arc<AtomicBool>,
    failure_context: &str,
    mut progress: Option<TranscriptionProgress>,
) -> JobResult<ProcessOutput> {
    ensure_not_cancelled(&cancel)?;
    hide_tokio_command_window(&mut command);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| JobError::failed(format!("{failure_context}: {error}")))?;
    let mut stderr = child.stderr.take();
    let output = Arc::new(Mutex::new(ProcessOutput::default()));
    let captured = output.clone();
    let mut stderr_reader = tokio::spawn(async move {
        let mut buffer = [0u8; 4096];
        if let Some(stderr) = stderr.as_mut() {
            loop {
                match stderr.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(count) => captured.lock().push(&buffer[..count]),
                }
            }
        }
        captured.lock().finish();
    });
    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            stderr_reader.abort();
            let _ = stderr_reader.await;
            return Err(JobError::Cancelled);
        }
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                let _ = child.kill().await;
                stderr_reader.abort();
                let _ = stderr_reader.await;
                return Err(JobError::failed(format!("{failure_context}: {error}")));
            }
        };
        if let Some(status) = status {
            // Drain final timing lines; a descendant inheriting stderr must not
            // keep the job alive indefinitely after the direct child exits.
            if tokio::time::timeout(Duration::from_secs(1), &mut stderr_reader)
                .await
                .is_err()
            {
                stderr_reader.abort();
                let _ = stderr_reader.await;
            }
            ensure_not_cancelled(&cancel)?;
            output.lock().finish();
            let captured = std::mem::take(&mut *output.lock());
            if status.success() {
                return Ok(captured);
            }
            return Err(JobError::failed(format!(
                "{failure_context}，退出码: {}{}",
                status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                captured.error_detail(),
            )));
        }
        if let Some(progress) = progress.as_mut() {
            progress.report(&output.lock());
        }
        sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{run_process, JobError, TranscriptionMode};
    use std::{
        fs, process,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use tokio::process::Command;

    #[test]
    fn successful_process_retains_progress_backend_and_final_timings() {
        tauri::async_runtime::block_on(async {
            let mut command = Command::new("sh");
            command.args(["-c", "printf '%s\n' 'whisper_backend_init_gpu: using Metal backend' 'whisper_print_progress_callback: progress = 50%' 'whisper_print_timings: load time = 1250.00 ms' >&2"]);
            let output = run_process(command, Arc::new(AtomicBool::new(false)), "test", None)
                .await
                .expect("mock process should succeed");
            assert_eq!(output.progress, Some(50));
            assert_eq!(output.backend.as_deref(), Some("Metal"));
            assert!(output.diagnostic_summary().contains("模型加载 1.25 秒"));
        });
    }

    #[test]
    fn failed_process_retains_only_bounded_error_tail() {
        tauri::async_runtime::block_on(async {
            let mut command = Command::new("sh");
            command.args(["-c", "i=0; while [ $i -lt 100 ]; do echo log-$i >&2; i=$((i+1)); done; echo final-error >&2; exit 7"]);
            let result = run_process(command, Arc::new(AtomicBool::new(false)), "test", None).await;
            let Err(JobError::Failed(message)) = result else {
                panic!("mock process should fail");
            };
            assert!(message.contains("7"));
            assert!(message.ends_with("final-error"));
            assert!(!message.contains("log-0\n"));
            assert!(message.lines().count() <= 9);
        });
    }

    #[test]
    fn cancellation_stops_a_running_process_promptly() {
        tauri::async_runtime::block_on(async {
            let cancel = Arc::new(AtomicBool::new(false));
            let requested = cancel.clone();
            let trigger = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                requested.store(true, Ordering::SeqCst);
            });
            let mut command = Command::new("sh");
            command.args(["-c", "exec sleep 5"]);
            let started = Instant::now();
            let result = run_process(command, cancel, "test", None).await;
            trigger.await.unwrap();
            assert!(matches!(result, Err(JobError::Cancelled)));
            assert!(started.elapsed() < Duration::from_secs(2));
        });
    }

    #[test]
    fn retry_progress_does_not_overlap_the_first_pass_or_completion() {
        let (_, first_start, first_end) = TranscriptionMode::Standard.progress_range();
        let (_, retry_start, retry_end) = TranscriptionMode::ConservativeRetry.progress_range();
        assert!(first_start < first_end);
        assert!(first_end < retry_start);
        assert!(retry_start < retry_end);
        assert!(retry_end < 0.9);
    }

    #[test]
    fn cancellation_before_launch_does_not_start_child() {
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
                None,
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
