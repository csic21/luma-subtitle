use std::sync::{atomic::AtomicBool, Arc};

use tauri::{AppHandle, Manager, State};

use crate::{
    state::{AppState, QueuedTaskOperation},
    task_db::{self, TaskRecord},
};

use super::task_runner::execute_task_operation;

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
            let completed =
                execute_task_operation(app_handle.clone(), next.0.clone(), next.1).await;
            let state = app_handle.state::<AppState>();
            state.tasks.lock().remove(&next.0.task_id);
            state.running_operations.lock().remove(&next.0.task_id);
            if completed {
                enqueue_next_link(&app_handle, &next.0);
            }
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

fn validate_task_operation(task: &TaskRecord, operation: &str) -> Result<(), String> {
    if matches!(task.status.as_str(), "running" | "queued") {
        return Err("任务正在运行或排队中".to_string());
    }
    match operation {
        "transcribe" => {
            if !matches!(task.source_type.as_str(), "video" | "audio") {
                return Err("只有视频或音频任务需要转写".to_string());
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

fn enqueue_next_link(app: &AppHandle, completed: &QueuedTaskOperation) {
    let Ok(settings) = task_db::load_queue_settings(app) else {
        return;
    };
    if !settings.auto_start_next {
        return;
    }
    let Some(operation) = next_operation(&completed.operation) else {
        return;
    };
    let Ok(task) = task_db::require_task(app, &completed.task_id) else {
        return;
    };
    if validate_task_operation(&task, operation).is_err() {
        return;
    }

    let state = app.state::<AppState>();
    let is_running = state.running_operations.lock().contains(&completed.task_id);
    let is_queued = state
        .queued_operations
        .lock()
        .iter()
        .any(|queued| queued.task_id == completed.task_id);
    if is_running || is_queued {
        return;
    }

    if task_db::set_queued(app, &completed.task_id, operation).is_err() {
        return;
    }
    state
        .queued_operations
        .lock()
        .push_back(QueuedTaskOperation {
            task_id: completed.task_id.clone(),
            operation: operation.to_string(),
        });
}

fn next_operation(operation: &str) -> Option<&'static str> {
    match operation {
        "transcribe" => Some("translate"),
        "translate" => Some("export"),
        _ => None,
    }
}
