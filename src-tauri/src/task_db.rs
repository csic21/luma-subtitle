use rusqlite::{params, OptionalExtension};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};

mod events;
mod models;
mod preferences;
mod schema;

pub(crate) use events::{
    record_job_event, set_exported, set_interrupted, set_queued, set_subtitle_result,
    set_translation_result,
};
pub(crate) use models::{QueueSettings, TaskRecord, TaskSettingsSnapshot};
pub(crate) use preferences::{
    has_api_key, load_api_key, load_queue_settings, save_api_key, save_queue_settings,
};
use events::{append_log, emit_task, mark_interrupted_tasks};
use schema::{app_data_dir, connection, task_from_row};
#[cfg(test)]
use schema::migrate;
#[cfg(test)]
use crate::translation::DEFAULT_TRANSLATION_SHARD_SIZE;

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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

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

        conn.execute(
            "INSERT INTO app_secrets(key, value, updated_at) VALUES('translation_api_key', 'sk-test', 1)",
            [],
        )
        .expect("app secret should insert");
        let api_key: String = conn
            .query_row(
                "SELECT value FROM app_secrets WHERE key = 'translation_api_key'",
                [],
                |row| row.get(0),
            )
            .expect("app secret should be readable");
        assert_eq!(api_key, "sk-test");
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
