use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager, State};
use tokio::{io::AsyncReadExt, process::Command, time::sleep};
use uuid::Uuid;

use crate::{
    job_events::{emit_job, ExportedSubtitlePaths, JobOutputs, JobStatus, StoredSubtitleResult},
    paths::{locate_binary, path_to_string, resolve_output_dir, safe_stem, sanitize_file_part},
    settings::{self, normalize_language},
    state::{ensure_not_cancelled, AppState, JobError, JobResult, QueuedTaskOperation},
    subtitles::{parse_srt_file, parse_whisper_json, render_srt, write_srt_text},
    task_db::{self, QueueSettings, TaskRecord, TaskSettingsSnapshot},
    translation::{
        normalize_translation_shard_size, translate_with_single_request, TranslationConfig,
        DEFAULT_TRANSLATION_SHARD_SIZE,
    },
};

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
    job_id: String,
    output_dir: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct TranslateSubtitlesRequest {
    job_id: String,
    target_language: String,
    base_url: String,
    model: String,
    temperature: f32,
    translation_shard_size: Option<usize>,
    api_key: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ImportSourceSrtRequest {
    srt_path: String,
    output_dir: Option<String>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct CreateVideoTaskRequest {
    video_path: String,
    output_dir: Option<String>,
    target_language: String,
    whisper_model_path: String,
    whisper_language: String,
    base_url: String,
    model: String,
    temperature: f32,
    translation_shard_size: Option<usize>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct CreateSrtTaskRequest {
    srt_path: String,
    output_dir: Option<String>,
    target_language: String,
    whisper_model_path: String,
    whisper_language: String,
    base_url: String,
    model: String,
    temperature: f32,
    translation_shard_size: Option<usize>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct UpdateTaskSettingsRequest {
    target_language: String,
    whisper_model_path: String,
    whisper_language: String,
    base_url: String,
    model: String,
    temperature: f32,
    translation_shard_size: Option<usize>,
}

#[derive(Clone, Serialize)]
pub(crate) struct SubtitlePreview {
    source_srt: String,
    translated_srt: Option<String>,
    source_file_name: String,
    translated_file_name: Option<String>,
}

#[tauri::command]
pub(crate) fn list_tasks(app: AppHandle) -> Result<Vec<TaskRecord>, String> {
    task_db::list_tasks(&app)
}

#[tauri::command]
pub(crate) fn get_task(app: AppHandle, task_id: String) -> Result<TaskRecord, String> {
    task_db::require_task(&app, &task_id)
}

#[tauri::command]
pub(crate) fn get_task_logs(app: AppHandle, task_id: String) -> Result<Vec<String>, String> {
    task_db::task_logs(&app, &task_id)
}

#[tauri::command]
pub(crate) fn apply_current_settings_to_task(
    app: AppHandle,
    task_id: String,
) -> Result<TaskRecord, String> {
    let task = task_db::require_task(&app, &task_id)?;
    if matches!(task.status.as_str(), "queued" | "running") {
        return Err("任务正在运行或排队中，稍后再应用当前设置".to_string());
    }
    let settings = settings::task_settings_from_current(&app, task.settings.output_dir.clone())?;
    task_db::update_task_settings(&app, &task_id, settings)
}

#[tauri::command]
pub(crate) fn update_task_settings(
    app: AppHandle,
    task_id: String,
    settings: UpdateTaskSettingsRequest,
) -> Result<TaskRecord, String> {
    let task = task_db::require_task(&app, &task_id)?;
    if matches!(task.status.as_str(), "queued" | "running") {
        return Err("任务正在运行或排队中，稍后再修改配置".to_string());
    }
    let settings = task_settings_from_update_request(task.settings.output_dir.clone(), &settings);
    task_db::update_task_settings(&app, &task_id, settings)
}

#[tauri::command]
pub(crate) fn load_queue_settings(app: AppHandle) -> Result<QueueSettings, String> {
    task_db::load_queue_settings(&app)
}

#[tauri::command]
pub(crate) fn save_queue_settings(
    app: AppHandle,
    max_concurrency: usize,
) -> Result<QueueSettings, String> {
    let settings = task_db::save_queue_settings(&app, QueueSettings { max_concurrency })?;
    dispatch_queue(app);
    Ok(settings)
}

#[tauri::command]
pub(crate) fn create_video_task(
    app: AppHandle,
    request: CreateVideoTaskRequest,
) -> Result<TaskRecord, String> {
    let video_path = PathBuf::from(request.video_path.trim());
    if !video_path.exists() {
        return Err("视频文件不存在".to_string());
    }
    let id = Uuid::new_v4().to_string();
    let settings = task_settings_from_video_request(&request);
    let output_dir = settings.output_dir.clone().or_else(|| {
        video_path
            .parent()
            .map(|path| path_to_string(path.to_path_buf()))
    });
    let now = task_db::now_ts();
    let record = TaskRecord {
        id,
        source_type: "video".to_string(),
        video_path: Some(path_to_string(video_path.clone())),
        srt_path: None,
        file_name: display_file_name(&video_path),
        status: "created".to_string(),
        stage: "created".to_string(),
        message: "任务已创建".to_string(),
        progress: 0.0,
        settings,
        source_srt_path: None,
        translated_srt_path: None,
        source_file_name: None,
        translated_file_name: None,
        output_dir,
        segment_count: None,
        exported_source_srt: None,
        exported_translated_srt: None,
        exported_output_dir: None,
        error: None,
        created_at: now,
        updated_at: now,
    };
    task_db::insert_task(&app, &record)
}

#[tauri::command]
pub(crate) async fn create_srt_task(
    app: AppHandle,
    state: State<'_, AppState>,
    request: CreateSrtTaskRequest,
) -> Result<TaskRecord, String> {
    let srt_path = PathBuf::from(request.srt_path.trim());
    if !srt_path.exists() {
        return Err("SRT 文件不存在".to_string());
    }
    let segments = parse_srt_file(&srt_path).map_err(job_error_to_string)?;
    let source_srt = render_srt(&segments, None);
    let source_file_name = display_file_name(&srt_path);
    let id = Uuid::new_v4().to_string();
    let work_dir = task_db::task_work_dir(&app, &id)?;
    let source_srt_path = work_dir.join(&source_file_name);
    write_srt_text(&source_srt_path, &source_srt)
        .await
        .map_err(job_error_to_string)?;
    let settings = task_settings_from_srt_request(&request);
    let output_dir = settings.output_dir.clone().or_else(|| {
        srt_path
            .parent()
            .map(|path| path_to_string(path.to_path_buf()))
    });
    state.subtitle_results.lock().insert(
        id.clone(),
        StoredSubtitleResult {
            source_srt,
            translated_srt: None,
            segments: segments.clone(),
            output_dir: output_dir.clone().unwrap_or_else(|| ".".to_string()),
            source_file_name: source_file_name.clone(),
            translated_file_name: None,
        },
    );
    let now = task_db::now_ts();
    let record = TaskRecord {
        id,
        source_type: "srt".to_string(),
        video_path: None,
        srt_path: Some(path_to_string(srt_path)),
        file_name: source_file_name.clone(),
        status: "completed".to_string(),
        stage: "source-ready".to_string(),
        message: "已导入原文 SRT".to_string(),
        progress: 1.0,
        settings,
        source_srt_path: Some(path_to_string(source_srt_path)),
        translated_srt_path: None,
        source_file_name: Some(source_file_name),
        translated_file_name: None,
        output_dir,
        segment_count: Some(segments.len()),
        exported_source_srt: None,
        exported_translated_srt: None,
        exported_output_dir: None,
        error: None,
        created_at: now,
        updated_at: now,
    };
    task_db::insert_task(&app, &record)
}

#[tauri::command]
pub(crate) fn delete_task(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    cancel_queued_task(&app, &state, &task_id);
    if let Some(cancel) = state.tasks.lock().get(&task_id) {
        cancel.store(true, Ordering::SeqCst);
    }
    task_db::delete_task(&app, &task_id)
}

#[tauri::command]
pub(crate) fn run_task_operation(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    operation: String,
) -> Result<(), String> {
    enqueue_task_operation(app, state, task_id, operation)
}

#[tauri::command]
pub(crate) fn run_task_operations(
    app: AppHandle,
    state: State<'_, AppState>,
    task_ids: Vec<String>,
    operation: String,
) -> Result<(), String> {
    for task_id in task_ids {
        enqueue_task_operation(app.clone(), state.clone(), task_id, operation.clone())?;
    }
    dispatch_queue(app);
    Ok(())
}

#[tauri::command]
pub(crate) fn cancel_task(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<bool, String> {
    let removed_from_queue = cancel_queued_task(&app, &state, &task_id);
    if removed_from_queue {
        emit_job(
            &app,
            &task_id,
            "cancelled",
            JobStatus::Cancelled,
            "任务已取消",
            0.0,
            None,
            Some("任务已取消".to_string()),
        );
        return Ok(true);
    }

    if let Some(cancel) = state.tasks.lock().get(&task_id) {
        cancel.store(true, Ordering::SeqCst);
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command]
pub(crate) fn import_source_srt(
    app: AppHandle,
    request: ImportSourceSrtRequest,
) -> Result<String, String> {
    let srt_path = PathBuf::from(request.srt_path.trim());
    let segments = parse_srt_file(&srt_path).map_err(job_error_to_string)?;
    let source_srt = render_srt(&segments, None);
    let source_file_name = srt_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| "imported.source.srt".to_string());
    let output_dir = request
        .output_dir
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            srt_path
                .parent()
                .map(|path| path_to_string(path.to_path_buf()))
        })
        .unwrap_or_else(|| ".".to_string());
    let job_id = Uuid::new_v4().to_string();
    let segment_count = segments.len();

    app.state::<AppState>().subtitle_results.lock().insert(
        job_id.clone(),
        StoredSubtitleResult {
            source_srt,
            translated_srt: None,
            segments,
            output_dir: output_dir.clone(),
            source_file_name: source_file_name.clone(),
            translated_file_name: None,
        },
    );

    emit_job(
        &app,
        &job_id,
        "completed",
        JobStatus::Completed,
        "已导入原文 SRT",
        1.0,
        Some(JobOutputs {
            source_file_name,
            translated_file_name: None,
            output_dir,
            segment_count,
        }),
        None,
    );

    Ok(job_id)
}

#[tauri::command]
pub(crate) fn start_job(
    app: AppHandle,
    state: State<'_, AppState>,
    request: JobRequest,
) -> Result<String, String> {
    validate_start_request(&request)?;
    let job_id = Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    state.tasks.lock().insert(job_id.clone(), cancel.clone());
    emit_job(
        &app,
        &job_id,
        "queued",
        JobStatus::Running,
        "任务已创建",
        0.0,
        None,
        None,
    );
    let app_handle = app.clone();
    let spawned_job_id = job_id.clone();
    tauri::async_runtime::spawn(async move {
        let result = run_job(app_handle.clone(), spawned_job_id.clone(), request, cancel).await;
        match result {
            Ok(outputs) => emit_job(
                &app_handle,
                &spawned_job_id,
                "completed",
                JobStatus::Completed,
                "SRT 已生成",
                1.0,
                Some(outputs),
                None,
            ),
            Err(JobError::Cancelled) => emit_job(
                &app_handle,
                &spawned_job_id,
                "cancelled",
                JobStatus::Cancelled,
                "任务已取消",
                0.0,
                None,
                Some("任务已取消".to_string()),
            ),
            Err(JobError::Failed(message)) => emit_job(
                &app_handle,
                &spawned_job_id,
                "failed",
                JobStatus::Failed,
                "任务失败",
                0.0,
                None,
                Some(message),
            ),
        }
        app_handle
            .state::<AppState>()
            .tasks
            .lock()
            .remove(&spawned_job_id);
    });

    Ok(job_id)
}

#[tauri::command]
pub(crate) fn cancel_job(state: State<'_, AppState>, job_id: String) -> bool {
    if let Some(cancel) = state.tasks.lock().get(&job_id) {
        cancel.store(true, Ordering::SeqCst);
        true
    } else {
        false
    }
}

#[tauri::command]
pub(crate) fn subtitle_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<SubtitlePreview, String> {
    if let Some(preview) = state
        .subtitle_results
        .lock()
        .get(&job_id)
        .cloned()
        .map(|result| SubtitlePreview {
            source_srt: result.source_srt,
            translated_srt: result.translated_srt,
            source_file_name: result.source_file_name,
            translated_file_name: result.translated_file_name,
        })
    {
        return Ok(preview);
    }

    let task = task_db::require_task(&app, &job_id)?;
    let source_srt_path = task
        .source_srt_path
        .as_ref()
        .ok_or_else(|| "没有找到可预览的字幕结果".to_string())?;
    let source_srt = std::fs::read_to_string(source_srt_path).map_err(|error| error.to_string())?;
    let translated_srt = task
        .translated_srt_path
        .as_ref()
        .map(std::fs::read_to_string)
        .transpose()
        .map_err(|error| error.to_string())?;
    Ok(SubtitlePreview {
        source_srt,
        translated_srt,
        source_file_name: task
            .source_file_name
            .unwrap_or_else(|| "source.srt".to_string()),
        translated_file_name: task.translated_file_name,
    })
}

#[tauri::command]
pub(crate) fn translate_subtitles(
    app: AppHandle,
    state: State<'_, AppState>,
    request: TranslateSubtitlesRequest,
) -> Result<(), String> {
    validate_translate_request(&request)?;
    emit_job(
        &app,
        &request.job_id,
        "preparing-translation",
        JobStatus::Running,
        "正在读取翻译配置",
        0.54,
        None,
        None,
    );
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .tasks
        .lock()
        .insert(request.job_id.clone(), cancel.clone());
    let app_handle = app.clone();
    let job_id = request.job_id.clone();
    tauri::async_runtime::spawn(async move {
        let result = resolve_translation_inputs(&app_handle, &request).map_err(JobError::failed);
        let result = match result {
            Ok((stored, api_key)) => {
                run_translation(&app_handle, &request, stored, &api_key, cancel).await
            }
            Err(error) => Err(error),
        };
        app_handle.state::<AppState>().tasks.lock().remove(&job_id);
        match result {
            Ok((stored, outputs)) => {
                app_handle
                    .state::<AppState>()
                    .subtitle_results
                    .lock()
                    .insert(job_id.clone(), stored);
                emit_job(
                    &app_handle,
                    &job_id,
                    "completed",
                    JobStatus::Completed,
                    "译文字幕已生成",
                    1.0,
                    Some(outputs),
                    None,
                );
            }
            Err(JobError::Cancelled) => emit_job(
                &app_handle,
                &job_id,
                "cancelled",
                JobStatus::Cancelled,
                "翻译已取消",
                0.0,
                None,
                Some("翻译已取消".to_string()),
            ),
            Err(JobError::Failed(message)) => emit_job(
                &app_handle,
                &job_id,
                "failed",
                JobStatus::Failed,
                "翻译失败",
                0.0,
                None,
                Some(message),
            ),
        }
    });

    Ok(())
}

fn resolve_translation_inputs(
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

async fn run_translation(
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

async fn run_job(
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

#[tauri::command]
pub(crate) async fn export_subtitles(
    app: AppHandle,
    request: ExportSubtitlesRequest,
) -> Result<ExportedSubtitlePaths, String> {
    let result = app
        .state::<AppState>()
        .subtitle_results
        .lock()
        .get(&request.job_id)
        .cloned()
        .ok_or_else(|| "没有找到可导出的字幕结果".to_string())?;

    let output_dir = request
        .output_dir
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(result.output_dir);
    let output_dir_path = PathBuf::from(output_dir);
    tokio::fs::create_dir_all(&output_dir_path)
        .await
        .map_err(|error| format!("创建导出目录失败: {error}"))?;

    let source_path = output_dir_path.join(&result.source_file_name);
    write_srt_text(&source_path, &result.source_srt)
        .await
        .map_err(job_error_to_string)?;
    let translated_srt = if let (Some(translated_file_name), Some(translated_srt)) =
        (result.translated_file_name, result.translated_srt)
    {
        let translated_path = output_dir_path.join(translated_file_name);
        write_srt_text(&translated_path, &translated_srt)
            .await
            .map_err(job_error_to_string)?;
        Some(path_to_string(translated_path))
    } else {
        None
    };

    Ok(ExportedSubtitlePaths {
        source_srt: path_to_string(source_path),
        translated_srt,
        output_dir: path_to_string(output_dir_path),
    })
}

fn enqueue_task_operation(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    operation: String,
) -> Result<(), String> {
    let operation = normalize_operation(&operation)?;
    let task = task_db::require_task(&app, &task_id)?;
    validate_task_operation(&task, &operation)?;

    if state.running_operations.lock().contains(&task_id)
        || state
            .queued_operations
            .lock()
            .iter()
            .any(|queued| queued.task_id == task_id)
    {
        return Ok(());
    }

    task_db::set_queued(&app, &task_id, &operation)?;
    state
        .queued_operations
        .lock()
        .push_back(QueuedTaskOperation { task_id, operation });
    dispatch_queue(app);
    Ok(())
}

fn dispatch_queue(app: AppHandle) {
    loop {
        let max_concurrency = task_db::load_queue_settings(&app)
            .map(|settings| settings.max_concurrency)
            .unwrap_or(2)
            .clamp(1, 4);

        let next = {
            let state = app.state::<AppState>();
            let mut running = state.running_operations.lock();
            if running.len() >= max_concurrency {
                return;
            }

            let Some(operation) = state.queued_operations.lock().pop_front() else {
                return;
            };
            if running.contains(&operation.task_id) {
                continue;
            }

            let cancel = Arc::new(AtomicBool::new(false));
            running.insert(operation.task_id.clone());
            state
                .tasks
                .lock()
                .insert(operation.task_id.clone(), cancel.clone());
            (operation, cancel)
        };

        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            execute_task_operation(app_handle.clone(), next.0.clone(), next.1).await;
            let state = app_handle.state::<AppState>();
            state.tasks.lock().remove(&next.0.task_id);
            state.running_operations.lock().remove(&next.0.task_id);
            dispatch_queue(app_handle);
        });
    }
}

async fn execute_task_operation(
    app: AppHandle,
    queued: QueuedTaskOperation,
    cancel: Arc<AtomicBool>,
) {
    let result = match queued.operation.as_str() {
        "transcribe" => run_transcribe_task(app.clone(), &queued.task_id, cancel).await,
        "translate" => run_translate_task(app.clone(), &queued.task_id, cancel).await,
        "export" => run_export_task(app.clone(), &queued.task_id, cancel).await,
        _ => Err(JobError::failed("未知任务操作")),
    };

    match result {
        Ok(()) => {}
        Err(JobError::Cancelled) => emit_job(
            &app,
            &queued.task_id,
            "cancelled",
            JobStatus::Cancelled,
            operation_cancelled_message(&queued.operation),
            0.0,
            None,
            Some("任务已取消".to_string()),
        ),
        Err(JobError::Failed(message)) => emit_job(
            &app,
            &queued.task_id,
            "failed",
            JobStatus::Failed,
            operation_failed_message(&queued.operation),
            0.0,
            None,
            Some(message),
        ),
    }
}

async fn run_transcribe_task(
    app: AppHandle,
    task_id: &str,
    cancel: Arc<AtomicBool>,
) -> JobResult<()> {
    let task = task_db::require_task(&app, task_id).map_err(JobError::failed)?;
    let video_path = task
        .video_path
        .clone()
        .ok_or_else(|| JobError::failed("该任务不是视频任务"))?;
    let request = JobRequest {
        video_path,
        output_dir: task.settings.output_dir.clone(),
        target_language: task.settings.target_language.clone(),
        whisper_model_path: task.settings.whisper_model_path.clone(),
        whisper_language: task.settings.whisper_language.clone(),
        base_url: task.settings.base_url.clone(),
        model: task.settings.model.clone(),
        temperature: task.settings.temperature,
        translation_shard_size: Some(task.settings.translation_shard_size),
    };
    validate_start_request(&request).map_err(JobError::failed)?;

    emit_job(
        &app,
        task_id,
        "transcribe",
        JobStatus::Running,
        "转写已开始",
        0.0,
        None,
        None,
    );

    let outputs = run_job(app.clone(), task_id.to_string(), request, cancel).await?;
    let stored = app
        .state::<AppState>()
        .subtitle_results
        .lock()
        .get(task_id)
        .cloned()
        .ok_or_else(|| JobError::failed("转写结果未写入内存"))?;
    let work_dir = task_db::task_work_dir(&app, task_id).map_err(JobError::failed)?;
    let source_srt_path = work_dir.join(&stored.source_file_name);
    write_srt_text(&source_srt_path, &stored.source_srt).await?;
    task_db::set_subtitle_result(
        &app,
        task_id,
        path_to_string(source_srt_path),
        stored.source_file_name,
        stored.output_dir,
        stored.segments.len(),
    )
    .map_err(JobError::failed)?;
    emit_job(
        &app,
        task_id,
        "completed",
        JobStatus::Completed,
        "SRT 已生成",
        1.0,
        Some(outputs),
        None,
    );
    Ok(())
}

async fn run_translate_task(
    app: AppHandle,
    task_id: &str,
    cancel: Arc<AtomicBool>,
) -> JobResult<()> {
    let task = task_db::require_task(&app, task_id).map_err(JobError::failed)?;
    let source_srt_path = task
        .source_srt_path
        .clone()
        .ok_or_else(|| JobError::failed("请先完成转写或导入 SRT"))?;
    let source_srt = tokio::fs::read_to_string(&source_srt_path)
        .await
        .map_err(|error| JobError::failed(format!("读取原文字幕失败: {error}")))?;
    let segments = parse_srt_file(Path::new(&source_srt_path))?;
    let source_file_name = task
        .source_file_name
        .clone()
        .unwrap_or_else(|| display_file_name(Path::new(&source_srt_path)));
    let output_dir = task
        .output_dir
        .clone()
        .or(task.settings.output_dir.clone())
        .unwrap_or_else(|| ".".to_string());
    let request = TranslateSubtitlesRequest {
        job_id: task_id.to_string(),
        target_language: task.settings.target_language.clone(),
        base_url: task.settings.base_url.clone(),
        model: task.settings.model.clone(),
        temperature: task.settings.temperature,
        translation_shard_size: Some(task.settings.translation_shard_size),
        api_key: None,
    };
    validate_translate_request(&request).map_err(JobError::failed)?;
    let api_key = task_db::load_api_key(&app)
        .map_err(JobError::failed)?
        .ok_or_else(|| JobError::failed("请先保存 OpenAI 兼容接口的 API Key"))?;
    let stored = StoredSubtitleResult {
        source_srt,
        translated_srt: None,
        segments,
        output_dir,
        source_file_name,
        translated_file_name: None,
    };

    emit_job(
        &app,
        task_id,
        "preparing-translation",
        JobStatus::Running,
        "正在读取翻译配置",
        0.54,
        None,
        None,
    );
    let (stored, outputs) = run_translation(&app, &request, stored, &api_key, cancel).await?;
    let translated_srt = stored
        .translated_srt
        .clone()
        .ok_or_else(|| JobError::failed("翻译结果为空"))?;
    let translated_file_name = stored
        .translated_file_name
        .clone()
        .ok_or_else(|| JobError::failed("翻译文件名为空"))?;
    let translated_srt_path = task_db::task_work_dir(&app, task_id)
        .map_err(JobError::failed)?
        .join(&translated_file_name);
    write_srt_text(&translated_srt_path, &translated_srt).await?;
    app.state::<AppState>()
        .subtitle_results
        .lock()
        .insert(task_id.to_string(), stored);
    task_db::set_translation_result(
        &app,
        task_id,
        path_to_string(translated_srt_path),
        translated_file_name,
    )
    .map_err(JobError::failed)?;
    emit_job(
        &app,
        task_id,
        "completed",
        JobStatus::Completed,
        "译文字幕已生成",
        1.0,
        Some(outputs),
        None,
    );
    Ok(())
}

async fn run_export_task(app: AppHandle, task_id: &str, cancel: Arc<AtomicBool>) -> JobResult<()> {
    ensure_not_cancelled(&cancel)?;
    let task = task_db::require_task(&app, task_id).map_err(JobError::failed)?;
    let source_srt_path = task
        .source_srt_path
        .clone()
        .ok_or_else(|| JobError::failed("没有可导出的原文字幕"))?;
    let source_file_name = task
        .source_file_name
        .clone()
        .unwrap_or_else(|| display_file_name(Path::new(&source_srt_path)));
    let output_dir = task
        .output_dir
        .clone()
        .or(task.settings.output_dir.clone())
        .ok_or_else(|| JobError::failed("无法确定导出目录"))?;
    let output_dir_path = PathBuf::from(output_dir);

    emit_job(
        &app,
        task_id,
        "exporting",
        JobStatus::Running,
        "正在导出字幕",
        0.92,
        None,
        None,
    );

    tokio::fs::create_dir_all(&output_dir_path)
        .await
        .map_err(|error| JobError::failed(format!("创建导出目录失败: {error}")))?;
    let source_srt = tokio::fs::read_to_string(&source_srt_path)
        .await
        .map_err(|error| JobError::failed(format!("读取原文字幕失败: {error}")))?;
    let source_path = output_dir_path.join(source_file_name);
    write_srt_text(&source_path, &source_srt).await?;
    ensure_not_cancelled(&cancel)?;

    let translated_srt = if let (Some(translated_path), Some(translated_file_name)) = (
        task.translated_srt_path.clone(),
        task.translated_file_name.clone(),
    ) {
        let body = tokio::fs::read_to_string(&translated_path)
            .await
            .map_err(|error| JobError::failed(format!("读取译文字幕失败: {error}")))?;
        let target = output_dir_path.join(translated_file_name);
        write_srt_text(&target, &body).await?;
        Some(path_to_string(target))
    } else {
        None
    };

    let exported = ExportedSubtitlePaths {
        source_srt: path_to_string(source_path),
        translated_srt,
        output_dir: path_to_string(output_dir_path),
    };
    emit_job(
        &app,
        task_id,
        "completed",
        JobStatus::Completed,
        "字幕已导出",
        1.0,
        Some(JobOutputs {
            source_file_name: task
                .source_file_name
                .clone()
                .unwrap_or_else(|| "source.srt".to_string()),
            translated_file_name: task.translated_file_name.clone(),
            output_dir: exported.output_dir.clone(),
            segment_count: task.segment_count.unwrap_or(0),
        }),
        None,
    );
    task_db::set_exported(&app, task_id, &exported).map_err(JobError::failed)?;
    Ok(())
}

fn cancel_queued_task(app: &AppHandle, state: &State<'_, AppState>, task_id: &str) -> bool {
    let mut queue = state.queued_operations.lock();
    let original_len = queue.len();
    queue.retain(|queued| queued.task_id != task_id);
    let removed = queue.len() != original_len;
    if removed {
        let _ = task_db::set_interrupted(app, task_id);
    }
    removed
}

fn validate_task_operation(task: &TaskRecord, operation: &str) -> Result<(), String> {
    if matches!(task.status.as_str(), "running" | "queued") {
        return Err("任务正在运行或排队中".to_string());
    }
    match operation {
        "transcribe" => {
            if task.source_type != "video" {
                return Err("只有视频任务需要转写".to_string());
            }
            if task.settings.whisper_model_path.trim().is_empty() {
                return Err("请先在设置页选择 Whisper 模型".to_string());
            }
        }
        "translate" => {
            if task.source_srt_path.is_none() {
                return Err("请先完成转写或导入 SRT".to_string());
            }
        }
        "export" => {
            if task.source_srt_path.is_none() {
                return Err("没有可导出的字幕".to_string());
            }
        }
        _ => return Err("未知任务操作".to_string()),
    }
    Ok(())
}

fn normalize_operation(operation: &str) -> Result<String, String> {
    match operation.trim() {
        "transcribe" | "translate" | "export" => Ok(operation.trim().to_string()),
        _ => Err("未知任务操作".to_string()),
    }
}

fn task_settings_from_video_request(request: &CreateVideoTaskRequest) -> TaskSettingsSnapshot {
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

fn task_settings_from_srt_request(request: &CreateSrtTaskRequest) -> TaskSettingsSnapshot {
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

fn task_settings_from_update_request(
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

fn display_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path_to_string(path.to_path_buf()))
}

fn operation_failed_message(operation: &str) -> &'static str {
    match operation {
        "transcribe" => "转写失败",
        "translate" => "翻译失败",
        "export" => "导出失败",
        _ => "任务失败",
    }
}

fn operation_cancelled_message(operation: &str) -> &'static str {
    match operation {
        "transcribe" => "转写已取消",
        "translate" => "翻译已取消",
        "export" => "导出已取消",
        _ => "任务已取消",
    }
}

fn job_error_to_string(error: JobError) -> String {
    match error {
        JobError::Cancelled => "导出已取消".to_string(),
        JobError::Failed(message) => message,
    }
}
async fn extract_audio(
    app: &AppHandle,
    video_path: &Path,
    audio_path: &Path,
    cancel: Arc<AtomicBool>,
) -> JobResult<()> {
    let ffmpeg = locate_binary(app, "ffmpeg")
        .ok_or_else(|| JobError::failed(missing_binary_message("ffmpeg")))?;
    let mut command = Command::new(ffmpeg);
    command
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg(audio_path);
    run_process(command, cancel, "FFmpeg 抽音频失败").await
}
async fn transcribe_audio(
    app: &AppHandle,
    model_path: &Path,
    audio_path: &Path,
    output_base: &Path,
    language: &str,
    cancel: Arc<AtomicBool>,
) -> JobResult<()> {
    if !model_path.exists() {
        return Err(JobError::failed("Whisper 模型文件不存在"));
    }
    let whisper = locate_binary(app, "whisper-cli")
        .ok_or_else(|| JobError::failed(missing_binary_message("whisper-cli")))?;
    let threads = std::thread::available_parallelism()
        .map(|count| count.get().saturating_sub(1).clamp(4, 12).to_string())
        .unwrap_or_else(|_| "8".to_string());
    let mut command = Command::new(whisper);
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    command.env("WHISPER_ARG_DEVICE", "0");
    command
        .arg("-m")
        .arg(model_path)
        .arg("-f")
        .arg(audio_path)
        .arg("-l")
        .arg(normalize_language(language))
        .arg("-t")
        .arg(threads)
        .arg("-oj")
        .arg("-of")
        .arg(output_base);
    run_process(command, cancel, "whisper.cpp 转写失败").await
}

fn missing_binary_message(name: &str) -> String {
    let binary_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let resource_dir = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "src-tauri/resources/bin/macos-arm64"
    } else {
        "src-tauri/resources/bin"
    };
    format!("未找到 {binary_name}，请放入 {resource_dir} 或加入 PATH")
}

async fn run_process(
    mut command: Command,
    cancel: Arc<AtomicBool>,
    failure_context: &str,
) -> JobResult<()> {
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| JobError::failed(format!("{failure_context}: {error}")))?;
    let mut stderr = child.stderr.take();
    let stderr_reader = tauri::async_runtime::spawn(async move {
        let mut buffer = Vec::new();
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.read_to_end(&mut buffer).await;
        }
        String::from_utf8_lossy(&buffer).to_string()
    });
    loop {
        ensure_not_cancelled(&cancel)?;
        match child
            .try_wait()
            .map_err(|error| JobError::failed(format!("{failure_context}: {error}")))?
        {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                let stderr = stderr_reader.await.unwrap_or_default();
                let detail = process_error_detail(&stderr);
                return Err(JobError::failed(format!(
                    "{failure_context}，退出码: {}{}",
                    status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    detail
                )));
            }
            None => sleep(Duration::from_millis(200)).await,
        }
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            return Err(JobError::Cancelled);
        }
    }
}

fn process_error_detail(stderr: &str) -> String {
    let lines = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    let detail = lines
        .iter()
        .rev()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    format!("\n{detail}")
}

fn translated_file_name(source_file_name: &str, target_language: &str) -> String {
    let stem = source_file_name
        .strip_suffix(".source.srt")
        .or_else(|| source_file_name.strip_suffix(".srt"))
        .unwrap_or(source_file_name);
    format!("{}.{}.srt", stem, sanitize_file_part(target_language))
}

fn validate_start_request(request: &JobRequest) -> Result<(), String> {
    let video_path = PathBuf::from(&request.video_path);
    if !video_path.exists() {
        return Err("视频文件不存在".to_string());
    }
    if request.whisper_model_path.trim().is_empty() {
        return Err("请选择 Whisper 模型文件".to_string());
    }
    Ok(())
}

fn validate_translate_request(request: &TranslateSubtitlesRequest) -> Result<(), String> {
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
