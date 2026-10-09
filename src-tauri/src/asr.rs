//! Optional, offline engines. The legacy whisper.cpp route never starts Python.
use crate::{
    job_events::{publish_job_event, JobEventDraft},
    state::{JobError, JobResult},
    subtitles::SubtitleSegment,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use tauri::{AppHandle, Manager};

mod process;
pub(crate) use process::AsrRuntime;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct AsrConfig {
    pub(crate) engine: String,
    pub(crate) python_path: String,
    pub(crate) model_path: String,
    pub(crate) aligner_path: String,
    pub(crate) device: String,
}
impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            engine: "whisper-cpp".into(),
            python_path: String::new(),
            model_path: String::new(),
            aligner_path: String::new(),
            device: "auto".into(),
        }
    }
}
impl AsrConfig {
    pub(crate) fn normalized(mut self) -> Self {
        self.engine = self.engine.trim().to_string();
        if self.engine.is_empty() {
            self.engine = "whisper-cpp".into();
        }
        self.device = self.device.trim().to_string();
        if self.device.is_empty() {
            self.device = "auto".into();
        }
        self.python_path = self.python_path.trim().to_string();
        self.model_path = self.model_path.trim().to_string();
        self.aligner_path = self.aligner_path.trim().to_string();
        self
    }
    pub(crate) fn is_legacy(&self) -> bool {
        self.engine.is_empty() || self.engine == "whisper-cpp"
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.is_legacy() {
            return Ok(());
        }
        if !matches!(self.engine.as_str(), "whisper-accelerated" | "qwen3-asr") {
            return Err(format!("Unknown ASR engine '{}'. Select an installed engine; the app will not silently switch engines.", self.engine));
        }
        if !matches!(self.device.as_str(), "auto" | "cpu" | "cuda" | "metal") {
            return Err("Unsupported ASR device. Choose auto, cpu, cuda or metal.".into());
        }
        let python = Path::new(&self.python_path);
        if !python.is_absolute() || !python.is_file() {
            return Err("Optional ASR engine component is not installed or its executable is missing. Install or repair the selected component in ASR settings, or check an advanced external runtime. Select whisper.cpp to use the original engine without this component.".into());
        }
        if !Path::new(&self.model_path).is_absolute() || !Path::new(&self.model_path).is_dir() {
            return Err("Optional ASR model directory is missing. Choose a complete local model directory for this backend. Existing GGML .bin files remain usable with whisper.cpp; no models are downloaded automatically.".into());
        }
        if self.engine == "qwen3-asr" {
            if self.device == "metal" {
                return Err("Qwen3-ASR currently supports CPU/CUDA only in Luma. Select CPU on macOS; Qwen Metal is not validated.".into());
            }
            if !Path::new(&self.aligner_path).is_absolute()
                || !Path::new(&self.aligner_path).is_dir()
            {
                return Err("Qwen subtitle timestamps require the local Qwen3-ForcedAligner-0.6B directory. Configure it before transcribing; timestamps will not be guessed.".into());
            }
        }
        Ok(())
    }
}

#[tauri::command]
pub(crate) async fn check_asr_backend(app: AppHandle, config: AsrConfig) -> Result<Value, String> {
    let config = config.normalized();
    if config.is_legacy() {
        return Ok(
            json!({"ready":true,"backend":"whisper.cpp","device":null,"model_bytes":0,"aligner_bytes":0,"total_bytes":0,"capabilities":{"offline":true,"word_timestamps":false,"supported_languages":null},"warnings":["Use the existing environment check for whisper-cli and its model. No Python is needed."]}),
        );
    }
    if let Err(error) = config.validate() {
        return Ok(probe_error(&error));
    }
    let runtime = app.state::<AsrRuntime>();
    let result = runtime
        .request(
            &config,
            "probe",
            None,
            None,
            Arc::new(AtomicBool::new(false)),
            Some(Duration::from_secs(60)),
            |_| {},
        )
        .await;
    match result {
        Ok(value) if value.get("event").and_then(Value::as_str) == Some("probe") => Ok(value),
        Ok(_) => Ok(probe_error(
            "Optional ASR probe returned an unexpected response.",
        )),
        Err(JobError::Failed(message)) => Ok(probe_error(&message)),
        Err(JobError::Cancelled) => Ok(probe_error("Optional ASR probe was cancelled.")),
    }
}
fn probe_error(error: &str) -> Value {
    json!({"ready":false,"backend":null,"device":null,"model_bytes":0,"aligner_bytes":0,"total_bytes":0,"capabilities":{"offline":true,"word_timestamps":false,"supported_languages":null},"warnings":[],"error":error})
}

#[tauri::command]
pub(crate) async fn release_asr_backend(app: AppHandle) -> Result<bool, String> {
    app.state::<AsrRuntime>().release_idle().await
}

pub(crate) async fn transcribe(
    app: &AppHandle,
    job_id: &str,
    config: &AsrConfig,
    audio: &Path,
    language: &str,
    cancel: Arc<AtomicBool>,
) -> JobResult<Vec<SubtitleSegment>> {
    config.validate().map_err(JobError::failed)?;
    let duration = wav_duration_ms(audio).map_err(JobError::failed)?;
    let mut last_progress = 0.26f32;
    let result = app
        .state::<AsrRuntime>()
        .request(
            config,
            "transcribe",
            Some(audio),
            Some(language),
            cancel,
            None,
            |event| {
                let fraction = event
                    .get("progress")
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite())
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                last_progress = last_progress.max(0.26 + fraction as f32 * 0.60).min(0.86);
                let message = event
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Optional ASR is processing audio locally");
                publish_job_event(
                    app,
                    JobEventDraft::running(job_id, "transcribing", message, last_progress),
                );
            },
        )
        .await?;
    if result.get("event").and_then(Value::as_str) != Some("result") {
        return Err(JobError::failed(
            "Optional ASR returned an unexpected terminal event.",
        ));
    }
    let segments = validate_segments(&result, duration)?;
    crate::subtitles::validate_whisper_repetition(&segments)?;
    let backend = result
        .get("backend")
        .and_then(Value::as_str)
        .unwrap_or("unreported");
    let device = result
        .get("device")
        .and_then(Value::as_str)
        .unwrap_or("unreported");
    let reused = result
        .get("reused")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let load = result
        .get("load_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let inference = result
        .get("inference_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    publish_job_event(
        app,
        JobEventDraft::running(
            job_id,
            "transcribing",
            format!(
                "{backend} · {device} · model {} · load {load:.2}s · inference {inference:.2}s",
                if reused { "reused" } else { "loaded" }
            ),
            0.88,
        ),
    );
    Ok(segments)
}

