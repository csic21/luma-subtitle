use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

use tauri::{AppHandle, Manager};

use crate::{
    job_events::{publish_job_event, JobEventDraft, JobOutputs, StoredSubtitleResult},
    paths::{path_to_string, resolve_output_dir, safe_stem},
    state::{ensure_not_cancelled, AppState, JobError, JobResult},
    subtitles::{parse_whisper_json, render_srt, validate_whisper_repetition, SubtitleSegment},
    translation::{
        normalize_translation_shard_size, translate_with_single_request, TranslationConfig,
        DEFAULT_TRANSLATION_SHARD_SIZE,
    },
};

use super::{
    helpers::translated_file_name,
    process::{prepare_audio, transcribe_audio, TranscriptionMode},
    JobRequest, TranslateSubtitlesRequest,
};

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
    let media_path = PathBuf::from(&request.media_path);
    let model_path = PathBuf::from(&request.whisper_model_path);
    let output_dir = resolve_output_dir(&media_path, request.output_dir.as_deref())?;
    let work_dir = output_dir.join(".luma-subtitle-work").join(&job_id);
    tokio::fs::create_dir_all(&work_dir)
        .await
        .map_err(|error| JobError::failed(format!("创建任务目录失败: {error}")))?;
    let transcript_base = work_dir.join("transcription");
    let stem = safe_stem(&media_path);
    let source_file_name = format!("{stem}.source.srt");

    let audio_path = prepare_transcription_audio(
        &app,
        &job_id,
        &request.source_type,
        &media_path,
        &work_dir,
        cancel.clone(),
    )
    .await?;
    let segments = transcribe_prepared_audio(
        &app,
        &job_id,
        &model_path,
        &audio_path,
        &transcript_base,
        &request.whisper_language,
        cancel.clone(),
    )
    .await?;
    let (stored, outputs) = source_subtitle_result(segments, &source_file_name, &output_dir);

    app.state::<AppState>()
        .subtitle_results
        .lock()
        .insert(job_id.clone(), stored);
    publish_job_event(
        &app,
        JobEventDraft::running(&job_id, "source-srt", "原文字幕已生成到内存", 0.9)
            .with_outputs(outputs.clone()),
    );

    Ok(outputs)
}

async fn prepare_transcription_audio(
    app: &AppHandle,
    job_id: &str,
    source_type: &str,
    media_path: &Path,
    work_dir: &Path,
    cancel: Arc<AtomicBool>,
) -> JobResult<PathBuf> {
    let audio_path = work_dir.join("audio.wav");
    let (stage, message) = if source_type == "audio" {
        ("preparing-audio", "正在准备音频")
    } else {
        ("extracting", "正在抽取音频")
    };
    ensure_not_cancelled(&cancel)?;
    publish_job_event(app, JobEventDraft::running(job_id, stage, message, 0.08));
    prepare_audio(app, media_path, &audio_path, cancel).await?;
    Ok(audio_path)
}

async fn transcribe_prepared_audio(
    app: &AppHandle,
    job_id: &str,
    model_path: &Path,
    audio_path: &Path,
    transcript_base: &Path,
    language: &str,
    cancel: Arc<AtomicBool>,
) -> JobResult<Vec<SubtitleSegment>> {
    let transcript_json = transcript_base.with_extension("json");
    ensure_not_cancelled(&cancel)?;
    publish_job_event(
        app,
        JobEventDraft::running(job_id, "transcribing", "正在本地转写", 0.26),
    );
    transcribe_audio(
        app,
        model_path,
        audio_path,
        transcript_base,
        language,
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
                job_id,
                "transcribing-retry",
                "检测到疑似重复幻觉，正在用 VAD 和保守参数重试转写",
                0.74,
            ),
        );
        transcribe_audio(
            app,
            model_path,
            audio_path,
            transcript_base,
            language,
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
    Ok(segments)
}

fn source_subtitle_result(
    segments: Vec<SubtitleSegment>,
    source_file_name: &str,
    output_dir: &Path,
) -> (StoredSubtitleResult, JobOutputs) {
    let source_srt = render_srt(&segments, None);
    let segment_count = segments.len();
    let output_dir = path_to_string(output_dir.to_path_buf());
    let outputs = JobOutputs {
        source_file_name: source_file_name.to_string(),
        translated_file_name: None,
        output_dir: output_dir.clone(),
        segment_count,
    };
    (
        StoredSubtitleResult {
            source_srt,
            translated_srt: None,
            segments,
            output_dir,
            source_file_name: source_file_name.to_string(),
            translated_file_name: None,
        },
        outputs,
    )
}

fn job_error_message(error: JobError) -> String {
    match error {
        JobError::Cancelled => "任务已取消".to_string(),
        JobError::Failed(message) => message,
    }
}
