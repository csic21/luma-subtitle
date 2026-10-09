use crate::state::{ensure_not_cancelled, JobError, JobResult};
use rusqlite::{params, Connection, Transaction};
use std::sync::{atomic::AtomicBool, Arc};
use tauri::{AppHandle, Emitter};

use crate::job_events::{ExportedSubtitlePaths, JobEvent, JobStatus};

use super::{
    get_task, require_task,
    schema::{connection, task_from_row},
    TaskRecord,
};

const SOURCE_RESULT_SQL: &str = "UPDATE tasks SET
            source_srt_path = ?1,
            source_file_name = ?2,
            output_dir = ?3,
            segment_count = ?4,
            translated_srt_path = NULL,
            translated_file_name = NULL,
            translation_completed_count = NULL,
            exported_source_srt = NULL,
            exported_translated_srt = NULL,
            exported_output_dir = NULL,
            result_revision = result_revision + 1,
            updated_at = ?5
        WHERE id = ?6";

const TRANSLATION_RESULT_SQL: &str = "UPDATE tasks SET
            translated_srt_path = ?1,
            translated_file_name = ?2,
            translation_completed_count = NULL,
            exported_source_srt = NULL,
            exported_translated_srt = NULL,
            exported_output_dir = NULL,
            result_revision = result_revision + 1,
            updated_at = ?3
        WHERE id = ?4";

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
    expected: &TaskRecord,
    cancel: &Arc<AtomicBool>,
    source_srt_path: String,
    source_file_name: String,
    output_dir: String,
    segment_count: usize,
) -> JobResult<TaskRecord> {
    let mut conn = connection(app).map_err(JobError::failed)?;
    let saved = apply_result_in_transaction(
        &mut conn,
        expected,
        cancel,
        SOURCE_RESULT_SQL,
        params![
            source_srt_path,
            source_file_name,
            output_dir,
            segment_count as i64,
            super::now_ts(),
            expected.id
        ],
    )?;
    emit_task(app, &expected.id);
    Ok(saved)
}

pub(crate) fn set_translation_result(
    app: &AppHandle,
    expected: &TaskRecord,
    cancel: &Arc<AtomicBool>,
    translated_srt_path: String,
    translated_file_name: String,
) -> JobResult<TaskRecord> {
    let mut conn = connection(app).map_err(JobError::failed)?;
    let saved = apply_result_in_transaction(
        &mut conn,
        expected,
        cancel,
        TRANSLATION_RESULT_SQL,
        params![
            translated_srt_path,
            translated_file_name,
            super::now_ts(),
            expected.id
        ],
    )?;
    emit_task(app, &expected.id);
    Ok(saved)
}

fn apply_result_in_transaction<P: rusqlite::Params>(
    conn: &mut Connection,
    expected: &TaskRecord,
    cancel: &Arc<AtomicBool>,
    sql: &str,
    params: P,
) -> JobResult<TaskRecord> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| JobError::failed(error.to_string()))?;
    ensure_result_owner(&tx, expected, cancel)?;
    tx.execute(sql, params)
        .map_err(|error| JobError::failed(error.to_string()))?;
    let saved = tx
        .query_row(
            "SELECT * FROM tasks WHERE id = ?1",
            params![expected.id],
            task_from_row,
        )
        .map_err(|error| JobError::failed(error.to_string()))?;
    ensure_not_cancelled(cancel)?;
    tx.commit()
        .map_err(|error| JobError::failed(error.to_string()))?;
    Ok(saved)
}

fn ensure_result_owner(
    tx: &Transaction<'_>,
    expected: &TaskRecord,
    cancel: &Arc<AtomicBool>,
) -> JobResult<()> {
    ensure_not_cancelled(cancel)?;
    let current = tx
        .query_row(
            "SELECT * FROM tasks WHERE id = ?1",
            params![expected.id],
            task_from_row,
        )
        .map_err(|error| JobError::failed(error.to_string()))?;
    if current.run_generation != expected.run_generation
        || current.result_revision != expected.result_revision
        || current.source_srt_path != expected.source_srt_path
        || current.translated_srt_path != expected.translated_srt_path
        || current.status != "running"
    {
        return Err(JobError::Cancelled);
    }
    Ok(())
}

pub(crate) fn set_translation_progress(
    app: &AppHandle,
    task_id: &str,
    completed_count: usize,
) -> Result<TaskRecord, String> {
    let conn = connection(app)?;
    let now = super::now_ts();
    conn.execute(
        "UPDATE tasks SET
            translation_completed_count = ?1,
            updated_at = ?2
        WHERE id = ?3",
        params![completed_count as i64, now, task_id],
    )
    .map_err(|error| error.to_string())?;
    emit_task(app, task_id);
    require_task(app, task_id)
}

pub(crate) fn clear_translation_progress(
    app: &AppHandle,
    task_id: &str,
) -> Result<TaskRecord, String> {
    let conn = connection(app)?;
    let now = super::now_ts();
    conn.execute(
        "UPDATE tasks SET
            translation_completed_count = NULL,
            updated_at = ?1
        WHERE id = ?2",
        params![now, task_id],
    )
    .map_err(|error| error.to_string())?;
    emit_task(app, task_id);
    require_task(app, task_id)
}