fn validate_segments(result: &Value, duration_ms: u64) -> JobResult<Vec<SubtitleSegment>> {
    let rows = result
        .get("segments")
        .and_then(Value::as_array)
        .ok_or_else(|| JobError::failed("ASR returned no timed subtitle segments."))?;
    if rows.is_empty() || rows.len() > 100_000 {
        return Err(JobError::failed(
            "ASR returned an empty or oversized subtitle result.",
        ));
    }
    let mut segments = Vec::with_capacity(rows.len());
    let mut previous_end = 0;
    for (index, row) in rows.iter().enumerate() {
        let start = row.get("start_ms").and_then(Value::as_u64);
        let end = row.get("end_ms").and_then(Value::as_u64);
        let text = row.get("text").and_then(Value::as_str).unwrap_or("").trim();
        let (Some(start), Some(end)) = (start, end) else {
            return Err(JobError::failed(
                "ASR returned missing or invalid timestamps; no estimated timing will be exported.",
            ));
        };
        if end <= start
            || start < previous_end
            || end > duration_ms.saturating_add(50)
            || text.is_empty()
            || text.len() > 65_536
        {
            return Err(JobError::failed(format!("ASR subtitle {} has invalid, overlapping or out-of-audio timestamps/text. Existing results have been preserved.", index + 1)));
        }
        segments.push(SubtitleSegment {
            id: index + 1,
            start_ms: start,
            end_ms: end,
            text: text.to_string(),
        });
        previous_end = end;
    }
    Ok(segments)
}

fn wav_duration_ms(path: &Path) -> Result<u64, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Cannot inspect prepared WAV: {e}"))?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let mut header = [0u8; 12];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return Err("Optional ASR requires the prepared PCM WAV file.".into());
    }
    let mut byte_rate = 0u64;
    for _ in 0..4096 {
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk)
            .map_err(|_| "WAV is missing its data chunk".to_string())?;
        let size = u32::from_le_bytes(chunk[4..8].try_into().unwrap()) as u64;
        let offset = file.stream_position().map_err(|e| e.to_string())?;
        if offset.saturating_add(size) > length {
            return Err("Prepared WAV is truncated.".into());
        }
        if &chunk[..4] == b"fmt " {
            if size < 16 {
                return Err("Invalid WAV format chunk.".into());
            }
            let mut format = [0u8; 16];
            file.read_exact(&mut format).map_err(|e| e.to_string())?;
            byte_rate = u32::from_le_bytes(format[8..12].try_into().unwrap()) as u64;
        } else if &chunk[..4] == b"data" {
            if byte_rate == 0 || size == 0 {
                return Err("Prepared WAV has no valid audio duration.".into());
            }
            return Ok(size.saturating_mul(1000) / byte_rate);
        }
        file.seek(SeekFrom::Start(offset + size + size % 2))
            .map_err(|e| e.to_string())?;
    }
    Err("Prepared WAV has too many chunks.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn omitted_and_partial_config_preserve_legacy() {
        let config: AsrConfig = serde_json::from_str("{}").unwrap();
        assert!(config.is_legacy());
        assert!(config.validate().is_ok());
        let partial: AsrConfig = serde_json::from_str(r#"{"engine":"qwen3-asr"}"#).unwrap();
        assert_eq!(partial.device, "auto");
        assert!(partial.validate().is_err());
        let unknown: AsrConfig = serde_json::from_str(r#"{"engine":"future-engine"}"#).unwrap();
        assert!(!unknown.is_legacy());
        assert!(unknown.validate().unwrap_err().contains("Unknown"));
    }
    #[test]
    fn aligned_segments_round_trip_import_export_with_original_text() {
        let value = json!({"segments":[{"start_ms":10,"end_ms":1000,"text":"你好，世界！"},{"start_ms":1200,"end_ms":2100,"text":"Hello, world."}]});
        let segments = validate_segments(&value, 2200).unwrap();
        let imported =
            crate::subtitles::parse_srt_text(&crate::subtitles::render_srt(&segments, None))
                .unwrap();
        assert_eq!(imported.len(), 2);
        assert_eq!(imported[0].text, "你好，世界！");
        assert_eq!(imported[1].start_ms, 1200);
        assert_eq!(imported[1].end_ms, 2100);
    }
    #[test]
    fn invalid_alignment_is_never_exported() {
        for segments in [
            json!([]),
            json!([{"start_ms":-1,"end_ms":100,"text":"a"}]),
            json!([{"start_ms":0,"end_ms":2000,"text":"a"}]),
            json!([{"start_ms":1,"end_ms":1,"text":"a"}]),
            json!([{"start_ms":0,"end_ms":200,"text":"a"},{"start_ms":199,"end_ms":300,"text":"b"}]),
        ] {
            assert!(validate_segments(&json!({"segments":segments}), 1000).is_err());
        }
    }
}
