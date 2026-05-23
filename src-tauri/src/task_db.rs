use rusqlite::{params, OptionalExtension};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};

mod events;
mod models;
mod preferences;
mod schema;

#[cfg(test)]
use crate::translation::DEFAULT_TRANSLATION_SHARD_SIZE;
use events::{append_log, emit_task, mark_interrupted_tasks};
pub(crate) use events::{
    record_job_event, set_exported, set_interrupted, set_queued, set_subtitle_result,
    set_translation_result,
};
pub(crate) use models::{QueueSettings, TaskRecord, TaskSettingsSnapshot};
pub(crate) use preferences::{
    has_api_key, load_api_key, load_queue_settings, save_api_key, save_queue_settings,
};
#[cfg(test)]
use schema::migrate;
use schema::{app_data_dir, connection, task_from_row};

pub(crate) fn init(app: &AppHandle) -> Result<(), String> {
    let _ = connection(app)?;
    mark_interrupted_tasks(app)?;

    let cleanup_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = cleanup_orphan_task_artifacts(&cleanup_app) {
            log_cleanup_error(&cleanup_app, "startup", &error);
        }
    });

    Ok(())
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

pub(crate) fn delete_task(app: &AppHandle, task_id: &str) -> Result<TaskRecord, String> {
    let task = require_task(app, task_id)?;
    let cleanup_dirs = task_artifact_dirs(app, &task)?;
    let mut conn = connection(app)?;
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    queue_artifact_cleanup_paths_in_tx(&tx, task_id, &cleanup_dirs)?;
    tx.execute("DELETE FROM task_logs WHERE task_id = ?1", params![task_id])
        .map_err(|error| error.to_string())?;
    tx.execute("DELETE FROM tasks WHERE id = ?1", params![task_id])
        .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    let _ = app.emit("task-deleted", task_id.to_string());
    Ok(task)
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
    let dir = task_work_dir_path(app, task_id)?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

fn task_work_dir_path(app: &AppHandle, task_id: &str) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("tasks").join(task_id))
}

pub(crate) fn cleanup_task_artifacts(app: &AppHandle, task: &TaskRecord) -> Result<(), String> {
    let dirs = match task_artifact_dirs(app, task) {
        Ok(dirs) => dirs,
        Err(error) => {
            log_cleanup_error(app, &task.id, &error);
            return Err(error);
        }
    };
    cleanup_artifact_dirs(app, &task.id, dirs)
}

pub(crate) fn cleanup_orphan_task_artifacts(app: &AppHandle) -> Result<(), String> {
    let pending = pending_artifact_cleanups(app)?;
    let pending_paths = pending
        .iter()
        .map(|(_, dir)| cleanup_path_value(dir))
        .collect::<HashSet<_>>();
    let mut errors = Vec::new();

    for (task_id, dir) in pending {
        if let Err(error) = cleanup_artifact_dirs(app, &task_id, vec![dir]) {
            errors.push(error);
        }
    }

    let conn = connection(app)?;
    let active_task_ids = active_task_ids(&conn)?;
    drop(conn);

    cleanup_orphan_app_task_dirs(app, &active_task_ids, &pending_paths, &mut errors)?;
    cleanup_orphan_output_work_dirs(app, &active_task_ids, &pending_paths, &mut errors)?;

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn task_artifact_dirs(app: &AppHandle, task: &TaskRecord) -> Result<Vec<PathBuf>, String> {
    let mut dirs = Vec::new();
    push_unique_path(&mut dirs, task_work_dir_path(app, &task.id)?);
    for dir in task_output_work_dirs(task) {
        push_unique_path(&mut dirs, dir);
    }
    Ok(dirs)
}

fn cleanup_artifact_dirs(app: &AppHandle, task_id: &str, dirs: Vec<PathBuf>) -> Result<(), String> {
    let mut errors = Vec::new();
    for dir in dirs {
        if !dir.exists() {
            let _ = remove_artifact_cleanup_path(app, task_id, &dir);
            continue;
        }
        if !dir.is_dir() {
            errors.push(format!("{} 不是目录", dir.display()));
            record_artifact_cleanup_error(app, task_id, &dir, "不是目录");
            continue;
        }
        if let Err(error) = fs::remove_dir_all(&dir) {
            let message = error.to_string();
            errors.push(format!("{}: {message}", dir.display()));
            record_artifact_cleanup_error(app, task_id, &dir, &message);
            continue;
        }
        let _ = remove_artifact_cleanup_path(app, task_id, &dir);
        remove_empty_parent_work_dir(&dir);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("清理任务文件失败: {}", errors.join("; ")))
    }
}

