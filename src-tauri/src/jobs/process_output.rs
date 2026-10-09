use std::collections::VecDeque;

const MAX_LINE_BYTES: usize = 8 * 1024;
const MAX_ERROR_LINES: usize = 8;

/// Keep errors bounded while retaining only non-content diagnostics on success.
#[derive(Default)]
pub(super) struct ProcessOutput {
    pending: Vec<u8>,
    truncated_line: bool,
    tail: VecDeque<String>,
    pub(super) progress: Option<u8>,
    pub(super) backend: Option<String>,
    encoder_backend: Option<&'static str>,
    vad_started: bool,
    load_ms: Option<f64>,
    encode_ms: Option<f64>,
    decode_ms: Option<f64>,
    batch_decode_ms: Option<f64>,
}

impl ProcessOutput {
    pub(super) fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' || byte == b'\r' {
                self.finish_line();
            } else if self.pending.len() < MAX_LINE_BYTES {
                self.pending.push(byte);
            } else {
                self.truncated_line = true;
            }
        }
    }

    pub(super) fn finish(&mut self) {
        self.finish_line();
    }

    fn finish_line(&mut self) {
        if self.pending.is_empty() && !self.truncated_line {
            return;
        }
        let bytes = std::mem::take(&mut self.pending);
        let mut line = String::from_utf8_lossy(&bytes).trim().to_string();
        if !self.truncated_line {
            self.observe(&line);
        } else {
            line.push_str(" …[truncated]");
        }
        self.truncated_line = false;
        if !line.is_empty() {
            if self.tail.len() == MAX_ERROR_LINES {
                self.tail.pop_front();
            }
            self.tail.push_back(line);
        }
    }

    fn observe(&mut self, line: &str) {
        if let Some(progress) = whisper_progress(line) {
            self.progress = Some(self.progress.unwrap_or(0).max(progress));
        }
        // VAD initializes its own (usually CPU) backend after Whisper. Do not
        // misreport that auxiliary backend as the transcription backend.
        if line.starts_with("whisper_vad: ") || line.starts_with("whisper_vad_init_") {
            self.vad_started = true;
        }
        if let Some(detail) = line
            .strip_prefix("whisper_backend_init_gpu: ")
            .filter(|_| !self.vad_started)
        {
            if detail == "no GPU found" {
                self.backend = Some("CPU".to_string());
            } else if let Some(backend) = detail
                .strip_prefix("using ")
                .and_then(|value| value.strip_suffix(" backend"))
                .filter(|value| !value.is_empty() && value.len() <= 80)
            {
                self.backend = Some(backend.to_string());
            } else if detail.starts_with("failed to initialize ") {
                // The preceding "using" line announces an attempt, not success.
                self.backend = None;
            }
        }
        if line == "whisper_init_state: Core ML model loaded" {
            self.encoder_backend = Some("Core ML");
        }
        if let Some(timing) = line.strip_prefix("whisper_print_timings:") {
            if let Some((label, value)) = timing.split_once('=') {
                let ms = value
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite() && *value >= 0.0);
                match label.trim() {
                    "load time" => self.load_ms = ms,
                    "encode time" => self.encode_ms = ms,
                    "decode time" => self.decode_ms = ms,
                    "batchd time" => self.batch_decode_ms = ms,
                    _ => {}
                }
            }
        }
    }

    pub(super) fn error_detail(&self) -> String {
        if self.tail.is_empty() {
            String::new()
        } else {
            format!(
                "\n{}",
                self.tail.iter().cloned().collect::<Vec<_>>().join("\n")
            )
        }
    }

    pub(super) fn diagnostic_summary(&self) -> String {
        let mut parts = vec![format!(
            "引擎报告后端：{}{}",
            self.backend.as_deref().unwrap_or("未报告"),
            self.encoder_backend
                .map(|encoder| format!(" / {encoder} 编码器"))
                .unwrap_or_default()
        )];
        for (label, value) in [
            ("模型加载", self.load_ms),
            ("编码", self.encode_ms),
            ("解码", self.decode_ms),
            ("批量解码", self.batch_decode_ms),
        ] {
            if let Some(ms) = value {
                parts.push(format!("{label} {:.2} 秒", ms / 1000.0));
            }
        }
        parts.join(" · ")
    }
}

fn whisper_progress(line: &str) -> Option<u8> {
    let value = line
        .strip_prefix("whisper_print_progress_callback:")?
        .trim()
        .strip_prefix("progress =")?
        .trim()
        .strip_suffix('%')?
        .trim()
        .parse::<u8>()
        .ok()?;
    (value <= 100).then_some(value)
}

