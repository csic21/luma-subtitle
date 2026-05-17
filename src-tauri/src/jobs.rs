use std::{
    path::PathBuf,
    sync::atomic::Ordering,
};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::{
    job_events::{emit_job, JobStatus, StoredSubtitleResult},
    paths::path_to_string,
    settings,
    state::AppState,
    subtitles::{parse_srt_file, render_srt, write_srt_text},
    task_db::{self, QueueSettings, TaskRecord},
};

pub(crate) mod adhoc;
mod helpers;
mod process;
mod queue;
mod requests;
mod single_job;
mod task_runner;

pub(crate) use requests::{
    CreateSrtTaskRequest, CreateVideoTaskRequest, JobRequest, TranslateSubtitlesRequest,
    UpdateTaskSettingsRequest,
};
use helpers::{
    display_file_name, job_error_to_string, task_settings_from_srt_request,
    task_settings_from_update_request, task_settings_from_video_request,
};
use queue::{cancel_queued_task, dispatch_queue, enqueue_task_operation};

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
