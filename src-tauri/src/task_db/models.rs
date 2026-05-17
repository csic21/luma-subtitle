use serde::{Deserialize, Serialize};

use crate::translation::DEFAULT_TRANSLATION_SHARD_SIZE;

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct TaskSettingsSnapshot {
    pub(crate) output_dir: Option<String>,
    pub(crate) target_language: String,
    pub(crate) whisper_model_path: String,
    pub(crate) whisper_language: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    #[serde(default = "default_translation_shard_size")]
    pub(crate) translation_shard_size: usize,
}

#[derive(Clone, Serialize)]
pub(crate) struct TaskRecord {
    pub(crate) id: String,
    pub(crate) source_type: String,
    pub(crate) video_path: Option<String>,
    pub(crate) srt_path: Option<String>,
    pub(crate) file_name: String,
    pub(crate) status: String,
    pub(crate) stage: String,
    pub(crate) message: String,
    pub(crate) progress: f32,
    pub(crate) settings: TaskSettingsSnapshot,
    pub(crate) source_srt_path: Option<String>,
    pub(crate) translated_srt_path: Option<String>,
    pub(crate) source_file_name: Option<String>,
    pub(crate) translated_file_name: Option<String>,
    pub(crate) output_dir: Option<String>,
    pub(crate) segment_count: Option<usize>,
    pub(crate) exported_source_srt: Option<String>,
    pub(crate) exported_translated_srt: Option<String>,
    pub(crate) exported_output_dir: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct QueueSettings {
    pub(crate) max_concurrency: usize,
}

fn default_translation_shard_size() -> usize {
    DEFAULT_TRANSLATION_SHARD_SIZE
}
