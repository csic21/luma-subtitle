use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use serde::Serialize;
use tokio::time::timeout;

use crate::{
    process_utils::hide_tokio_command_window,
    state::{JobError, JobResult},
    subtitles::{SubtitleSegment, TranslatedSegment},
};

use super::{
    normalize_translation_cli_tool,
    parser::parse_translation_content,
    prompt::{shard_translation_prompt, translation_system_prompt},
    TranslationConfig,
};

const CLI_SHARD_TIMEOUT_SECS: u64 = 240;
const MAX_CLI_OUTPUT_CHARS: usize = 20_000;

pub(super) fn cli_display_name(config: &TranslationConfig) -> String {
    let tool = normalize_translation_cli_tool(&config.cli_tool);
    let command = config.cli_command.trim();
    let model = config.cli_model.trim();
    if model.is_empty() {
        if command.is_empty() {
            tool
        } else {
            format!("{tool} ({command})")
        }
    } else if command.is_empty() {
        format!("{tool} {model}")
    } else {
        format!("{tool} {command} {model}")
    }
}

pub(super) fn validate_cli_config(config: &TranslationConfig) -> JobResult<()> {
    if config.cli_command.trim().is_empty() {
        return Err(JobError::failed("CLI 翻译缺少可执行命令，请在设置中填写"));
    }
    if normalize_translation_cli_tool(&config.cli_tool) == "opencode"
        && config.cli_model.trim().is_empty()
    {
        return Err(JobError::failed(
            "CLI 翻译缺少模型，请填写 opencode 模型（例如 provider/model）",
        ));
    }
    Ok(())
}

pub(super) async fn translate_shard_via_cli(
    config: &TranslationConfig,
    shard: &[SubtitleSegment],
    shard_index: usize,
    total_shards: usize,
    cancel: Arc<AtomicBool>,
) -> JobResult<Vec<TranslatedSegment>> {
    if cancel.load(Ordering::SeqCst) {
        return Err(JobError::Cancelled);
    }
    validate_cli_config(config)?;
    let full_prompt = build_cli_prompt(config, shard, shard_index, total_shards);
    let raw_output = if normalize_translation_cli_tool(&config.cli_tool) == "custom" {
        run_custom_cli(config, &full_prompt, cancel).await?
    } else {
        run_opencode_cli(config, &full_prompt, cancel).await?
    };
    parse_translation_content(&raw_output, shard).map_err(|error| match error {
        JobError::Cancelled => JobError::Cancelled,
        JobError::Failed(message) => JobError::failed(format!(
            "{message}\n\nCLI 输出（{} 字符）：\n{}",
            raw_output.chars().count(),
            preview_cli_output(&raw_output),
        )),
    })
}

pub(super) fn build_cli_prompt(
    config: &TranslationConfig,
    shard: &[SubtitleSegment],
    shard_index: usize,
    total_shards: usize,
) -> String {
    format!(
        "{}\n\n{}",
        translation_system_prompt(),
        shard_translation_prompt(&config.target_language, shard, shard_index, total_shards)
    )
}

async fn run_opencode_cli(
    config: &TranslationConfig,
    full_prompt: &str,
    cancel: Arc<AtomicBool>,
) -> JobResult<String> {
    let command = config.cli_command.trim();
    let model = config.cli_model.trim();
    let mut cmd = tokio::process::Command::new(command);
    hide_tokio_command_window(&mut cmd);
    cmd.args(["run", "-m", model, "--format", "json", full_prompt]);
    let output = run_cli_command(&mut cmd, cancel)
        .await
        .map_err(|error| match error {
            JobError::Cancelled => JobError::Cancelled,
            JobError::Failed(message) => {
                JobError::failed(format!("opencode CLI 调用失败: {message}"))
            }
        })?;
    if output.is_empty() {
        return Err(JobError::failed(
            "opencode CLI 没有返回可用输出（--format json 为空）",
        ));
    }
    let text = extract_opencode_text_output(&output);
    if text.trim().is_empty() {
        return Err(JobError::failed(format!(
            "opencode CLI 没有返回文本内容，原始输出（{} 字符）：\n{}",
            output.chars().count(),
            preview_cli_output(&output),
        )));
    }
    Ok(text)
}

async fn run_custom_cli(
    config: &TranslationConfig,
    full_prompt: &str,
    cancel: Arc<AtomicBool>,
) -> JobResult<String> {
    let command = config.cli_command.trim();
    let args = build_custom_args(config, full_prompt);
    let mut cmd = tokio::process::Command::new(command);
    hide_tokio_command_window(&mut cmd);
    cmd.args(&args);
    let output = run_cli_command(&mut cmd, cancel)
        .await
        .map_err(|error| match error {
            JobError::Cancelled => JobError::Cancelled,
            JobError::Failed(message) => {
                JobError::failed(format!("自定义 CLI 调用失败: {message}"))
            }
        })?;
    if output.trim().is_empty() {
        return Err(JobError::failed("自定义 CLI 没有返回可用输出"));
    }
    Ok(output)
}