fn queue_artifact_cleanup_paths_in_tx(
    tx: &rusqlite::Transaction<'_>,
    task_id: &str,
    dirs: &[PathBuf],
) -> Result<(), String> {
    let now = now_ts();
    for dir in dirs {
        tx.execute(
            "INSERT INTO task_artifact_cleanups(task_id, path, created_at, updated_at, last_error)
             VALUES(?1, ?2, ?3, ?3, NULL)
             ON CONFLICT(task_id, path) DO UPDATE SET updated_at = excluded.updated_at, last_error = NULL",
            params![task_id, cleanup_path_value(dir), now],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn queue_artifact_cleanup_paths(
    app: &AppHandle,
    task_id: &str,
    dirs: &[PathBuf],
) -> Result<(), String> {
    let mut conn = connection(app)?;
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    queue_artifact_cleanup_paths_in_tx(&tx, task_id, dirs)?;
    tx.commit().map_err(|error| error.to_string())
}

fn pending_artifact_cleanups(app: &AppHandle) -> Result<Vec<(String, PathBuf)>, String> {
    let conn = connection(app)?;
    let mut statement = conn
        .prepare("SELECT task_id, path FROM task_artifact_cleanups ORDER BY created_at ASC")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                PathBuf::from(row.get::<_, String>(1)?),
            ))
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn active_task_ids(conn: &rusqlite::Connection) -> Result<HashSet<String>, String> {
    let mut statement = conn
        .prepare("SELECT id FROM tasks")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|error| error.to_string())
}

fn cleanup_orphan_app_task_dirs(
    app: &AppHandle,
    active_task_ids: &HashSet<String>,
    skipped_paths: &HashSet<String>,
    errors: &mut Vec<String>,
) -> Result<(), String> {
    let tasks_dir = app_data_dir(app)?.join("tasks");
    if !tasks_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&tasks_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if skipped_paths.contains(&cleanup_path_value(&path)) {
            continue;
        }
        let Some(task_id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if active_task_ids.contains(&task_id) {
            continue;
        }
        let dirs = vec![path];
        let _ = queue_artifact_cleanup_paths(app, &task_id, &dirs);
        if let Err(error) = cleanup_artifact_dirs(app, &task_id, dirs) {
            errors.push(error);
        }
    }
    Ok(())
}

fn cleanup_orphan_output_work_dirs(
    app: &AppHandle,
    active_task_ids: &HashSet<String>,
    skipped_paths: &HashSet<String>,
    errors: &mut Vec<String>,
) -> Result<(), String> {
    let tasks = list_tasks(app)?;
    let mut work_parents = Vec::new();
    for task in tasks {
        for dir in task_output_work_dirs(&task) {
            if let Some(parent) = dir.parent() {
                push_unique_path(&mut work_parents, parent.to_path_buf());
            }
        }
    }

    for parent in work_parents {
        if !parent.is_dir()
            || !parent
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == ".luma-subtitle-work")
        {
            continue;
        }
        for entry in fs::read_dir(&parent).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if skipped_paths.contains(&cleanup_path_value(&path)) {
                continue;
            }
            let Some(task_id) = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            if active_task_ids.contains(&task_id) {
                continue;
            }
            let dirs = vec![path];
            let _ = queue_artifact_cleanup_paths(app, &task_id, &dirs);
            if let Err(error) = cleanup_artifact_dirs(app, &task_id, dirs) {
                errors.push(error);
            }
        }
        remove_empty_work_dir(&parent);
    }

    Ok(())
}

