use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager};

use crate::{
    task_db::{self, TaskSettingsSnapshot},
    translation::{
        normalize_translation_cli_args, normalize_translation_cli_command,
        normalize_translation_cli_model, normalize_translation_cli_tool,
        normalize_translation_provider, normalize_translation_shard_size,
        DEFAULT_TRANSLATION_CLI_COMMAND, DEFAULT_TRANSLATION_CLI_TOOL,
        DEFAULT_TRANSLATION_PROVIDER, DEFAULT_TRANSLATION_SHARD_SIZE,
    },
};

#[derive(Clone, Deserialize, Serialize)]
struct PersistedSettings {
    base_url: String,
    #[serde(default)]
    base_url_is_complete: bool,
    model: String,
    temperature: f32,
    whisper_model_path: String,
    whisper_language: String,
    target_language: String,
    #[serde(default)]
    has_api_key: bool,
    #[serde(default = "default_translation_shard_size")]
    translation_shard_size: usize,
    #[serde(default = "default_translation_provider")]
    translation_provider: String,
    #[serde(default = "default_translation_cli_tool")]
    translation_cli_tool: String,
    #[serde(default = "default_translation_cli_command")]
    translation_cli_command: String,
    #[serde(default)]
    translation_cli_model: String,
    #[serde(default)]
    translation_cli_args: String,
}
impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com".to_string(),
            base_url_is_complete: false,
            model: "gpt-4o-mini".to_string(),
            temperature: 0.2,
            whisper_model_path: String::new(),
            whisper_language: "auto".to_string(),
            target_language: "简体中文".to_string(),
            has_api_key: false,
            translation_shard_size: DEFAULT_TRANSLATION_SHARD_SIZE,
            translation_provider: DEFAULT_TRANSLATION_PROVIDER.to_string(),
            translation_cli_tool: DEFAULT_TRANSLATION_CLI_TOOL.to_string(),
            translation_cli_command: DEFAULT_TRANSLATION_CLI_COMMAND.to_string(),
            translation_cli_model: String::new(),
            translation_cli_args: String::new(),
        }
    }
}
#[derive(Deserialize)]
pub(crate) struct SettingsPayload {
    base_url: String,
    #[serde(default)]
    base_url_is_complete: bool,
    model: String,
    temperature: f32,
    whisper_model_path: String,
    whisper_language: String,
    target_language: String,
    translation_shard_size: Option<usize>,
    api_key: Option<String>,
    has_api_key: Option<bool>,
    #[serde(default)]
    translation_provider: Option<String>,
    #[serde(default)]
    translation_cli_tool: Option<String>,
    #[serde(default)]
    translation_cli_command: Option<String>,
    #[serde(default)]
    translation_cli_model: Option<String>,
    #[serde(default)]
    translation_cli_args: Option<String>,
}
#[derive(Serialize)]
pub(crate) struct SettingsResponse {
    base_url: String,
    base_url_is_complete: bool,
    model: String,
    temperature: f32,
    whisper_model_path: String,
    whisper_language: String,
    target_language: String,
    translation_shard_size: usize,
    has_api_key: bool,
    translation_provider: String,
    translation_cli_tool: String,
    translation_cli_command: String,
    translation_cli_model: String,
    translation_cli_args: String,
}

#[tauri::command]
pub(crate) async fn load_settings(app: AppHandle) -> Result<SettingsResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = read_settings(&app)?;
        Ok(settings.into_response(task_db::has_api_key(&app)?))
    })
    .await
    .map_err(|error| format!("读取设置失败: {error}"))?
}
#[tauri::command]
pub(crate) fn save_settings(
    app: AppHandle,
    payload: SettingsPayload,
) -> Result<SettingsResponse, String> {
    let _ = payload.has_api_key;
    let has_api_key = task_db::has_api_key(&app)?;
    let base_url_is_complete = payload.base_url_is_complete;
    let read_previous = read_settings(&app).unwrap_or_default();
    let settings = PersistedSettings {
        base_url: normalize_base_url(&payload.base_url, base_url_is_complete),
        base_url_is_complete,
        model: payload.model.trim().to_string(),
        temperature: payload.temperature.clamp(0.0, 1.0),
        whisper_model_path: payload.whisper_model_path.trim().to_string(),
        whisper_language: normalize_language(&payload.whisper_language),
        target_language: payload.target_language.trim().to_string(),
        has_api_key,
        translation_shard_size: normalize_translation_shard_size(
            payload
                .translation_shard_size
                .unwrap_or(DEFAULT_TRANSLATION_SHARD_SIZE),
        ),
        translation_provider: normalize_translation_provider(
            payload
                .translation_provider
                .as_deref()
                .unwrap_or(&read_previous.translation_provider),
        ),
        translation_cli_tool: normalize_translation_cli_tool(
            payload
                .translation_cli_tool
                .as_deref()
                .unwrap_or(&read_previous.translation_cli_tool),
        ),
        translation_cli_command: normalize_translation_cli_command(
            payload
                .translation_cli_command
                .as_deref()
                .unwrap_or(&read_previous.translation_cli_command),
        ),
        translation_cli_model: normalize_translation_cli_model(
            payload
                .translation_cli_model
                .as_deref()
                .unwrap_or(&read_previous.translation_cli_model),
        ),
        translation_cli_args: normalize_translation_cli_args(
            payload
                .translation_cli_args
                .as_deref()
                .unwrap_or(&read_previous.translation_cli_args),
        ),
    };
    if let Some(api_key) = payload.api_key {
        let api_key = api_key.trim();
        if !api_key.is_empty() {
            task_db::save_api_key(&app, api_key)
                .map_err(|error| format!("API Key 保存失败: {error}"))?;
        }
    }
    let settings = PersistedSettings {
        has_api_key: task_db::has_api_key(&app)?,
        ..settings
    };
    let path = settings_path(&app)?;
    let body = serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?;
    fs::write(path, body).map_err(|error| error.to_string())?;
    let has_api_key = settings.has_api_key;
    Ok(settings.into_response(has_api_key))
}

