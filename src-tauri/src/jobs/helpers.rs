use std::path::{Path, PathBuf};

use crate::{
    paths::{path_to_string, sanitize_file_part},
    settings::normalize_language,
    state::JobError,
    task_db::TaskSettingsSnapshot,
    translation::{normalize_translation_shard_size, DEFAULT_TRANSLATION_SHARD_SIZE},
};

use super::{
    CreateSrtTaskRequest, CreateVideoTaskRequest, JobRequest, TranslateSubtitlesRequest,
    UpdateTaskSettingsRequest,
};

pub(super) fn task_settings_from_video_request(
    request: &CreateVideoTaskRequest,
) -> TaskSettingsSnapshot {
    TaskSettingsSnapshot {
        output_dir: request
            .output_dir
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        target_language: request.target_language.trim().to_string(),
        whisper_model_path: request.whisper_model_path.trim().to_string(),
        whisper_language: normalize_language(&request.whisper_language),
        base_url: request.base_url.trim().trim_end_matches('/').to_string(),
        model: request.model.trim().to_string(),
        temperature: request.temperature.clamp(0.0, 1.0),
        translation_shard_size: normalize_translation_shard_size(
            request
                .translation_shard_size
                .unwrap_or(DEFAULT_TRANSLATION_SHARD_SIZE),
        ),
    }
}

pub(super) fn task_settings_from_srt_request(
    request: &CreateSrtTaskRequest,
) -> TaskSettingsSnapshot {
    TaskSettingsSnapshot {
        output_dir: request
            .output_dir
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        target_language: request.target_language.trim().to_string(),
        whisper_model_path: request.whisper_model_path.trim().to_string(),
        whisper_language: normalize_language(&request.whisper_language),
        base_url: request.base_url.trim().trim_end_matches('/').to_string(),
        model: request.model.trim().to_string(),
        temperature: request.temperature.clamp(0.0, 1.0),
        translation_shard_size: normalize_translation_shard_size(
            request
                .translation_shard_size
                .unwrap_or(DEFAULT_TRANSLATION_SHARD_SIZE),
        ),
    }
}

pub(super) fn task_settings_from_update_request(
    output_dir: Option<String>,
    request: &UpdateTaskSettingsRequest,
) -> TaskSettingsSnapshot {
    TaskSettingsSnapshot {
        output_dir,
        target_language: request.target_language.trim().to_string(),
        whisper_model_path: request.whisper_model_path.trim().to_string(),
        whisper_language: normalize_language(&request.whisper_language),
        base_url: request.base_url.trim().trim_end_matches('/').to_string(),
        model: request.model.trim().to_string(),
        temperature: request.temperature.clamp(0.0, 1.0),
        translation_shard_size: normalize_translation_shard_size(
            request
                .translation_shard_size
                .unwrap_or(DEFAULT_TRANSLATION_SHARD_SIZE),
        ),
    }
}

pub(super) fn display_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path_to_string(path.to_path_buf()))
}

pub(super) fn operation_failed_message(operation: &str) -> &'static str {
    match operation {
        "transcribe" => "转写失败",
        "translate" => "翻译失败",
        "export" => "导出失败",
        _ => "任务失败",
    }
}

pub(super) fn operation_cancelled_message(operation: &str) -> &'static str {
    match operation {
        "transcribe" => "转写已取消",
        "translate" => "翻译已取消",
        "export" => "导出已取消",
        _ => "任务已取消",
    }
}

pub(super) fn job_error_to_string(error: JobError) -> String {
    match error {
        JobError::Cancelled => "导出已取消".to_string(),
        JobError::Failed(message) => message,
    }
}

pub(super) fn translated_file_name(source_file_name: &str, target_language: &str) -> String {
    let stem = source_file_name
        .strip_suffix(".source.srt")
        .or_else(|| source_file_name.strip_suffix(".srt"))
        .unwrap_or(source_file_name);
    format!("{}.{}.srt", stem, sanitize_file_part(target_language))
}

pub(super) fn validate_start_request(request: &JobRequest) -> Result<(), String> {
    let video_path = PathBuf::from(&request.video_path);
    if !video_path.exists() {
        return Err("视频文件不存在".to_string());
    }
    if request.whisper_model_path.trim().is_empty() {
        return Err("请选择 Whisper 模型文件".to_string());
    }
    Ok(())
}

pub(super) fn validate_translate_request(
    request: &TranslateSubtitlesRequest,
) -> Result<(), String> {
    if request.job_id.trim().is_empty() {
        return Err("没有可翻译的任务".to_string());
    }
    if request.target_language.trim().is_empty() {
        return Err("目标语言不能为空".to_string());
    }
    if request.base_url.trim().is_empty() || request.model.trim().is_empty() {
        return Err("翻译接口配置不完整".to_string());
    }
    Ok(())
}
