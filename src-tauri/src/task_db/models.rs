use serde::{Deserialize, Serialize};

use crate::translation::{
    DEFAULT_TRANSLATION_CLI_COMMAND, DEFAULT_TRANSLATION_CLI_TOOL, DEFAULT_TRANSLATION_PROVIDER,
    DEFAULT_TRANSLATION_SHARD_SIZE,
};

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct TaskSettingsSnapshot {
    pub(crate) output_dir: Option<String>,
    pub(crate) target_language: String,
    pub(crate) whisper_model_path: String,
    pub(crate) whisper_language: String,
    pub(crate) base_url: String,
    #[serde(default)]
    pub(crate) base_url_is_complete: bool,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    #[serde(default = "default_translation_shard_size")]
    pub(crate) translation_shard_size: usize,
    #[serde(default = "default_translation_provider")]
    pub(crate) translation_provider: String,
    #[serde(default = "default_translation_cli_tool")]
    pub(crate) translation_cli_tool: String,
    #[serde(default = "default_translation_cli_command")]
    pub(crate) translation_cli_command: String,
    #[serde(default)]
    pub(crate) translation_cli_model: String,
    #[serde(default)]
    pub(crate) translation_cli_args: String,
}

#[derive(Clone, Serialize)]
pub(crate) struct TaskRecord {
    pub(crate) id: String,
    pub(crate) source_type: String,
    pub(crate) video_path: Option<String>,
    pub(crate) audio_path: Option<String>,
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
    pub(crate) auto_start_next: bool,
}

fn default_translation_shard_size() -> usize {
    DEFAULT_TRANSLATION_SHARD_SIZE
}

fn default_translation_provider() -> String {
    DEFAULT_TRANSLATION_PROVIDER.to_string()
}

fn default_translation_cli_tool() -> String {
    DEFAULT_TRANSLATION_CLI_TOOL.to_string()
}

fn default_translation_cli_command() -> String {
    DEFAULT_TRANSLATION_CLI_COMMAND.to_string()
}
