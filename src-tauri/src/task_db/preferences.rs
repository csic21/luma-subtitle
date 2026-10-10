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

// Credential scope is derived at the last possible moment from the actual saved
// task destination, never from the current global settings. Paths/models may
// change within an origin; scheme, host and effective port must match exactly.
pub(crate) fn credential_scope(provider: &str, base_url: &str) -> Result<String, String> {
    if crate::translation::normalize_translation_provider(provider) != "api" {
        return Err("该翻译方式不使用 API Key".to_string());
    }
    let url = url::Url::parse(base_url.trim()).map_err(|_| "API 地址无效".to_string())?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err("API 地址不能包含用户名、密码或片段".to_string());
    }
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err("API Key 仅允许通过 HTTPS 或本机回环地址使用".to_string());
    }
    Ok(format!("api|{}", url.origin().ascii_serialization()))
}

const SCOPED_KEY_PREFIX: &str = "translation_api_key_v2:";

fn scoped_key(scope: &str) -> String {
    format!("{SCOPED_KEY_PREFIX}{scope}")
}

fn read_key(conn: &rusqlite::Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM app_secrets WHERE key = ?1", params![key], |row| row.get::<_, String>(0))
        .optional().map(|value| value.filter(|value| !value.trim().is_empty()))
        .map_err(|error| error.to_string())
}

fn save_scoped_key(conn: &rusqlite::Connection, scope: &str, api_key: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO app_secrets(key, value, updated_at) VALUES(?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![scoped_key(scope), api_key.trim(), super::now_ts()],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn save_api_key(app: &AppHandle, provider: &str, base_url: &str, api_key: &str) -> Result<(), String> {
    if api_key.trim().is_empty() { return Ok(()); }
    let scope = credential_scope(provider, base_url)?;
    save_scoped_key(&connection(app)?, &scope, api_key)
}

pub(crate) fn load_api_key(app: &AppHandle, provider: &str, base_url: &str) -> Result<Option<String>, String> {
    if crate::translation::normalize_translation_provider(provider) != "api" { return Ok(None); }
    let scope = credential_scope(provider, base_url)?;
    let conn = connection(app)?;
    // An old unscoped key is deliberately never a fallback. An explicit user
    // choice in Settings is required before it can be associated with an origin.
    let key = read_key(&conn, &scoped_key(&scope))?;
    if key.is_none() {
        return Err(format!("没有与 {scope} 绑定的 API Key。请在设置中为该地址保存密钥，或明确绑定旧密钥"));
    }
    Ok(key)
}

pub(crate) fn api_key_scopes(app: &AppHandle) -> Result<Vec<String>, String> {
    let conn = connection(app)?;
    let mut statement = conn.prepare("SELECT key FROM app_secrets WHERE key LIKE 'translation_api_key_v2:%' AND length(trim(value)) > 0 ORDER BY key")
        .map_err(|error| error.to_string())?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0)).map_err(|error| error.to_string())?;
    rows.map(|row| row.map(|key| key[SCOPED_KEY_PREFIX.len()..].to_string()).map_err(|error| error.to_string())).collect()
}

pub(crate) fn has_legacy_api_key(app: &AppHandle) -> Result<bool, String> {
    Ok(read_key(&connection(app)?, API_KEY_SETTING)?.is_some())
}

fn bind_legacy_key(conn: &rusqlite::Connection, scope: &str) -> Result<(), String> {
    // Rename rather than copy, so one ambiguous legacy credential cannot be
    // silently reused for several origins. Never replace an existing scoped key.
    let changed = conn.execute("UPDATE app_secrets SET key = ?1, updated_at = ?2 WHERE key = ?3 AND NOT EXISTS (SELECT 1 FROM app_secrets WHERE key = ?1)",
        params![scoped_key(scope), super::now_ts(), API_KEY_SETTING]).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("没有可绑定的旧密钥，或该地址已有密钥".to_string()); }
    Ok(())
}

pub(crate) fn bind_legacy_api_key(app: &AppHandle, provider: &str, base_url: &str) -> Result<(), String> {
    let scope = credential_scope(provider, base_url)?;
    bind_legacy_key(&connection(app)?, &scope)
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    fn database() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        super::super::schema::migrate(&conn).unwrap();
        conn
    }
    #[test]
    fn origins_are_normalized_without_mixing_destinations() {
        assert_eq!(credential_scope("API", "https://EXAMPLE.test:443/v1/").unwrap(), "api|https://example.test");
        for other in ["https://example.test:444", "https://other.test", "http://localhost:1234"] {
            assert_ne!(credential_scope("api", other).unwrap(), "api|https://example.test");
        }
        for invalid in ["http://example.test", "https://user:secret@example.test", "https://example.test/#x", "file:///tmp/x", "invalid"] {
            assert!(credential_scope("api", invalid).is_err());
        }
        assert!(credential_scope("cli", "https://example.test").is_err());
    }
    #[test]
    fn saved_task_origin_does_not_receive_current_global_key() {
        let conn = database();
        let old = credential_scope("api", "https://a.test/v1").unwrap();
        let current = credential_scope("api", "https://b.test/v1").unwrap();
        save_scoped_key(&conn, &current, "dummy-b").unwrap();
        assert_eq!(read_key(&conn, &scoped_key(&old)).unwrap(), None);
        save_scoped_key(&conn, &old, "dummy-a").unwrap();
        assert_eq!(read_key(&conn, &scoped_key(&old)).unwrap().as_deref(), Some("dummy-a"));
        assert_eq!(read_key(&conn, &scoped_key(&current)).unwrap().as_deref(), Some("dummy-b"));
    }
    #[test]
    fn legacy_key_needs_explicit_single_origin_binding() {
        let conn = database();
        conn.execute("INSERT INTO app_secrets VALUES (?1, 'dummy-legacy', 1)", params![API_KEY_SETTING]).unwrap();
        let scope = credential_scope("api", "https://a.test").unwrap();
        assert!(read_key(&conn, &scoped_key(&scope)).unwrap().is_none());
        bind_legacy_key(&conn, &scope).unwrap();
        assert_eq!(read_key(&conn, &scoped_key(&scope)).unwrap().as_deref(), Some("dummy-legacy"));
        assert!(read_key(&conn, API_KEY_SETTING).unwrap().is_none());
        assert!(bind_legacy_key(&conn, "api|https://b.test").is_err());
    }
}
