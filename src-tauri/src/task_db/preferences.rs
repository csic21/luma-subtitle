use rusqlite::{params, OptionalExtension};
use tauri::AppHandle;

use super::{schema::connection, QueueSettings};

const DEFAULT_MAX_CONCURRENCY: usize = 2;
const DEFAULT_AUTO_START_NEXT: bool = false;
const API_KEY_SETTING: &str = "translation_api_key";

pub(crate) fn load_queue_settings(app: &AppHandle) -> Result<QueueSettings, String> {
    let conn = connection(app)?;
    let max_concurrency_value = conn
        .query_row(
            "SELECT value FROM queue_settings WHERE key = 'max_concurrency'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let auto_start_next_value = conn
        .query_row(
            "SELECT value FROM queue_settings WHERE key = 'auto_start_next'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let max_concurrency = max_concurrency_value
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_CONCURRENCY)
        .clamp(1, 4);
    let auto_start_next = auto_start_next_value
        .as_deref()
        .map(parse_bool_setting)
        .unwrap_or(DEFAULT_AUTO_START_NEXT);
    Ok(QueueSettings {
        max_concurrency,
        auto_start_next,
    })
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
    conn.execute(
        "INSERT INTO queue_settings(key, value) VALUES('auto_start_next', ?1)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![settings.auto_start_next.to_string()],
    )
    .map_err(|error| error.to_string())?;
    Ok(QueueSettings {
        max_concurrency,
        auto_start_next: settings.auto_start_next,
    })
}

fn parse_bool_setting(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}

pub(crate) fn save_api_key(app: &AppHandle, api_key: &str) -> Result<(), String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Ok(());
    }
    let conn = connection(app)?;
    conn.execute(
        "INSERT INTO app_secrets(key, value, updated_at) VALUES(?1, ?2, ?3)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![API_KEY_SETTING, api_key, super::now_ts()],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn load_api_key(app: &AppHandle) -> Result<Option<String>, String> {
    let conn = connection(app)?;
    conn.query_row(
        "SELECT value FROM app_secrets WHERE key = ?1",
        params![API_KEY_SETTING],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map(|value| value.filter(|api_key| !api_key.trim().is_empty()))
    .map_err(|error| error.to_string())
}

pub(crate) fn has_api_key(app: &AppHandle) -> Result<bool, String> {
    Ok(load_api_key(app)?.is_some())
}
