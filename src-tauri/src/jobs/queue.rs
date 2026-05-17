use std::{
    path::Path,
    sync::{
        atomic::AtomicBool,
        Arc,
    },
};

use tauri::{AppHandle, Manager, State};

use crate::{
    job_events::{emit_job, ExportedSubtitlePaths, JobOutputs, JobStatus, StoredSubtitleResult},
    paths::path_to_string,
    state::{ensure_not_cancelled, AppState, JobError, JobResult, QueuedTaskOperation},
    subtitles::{parse_srt_file, write_srt_text},
    task_db::{self, TaskRecord},
};

use super::{
    helpers::{
        display_file_name, operation_cancelled_message, operation_failed_message,
        validate_start_request, validate_translate_request,
    },
    run_job, run_translation, JobRequest, TranslateSubtitlesRequest,
};

pub(super) fn enqueue_task_operation(
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

pub(super) fn dispatch_queue(app: AppHandle) {
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

pub(super) fn cancel_queued_task(
    app: &AppHandle,
    state: &State<'_, AppState>,
    task_id: &str,
) -> bool {
    let mut queue = state.queued_operations.lock();
    let original_len = queue.len();
    queue.retain(|queued| queued.task_id != task_id);
    let removed = queue.len() != original_len;
    if removed {
        let _ = task_db::set_interrupted(app, task_id);
    }
    removed
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
    let output_dir_path = Path::new(&output_dir);

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

    tokio::fs::create_dir_all(output_dir_path)
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
        output_dir: path_to_string(output_dir_path.to_path_buf()),
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