fn build_custom_args(config: &TranslationConfig, full_prompt: &str) -> Vec<String> {
    let template = config.cli_args.trim();
    if template.is_empty() {
        return vec![full_prompt.to_string()];
    }
    let mut args = split_cli_args(template);
    let mut replaced = false;
    for arg in args.iter_mut() {
        if arg.contains("{prompt}") {
            *arg = arg.replace("{prompt}", full_prompt);
            replaced = true;
        }
        if arg.contains("{model}") {
            *arg = arg.replace("{model}", config.cli_model.trim());
            replaced = true;
        }
        if arg.contains("{target}") {
            *arg = arg.replace("{target}", config.target_language.trim());
            replaced = true;
        }
    }
    if !replaced {
        args.push(full_prompt.to_string());
    }
    args
}

pub(crate) fn split_cli_args(template: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = template.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut has_content = false;

    while let Some(ch) = chars.next() {
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                current.push(ch);
                has_content = true;
            }
            continue;
        }
        if in_double {
            if ch == '"' {
                in_double = false;
            } else if ch == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
                has_content = true;
            } else {
                current.push(ch);
                has_content = true;
            }
            continue;
        }
        match ch {
            '\'' => {
                in_single = true;
                has_content = true;
            }
            '"' => {
                in_double = true;
                has_content = true;
            }
            c if c.is_whitespace() => {
                if has_content {
                    args.push(std::mem::take(&mut current));
                    has_content = false;
                }
            }
            _ => {
                current.push(ch);
                has_content = true;
            }
        }
    }
    if has_content {
        args.push(current);
    }
    args
}

