use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    job_events::{ExportedSubtitlePaths, JobEvent, JobStatus},
    translation::DEFAULT_TRANSLATION_SHARD_SIZE,
};

const DEFAULT_MAX_CONCURRENCY: usize = 2;

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct TaskSettingsSnapshot {
    pub(crate) output_dir: Option<String>,
    pub(crate) target_language: String,
    pub(crate) whisper_model_path: String,
    pub(crate) whisper_language: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    #[serde(default = "default_translation_shard_size")]
    pub(crate) translation_shard_size: usize,
}

#[derive(Clone, Serialize)]
pub(crate) struct TaskRecord {
    pub(crate) id: String,
    pub(crate) source_type: String,
    pub(crate) video_path: Option<String>,
    pub(crate) srt_path: Option<String>,
    pub(crate) file_name: String,
    pub(crate) status: String,
    pub(crate) stage: String,
    pub(crate) message: String,
    pub(crate) progress: f32,
    pub(crate) settings: TaskSettingsSnapshot,
    pub(crate) source_srt_path: Option<String>,
    pub(crate) translated_srt_path: Option<String>,
    pub(crate) source_file_name: Option<String>,
    pub(crate) translated_file_name: Option<String>,
    pub(crate) output_dir: Option<String>,
    pub(crate) segment_count: Option<usize>,
    pub(crate) exported_source_srt: Option<String>,
    pub(crate) exported_translated_srt: Option<String>,
    pub(crate) exported_output_dir: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct QueueSettings {
    pub(crate) max_concurrency: usize,
}

pub(crate) fn init(app: &AppHandle) -> Result<(), String> {
    let _ = connection(app)?;
    mark_interrupted_tasks(app)
}

pub(crate) fn list_tasks(app: &AppHandle) -> Result<Vec<TaskRecord>, String> {
    let conn = connection(app)?;
    let mut statement = conn
        .prepare("SELECT * FROM tasks ORDER BY updated_at DESC, created_at DESC")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], task_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

pub(crate) fn get_task(app: &AppHandle, task_id: &str) -> Result<Option<TaskRecord>, String> {
    let conn = connection(app)?;
    conn.query_row(
        "SELECT * FROM tasks WHERE id = ?1",
        params![task_id],
        task_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(crate) fn require_task(app: &AppHandle, task_id: &str) -> Result<TaskRecord, String> {
    get_task(app, task_id)?.ok_or_else(|| "没有找到任务".to_string())
}

pub(crate) fn insert_task(app: &AppHandle, record: &TaskRecord) -> Result<TaskRecord, String> {
    let conn = connection(app)?;
    let settings_json =
        serde_json::to_string(&record.settings).map_err(|error| error.to_string())?;
    conn.execute(
        "INSERT INTO tasks (
            id, source_type, video_path, srt_path, file_name, status, stage, message, progress,
            settings_json, source_srt_path, translated_srt_path, source_file_name,
            translated_file_name, output_dir, segment_count, exported_source_srt,
            exported_translated_srt, exported_output_dir, error, created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22
        )",
        params![
            record.id,
            record.source_type,
            record.video_path,
            record.srt_path,
            record.file_name,
            record.status,
            record.stage,
            record.message,
            record.progress,
            settings_json,
            record.source_srt_path,
            record.translated_srt_path,
            record.source_file_name,
            record.translated_file_name,
            record.output_dir,
            record.segment_count.map(|value| value as i64),
            record.exported_source_srt,
            record.exported_translated_srt,
            record.exported_output_dir,
            record.error,
            record.created_at,
            record.updated_at,
        ],
    )
    .map_err(|error| error.to_string())?;
    append_log(app, &record.id, &record.message)?;
    emit_task(app, &record.id);
    require_task(app, &record.id)
}

pub(crate) fn delete_task(app: &AppHandle, task_id: &str) -> Result<(), String> {
    let conn = connection(app)?;
    conn.execute("DELETE FROM task_logs WHERE task_id = ?1", params![task_id])
        .map_err(|error| error.to_string())?;
    conn.execute("DELETE FROM tasks WHERE id = ?1", params![task_id])
        .map_err(|error| error.to_string())?;
    let _ = app.emit("task-deleted", task_id.to_string());
    Ok(())
}

pub(crate) fn task_logs(app: &AppHandle, task_id: &str) -> Result<Vec<String>, String> {
    let conn = connection(app)?;
    let mut statement = conn
        .prepare("SELECT line FROM task_logs WHERE task_id = ?1 ORDER BY id ASC")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![task_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

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
    let now = now_ts();
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
    let now = now_ts();
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
    let now = now_ts();
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

pub(crate) fn update_task_settings(
    app: &AppHandle,
    task_id: &str,
    settings: TaskSettingsSnapshot,
) -> Result<TaskRecord, String> {
    let conn = connection(app)?;
    let settings_json = serde_json::to_string(&settings).map_err(|error| error.to_string())?;
    let now = now_ts();
    conn.execute(
        "UPDATE tasks SET
            settings_json = ?1,
            updated_at = ?2
        WHERE id = ?3",
        params![settings_json, now, task_id],
    )
    .map_err(|error| error.to_string())?;
    append_log(app, task_id, "settings · 任务配置已更新")?;
    emit_task(app, task_id);
    require_task(app, task_id)
}

pub(crate) fn record_job_event(app: &AppHandle, event: &JobEvent) -> Result<(), String> {
    if get_task(app, &event.job_id)?.is_none() {
        return Ok(());
    }
    let conn = connection(app)?;
    let now = now_ts();
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

pub(crate) fn load_queue_settings(app: &AppHandle) -> Result<QueueSettings, String> {
    let conn = connection(app)?;
    let value = conn
        .query_row(
            "SELECT value FROM queue_settings WHERE key = 'max_concurrency'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let max_concurrency = value
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_CONCURRENCY)
        .clamp(1, 4);
    Ok(QueueSettings { max_concurrency })
}

pub(crate) fn save_queue_settings(
    app: &AppHandle,
    settings: QueueSettings,
) -> Result<QueueSettings, String> {
    let max_concurrency = settings.max_concurrency.clamp(1, 4);
    let conn = connection(app)?;
    conn.execute(
        "INSERT INTO queue_settings(key, value) VALUES('max_concurrency', ?1)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![max_concurrency.to_string()],
    )
    .map_err(|error| error.to_string())?;
    Ok(QueueSettings { max_concurrency })
}

pub(crate) fn task_work_dir(app: &AppHandle, task_id: &str) -> Result<PathBuf, String> {
    let dir = app_data_dir(app)?.join("tasks").join(task_id);
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

pub(crate) fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
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
    let now = now_ts();
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

fn append_log(app: &AppHandle, task_id: &str, line: &str) -> Result<(), String> {
    let conn = connection(app)?;
    conn.execute(
        "INSERT INTO task_logs(task_id, created_at, line) VALUES(?1, ?2, ?3)",
        params![task_id, now_ts(), line],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn mark_interrupted_tasks(app: &AppHandle) -> Result<(), String> {
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

fn emit_task(app: &AppHandle, task_id: &str) {
    if let Ok(Some(task)) = get_task(app, task_id) {
        let _ = app.emit("task-updated", task);
    }
}

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<TaskRecord> {
    let settings_json: String = row.get("settings_json")?;
    let settings = serde_json::from_str(&settings_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            settings_json.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    let segment_count = row
        .get::<_, Option<i64>>("segment_count")?
        .map(|value| value.max(0) as usize);

    Ok(TaskRecord {
        id: row.get("id")?,
        source_type: row.get("source_type")?,
        video_path: row.get("video_path")?,
        srt_path: row.get("srt_path")?,
        file_name: row.get("file_name")?,
        status: row.get("status")?,
        stage: row.get("stage")?,
        message: row.get("message")?,
        progress: row.get("progress")?,
        settings,
        source_srt_path: row.get("source_srt_path")?,
        translated_srt_path: row.get("translated_srt_path")?,
        source_file_name: row.get("source_file_name")?,
        translated_file_name: row.get("translated_file_name")?,
        output_dir: row.get("output_dir")?,
        segment_count,
        exported_source_srt: row.get("exported_source_srt")?,
        exported_translated_srt: row.get("exported_translated_srt")?,
        exported_output_dir: row.get("exported_output_dir")?,
        error: row.get("error")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn connection(app: &AppHandle) -> Result<Connection, String> {
    let path = app_data_dir(app)?.join("luma.sqlite3");
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    migrate(&conn)?;
    Ok(conn)
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .or_else(|_| app.path().app_config_dir())
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            source_type TEXT NOT NULL,
            video_path TEXT,
            srt_path TEXT,
            file_name TEXT NOT NULL,
            status TEXT NOT NULL,
            stage TEXT NOT NULL,
            message TEXT NOT NULL,
            progress REAL NOT NULL,
            settings_json TEXT NOT NULL,
            source_srt_path TEXT,
            translated_srt_path TEXT,
            source_file_name TEXT,
            translated_file_name TEXT,
            output_dir TEXT,
            segment_count INTEGER,
            exported_source_srt TEXT,
            exported_translated_srt TEXT,
            exported_output_dir TEXT,
            error TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS task_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            line TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_task_logs_task_id ON task_logs(task_id, id);
        CREATE TABLE IF NOT EXISTS queue_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        INSERT OR IGNORE INTO queue_settings(key, value) VALUES('max_concurrency', '2');
        ",
    )
    .map_err(|error| error.to_string())
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

fn default_translation_shard_size() -> usize {
    DEFAULT_TRANSLATION_SHARD_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_task_schema_with_default_queue_settings() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        migrate(&conn).expect("migration should run");

        let max_concurrency: String = conn
            .query_row(
                "SELECT value FROM queue_settings WHERE key = 'max_concurrency'",
                [],
                |row| row.get(0),
            )
            .expect("default queue setting should exist");
        assert_eq!(max_concurrency, "2");
    }

    #[test]
    fn decodes_task_record_from_row() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        migrate(&conn).expect("migration should run");
        let settings = TaskSettingsSnapshot {
            output_dir: Some("D:/out".to_string()),
            target_language: "简体中文".to_string(),
            whisper_model_path: "D:/models/ggml.bin".to_string(),
            whisper_language: "auto".to_string(),
            base_url: "https://example.test".to_string(),
            model: "test-model".to_string(),
            temperature: 0.2,
            translation_shard_size: 120,
        };
        conn.execute(
            "INSERT INTO tasks (
                id, source_type, video_path, srt_path, file_name, status, stage, message,
                progress, settings_json, created_at, updated_at
            ) VALUES (?1, 'video', ?2, NULL, 'clip.mp4', 'created', 'created', '任务已创建', 0.0, ?3, 1, 1)",
            params![
                "task-1",
                "D:/video/clip.mp4",
                serde_json::to_string(&settings).expect("settings should serialize"),
            ],
        )
        .expect("task row should insert");

        let record = conn
            .query_row("SELECT * FROM tasks WHERE id = 'task-1'", [], task_from_row)
            .expect("task row should decode");
        assert_eq!(record.id, "task-1");
        assert_eq!(record.settings.model, "test-model");
        assert_eq!(record.settings.output_dir.as_deref(), Some("D:/out"));
        assert_eq!(record.settings.translation_shard_size, 120);
    }

    #[test]
    fn decodes_old_task_settings_with_default_shard_size() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite should open");
        migrate(&conn).expect("migration should run");
        let settings_json = serde_json::json!({
            "output_dir": null,
            "target_language": "简体中文",
            "whisper_model_path": "D:/models/ggml.bin",
            "whisper_language": "auto",
            "base_url": "https://example.test",
            "model": "test-model",
            "temperature": 0.2
        })
        .to_string();
        conn.execute(
            "INSERT INTO tasks (
                id, source_type, video_path, srt_path, file_name, status, stage, message,
                progress, settings_json, created_at, updated_at
            ) VALUES (?1, 'video', ?2, NULL, 'clip.mp4', 'created', 'created', '任务已创建', 0.0, ?3, 1, 1)",
            params!["task-1", "D:/video/clip.mp4", settings_json],
        )
        .expect("task row should insert");

        let record = conn
            .query_row("SELECT * FROM tasks WHERE id = 'task-1'", [], task_from_row)
            .expect("task row should decode");

        assert_eq!(
            record.settings.translation_shard_size,
            DEFAULT_TRANSLATION_SHARD_SIZE
        );
    }
}