pub(crate) fn set_exported(
    app: &AppHandle,
    expected: &TaskRecord,
    cancel: &Arc<AtomicBool>,
    publish: impl FnOnce() -> JobResult<ExportedSubtitlePaths>,
) -> JobResult<TaskRecord> {
    let mut conn = connection(app).map_err(JobError::failed)?;
    let saved = commit_export_in_transaction(&mut conn, expected, cancel, publish)?;
    emit_task(app, &expected.id);
    Ok(saved)
}

fn commit_export_in_transaction(
    conn: &mut Connection,
    expected: &TaskRecord,
    cancel: &Arc<AtomicBool>,
    publish: impl FnOnce() -> JobResult<ExportedSubtitlePaths>,
) -> JobResult<TaskRecord> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| JobError::failed(error.to_string()))?;
    ensure_result_owner(&tx, expected, cancel)?;
    // The caller holds task_mutations through this entire non-async publication.
    // It owns rollback guards for any newly created export files.
    let exported = publish()?;
    ensure_not_cancelled(cancel)?;
    let now = super::now_ts();
    tx.execute(
        "UPDATE tasks SET status = 'exported', stage = 'exported', message = '字幕已导出',
         progress = 1.0, exported_source_srt = ?1, exported_translated_srt = ?2,
         exported_output_dir = ?3, error = NULL, updated_at = ?4 WHERE id = ?5",
        params![
            exported.source_srt,
            exported.translated_srt,
            exported.output_dir,
            now,
            expected.id
        ],
    )
    .map_err(|error| JobError::failed(error.to_string()))?;
    append_log_in_transaction(&tx, &expected.id, "exported · 字幕已导出", now)
        .map_err(JobError::failed)?;
    let saved = tx
        .query_row(
            "SELECT * FROM tasks WHERE id = ?1",
            params![expected.id],
            task_from_row,
        )
        .map_err(|error| JobError::failed(error.to_string()))?;
    ensure_not_cancelled(cancel)?;
    tx.commit()
        .map_err(|error| JobError::failed(error.to_string()))?;
    Ok(saved)
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
            run_generation = run_generation + CASE WHEN ?1 = 'queued' THEN 1 ELSE 0 END,
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
        "resume_translate" => "续翻",
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

    fn running_fixture() -> (Connection, TaskRecord) {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let settings = r#"{"output_dir":null,"target_language":"简体中文","whisper_model_path":"","whisper_language":"auto","base_url":"","model":"test","temperature":0.2}"#;
        conn.execute(
            "INSERT INTO tasks(id, source_type, file_name, status, stage, message,
            progress, settings_json, source_srt_path, translated_srt_path, run_generation,
            created_at, updated_at) VALUES('task', 'video', 'clip', 'running', 'translate', '',
            0.5, ?1, 'source', 'previous-translation', 1, 1, 1)",
            params![settings],
        )
        .unwrap();
        let task = conn
            .query_row("SELECT * FROM tasks", [], task_from_row)
            .unwrap();
        (conn, task)
    }

    #[test]
    fn cancelled_or_obsolete_export_never_publishes_files() {
        for obsolete in [false, true] {
            let (mut conn, attempt) = running_fixture();
            if obsolete {
                conn.execute("UPDATE tasks SET run_generation = 2", [])
                    .unwrap();
            }
            let result = commit_export_in_transaction(
                &mut conn,
                &attempt,
                &Arc::new(AtomicBool::new(!obsolete)),
                || panic!("rejected export must not publish"),
            );
            assert!(matches!(result, Err(JobError::Cancelled)));
        }
    }

    #[test]
    fn export_database_failure_rolls_back_new_files_and_preserves_prior_export() {
        let (mut conn, attempt) = running_fixture();
        let dir =
            std::env::temp_dir().join(format!("luma-export-transaction-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let previous = dir.join("clip.srt");
        std::fs::write(&previous, "previous export").unwrap();
        conn.execute(
            "UPDATE tasks SET exported_source_srt = ?1",
            params![previous.to_str()],
        )
        .unwrap();
        conn.execute_batch("CREATE TRIGGER reject_export_log BEFORE INSERT ON task_logs BEGIN SELECT RAISE(ABORT, 'no log'); END;").unwrap();
        {
            let mut files = crate::jobs::ExportFiles::default();
            let staged = files.stage_path(&dir);
            std::fs::write(&staged, "new export").unwrap();
            let result = commit_export_in_transaction(
                &mut conn,
                &attempt,
                &Arc::new(AtomicBool::new(false)),
                || {
                    let published = files.publish(&dir, "clip.srt", "task", &staged)?;
                    Ok(ExportedSubtitlePaths {
                        source_srt: published.to_string_lossy().into_owned(),
                        translated_srt: None,
                        output_dir: dir.to_string_lossy().into_owned(),
                    })
                },
            );
            assert!(matches!(result, Err(JobError::Failed(_))));
        }
        assert_eq!(
            std::fs::read_to_string(&previous).unwrap(),
            "previous export"
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let saved: String = conn
            .query_row("SELECT exported_source_srt FROM tasks", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(saved, previous.to_string_lossy());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cancelled_after_export_publication_rolls_back_metadata_and_new_files() {
        let (mut conn, attempt) = running_fixture();
        let dir = std::env::temp_dir().join(format!("luma-export-cancel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut files = crate::jobs::ExportFiles::default();
            let staged = files.stage_path(&dir);
            std::fs::write(&staged, "new export").unwrap();
            let result = commit_export_in_transaction(&mut conn, &attempt, &cancel, || {
                let published = files.publish(&dir, "clip.srt", "task", &staged)?;
                cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(ExportedSubtitlePaths {
                    source_srt: published.to_string_lossy().into_owned(),
                    translated_srt: None,
                    output_dir: dir.to_string_lossy().into_owned(),
                })
            });
            assert!(matches!(result, Err(JobError::Cancelled)));
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let status: String = conn
            .query_row("SELECT status FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(status, "running");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn obsolete_worker_cannot_attach_a_result_after_restart_or_source_change() {
        for update in [
            "UPDATE tasks SET run_generation = 2",
            "UPDATE tasks SET result_revision = 1, source_srt_path = 'new-source'",
            "UPDATE tasks SET status = 'cancelled'",
        ] {
            let (mut conn, old_attempt) = running_fixture();
            conn.execute(update, []).unwrap();
            let result = apply_result_in_transaction(
                &mut conn,
                &old_attempt,
                &Arc::new(AtomicBool::new(false)),
                TRANSLATION_RESULT_SQL,
                params!["late-result", "late.srt", 3, "task"],
            );
            assert!(matches!(result, Err(JobError::Cancelled)), "{update}");
            let path: String = conn
                .query_row("SELECT translated_srt_path FROM tasks", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(path, "previous-translation");
        }
    }

    #[test]
    fn cancellation_is_checked_inside_result_transaction() {
        let (mut conn, attempt) = running_fixture();
        let result = apply_result_in_transaction(
            &mut conn,
            &attempt,
            &Arc::new(AtomicBool::new(true)),
            TRANSLATION_RESULT_SQL,
            params!["cancelled-result", "target.srt", 3, "task"],
        );
        assert!(matches!(result, Err(JobError::Cancelled)));
        let path: String = conn
            .query_row("SELECT translated_srt_path FROM tasks", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(path, "previous-translation");
    }

    #[test]
    fn current_attempt_commits_result_and_advances_version_atomically() {
        let (mut conn, attempt) = running_fixture();
        let saved = apply_result_in_transaction(
            &mut conn,
            &attempt,
            &Arc::new(AtomicBool::new(false)),
            TRANSLATION_RESULT_SQL,
            params!["new-result", "target.srt", 3, "task"],
        )
        .unwrap();
        assert_eq!(saved.translated_srt_path.as_deref(), Some("new-result"));
        assert_eq!(saved.run_generation, 1);
        assert_eq!(saved.result_revision, 1);
    }

    #[test]
    fn new_source_invalidates_old_translation_exports_and_resume() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO tasks(id, source_type, file_name, status, stage, message,
            progress, settings_json, source_srt_path, translated_srt_path, translated_file_name,
            translation_completed_count, exported_source_srt, exported_translated_srt,
            exported_output_dir, created_at, updated_at)
            VALUES('task', 'video', 'clip', 'exported', 'exported', '', 1, '{}', 'old.source',
            'old.target', 'target.srt', 25, 'export.source', 'export.target', 'exports', 1, 1);",
        )
        .unwrap();
        conn.execute(
            SOURCE_RESULT_SQL,
            params!["new.source", "source.srt", "out", 30, 2, "task"],
        )
        .unwrap();
        let row: (String, Option<String>, Option<String>, Option<i64>, i64) = conn
            .query_row(
                "SELECT source_srt_path, translated_srt_path, exported_translated_srt,
             translation_completed_count, result_revision FROM tasks",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row, ("new.source".into(), None, None, None, 1));
    }

    #[test]
    fn same_path_translation_still_advances_revision_and_invalidates_exports() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute_batch("INSERT INTO tasks(id, source_type, file_name, status, stage, message,
            progress, settings_json, translated_srt_path, exported_translated_srt, created_at, updated_at)
            VALUES('task', 'srt', 'clip', 'completed', 'completed', '', 1, '{}', 'target', 'old-export', 1, 1);").unwrap();
        for revision in 1..=2 {
            conn.execute(
                TRANSLATION_RESULT_SQL,
                params!["target", "target.srt", 2, "task"],
            )
            .unwrap();
            let row: (i64, Option<String>) = conn
                .query_row(
                    "SELECT result_revision, exported_translated_srt FROM tasks",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(row, (revision, None));
        }
    }

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
