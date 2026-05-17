use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Clone, Deserialize)]
pub(crate) struct JobRequest {
    pub(crate) video_path: String,
    pub(crate) output_dir: Option<String>,
    pub(crate) target_language: String,
    pub(crate) whisper_model_path: String,
    pub(crate) whisper_language: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    pub(crate) translation_shard_size: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct ExportSubtitlesRequest {
    pub(crate) job_id: String,
    pub(crate) output_dir: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct TranslateSubtitlesRequest {
    pub(crate) job_id: String,
    pub(crate) target_language: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    pub(crate) translation_shard_size: Option<usize>,
    pub(crate) api_key: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ImportSourceSrtRequest {
    pub(crate) srt_path: String,
    pub(crate) output_dir: Option<String>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct CreateVideoTaskRequest {
    pub(crate) video_path: String,
    pub(crate) output_dir: Option<String>,
    pub(crate) target_language: String,
    pub(crate) whisper_model_path: String,
    pub(crate) whisper_language: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    pub(crate) translation_shard_size: Option<usize>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct CreateSrtTaskRequest {
    pub(crate) srt_path: String,
    pub(crate) output_dir: Option<String>,
    pub(crate) target_language: String,
    pub(crate) whisper_model_path: String,
    pub(crate) whisper_language: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    pub(crate) translation_shard_size: Option<usize>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct UpdateTaskSettingsRequest {
    pub(crate) target_language: String,
    pub(crate) whisper_model_path: String,
    pub(crate) whisper_language: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    pub(crate) translation_shard_size: Option<usize>,
}

#[derive(Clone, Serialize)]
pub(crate) struct SubtitlePreview {
    pub(crate) source_srt: String,
    pub(crate) translated_srt: Option<String>,
    pub(crate) source_file_name: String,
    pub(crate) translated_file_name: Option<String>,
}
