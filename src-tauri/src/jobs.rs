use std::{path::PathBuf, sync::atomic::Ordering};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::{
    job_events::{publish_job_event, JobEventDraft, StoredSubtitleResult},
    paths::path_to_string,
    settings,
    state::AppState,
    subtitles::{parse_srt_file, render_srt, write_srt_text},
    task_db::{self, QueueSettings, TaskRecord},
};

mod helpers;
mod process;
mod queue;
mod requests;
mod single_job;
mod task_runner;

use helpers::{
    display_file_name, job_error_to_string, task_settings_from_audio_request,
    task_settings_from_srt_request, task_settings_from_update_request,
    task_settings_from_video_request,
};
use queue::{cancel_queued_task, dispatch_queue, enqueue_task_operation};
pub(crate) use requests::{
    CreateAudioTaskRequest, CreateSrtTaskRequest, CreateVideoTaskRequest, JobRequest,
    SubtitlePreview, TranslateSubtitlesRequest, UpdateTaskSettingsRequest,
};

#[tauri::command]
pub(crate) async fn list_tasks(app: AppHandle) -> Result<Vec<TaskRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || task_db::list_tasks(&app))
        .await
        .map_err(|error| format!("读取任务列表失败: {error}"))?
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
pub(crate) fn subtitle_preview(app: AppHandle, job_id: String) -> Result<SubtitlePreview, String> {
    let task = task_db::require_task(&app, &job_id)?;
    let source_srt_path = task
        .source_srt_path
        .as_deref()
        .ok_or_else(|| "没有找到可预览的字幕结果".to_string())?;
    let source_srt = std::fs::read_to_string(source_srt_path).map_err(|error| error.to_string())?;
    let translated_srt = task
        .translated_srt_path
        .as_deref()
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
pub(crate) async fn load_queue_settings(app: AppHandle) -> Result<QueueSettings, String> {
    tauri::async_runtime::spawn_blocking(move || task_db::load_queue_settings(&app))
        .await
        .map_err(|error| format!("读取队列设置失败: {error}"))?
}

#[tauri::command]
pub(crate) fn save_queue_settings(
    app: AppHandle,
    max_concurrency: usize,
    auto_start_next: bool,
) -> Result<QueueSettings, String> {
    let settings = task_db::save_queue_settings(
        &app,
        QueueSettings {
            max_concurrency,
            auto_start_next,
        },
    )?;
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
    let settings = task_settings_from_video_request(&request);
    create_media_task(&app, "video", video_path, settings)
}

#[tauri::command]
pub(crate) fn create_audio_task(
    app: AppHandle,
    request: CreateAudioTaskRequest,
) -> Result<TaskRecord, String> {
    let audio_path = PathBuf::from(request.audio_path.trim());
    if !audio_path.exists() {
        return Err("音频文件不存在".to_string());
    }
    let settings = task_settings_from_audio_request(&request);
    create_media_task(&app, "audio", audio_path, settings)
}

fn create_media_task(
    app: &AppHandle,
    source_type: &str,
    media_path: PathBuf,
    settings: task_db::TaskSettingsSnapshot,
) -> Result<TaskRecord, String> {
    let id = Uuid::new_v4().to_string();
    let output_dir = settings.output_dir.clone().or_else(|| {
        media_path
            .parent()
            .map(|path| path_to_string(path.to_path_buf()))
    });
    let now = task_db::now_ts();
    let record = TaskRecord {
        id,
        source_type: source_type.to_string(),
        video_path: (source_type == "video").then(|| path_to_string(media_path.clone())),
        audio_path: (source_type == "audio").then(|| path_to_string(media_path.clone())),
        srt_path: None,
        file_name: display_file_name(&media_path),
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
    task_db::insert_task(app, &record)
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
        audio_path: None,
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
pub(crate) async fn delete_task(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    cancel_queued_task(&app, &state, &task_id);
    if let Some(cancel) = state.tasks.lock().get(&task_id) {
        cancel.store(true, Ordering::SeqCst);
        return Err("任务正在运行，已请求取消，请停止后再删除".to_string());
    }

    let delete_app = app.clone();
    let delete_task_id = task_id.clone();
    let deleted_task = tauri::async_runtime::spawn_blocking(move || {
        task_db::delete_task(&delete_app, &delete_task_id)
    })
    .await
    .map_err(|error| format!("删除任务记录失败: {error}"))??;

    state.subtitle_results.lock().remove(&task_id);

    let cleanup_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = task_db::cleanup_task_artifacts(&cleanup_app, &deleted_task);
    });

    Ok(())
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
        publish_job_event(
            &app,
            JobEventDraft::cancelled(&task_id, "cancelled", "任务已取消", "任务已取消"),
        );
        return Ok(true);
    }

    if let Some(cancel) = state.tasks.lock().get(&task_id) {
        cancel.store(true, Ordering::SeqCst);
        return Ok(true);
    }
    Ok(false)
}
