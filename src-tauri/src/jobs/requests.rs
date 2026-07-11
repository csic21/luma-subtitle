use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Clone, Deserialize)]
pub(crate) struct JobRequest {
    #[serde(alias = "video_path")]
    pub(crate) media_path: String,
    #[serde(default = "default_media_source_type")]
    pub(crate) source_type: String,
    pub(crate) output_dir: Option<String>,
    pub(crate) target_language: String,
    pub(crate) whisper_model_path: String,
    pub(crate) whisper_language: String,
    pub(crate) base_url: String,
    #[serde(default)]
    pub(crate) base_url_is_complete: bool,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    pub(crate) translation_shard_size: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct TranslateSubtitlesRequest {
    pub(crate) job_id: String,
    pub(crate) target_language: String,
    pub(crate) base_url: String,
    #[serde(default)]
    pub(crate) base_url_is_complete: bool,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    pub(crate) translation_shard_size: Option<usize>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct CreateVideoTaskRequest {
    pub(crate) video_path: String,
    pub(crate) output_dir: Option<String>,
    pub(crate) target_language: String,
    pub(crate) whisper_model_path: String,
    pub(crate) whisper_language: String,
    pub(crate) base_url: String,
    #[serde(default)]
    pub(crate) base_url_is_complete: bool,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    pub(crate) translation_shard_size: Option<usize>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct CreateAudioTaskRequest {
    pub(crate) audio_path: String,
    pub(crate) output_dir: Option<String>,
    pub(crate) target_language: String,
    pub(crate) whisper_model_path: String,
    pub(crate) whisper_language: String,
    pub(crate) base_url: String,
    #[serde(default)]
    pub(crate) base_url_is_complete: bool,
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
    #[serde(default)]
    pub(crate) base_url_is_complete: bool,
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
    #[serde(default)]
    pub(crate) base_url_is_complete: bool,
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

fn default_media_source_type() -> String {
    "video".to_string()
}
