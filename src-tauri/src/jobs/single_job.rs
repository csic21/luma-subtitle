use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

use tauri::{AppHandle, Manager};

use crate::{
    job_events::{publish_job_event, JobEventDraft, JobOutputs, StoredSubtitleResult},
    paths::{path_to_string, resolve_output_dir, safe_stem},
    state::{ensure_not_cancelled, AppState, JobError, JobResult},
    subtitles::{parse_whisper_json, render_srt, validate_whisper_repetition},
    task_db,
    translation::{
        normalize_translation_shard_size, translate_with_single_request, TranslationConfig,
        DEFAULT_TRANSLATION_SHARD_SIZE,
    },
};

use super::{
    helpers::translated_file_name,
    process::{extract_audio, transcribe_audio, TranscriptionMode},
    JobRequest, TranslateSubtitlesRequest,
};

pub(super) fn resolve_translation_inputs(
    app: &AppHandle,
    request: &TranslateSubtitlesRequest,
) -> Result<(StoredSubtitleResult, String), String> {
    let api_key = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .map(Ok)
        .unwrap_or_else(|| {
            task_db::load_api_key(app)?
                .ok_or_else(|| "请先保存 OpenAI 兼容接口的 API Key".to_string())
        })?;
    let stored = app
        .state::<AppState>()
        .subtitle_results
        .lock()
        .get(&request.job_id)
        .cloned()
        .ok_or_else(|| "没有找到可翻译的字幕结果".to_string())?;
    Ok((stored, api_key))
}

pub(super) async fn run_translation(
    app: &AppHandle,
    request: &TranslateSubtitlesRequest,
    mut stored: StoredSubtitleResult,
    api_key: &str,
    cancel: Arc<AtomicBool>,
) -> JobResult<(StoredSubtitleResult, JobOutputs)> {
    let config = TranslationConfig {
        target_language: request.target_language.trim().to_string(),
        base_url: crate::settings::normalize_base_url(
            &request.base_url,
            request.base_url_is_complete,
        ),
        base_url_is_complete: request.base_url_is_complete,
        model: request.model.trim().to_string(),
        temperature: request.temperature,
        shard_size: normalize_translation_shard_size(
            request
                .translation_shard_size
                .unwrap_or(DEFAULT_TRANSLATION_SHARD_SIZE),
        ),
    };
    let translations = translate_with_single_request(
        app,
        &request.job_id,
        &config,
        api_key,
        &stored.segments,
        &stored.source_srt,
        &stored.source_file_name,
        cancel,
    )
    .await?;
    publish_job_event(
        app,
        JobEventDraft::running(
            &request.job_id,
            "render-translated-srt",
            "正在生成译文 SRT",
            0.96,
        ),
    );
    let translated_srt = render_srt(&stored.segments, Some(&translations));
    let translated_file_name =
        translated_file_name(&stored.source_file_name, &config.target_language);
    stored.translated_srt = Some(translated_srt);
    stored.translated_file_name = Some(translated_file_name.clone());

    let outputs = JobOutputs {
        source_file_name: stored.source_file_name.clone(),
        translated_file_name: Some(translated_file_name),
        output_dir: stored.output_dir.clone(),
        segment_count: stored.segments.len(),
    };
    Ok((stored, outputs))
}

pub(super) async fn run_job(
    app: AppHandle,
    job_id: String,
    request: JobRequest,
    cancel: Arc<AtomicBool>,
) -> JobResult<JobOutputs> {
    let video_path = PathBuf::from(&request.video_path);
    let model_path = PathBuf::from(&request.whisper_model_path);
    let output_dir = resolve_output_dir(&video_path, request.output_dir.as_deref())?;
    let work_dir = output_dir.join(".luma-subtitle-work").join(&job_id);
    tokio::fs::create_dir_all(&work_dir)
        .await
        .map_err(|error| JobError::failed(format!("创建任务目录失败: {error}")))?;
    let audio_path = work_dir.join("audio.wav");
    let transcript_base = work_dir.join("transcription");
    let transcript_json = transcript_base.with_extension("json");
    let stem = safe_stem(&video_path);
    let source_file_name = format!("{stem}.source.srt");
    ensure_not_cancelled(&cancel)?;
    publish_job_event(
        &app,
        JobEventDraft::running(&job_id, "extracting", "正在抽取音频", 0.08),
    );
    extract_audio(&app, &video_path, &audio_path, cancel.clone()).await?;
    ensure_not_cancelled(&cancel)?;
    publish_job_event(
        &app,
        JobEventDraft::running(&job_id, "transcribing", "正在本地转写", 0.26),
    );
    transcribe_audio(
        &app,
        &model_path,
        &audio_path,
        &transcript_base,
        &request.whisper_language,
        TranscriptionMode::Standard,
        cancel.clone(),
    )
    .await?;
    let mut segments = parse_whisper_json(&transcript_json)?;
    if segments.is_empty() {
        return Err(JobError::failed("Whisper 没有返回可用字幕段"));
    }
    if let Err(first_error) = validate_whisper_repetition(&segments) {
        let first_message = job_error_message(first_error);
        publish_job_event(
            &app,
            JobEventDraft::running(
                &job_id,
                "transcribing-retry",
                "检测到疑似重复幻觉，正在用 VAD 和保守参数重试转写",
                0.74,
            ),
        );
        transcribe_audio(
            &app,
            &model_path,
            &audio_path,
            &transcript_base,
            &request.whisper_language,
            TranscriptionMode::ConservativeRetry,
            cancel.clone(),
        )
        .await?;
        segments = parse_whisper_json(&transcript_json)?;
        if segments.is_empty() {
            return Err(JobError::failed("Whisper 重试后没有返回可用字幕段"));
        }
        validate_whisper_repetition(&segments).map_err(|retry_error| {
            JobError::failed(format!(
                "{}\n\n已自动用 VAD 和保守参数重试一次，但仍检测到重复。首次检测结果：{}",
                job_error_message(retry_error),
                first_message
            ))
        })?;
    }
    let source_srt = render_srt(&segments, None);
    let segment_count = segments.len();
    let output_dir = path_to_string(output_dir);
    let outputs = JobOutputs {
        source_file_name: source_file_name.clone(),
        translated_file_name: None,
        output_dir: output_dir.clone(),
        segment_count,
    };

    app.state::<AppState>().subtitle_results.lock().insert(
        job_id.clone(),
        StoredSubtitleResult {
            source_srt,
            translated_srt: None,
            segments,
            output_dir,
            source_file_name,
            translated_file_name: None,
        },
    );
    publish_job_event(
        &app,
        JobEventDraft::running(&job_id, "source-srt", "原文字幕已生成到内存", 0.9)
            .with_outputs(outputs.clone()),
    );

    Ok(outputs)
}

fn job_error_message(error: JobError) -> String {
    match error {
        JobError::Cancelled => "任务已取消".to_string(),
        JobError::Failed(message) => message,
    }
}
