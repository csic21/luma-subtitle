use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use serde::Serialize;

use crate::{
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

/// Resolve a user-configured CLI command to an executable path.
///
/// Bare names are looked up on `PATH` first, then in well-known install
/// directories per OS. GUI apps (launched from Finder/Dock on macOS, or with
/// a minimal environment on Windows/Linux) don't inherit the shell `PATH`,
/// so e.g. Homebrew installs would otherwise fail with
/// "No such file or directory".
///
/// Note: on Windows only real executables (`.exe`) are considered. npm
/// `.cmd` shims can't be spawned directly via `CreateProcess` and would need
/// `cmd /C` wrapping, so prefer a WinGet/Scoop/installer based install.
pub(crate) fn resolve_cli_path(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    if command.is_empty() {
        return None;
    }
    if command.contains('/') || command.contains('\\') {
        let path = PathBuf::from(command);
        return path.exists().then_some(path);
    }
    if let Ok(path) = which::which(command) {
        return Some(path);
    }
    for dir in cli_search_dirs() {
        for name in cli_candidate_names(command) {
            let path = dir.join(&name);
            if path.exists() {
                return Some(path);
            }
        }
    }
    None
}

/// File names to try inside each search directory. Windows needs the explicit
/// `.exe` variant because a bare name on disk has no extension.
fn cli_candidate_names(command: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        if command.contains('.') {
            vec![command.to_string()]
        } else {
            vec![command.to_string(), format!("{command}.exe")]
        }
    }
    #[cfg(not(windows))]
    {
        vec![command.to_string()]
    }
}

fn cli_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    dirs.extend(
        [
            "/opt/homebrew/bin",
            "/opt/homebrew/sbin",
            "/usr/local/bin",
            "/usr/local/sbin",
            "/opt/local/bin",
        ]
        .iter()
        .map(Path::new)
        .map(Path::to_path_buf),
    );
    #[cfg(target_os = "linux")]
    dirs.extend(
        [
            "/usr/local/bin",
            "/home/linuxbrew/.linuxbrew/bin",
            "/snap/bin",
        ]
        .iter()
        .map(Path::new)
        .map(Path::to_path_buf),
    );
    #[cfg(unix)]
    if let Some(home) = home::home_dir() {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".bun").join("bin"));
        dirs.push(home.join(".cargo").join("bin"));
    }
    #[cfg(windows)]
    {
        if let Some(home) = home::home_dir() {
            // Scoop shims, Cargo installs, WinGet links.
            dirs.push(home.join("scoop").join("shims"));
            dirs.push(home.join(".cargo").join("bin"));
            dirs.push(
                home.join("AppData")
                    .join("Local")
                    .join("Microsoft")
                    .join("WinGet")
                    .join("Links"),
            );
        }
        // Global npm installs (`npm i -g`).
        if let Ok(appdata) = std::env::var("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
        // Chocolatey.
        dirs.push(PathBuf::from(r"C:\ProgramData\chocolatey\bin"));
    }
    dirs
}