async fn run_cli_command(
    cmd: &mut tokio::process::Command,
    cancel: Arc<AtomicBool>,
) -> JobResult<String> {
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| JobError::failed(format!("启动 CLI 失败: {error}")))?;
    let output = timeout(
        Duration::from_secs(CLI_SHARD_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| {
        let _ = child.start_kill();
        JobError::failed(format!("CLI 调用超时（{CLI_SHARD_TIMEOUT_SECS}s）"))
    })
    .map_err(|error| match error {
        JobError::Cancelled => JobError::Cancelled,
        other => other,
    })?
    .map_err(|error| JobError::failed(format!("等待 CLI 结束失败: {error}")))?;
    if cancel.load(Ordering::SeqCst) {
        return Err(JobError::Cancelled);
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else if !stdout.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            format!("exit {}", output.status)
        };
        return Err(JobError::failed(format!(
            "CLI 返回非零状态 {}: {}",
            output.status,
            preview_cli_output(&detail),
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub(crate) fn extract_opencode_text_output(stdout: &str) -> String {
    let mut collected = String::new();
    let mut fallback_text = String::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if event_type == "text" {
            if let Some(text) = value
                .get("part")
                .and_then(|part| part.get("text"))
                .and_then(|text| text.as_str())
            {
                collected.push_str(text);
            } else if let Some(text) = value.get("text").and_then(|text| text.as_str()) {
                collected.push_str(text);
            }
            continue;
        }
        // Some opencode versions emit message parts differently; keep a fallback.
        if event_type.is_empty() {
            if let Some(text) = value
                .get("part")
                .and_then(|part| part.get("text"))
                .and_then(|text| text.as_str())
            {
                fallback_text.push_str(text);
            }
        }
    }
    if !collected.is_empty() {
        return collected;
    }
    if !fallback_text.is_empty() {
        return fallback_text;
    }
    // If the CLI already outputs raw text/JSON (no JSONL envelope), return as-is.
    stdout.trim().to_string()
}

fn preview_cli_output(output: &str) -> String {
    let char_count = output.chars().count();
    if char_count <= MAX_CLI_OUTPUT_CHARS {
        return output.trim().to_string();
    }
    let head = output
        .chars()
        .take(MAX_CLI_OUTPUT_CHARS / 2)
        .collect::<String>();
    let tail = output
        .chars()
        .skip(char_count.saturating_sub(MAX_CLI_OUTPUT_CHARS / 2))
        .collect::<String>();
    format!(
        "{head}\n\n... 中间省略 {} 字符 ...\n\n{tail}",
        char_count - MAX_CLI_OUTPUT_CHARS
    )
}

// --- Tauri commands for CLI detection ---

#[derive(Serialize)]
pub(crate) struct TranslationCliStatus {
    available: bool,
    path: Option<String>,
    version: Option<String>,
    error: Option<String>,
}

#[tauri::command]
pub(crate) async fn check_translation_cli(
    command: String,
    tool: Option<String>,
) -> Result<TranslationCliStatus, String> {
    let command = command.trim().to_string();
    if command.is_empty() {
        return Ok(TranslationCliStatus {
            available: false,
            path: None,
            version: None,
            error: Some("请先填写 CLI 命令".to_string()),
        });
    }
    let tool = tool.unwrap_or_else(|| "opencode".to_string());
    tauri::async_runtime::spawn_blocking(move || check_cli_blocking(&command, &tool))
        .await
        .map_err(|error| format!("检测 CLI 失败: {error}"))
}

fn check_cli_blocking(command: &str, tool: &str) -> Result<TranslationCliStatus, String> {
    let path = which::which(command)
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    let version_args: Vec<&str> = if normalize_translation_cli_tool(tool) == "custom" {
        vec!["--version"]
    } else {
        vec!["--version"]
    };
    let mut cmd = std::process::Command::new(command);
    #[cfg(not(target_os = "macos"))]
    crate::process_utils::hide_std_command_window(&mut cmd);
    let output = cmd.args(&version_args).output();
    match output {
        Ok(output) if output.status.success() => {
            let version = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .trim()
            .to_string();
            Ok(TranslationCliStatus {
                available: true,
                path,
                version: if version.is_empty() {
                    None
                } else {
                    Some(version)
                },
                error: None,
            })
        }
        Ok(output) => {
            // Custom CLIs may not support --version; fall back to --help.
            let mut help_cmd = std::process::Command::new(command);
            #[cfg(not(target_os = "macos"))]
            crate::process_utils::hide_std_command_window(&mut help_cmd);
            let help = help_cmd.arg("--help").output();
            if help.map(|o| o.status.success()).unwrap_or(false) {
                return Ok(TranslationCliStatus {
                    available: true,
                    path,
                    version: None,
                    error: None,
                });
            }
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Ok(TranslationCliStatus {
                available: false,
                path,
                version: None,
                error: Some(if stderr.is_empty() {
                    format!("CLI 返回非零状态: {}", output.status)
                } else {
                    stderr.chars().take(500).collect::<String>()
                }),
            })
        }
        Err(error) => Ok(TranslationCliStatus {
            available: false,
            path,
            version: None,
            error: Some(format!("启动 CLI 失败: {error}")),
        }),
    }
}

#[tauri::command]
pub(crate) async fn list_translation_cli_models(command: String) -> Result<Vec<String>, String> {
    let command = command.trim().to_string();
    if command.is_empty() {
        return Err("请先填写 CLI 命令".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || list_opencode_models_blocking(&command))
        .await
        .map_err(|error| format!("读取 CLI 模型失败: {error}"))?
}

fn list_opencode_models_blocking(command: &str) -> Result<Vec<String>, String> {
    let mut cmd = std::process::Command::new(command);
    #[cfg(not(target_os = "macos"))]
    crate::process_utils::hide_std_command_window(&mut cmd);
    let output = cmd
        .arg("models")
        .output()
        .map_err(|error| format!("启动 CLI 失败: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("CLI 返回非零状态: {}", output.status)
        } else {
            stderr.chars().take(1000).collect::<String>()
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut models = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::{build_custom_args, extract_opencode_text_output, split_cli_args};
    use crate::translation::TranslationConfig;

    fn test_config(tool: &str, args: &str) -> TranslationConfig {
        TranslationConfig {
            target_language: "简体中文".to_string(),
            base_url: String::new(),
            base_url_is_complete: false,
            model: String::new(),
            temperature: 0.2,
            shard_size: 200,
            provider: "cli".to_string(),
            cli_tool: tool.to_string(),
            cli_command: "opencode".to_string(),
            cli_model: "provider/model".to_string(),
            cli_args: args.to_string(),
        }
    }

    #[test]
    fn extracts_text_parts_from_opencode_jsonl() {
        let stdout = "{\"type\":\"step_start\"}\n{\"type\":\"text\",\"part\":{\"type\":\"text\",\"text\":\"{\\\"items\\\":[\"}}\n{\"type\":\"text\",\"part\":{\"type\":\"text\",\"text\":\"{\\\"id\\\":1}\"}}\n";
        assert_eq!(
            extract_opencode_text_output(stdout),
            "{\"items\":[{\"id\":1}"
        );
    }

    #[test]
    fn falls_back_to_raw_output_when_not_jsonl() {
        assert_eq!(
            extract_opencode_text_output("{\"items\":[{\"id\":1,\"text\":\"你好\"}]}"),
            "{\"items\":[{\"id\":1,\"text\":\"你好\"}]}"
        );
    }

    #[test]
    fn splits_custom_args_respecting_quotes() {
        assert_eq!(
            split_cli_args("run --prompt \"hello world\" --model 'a b'"),
            vec!["run", "--prompt", "hello world", "--model", "a b"]
        );
    }

    #[test]
    fn builds_custom_args_with_placeholders() {
        let config = test_config("custom", "exec -m {model} --target {target} {prompt}");
        let args = build_custom_args(&config, "PROMPT");
        assert_eq!(
            args,
            vec![
                "exec",
                "-m",
                "provider/model",
                "--target",
                "简体中文",
                "PROMPT"
            ]
        );
    }

    #[test]
    fn appends_prompt_when_no_placeholder() {
        let config = test_config("custom", "--json");
        assert_eq!(
            build_custom_args(&config, "PROMPT"),
            vec!["--json", "PROMPT"]
        );
    }
}