pub(crate) fn normalize_language(language: &str) -> String {
    match language.trim().to_lowercase().as_str() {
        "" | "auto" | "自动" | "自动检测" => "auto".to_string(),
        "zh" | "zh-cn" | "zh-tw" | "cn" | "chinese" | "中文" | "简体中文" | "繁体中文"
        | "普通话" => "zh".to_string(),
        "en" | "eng" | "english" | "英语" | "英文" => "en".to_string(),
        "ja" | "jp" | "japanese" | "日本語" | "日语" | "日文" => "ja".to_string(),
        "ko" | "kr" | "korean" | "한국어" | "韩语" | "韩文" => "ko".to_string(),
        "de" | "ger" | "german" | "deutsch" | "德语" => "de".to_string(),
        "fr" | "fre" | "french" | "français" | "francais" | "法语" => "fr".to_string(),
        "es" | "spa" | "spanish" | "español" | "espanol" | "西班牙语" => "es".to_string(),
        "it" | "ita" | "italian" | "italiano" | "意大利语" => "it".to_string(),
        "pt" | "por" | "portuguese" | "português" | "portugues" | "葡萄牙语" => {
            "pt".to_string()
        }
        "ru" | "rus" | "russian" | "русский" | "俄语" => "ru".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn normalize_base_url(base_url: &str, is_complete: bool) -> String {
    let trimmed = base_url.trim();
    if is_complete {
        trimmed.to_string()
    } else {
        trimmed.trim_end_matches('/').to_string()
    }
}

pub(crate) fn task_settings_from_current(
    app: &AppHandle,
    output_dir: Option<String>,
) -> Result<TaskSettingsSnapshot, String> {
    let settings = read_settings(app)?;
    Ok(TaskSettingsSnapshot {
        output_dir,
        target_language: settings.target_language.trim().to_string(),
        whisper_model_path: settings.whisper_model_path.trim().to_string(),
        whisper_language: normalize_language(&settings.whisper_language),
        base_url: normalize_base_url(&settings.base_url, settings.base_url_is_complete),
        base_url_is_complete: settings.base_url_is_complete,
        model: settings.model.trim().to_string(),
        temperature: settings.temperature.clamp(0.0, 1.0),
        translation_shard_size: normalize_translation_shard_size(settings.translation_shard_size),
        translation_provider: normalize_translation_provider(&settings.translation_provider),
        translation_cli_tool: normalize_translation_cli_tool(&settings.translation_cli_tool),
        translation_cli_command: normalize_translation_cli_command(
            &settings.translation_cli_command,
        ),
        translation_cli_model: normalize_translation_cli_model(&settings.translation_cli_model),
        translation_cli_args: normalize_translation_cli_args(&settings.translation_cli_args),
    })
}

fn read_settings(app: &AppHandle) -> Result<PersistedSettings, String> {
    let path = settings_path(app)?;
    if !path.exists() {
        return Ok(PersistedSettings::default());
    }
    let body = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&body).map_err(|error| error.to_string())
}
fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir.join("settings.json"))
}
fn default_translation_shard_size() -> usize {
    DEFAULT_TRANSLATION_SHARD_SIZE
}
fn default_translation_provider() -> String {
    DEFAULT_TRANSLATION_PROVIDER.to_string()
}
fn default_translation_cli_tool() -> String {
    DEFAULT_TRANSLATION_CLI_TOOL.to_string()
}
fn default_translation_cli_command() -> String {
    DEFAULT_TRANSLATION_CLI_COMMAND.to_string()
}
impl PersistedSettings {
    fn into_response(self, has_api_key: bool) -> SettingsResponse {
        SettingsResponse {
            base_url: self.base_url,
            base_url_is_complete: self.base_url_is_complete,
            model: self.model,
            temperature: self.temperature,
            whisper_model_path: self.whisper_model_path,
            whisper_language: normalize_language(&self.whisper_language),
            target_language: self.target_language,
            translation_shard_size: normalize_translation_shard_size(self.translation_shard_size),
            has_api_key,
            translation_provider: normalize_translation_provider(&self.translation_provider),
            translation_cli_tool: normalize_translation_cli_tool(&self.translation_cli_tool),
            translation_cli_command: normalize_translation_cli_command(
                &self.translation_cli_command,
            ),
            translation_cli_model: normalize_translation_cli_model(&self.translation_cli_model),
            translation_cli_args: normalize_translation_cli_args(&self.translation_cli_args),
        }
    }
}
