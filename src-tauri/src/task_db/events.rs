use rusqlite::{params, Connection, Transaction};
use tauri::{AppHandle, Emitter};

use crate::job_events::{ExportedSubtitlePaths, JobEvent, JobStatus};

use super::{get_task, require_task, schema::connection, TaskRecord};

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
    let mut conn = connection(app)?;
    if record_job_event_in_transaction(&mut conn, event)? {
        emit_task(app, &event.job_id);
    }
    Ok(())
}

fn record_job_event_in_transaction(
    conn: &mut Connection,
    event: &JobEvent,
) -> Result<bool, String> {
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    let now = super::now_ts();
    let status = job_status_name(&event.status);
    let outputs = event.outputs.as_ref();
    let error = event.error.as_deref();
    let updated = tx
        .execute(
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
                event.stage.as_str(),
                event.message.as_str(),
                event.progress,
                outputs.map(|value| value.source_file_name.as_str()),
                outputs.and_then(|value| value.translated_file_name.as_deref()),
                outputs.map(|value| value.output_dir.as_str()),
                outputs.map(|value| value.segment_count as i64),
                error,
                now,
                event.job_id.as_str(),
            ],
        )
        .map_err(|error| error.to_string())?;
    if updated == 0 {
        return Ok(false);
    }
    append_log_in_transaction(
        &tx,
        &event.job_id,
        &format!("{} · {}", event.stage, event.message),
        now,
    )?;
    if let Some(error) = error {
        append_log_in_transaction(&tx, &event.job_id, &format!("error · {error}"), now)?;
    }
    tx.commit().map_err(|error| error.to_string())?;
    Ok(true)
}

fn append_log_in_transaction(
    tx: &Transaction<'_>,
    task_id: &str,
    line: &str,
    created_at: i64,
) -> Result<(), String> {
    tx.execute(
        "INSERT INTO task_logs(task_id, created_at, line) VALUES(?1, ?2, ?3)",
        params![task_id, created_at, line],
    )
    .map_err(|error| error.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job_events::{JobEvent, JobStatus};
    use crate::task_db::schema::migrate;

    #[test]
    fn job_event_rolls_back_task_update_when_log_insert_fails() {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        migrate(&conn).expect("migration should run");
        conn.execute(
            "INSERT INTO tasks (
                id, source_type, file_name, status, stage, message, progress, settings_json, created_at, updated_at
            ) VALUES ('task-1', 'video', 'clip.mp4', 'created', 'created', 'pending', 0.0, '{}', 1, 1)",
            [],
        )
        .expect("task should insert");
        conn.execute_batch(
            "CREATE TRIGGER reject_task_logs BEFORE INSERT ON task_logs
             BEGIN SELECT RAISE(ABORT, 'log write rejected'); END;",
        )
        .expect("failure trigger should create");

        let event = JobEvent {
            job_id: "task-1".to_string(),
            stage: "transcribing".to_string(),
            status: JobStatus::Running,
            message: "working".to_string(),
            progress: 0.5,
            outputs: None,
            error: None,
        };

        let error = record_job_event_in_transaction(&mut conn, &event)
            .expect_err("failed log write should abort the event transaction");
        assert!(error.contains("log write rejected"));

        let (status, stage, message, progress): (String, String, String, f64) = conn
            .query_row(
                "SELECT status, stage, message, progress FROM tasks WHERE id = 'task-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("task should still exist");
        assert_eq!(
            (status, stage, message, progress),
            (
                "created".to_string(),
                "created".to_string(),
                "pending".to_string(),
                0.0
            )
        );
        let log_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM task_logs WHERE task_id = 'task-1'",
                [],
                |row| row.get(0),
            )
            .expect("log count should be readable");
        assert_eq!(log_count, 0);
    }
}