fn remove_artifact_cleanup_path(app: &AppHandle, task_id: &str, dir: &Path) -> Result<(), String> {
    let conn = connection(app)?;
    conn.execute(
        "DELETE FROM task_artifact_cleanups WHERE task_id = ?1 AND path = ?2",
        params![task_id, cleanup_path_value(dir)],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn record_artifact_cleanup_error(app: &AppHandle, task_id: &str, dir: &Path, error: &str) {
    let now = now_ts();
    if let Ok(conn) = connection(app) {
        let _ = conn.execute(
            "INSERT INTO task_artifact_cleanups(task_id, path, created_at, updated_at, last_error)
             VALUES(?1, ?2, ?3, ?3, ?4)
             ON CONFLICT(task_id, path) DO UPDATE SET updated_at = excluded.updated_at, last_error = excluded.last_error",
            params![task_id, cleanup_path_value(dir), now, error],
        );
    }
    log_cleanup_error(app, task_id, &format!("{}: {error}", dir.display()));
}

fn cleanup_path_value(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn log_cleanup_error(app: &AppHandle, task_id: &str, message: &str) {
    eprintln!("清理任务文件失败 [{task_id}]: {message}");
    let Ok(dir) = app_data_dir(app) else {
        return;
    };
    let log_path = dir.join("cleanup.log");
    let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    else {
        return;
    };
    let _ = writeln!(file, "[{}] {task_id}: {message}", now_ts());
}

fn task_output_work_dirs(task: &TaskRecord) -> Vec<PathBuf> {
    let mut output_dirs = Vec::new();
    if let Some(dir) = task.output_dir.as_deref() {
        push_unique_path(&mut output_dirs, PathBuf::from(dir));
    }
    if let Some(dir) = task.settings.output_dir.as_deref() {
        push_unique_path(&mut output_dirs, PathBuf::from(dir));
    }
    if let Some(video_path) = task.video_path.as_deref() {
        if let Some(parent) = Path::new(video_path).parent() {
            push_unique_path(&mut output_dirs, parent.to_path_buf());
        }
    }

    output_dirs
        .into_iter()
        .map(|dir| dir.join(".luma-subtitle-work").join(&task.id))
        .collect()
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|current| current == &path) {
        paths.push(path);
    }
}

fn remove_empty_parent_work_dir(dir: &Path) {
    let Some(parent) = dir.parent() else {
        return;
    };
    remove_empty_work_dir(parent);
}

fn remove_empty_work_dir(parent: &Path) {
    if parent
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".luma-subtitle-work")
    {
        let _ = fs::remove_dir(parent);
    }
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
        let auto_start_next: String = conn
            .query_row(
                "SELECT value FROM queue_settings WHERE key = 'auto_start_next'",
                [],
                |row| row.get(0),
            )
            .expect("default auto-chain setting should exist");
        assert_eq!(auto_start_next, "false");

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

        conn.execute(
            "INSERT INTO task_artifact_cleanups(task_id, path, created_at, updated_at)
             VALUES('task-1', '/tmp/luma/task-1', 1, 1)",
            [],
        )
        .expect("artifact cleanup should insert");
        let cleanup_path: String = conn
            .query_row(
                "SELECT path FROM task_artifact_cleanups WHERE task_id = 'task-1'",
                [],
                |row| row.get(0),
            )
            .expect("artifact cleanup should be readable");
        assert_eq!(cleanup_path, "/tmp/luma/task-1");
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
            base_url_is_complete: false,
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
        assert!(!record.settings.base_url_is_complete);
    }

    #[test]
    fn task_output_work_dirs_cover_known_intermediate_locations() {
        let task = TaskRecord {
            id: "task-1".to_string(),
            source_type: "video".to_string(),
            video_path: Some("videos/clip.mp4".to_string()),
            srt_path: None,
            file_name: "clip.mp4".to_string(),
            status: "completed".to_string(),
            stage: "completed".to_string(),
            message: "SRT 已生成".to_string(),
            progress: 1.0,
            settings: TaskSettingsSnapshot {
                output_dir: Some("exports".to_string()),
                target_language: "简体中文".to_string(),
                whisper_model_path: "models/ggml.bin".to_string(),
                whisper_language: "auto".to_string(),
                base_url: "https://example.test".to_string(),
                base_url_is_complete: false,
                model: "test-model".to_string(),
                temperature: 0.2,
                translation_shard_size: 120,
            },
            source_srt_path: Some("app-data/tasks/task-1/clip.source.srt".to_string()),
            translated_srt_path: Some("app-data/tasks/task-1/clip.zh.srt".to_string()),
            source_file_name: Some("clip.source.srt".to_string()),
            translated_file_name: Some("clip.zh.srt".to_string()),
            output_dir: Some("exports".to_string()),
            segment_count: Some(10),
            exported_source_srt: Some("exports/clip.source.srt".to_string()),
            exported_translated_srt: Some("exports/clip.zh.srt".to_string()),
            exported_output_dir: Some("exports".to_string()),
            error: None,
            created_at: 1,
            updated_at: 1,
        };

        let dirs = task_output_work_dirs(&task);

        assert_eq!(
            dirs,
            vec![
                PathBuf::from("exports")
                    .join(".luma-subtitle-work")
                    .join("task-1"),
                PathBuf::from("videos")
                    .join(".luma-subtitle-work")
                    .join("task-1"),
            ]
        );
    }
}