fn cli_not_found_message(command: &str) -> String {
    format!(
        "找不到 CLI 命令 `{}`，请检查该命令是否已安装，或填写它的绝对路径",
        command.trim()
    )
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
    let resolved = resolve_cli_path(command)
        .ok_or_else(|| JobError::failed(cli_not_found_message(command)))?;
    let output = super::opencode_isolation::translate(&resolved, model, full_prompt, cancel).await?;
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
    let resolved = resolve_cli_path(command)
        .ok_or_else(|| JobError::failed(cli_not_found_message(command)))?;
    let mut cmd = std::process::Command::new(resolved);
    cmd.args(&args);
    let output = run_cli_command(cmd, cancel)
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

async fn run_cli_command(cmd: std::process::Command, cancel: Arc<AtomicBool>) -> JobResult<String> {
    let output = crate::owned_process::output(cmd, cancel, Duration::from_secs(CLI_SHARD_TIMEOUT_SECS), 8 * 1024 * 1024).await?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        // CLI errors may contain provider tokens. Do not put subprocess output
        // in persisted task logs; users can diagnose the executable separately.
        let _ = detail;
        return Err(JobError::failed(format!("CLI returned nonzero status {}", output.status)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn probe(resolved: &Path, args: &[&str]) -> JobResult<std::process::Output> {
    let mut command = std::process::Command::new(resolved);
    command.args(args);
    crate::owned_process::output(command, Arc::new(AtomicBool::new(false)), Duration::from_secs(10), 64 * 1024).await
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
    let Some(resolved) = resolve_cli_path(&command) else {
        return Ok(TranslationCliStatus { available: false, path: None, version: None, error: Some(cli_not_found_message(&command)) });
    };
    let path = Some(resolved.to_string_lossy().to_string());
    let result = probe(&resolved, &["--version"]).await;
    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if normalize_translation_cli_tool(&tool) == "opencode" {
                if let Err(error) = super::opencode_isolation::validate_version(&version) {
                    return Ok(TranslationCliStatus { available: false, path, version: Some(version), error: Some(error) });
                }
            }
            Ok(TranslationCliStatus { available: true, path, version: Some(version), error: None })
        }
        Ok(_) if normalize_translation_cli_tool(&tool) == "custom" => {
            let ready = probe(&resolved, &["--help"]).await.is_ok_and(|output| output.status.success());
            Ok(TranslationCliStatus { available: ready, path, version: None, error: (!ready).then(|| "CLI probe failed".to_string()) })
        }
        _ => Ok(TranslationCliStatus { available: false, path, version: None, error: Some("CLI probe failed or exceeded its time/output limit".to_string()) }),
    }
}

#[tauri::command]
pub(crate) async fn list_translation_cli_models(command: String) -> Result<Vec<String>, String> {
    let resolved = resolve_cli_path(&command).ok_or_else(|| cli_not_found_message(&command))?;
    let sandbox = super::opencode_isolation::Sandbox::new(None)?;
    let mut version_command = sandbox.command(&resolved);
    version_command.arg("--version");
    let output = crate::owned_process::output(version_command, Arc::new(AtomicBool::new(false)), Duration::from_secs(10), 64 * 1024).await
        .map_err(|_| "CLI version check failed".to_string())?;
    if !output.status.success() { return Err("CLI version check failed".into()); }
    super::opencode_isolation::validate_version(String::from_utf8_lossy(&output.stdout).trim())?;
    let mut command = sandbox.command(&resolved); command.arg("models");
    let output = crate::owned_process::output(command, Arc::new(AtomicBool::new(false)), Duration::from_secs(15), 256 * 1024).await
        .map_err(|_| "CLI model probe failed or exceeded its time/output limit".to_string())?;
    if !output.status.success() { return Err("CLI model probe failed".into()); }
    let mut models = String::from_utf8_lossy(&output.stdout).lines().map(str::trim)
        .filter(|line| !line.is_empty()).map(str::to_string).collect::<Vec<_>>();
    models.sort(); models.dedup(); Ok(models)
}

#[cfg(test)]
mod tests {
    use super::{
        build_custom_args, cli_candidate_names, extract_opencode_text_output, resolve_cli_path,
        split_cli_args,
    };
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
            local_model_path: String::new(),
        }
    }

    #[test]
    fn resolves_existing_path_command_as_is() {
        let exe = std::env::current_exe().expect("test binary should exist");
        assert_eq!(resolve_cli_path(&exe.to_string_lossy()), Some(exe));
    }

    #[test]
    fn returns_none_for_blank_or_missing_commands() {
        assert_eq!(resolve_cli_path("   "), None);
        assert_eq!(
            resolve_cli_path("definitely-not-a-real-luma-cli-binary"),
            None
        );
        assert_eq!(resolve_cli_path("/definitely/not/here/luma-cli"), None);
    }

    #[test]
    fn candidate_names_always_include_bare_command() {
        let names = cli_candidate_names("opencode");
        assert!(names.contains(&"opencode".to_string()));
        #[cfg(windows)]
        assert!(names.contains(&"opencode.exe".to_string()));
        #[cfg(not(windows))]
        assert_eq!(names, vec!["opencode".to_string()]);
    }

    #[test]
    fn resolves_bare_command_found_on_search_dirs() {
        // `opencode` itself may not exist on CI; exercise the lookup with a
        // binary that is virtually always on PATH instead.
        #[cfg(unix)]
        {
            let expected = which::which("sh").expect("sh should be on PATH");
            assert_eq!(resolve_cli_path("sh"), Some(expected));
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

#[cfg(all(test, unix))]
mod process_tests {
    use super::*;

    #[test]
    fn cancellation_kills_cli_and_joins_readers_promptly() {
        tauri::async_runtime::block_on(async {
            let cancel = Arc::new(AtomicBool::new(false));
            let marker =
                std::env::temp_dir().join(format!("luma-cli-cancel-{}", uuid::Uuid::new_v4()));
            let mut command = std::process::Command::new("sh");
            command
                .args(["-c", "echo $$ > \"$1\"; exec sleep 30", "sh"])
                .arg(&marker);
            let trigger = cancel.clone();
            let task = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                trigger.store(true, Ordering::SeqCst);
            });
            let start = Instant::now();
            let result = run_cli_command(&mut command, cancel).await;
            assert!(matches!(result, Err(JobError::Cancelled)));
            assert!(start.elapsed() < Duration::from_secs(2));
            task.await.unwrap();
            let pid = std::fs::read_to_string(&marker).unwrap();
            let alive = std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success();
            let _ = std::fs::remove_file(marker);
            assert!(
                !alive,
                "CLI process must be reaped before cancellation returns"
            );
        });
    }

    #[test]
    fn drains_both_cli_pipes_while_waiting_for_exit() {
        tauri::async_runtime::block_on(async {
            let mut command = std::process::Command::new("sh");
            command.args([
                "-c",
                "head -c 262144 /dev/zero; head -c 262144 /dev/zero >&2",
            ]);
            let result = tokio::time::timeout(
                Duration::from_secs(3),
                run_cli_command(&mut command, Arc::new(AtomicBool::new(false))),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(result.len(), 262144);
        });
    }
}
