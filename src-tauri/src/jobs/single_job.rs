use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

use tauri::{AppHandle, Manager};

use crate::{
    job_events::{emit_job, JobOutputs, JobStatus, StoredSubtitleResult},
    paths::{path_to_string, resolve_output_dir, safe_stem},
    state::{ensure_not_cancelled, AppState, JobError, JobResult},
    subtitles::{parse_whisper_json, render_srt},
    task_db,
    translation::{
        normalize_translation_shard_size, translate_with_single_request, TranslationConfig,
        DEFAULT_TRANSLATION_SHARD_SIZE,
    },
};

use super::{
    helpers::translated_file_name,
    process::{extract_audio, transcribe_audio},
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
        base_url: request.base_url.trim().trim_end_matches('/').to_string(),
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
    emit_job(
        app,
        &request.job_id,
        "render-translated-srt",
        JobStatus::Running,
        "正在生成译文 SRT",
        0.96,
        None,
        None,
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
    emit_job(
        &app,
        &job_id,
        "extracting",
        JobStatus::Running,
        "正在抽取音频",
        0.08,
        None,
        None,
    );
    extract_audio(&app, &video_path, &audio_path, cancel.clone()).await?;
    ensure_not_cancelled(&cancel)?;
    emit_job(
        &app,
        &job_id,
        "transcribing",
        JobStatus::Running,
        "正在本地转写",
        0.26,
        None,
        None,
    );
    transcribe_audio(
        &app,
        &model_path,
        &audio_path,
        &transcript_base,
        &request.whisper_language,
        cancel.clone(),
    )
    .await?;
    let segments = parse_whisper_json(&transcript_json)?;
    if segments.is_empty() {
        return Err(JobError::failed("Whisper 没有返回可用字幕段"));
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
    emit_job(
        &app,
        &job_id,
        "source-srt",
        JobStatus::Running,
        "原文字幕已生成到内存",
        0.9,
        Some(outputs.clone()),
        None,
    );

    Ok(outputs)
}
