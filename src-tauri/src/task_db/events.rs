use rusqlite::params;
use tauri::{AppHandle, Emitter};

use crate::job_events::{ExportedSubtitlePaths, JobEvent, JobStatus};

use super::{
    get_task, require_task,
    schema::connection,
    TaskRecord,
};

pub(crate) fn set_queued(
    app: &AppHandle,
    task_id: &str,
    operation: &str,
) -> Result<TaskRecord, String> {
    update_status(
        app,
        task_id,
        "queued",
        operation,
        operation_message(operation, "等待执行"),
        None,
        None,
    )
}

pub(crate) fn set_interrupted(app: &AppHandle, task_id: &str) -> Result<TaskRecord, String> {
    update_status(
        app,
        task_id,
        "interrupted",
        "interrupted",
        "上次运行被中断，可重试".to_string(),
        None,
        Some("应用重启或任务被中断".to_string()),
    )
}

pub(crate) fn set_subtitle_result(
    app: &AppHandle,
    task_id: &str,
    source_srt_path: String,
    source_file_name: String,
    output_dir: String,
    segment_count: usize,
) -> Result<TaskRecord, String> {
    let conn = connection(app)?;
    let now = super::now_ts();
    conn.execute(
        "UPDATE tasks SET
            source_srt_path = ?1,
            source_file_name = ?2,
            output_dir = ?3,
            segment_count = ?4,
            updated_at = ?5
        WHERE id = ?6",
        params![
            source_srt_path,
            source_file_name,
            output_dir,
            segment_count as i64,
            now,
            task_id,
        ],
    )
    .map_err(|error| error.to_string())?;
    emit_task(app, task_id);
    require_task(app, task_id)
}

pub(crate) fn set_translation_result(
    app: &AppHandle,
    task_id: &str,
    translated_srt_path: String,
    translated_file_name: String,
) -> Result<TaskRecord, String> {
    let conn = connection(app)?;
    let now = super::now_ts();
    conn.execute(
        "UPDATE tasks SET
            translated_srt_path = ?1,
            translated_file_name = ?2,
            updated_at = ?3
        WHERE id = ?4",
        params![translated_srt_path, translated_file_name, now, task_id],
    )
    .map_err(|error| error.to_string())?;
    emit_task(app, task_id);
    require_task(app, task_id)
}

pub(crate) fn set_exported(
    app: &AppHandle,
    task_id: &str,
    exported: &ExportedSubtitlePaths,
) -> Result<TaskRecord, String> {
    let conn = connection(app)?;
    let now = super::now_ts();
    conn.execute(
        "UPDATE tasks SET
            status = 'exported',
            stage = 'exported',
            message = '字幕已导出',
            progress = 1.0,
            exported_source_srt = ?1,
            exported_translated_srt = ?2,
            exported_output_dir = ?3,
            error = NULL,
            updated_at = ?4
        WHERE id = ?5",
        params![
            exported.source_srt,
            exported.translated_srt,
            exported.output_dir,
            now,
            task_id,
        ],
    )
    .map_err(|error| error.to_string())?;
    append_log(app, task_id, "exported · 字幕已导出")?;
    emit_task(app, task_id);
    require_task(app, task_id)
}

pub(crate) fn record_job_event(app: &AppHandle, event: &JobEvent) -> Result<(), String> {
    if get_task(app, &event.job_id)?.is_none() {
        return Ok(());
    }
    let conn = connection(app)?;
    let now = super::now_ts();
    let status = job_status_name(&event.status);
    let outputs = event.outputs.as_ref();
    conn.execute(
        "UPDATE tasks SET
            status = ?1,
            stage = ?2,
            message = ?3,
            progress = ?4,
            source_file_name = COALESCE(?5, source_file_name),
            translated_file_name = COALESCE(?6, translated_file_name),
            output_dir = COALESCE(?7, output_dir),
            segment_count = COALESCE(?8, segment_count),
            error = ?9,
            updated_at = ?10
        WHERE id = ?11",
        params![
            status,
            event.stage,
            event.message,
            event.progress,
            outputs.map(|value| value.source_file_name.clone()),
            outputs.and_then(|value| value.translated_file_name.clone()),
            outputs.map(|value| value.output_dir.clone()),
            outputs.map(|value| value.segment_count as i64),
            event.error,
            now,
            event.job_id,
        ],
    )
    .map_err(|error| error.to_string())?;
    append_log(
        app,
        &event.job_id,
        &format!("{} · {}", event.stage, event.message),
    )?;
    if let Some(error) = event.error.as_ref() {
        append_log(app, &event.job_id, &format!("error · {error}"))?;
    }
    emit_task(app, &event.job_id);
    Ok(())
}

pub(super) fn append_log(app: &AppHandle, task_id: &str, line: &str) -> Result<(), String> {
    let conn = connection(app)?;
    conn.execute(
        "INSERT INTO task_logs(task_id, created_at, line) VALUES(?1, ?2, ?3)",
        params![task_id, super::now_ts(), line],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn mark_interrupted_tasks(app: &AppHandle) -> Result<(), String> {
    let conn = connection(app)?;
    let mut statement = conn
        .prepare("SELECT id FROM tasks WHERE status IN ('queued', 'running')")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    let task_ids = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    for task_id in task_ids {
        let _ = set_interrupted(app, &task_id);
    }
    Ok(())
}

pub(super) fn emit_task(app: &AppHandle, task_id: &str) {
    if let Ok(Some(task)) = get_task(app, task_id) {
        let _ = app.emit("task-updated", task);
    }
}

fn update_status(
    app: &AppHandle,
    task_id: &str,
    status: &str,
    stage: &str,
    message: String,
    progress: Option<f32>,
    error: Option<String>,
) -> Result<TaskRecord, String> {
    let conn = connection(app)?;
    let now = super::now_ts();
    conn.execute(
        "UPDATE tasks SET
            status = ?1,
            stage = ?2,
            message = ?3,
            progress = COALESCE(?4, progress),
            error = ?5,
            updated_at = ?6
        WHERE id = ?7",
        params![status, stage, message, progress, error, now, task_id],
    )
    .map_err(|error| error.to_string())?;
    append_log(app, task_id, &format!("{stage} · {message}"))?;
    emit_task(app, task_id);
    require_task(app, task_id)
}

fn job_status_name(status: &JobStatus) -> &'static str {
    match status {
        JobStatus::Running => "running",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
        JobStatus::Cancelled => "cancelled",
    }
}

fn operation_message(operation: &str, suffix: &str) -> String {
    let label = match operation {
        "transcribe" => "转写",
        "translate" => "翻译",
        "export" => "导出",
        _ => "任务",
    };
    format!("{label}{suffix}")
}