pub(super) fn transcription_threads(available: usize) -> usize {
    // Leave a core free when possible, but never oversubscribe small machines.
    available.saturating_sub(1).clamp(1, 12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_split_progress_and_timing_lines() {
        let mut output = ProcessOutput::default();
        output.push(b"whisper_print_progress_call");
        output.push(b"back: progress =  25%\r\nwhisper_print_timings: load time = 1500.00 ms\n");
        output.push(b"whisper_print_timings: encode time = 2000.00 ms / 3 runs\n");
        output.push(b"whisper_print_timings: decode time = 500.00 ms");
        output.finish();
        assert_eq!(output.progress, Some(25));
        assert!(output.diagnostic_summary().contains("模型加载 1.50 秒"));
        assert!(output.diagnostic_summary().contains("编码 2.00 秒"));
        assert!(output.diagnostic_summary().contains("解码 0.50 秒"));
    }

    #[test]
    fn rejects_malformed_progress_and_never_moves_backwards() {
        let mut output = ProcessOutput::default();
        for line in [
            "other: progress = 99%",
            "whisper_print_progress_callback: progress = 101%",
            "whisper_print_progress_callback: progress = NaN%",
        ] {
            output.push(format!("{line}\n").as_bytes());
        }
        assert_eq!(output.progress, None);
        output.push(b"whisper_print_progress_callback: progress = 50%\n");
        output.push(b"whisper_print_progress_callback: progress = 10%\n");
        assert_eq!(output.progress, Some(50));
    }

    #[test]
    fn hardware_capability_and_gpu_request_are_not_backend_evidence() {
        let mut output = ProcessOutput::default();
        output.push(
            b"system_info: Metal = 1 | CUDA = 1\nwhisper_init_with_params_no_state: use gpu = 1\n",
        );
        assert_eq!(output.backend, None);
        output.push(b"whisper_backend_init_gpu: using Metal backend\n");
        assert_eq!(output.backend.as_deref(), Some("Metal"));
        output.push(b"whisper_backend_init_gpu: failed to initialize Metal backend\n");
        assert_eq!(output.backend, None);
        output.push(b"whisper_backend_init_gpu: no GPU found\n");
        assert_eq!(output.backend.as_deref(), Some("CPU"));
    }

    #[test]
    fn vad_cpu_backend_does_not_replace_transcription_gpu_evidence() {
        let mut output = ProcessOutput::default();
        output.push(b"whisper_backend_init_gpu: using Metal backend\n");
        output.push(b"whisper_vad: VAD is enabled, processing speech segments only\n");
        output.push(b"whisper_backend_init_gpu: no GPU found\n");
        assert_eq!(output.backend.as_deref(), Some("Metal"));
        output.push(b"whisper_print_timings: batchd time = 2500.00 ms / 3 runs\n");
        assert!(output.diagnostic_summary().contains("批量解码 2.50 秒"));
    }

    #[test]
    fn retains_cuda_device_and_coreml_encoder_evidence() {
        let mut output = ProcessOutput::default();
        output.push(b"whisper_backend_init_gpu: using CUDA0 backend\n");
        assert_eq!(output.backend.as_deref(), Some("CUDA0"));
        output.push(b"whisper_init_state: Core ML model loaded\n");
        assert!(output.diagnostic_summary().contains("Core ML 编码器"));
    }

    #[test]
    fn bounds_unterminated_lines_and_error_history() {
        let mut output = ProcessOutput::default();
        output.push(&vec![b'x'; MAX_LINE_BYTES * 20]);
        assert_eq!(output.pending.len(), MAX_LINE_BYTES);
        output.finish();
        assert!(output.error_detail().contains("[truncated]"));
        for index in 0..20 {
            output.push(format!("line {index}\n").as_bytes());
        }
        assert_eq!(output.tail.len(), MAX_ERROR_LINES);
        assert!(output.error_detail().starts_with("\nline 12\n"));
        assert!(output.error_detail().ends_with("line 19"));
    }

    #[test]
    fn ignores_nonfinite_timings_and_preserves_lossy_error_text() {
        let mut output = ProcessOutput::default();
        output.push(b"whisper_print_timings: load time = NaN ms\nfailed: \xff\n");
        assert!(!output.diagnostic_summary().contains("NaN"));
        assert!(output.error_detail().contains("failed:"));
    }

    #[test]
    fn threads_do_not_exceed_small_machine_capacity() {
        assert_eq!(transcription_threads(0), 1);
        assert_eq!(transcription_threads(1), 1);
        assert_eq!(transcription_threads(2), 1);
        assert_eq!(transcription_threads(4), 3);
        assert_eq!(transcription_threads(8), 7);
        assert_eq!(transcription_threads(64), 12);
    }
}
