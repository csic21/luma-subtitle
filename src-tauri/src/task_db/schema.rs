use rusqlite::{Connection, Row};
use std::{fs, path::PathBuf, time::Duration};
use tauri::{AppHandle, Manager};

use super::TaskRecord;

const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn task_from_row(row: &Row<'_>) -> rusqlite::Result<TaskRecord> {
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
        audio_path: row.get("audio_path")?,
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

pub(super) fn connection(app: &AppHandle) -> Result<Connection, String> {
    let path = app_data_dir(app)?.join("luma.sqlite3");
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    configure_connection(&conn)?;
    Ok(conn)
}

fn configure_connection(conn: &Connection) -> Result<(), String> {
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)
        .map_err(|error| error.to_string())
}

pub(super) fn enable_wal(conn: &Connection) -> Result<(), String> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| error.to_string())
}

pub(super) fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .or_else(|_| app.path().app_config_dir())
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            source_type TEXT NOT NULL,
            video_path TEXT,
            audio_path TEXT,
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
        CREATE TABLE IF NOT EXISTS app_secrets (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS task_artifact_cleanups (
            task_id TEXT NOT NULL,
            path TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_error TEXT,
            PRIMARY KEY(task_id, path)
        );
        INSERT OR IGNORE INTO queue_settings(key, value) VALUES('max_concurrency', '2');
        INSERT OR IGNORE INTO queue_settings(key, value) VALUES('auto_start_next', 'false');
        ",
    )
    .map_err(|error| error.to_string())?;
    ensure_column(conn, "tasks", "audio_path", "TEXT")?;
    Ok(())
}

fn ensure_column(
    conn: &Connection,
    table_name: &str,
    column_name: &str,
    column_definition: &str,
) -> Result<(), String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .map_err(|error| error.to_string())?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if columns.iter().any(|column| column == column_name) {
        return Ok(());
    }
    conn.execute(
        &format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {column_definition}"),
        [],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn configures_file_connections_for_wal_and_busy_timeout() {
        let path =
            std::env::temp_dir().join(format!("luma-subtitle-schema-{}.sqlite3", Uuid::new_v4()));
        let conn = Connection::open(&path).expect("temporary sqlite database should open");

        configure_connection(&conn).expect("busy timeout should apply");
        enable_wal(&conn).expect("WAL should apply");

        let journal_mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .expect("journal mode should be readable");
        let busy_timeout: i64 = conn
            .pragma_query_value(None, "busy_timeout", |row| row.get(0))
            .expect("busy timeout should be readable");
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(busy_timeout, SQLITE_BUSY_TIMEOUT.as_millis() as i64);

        drop(conn);
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }
}
