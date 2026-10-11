use std::{
    collections::VecDeque,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use serde::Deserialize;
use serde_json::json;
use tauri::AppHandle;
use tokio::{io::AsyncReadExt, process::Child, task::JoinSet, time::sleep};

use crate::{
    job_events::{publish_job_event, JobEventDraft},
    paths::{is_existing_file, locate_binary, path_to_string},
    process_utils::hide_tokio_command_window,
    state::{ensure_not_cancelled, JobError, JobResult},
    subtitles::{collapse_repeated_vocalization, SubtitleSegment, TranslatedSegment},
};

use super::{
    parser::parse_delimited_translation_content, runtime::cancellable, TranslationConfig,
    DEFAULT_LOCAL_TRANSLATION_SHARD_SIZE, MAX_LOCAL_TRANSLATION_SHARD_SIZE,
};

const LOCAL_CUE_DELIMITER: &str = "<<<CUE>>>";
const SERVER_START_TIMEOUT_SECS: u64 = 120;
const SHARD_TIMEOUT_SECS: u64 = 600;
const STARTUP_STDERR_BYTES: usize = 16 * 1024;
const STARTUP_STDERR_LINE_BYTES: usize = 4096;
const STARTUP_DIAGNOSTIC_BYTES: usize = 4096;

/// Keep only complete, sanitized startup lines. Oversized lines are discarded
/// whole, so clipping cannot turn a secret into an unrecognizable suffix.
struct StartupStderr {
    lines: VecDeque<String>,
    retained_bytes: usize,
    pending: Vec<u8>,
    oversized_line: bool,
    capturing: bool,
    model_strings: Vec<String>,
}

impl StartupStderr {
    fn new(model_path: &Path) -> Self {
        let model = model_path.to_string_lossy().into_owned();
        let mut model_strings = vec![
            model.clone(),
            model.replace('\\', "\\\\"),
            model.replace('\\', "/"),
        ];
        if let Some(name) = model.rsplit(['/', '\\']).next() {
            model_strings.push(name.to_string());
        }
        model_strings.retain(|value| !value.is_empty());
        // Replace full paths before their file names.
        model_strings.sort_by_key(|value| std::cmp::Reverse(value.len()));
        model_strings.dedup();
        Self {
            lines: VecDeque::new(),
            retained_bytes: 0,
            pending: Vec::new(),
            oversized_line: false,
            capturing: true,
            model_strings,
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        if !self.capturing {
            return;
        }
        for &byte in bytes {
            if byte == b'\n' {
                self.finish_line();
            } else if !self.oversized_line {
                if self.pending.len() == STARTUP_STDERR_LINE_BYTES {
                    self.pending.clear();
                    self.oversized_line = true;
                } else {
                    self.pending.push(byte);
                }
            }
        }
    }

    fn finish_line(&mut self) {
        let line = if self.oversized_line {
            "[oversized stderr line omitted]".to_string()
        } else {
            sanitize_startup_line(&String::from_utf8_lossy(&self.pending), &self.model_strings)
        };
        self.pending.clear();
        self.oversized_line = false;
        if line.is_empty() || !self.capturing {
            return;
        }
        // UTF-8 replacement characters can expand invalid input. Bound the
        // sanitized representation as well as the raw input.
        let line = if line.len() > STARTUP_STDERR_LINE_BYTES {
            "[oversized stderr line omitted]".to_string()
        } else {
            line
        };
        let line = format!("{line}\n");
        while self.retained_bytes + line.len() > STARTUP_STDERR_BYTES {
            if let Some(oldest) = self.lines.pop_front() {
                self.retained_bytes -= oldest.len();
            } else {
                break;
            }
        }
        self.retained_bytes += line.len();
        self.lines.push_back(line);
    }

    fn diagnostic(&mut self) -> String {
        self.finish_line();
        let text = self.lines.iter().map(String::as_str).collect::<String>();
        if text.len() <= STARTUP_DIAGNOSTIC_BYTES {
            return text.trim().to_string();
        }
        let minimum = text.len() - (STARTUP_DIAGNOSTIC_BYTES - 3);
        let start = text
            .char_indices()
            .find(|(index, _)| *index >= minimum)
            .map(|(index, _)| index)
            .unwrap_or(text.len());
        format!("...{}", text[start..].trim())
    }

    fn stop_capture(&mut self) {
        self.capturing = false;
        self.lines.clear();
        self.retained_bytes = 0;
        self.pending.clear();
        self.model_strings.clear();
    }
}

fn strip_diagnostic_controls(text: &str) -> String {
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        let escape = if ch == '\u{1b}' { chars.next() } else { Some(ch) };
        match escape {
            Some('[') if ch == '\u{1b}' => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) { break; }
                }
            }
            Some('\u{9b}') => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) { break; }
                }
            }
            Some(']' | 'P' | 'X' | '^' | '_') if ch == '\u{1b}' => {
                while let Some(next) = chars.next() {
                    if next == '\u{7}' || (next == '\u{1b}' && chars.next() == Some('\\')) {
                        break;
                    }
                }
            }
            Some('\u{9d}' | '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}') => {
                while let Some(next) = chars.next() {
                    if next == '\u{7}' || next == '\u{9c}'
                        || (next == '\u{1b}' && chars.next() == Some('\\')) {
                        break;
                    }
                }
            }
            _ if ch == '\u{1b}' => {}
            Some('\t') => output.push(' '),
            Some(value) if !value.is_control()
                && !matches!(value, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}' | '\u{feff}') => output.push(value),
            _ => {}
        }
    }
    output
}

fn sanitize_startup_line(line: &str, model_strings: &[String]) -> String {
    let mut line = strip_diagnostic_controls(line);
    let lower = line.to_ascii_lowercase();
    // Model metadata and templates are arbitrary user-controlled text, and
    // URLs can carry signed query parameters. They are not startup diagnostics.
    if ["://", "www.", "general.", "tokenizer.", "chat_template", "chat template",
        "metadata", " - kv ", " - type ", " str ="]
        .iter().any(|marker| lower.contains(*marker))
    {
        return String::new();
    }
    let compact = lower.chars().filter(|ch| ch.is_ascii_alphanumeric()).collect::<String>();
    if ["authorization", "apikey", "accesstoken", "refreshtoken", "password", "bearer", "secret"]
        .iter().any(|marker| compact.contains(*marker))
        || ["sk-", "hf_", "token=", "token:", "token\"", "token'", "_token", "-token"]
            .iter().any(|marker| lower.contains(*marker))
        || lower.split(|ch: char| !ch.is_ascii_alphanumeric()).any(|word| word == "token")
    {
        return "[credential-related stderr omitted]".to_string();
    }
    for model in model_strings {
        line = line.replace(model.as_str(), "[model]");
    }
    // Unrecognized paths may contain personal directory names. Known model
    // paths above keep the useful error text; omit other path-bearing lines.
    let contains_path = line.split(|ch: char| {
        ch.is_whitespace() || matches!(ch, '\'' | '"' | '(' | ')' | '=' | '[' | ']' | ',' | ';')
    }).any(|word| {
        let bytes = word.as_bytes();
        word.starts_with('/') || word.starts_with('\\')
            || word.to_ascii_lowercase().contains(".gguf")
            || (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':' && matches!(bytes[2], b'/' | b'\\'))
    });
    if contains_path {
        "[path-related stderr omitted]".to_string()
    } else {
        // Keep known failure/backend categories, not the startup inventory or
        // unstructured template continuation text. Never retain model prompts.
        let structured = ["ggml_", "llama_", "common_", "load_", "main:", "srv ",
            "error", "warning", "failed to", "loading ", "cuda", "vulkan", "insufficient"]
            .iter().any(|prefix| lower.trim_start().starts_with(*prefix));
        let failure = ["error", "failed", "failure", "cannot", "could not", "unable",
            "invalid", "unsupported", "not found", "missing", "out of memory", "insufficient",
            "allocation", "allocating"]
            .iter().any(|marker| lower.contains(*marker));
        if structured && failure {
            line.trim().to_string()
        } else {
            String::new()
        }
    }
}

fn diagnostic_exit_status(status: std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code} (0x{:08X})", code as u32),
        None => status.to_string(),
    }
}

struct LocalLlamaServer {
    child: Child,
    base_url: String,
    readers: JoinSet<()>,
    stderr: Arc<parking_lot::Mutex<StartupStderr>>,
}

impl LocalLlamaServer {
    fn new(mut child: Child, base_url: String, model_path: &Path) -> Self {
        let mut readers = JoinSet::new();
        let diagnostics = Arc::new(parking_lot::Mutex::new(StartupStderr::new(model_path)));
        if let Some(mut stdout) = child.stdout.take() {
            readers.spawn(async move {
                let _ = tokio::io::copy(&mut stdout, &mut tokio::io::sink()).await;
            });
        }
        if let Some(mut stderr) = child.stderr.take() {
            let diagnostics = diagnostics.clone();
            readers.spawn(async move {
                let mut chunk = [0u8; 2048];
                loop {
                    match stderr.read(&mut chunk).await {
                        Ok(0) | Err(_) => {
                            diagnostics.lock().finish_line();
                            break;
                        }
                        Ok(count) => diagnostics.lock().append(&chunk[..count]),
                    }
                }
            });
        }
        Self {
            child,
            base_url,
            readers,
            stderr: diagnostics,
        }
    }

    async fn shutdown(&mut self) {
        // kill() also waits/reaps. Briefly drain final startup errors, but an
        // inherited pipe handle must not hold cancellation open indefinitely.
        let _ = self.child.kill().await;
        let _ = tokio::time::timeout(Duration::from_millis(200), async {
            while self.readers.join_next().await.is_some() {}
        }).await;
        self.readers.shutdown().await;
    }

    fn startup_error(&mut self, error: JobError) -> JobError {
        match error {
            JobError::Cancelled => JobError::Cancelled,
            JobError::Failed(mut message) => {
                if let Ok(Some(status)) = self.child.try_wait() {
                    message.push_str(&format!("\n状态: {}", diagnostic_exit_status(status)));
                }
                let diagnostic = self.stderr.lock().diagnostic();
                if !diagnostic.is_empty() {
                    message.push_str(&format!("\nllama-server stderr:\n{diagnostic}"));
                }
                JobError::failed(message)
            }
        }
    }
}

impl Drop for LocalLlamaServer {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

pub(super) async fn translate_shards_via_local(
    app: &AppHandle,
    job_id: &str,
    config: &TranslationConfig,
    all_segments: &[SubtitleSegment],
    cancel: Arc<AtomicBool>,
    progress: super::TranslationProgress,
) -> JobResult<Vec<TranslatedSegment>> {
    let remaining = progress.remaining_owned();
    if remaining.is_empty() {
        return Ok(progress.into_completed());
    }

    let model_path = validate_local_model_path(&config.local_model_path)?;
    let llama_server = locate_binary(app, "llama-server").ok_or_else(|| {
        JobError::failed("未找到 llama-server。请在设置里安装翻译引擎（llama.cpp）")
    })?;
    let shard_size = normalize_local_shard_size(config.shard_size);
    publish_job_event(
        app,
        JobEventDraft::running(
            job_id,
            "translate-shards",
            format!("正在启动本地翻译引擎（每片 {shard_size} 条字幕，顺序翻译）"),
            super::cue_progress(progress.completed_count(), all_segments.len()),
        ),
    );

    let mut server = start_llama_server(&llama_server, &model_path, cancel.clone()).await?;
    let result = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(SHARD_TIMEOUT_SECS))
            .build()
            .map_err(|error| JobError::failed(format!("创建本地翻译客户端失败: {error}")))?;

        let shards = remaining
            .chunks(shard_size)
            .map(|chunk| chunk.to_vec())
            .collect::<Vec<_>>();
        let total_shards = shards.len().max(1);
        let total_cues = all_segments.len();

        for (index, shard) in shards.iter().enumerate() {
            ensure_not_cancelled(&cancel)?;
            let shard_index = index + 1;
            publish_job_event(
                app,
                JobEventDraft::running(
                    job_id,
                    "translate-shard",
                    format!(
                        "本地分片 {shard_index}/{total_shards} 翻译中（{} 条字幕）",
                        shard.len()
                    ),
                    super::cue_progress(progress.completed_count(), total_cues),
                ),
            );
            let items = translate_shard_via_local(
                &client,
                &server.base_url,
                config,
                shard,
                shard_index,
                total_shards,
                cancel.clone(),
            )
            .await
            .map_err(|error| prefix_shard_error(error, shard_index, total_shards))?;
            progress.append_and_persist(items)?;
            publish_job_event(
                app,
                JobEventDraft::running(
                    job_id,
                    "translate-shard",
                    format!(
                        "本地分片 {shard_index}/{total_shards} 已完成（累计 {}/{total_cues}）",
                        progress.completed_count()
                    ),
                    super::cue_progress(progress.completed_count(), total_cues),
                ),
            );
        }

        Ok(progress.into_completed())
    }
    .await;
    server.shutdown().await;
    result
}

pub(crate) fn normalize_local_shard_size(size: usize) -> usize {
    if size == 0 || size > MAX_LOCAL_TRANSLATION_SHARD_SIZE {
        DEFAULT_LOCAL_TRANSLATION_SHARD_SIZE
    } else {
        size
    }
}

pub(crate) fn hy_mt2_target_language(target: &str) -> String {
    match target.trim() {
        "简体中文" => "简体中文".to_string(),
        "繁体中文" => "繁体中文".to_string(),
        "English" | "英语" | "英文" => "英语".to_string(),
        "日本語" | "日语" | "日文" => "日语".to_string(),
        "한국어" | "韩语" | "韩文" => "韩语".to_string(),
        "Deutsch" | "德语" => "德语".to_string(),
        "Français" | "Francais" | "法语" => "法语".to_string(),
        "Español" | "Espanol" | "西班牙语" => "西班牙语".to_string(),
        other if other.is_empty() => "简体中文".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn local_translation_prompt(
    target_language: &str,
    segments: &[SubtitleSegment],
) -> String {
    let target = hy_mt2_target_language(target_language);
    let texts = segments
        .iter()
        .map(|segment| collapse_repeated_vocalization(&segment.text))
        .collect::<Vec<_>>();
    if texts.len() <= 1 {
        return format!(
            "将以下文本翻译为{target}，注意只需要输出翻译后的结果，不要额外解释：\n\n{}",
            texts.first().cloned().unwrap_or_default()
        );
    }
    format!(
        "请将以下文本准确翻译为{target}。\n\
        你必须在译文中保留等量的分隔符 {LOCAL_CUE_DELIMITER}，绝对不可遗漏、转义或翻译该符号，并注意分隔符的位置。\n\
        只输出翻译后的结果，不要额外解释。\n\n{}",
        texts.join(LOCAL_CUE_DELIMITER)
    )
}

fn validate_local_model_path(path: &str) -> JobResult<PathBuf> {
    let path = PathBuf::from(path.trim());
    if path.as_os_str().is_empty() {
        return Err(JobError::failed(
            "本地翻译缺少模型文件，请在设置中下载或选择 Hy-MT2 GGUF",
        ));
    }
    if !is_existing_file(&path) {
        return Err(JobError::failed(format!(
            "本地翻译模型不存在: {}",
            path_to_string(path)
        )));
    }
    Ok(path)
}

async fn start_llama_server(
    binary: &Path,
    model_path: &Path,
    cancel: Arc<AtomicBool>,
) -> JobResult<LocalLlamaServer> {
    let port = unused_localhost_port()?;
    let ngl = gpu_offload_layers(binary);
    let threads = std::thread::available_parallelism()
        .map(|count| count.get().saturating_sub(1).clamp(2, 8).to_string())
        .unwrap_or_else(|_| "4".to_string());
    let mut command = tokio::process::Command::new(binary);
    hide_tokio_command_window(&mut command);
    if let Some(dir) = binary.parent() {
        command.current_dir(dir);
        prepend_path_env(&mut command, dir);
    }
    command
        .arg("-m")
        .arg(model_path)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--jinja")
        .arg("-c")
        .arg("4096")
        .arg("-ngl")
        .arg(ngl.to_string())
        .arg("-t")
        .arg(threads)
        .arg("--parallel")
        .arg("1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    launch_llama_server(command, format!("http://127.0.0.1:{port}"), model_path, cancel).await
}

async fn launch_llama_server(
    mut command: tokio::process::Command,
    base_url: String,
    model_path: &Path,
    cancel: Arc<AtomicBool>,
) -> JobResult<LocalLlamaServer> {
    ensure_not_cancelled(&cancel)?;
    let child = command
        .spawn()
        .map_err(|error| JobError::failed(format!("启动 llama-server 失败: {error}")))?;
    let mut server = LocalLlamaServer::new(child, base_url, model_path);
    if let Err(error) = wait_for_server(&server.base_url, &mut server.child, cancel).await {
        server.shutdown().await;
        return Err(server.startup_error(error));
    }
    // Runtime output can include user text. Keep draining both pipes without
    // retaining anything after startup, and never restart a partially run job.
    server.stderr.lock().stop_capture();
    Ok(server)
}

async fn wait_for_server(
    base_url: &str,
    child: &mut Child,
    cancel: Arc<AtomicBool>,
) -> JobResult<()> {
    let health = format!("{base_url}/health");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|error| JobError::failed(format!("创建健康检查客户端失败: {error}")))?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(SERVER_START_TIMEOUT_SECS);
    loop {
        ensure_not_cancelled(&cancel)?;
        if let Some(status) = child
            .try_wait()
            .map_err(|error| JobError::failed(format!("读取 llama-server 状态失败: {error}")))?
        {
            return Err(JobError::failed(format!(
                "llama-server 在模型加载完成前退出: {status}"
            )));
        }
        let response = cancellable(&cancel, async { Ok(client.get(&health).send().await) }).await?;
        if response.is_ok_and(|response| response.status().is_success()) {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = child.start_kill();
            return Err(JobError::failed(
                "llama-server 启动超时。请确认模型文件完整，并检查本机内存是否足够",
            ));
        }
        cancellable(&cancel, async {
            sleep(Duration::from_millis(400)).await;
            Ok(())
        })
        .await?;
    }
}

async fn translate_shard_via_local(
    client: &reqwest::Client,
    base_url: &str,
    config: &TranslationConfig,
    shard: &[SubtitleSegment],
    shard_index: usize,
    total_shards: usize,
    cancel: Arc<AtomicBool>,
) -> JobResult<Vec<TranslatedSegment>> {
    let content = translate_local_content(client, base_url, config, shard, &cancel).await?;
    match parse_delimited_translation_content(&content, shard) {
        Ok(items) => Ok(items),
        Err(JobError::Cancelled) => Err(JobError::Cancelled),
        Err(error) => {
            if shard.len() <= 1 {
                return Err(error);
            }
            let mut items = Vec::with_capacity(shard.len());
            for (offset, segment) in shard.iter().enumerate() {
                ensure_not_cancelled(&cancel)?;
                let one = translate_shard_once(
                    client,
                    base_url,
                    config,
                    std::slice::from_ref(segment),
                    &cancel,
                )
                .await
                .map_err(|inner| match inner {
                    JobError::Cancelled => JobError::Cancelled,
                    JobError::Failed(message) => JobError::failed(format!(
                        "分片 {shard_index}/{total_shards} 第 {} 条回退翻译失败: {message}",
                        offset + 1
                    )),
                })?;
                items.extend(one);
            }
            Ok(items)
        }
    }
}

async fn translate_shard_once(
    client: &reqwest::Client,
    base_url: &str,
    config: &TranslationConfig,
    shard: &[SubtitleSegment],
    cancel: &Arc<AtomicBool>,
) -> JobResult<Vec<TranslatedSegment>> {
    let content = translate_local_content(client, base_url, config, shard, cancel).await?;
    parse_delimited_translation_content(&content, shard)
}

async fn translate_local_content(
    client: &reqwest::Client,
    base_url: &str,
    config: &TranslationConfig,
    shard: &[SubtitleSegment],
    cancel: &Arc<AtomicBool>,
) -> JobResult<String> {
    let prompt = local_translation_prompt(&config.target_language, shard);
    let payload = json!({
        "model": "local",
        "temperature": config.temperature.clamp(0.0, 0.4),
        "top_p": 0.6,
        "top_k": 20,
        "repeat_penalty": 1.05,
        "max_tokens": 4096,
        "messages": [{ "role": "user", "content": prompt }]
    });
    let (status, body) = super::client::send_chat_request(
        client,
        &format!("{base_url}/v1/chat/completions"),
        None,
        &payload,
        cancel,
    )
    .await?;
    if !status.is_success() {
        return Err(JobError::failed(format!(
            "本地翻译失败: HTTP {status}: {}",
            trim_error_body(&body)
        )));
    }
    let chat: ChatResponse = serde_json::from_str(&body)
        .map_err(|error| JobError::failed(format!("本地翻译响应解析失败: {error}")))?;
    let content = chat
        .choices
        .first()
        .map(|choice| choice.message.content.trim())
        .filter(|content| !content.is_empty())
        .ok_or_else(|| JobError::failed("本地翻译没有返回文本"))?;
    Ok(content.to_string())
}

fn unused_localhost_port() -> JobResult<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| JobError::failed(format!("分配本地端口失败: {error}")))?;
    let port = listener
        .local_addr()
        .map_err(|error| JobError::failed(format!("读取本地端口失败: {error}")))?
        .port();
    Ok(port)
}

fn gpu_offload_layers(binary: &Path) -> i32 {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return 99;
    }
    let Some(dir) = binary.parent() else {
        return 0;
    };
    let names = [
        "ggml-cuda.dll",
        "ggml-vulkan.dll",
        "ggml-cuda.dylib",
        "ggml-metal.dylib",
        "ggml-cuda.so",
        "ggml-vulkan.so",
    ];
    if names.iter().any(|name| dir.join(name).exists()) {
        99
    } else {
        0
    }
}

fn prepend_path_env(command: &mut tokio::process::Command, dir: &Path) {
    let dir = path_to_string(dir.to_path_buf());
    let key = if cfg!(windows) { "Path" } else { "PATH" };
    let separator = if cfg!(windows) { ";" } else { ":" };
    let current = std::env::var_os(key).unwrap_or_default();
    let mut value = std::ffi::OsString::from(&dir);
    value.push(separator);
    value.push(current);
    command.env(key, value);
}

fn prefix_shard_error(error: JobError, shard_index: usize, total_shards: usize) -> JobError {
    match error {
        JobError::Cancelled => JobError::Cancelled,
        JobError::Failed(message) => {
            JobError::failed(format!("分片 {shard_index}/{total_shards} 失败: {message}"))
        }
    }
}

fn trim_error_body(body: &str) -> String {
    const MAX_ERROR_BODY: usize = 1_500;
    let trimmed = body.trim();
    if trimmed.chars().count() <= MAX_ERROR_BODY {
        return trimmed.to_string();
    }
    let preview = trimmed.chars().take(MAX_ERROR_BODY).collect::<String>();
    format!("{preview}...")
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: String,
}

#[cfg(test)]
mod tests {
    use super::{hy_mt2_target_language, local_translation_prompt, normalize_local_shard_size};
    use crate::subtitles::SubtitleSegment;
    use crate::translation::{
        DEFAULT_LOCAL_TRANSLATION_SHARD_SIZE, MAX_LOCAL_TRANSLATION_SHARD_SIZE,
    };

    fn segment(id: usize, text: &str) -> SubtitleSegment {
        SubtitleSegment {
            id,
            start_ms: 0,
            end_ms: 1000,
            text: text.to_string(),
        }
    }

    #[test]
    fn maps_app_language_labels_to_hy_mt2_names() {
        assert_eq!(hy_mt2_target_language("简体中文"), "简体中文");
        assert_eq!(hy_mt2_target_language("English"), "英语");
        assert_eq!(hy_mt2_target_language("日本語"), "日语");
        assert_eq!(hy_mt2_target_language("한국어"), "韩语");
        assert_eq!(hy_mt2_target_language("Deutsch"), "德语");
        assert_eq!(hy_mt2_target_language("Français"), "法语");
        assert_eq!(hy_mt2_target_language("Español"), "西班牙语");
        assert_eq!(hy_mt2_target_language("繁体中文"), "繁体中文");
    }

    #[test]
    fn clamps_oversized_local_shards_to_the_default_batch() {
        assert_eq!(
            normalize_local_shard_size(200),
            DEFAULT_LOCAL_TRANSLATION_SHARD_SIZE
        );
        assert_eq!(normalize_local_shard_size(8), 8);
        assert_eq!(
            normalize_local_shard_size(MAX_LOCAL_TRANSLATION_SHARD_SIZE),
            MAX_LOCAL_TRANSLATION_SHARD_SIZE
        );
    }

    #[test]
    fn builds_delimited_prompt_for_multi_cue_shards() {
        let prompt = local_translation_prompt("English", &[segment(1, "你好"), segment(2, "世界")]);
        assert!(prompt.contains("英语"));
        assert!(prompt.contains("<<<CUE>>>"));
        assert!(prompt.contains("你好<<<CUE>>>世界"));
        assert!(!prompt.contains("JSON"));
    }

    #[test]
    fn builds_simple_prompt_for_a_single_cue() {
        let prompt = local_translation_prompt("简体中文", &[segment(1, "Hello")]);
        assert!(prompt.contains("Hello"));
        assert!(!prompt.contains("<<<CUE>>>"));
    }
}

#[cfg(all(test, unix))]
mod process_tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn cancelled_startup_reaps_server_and_joins_pipe_readers() {
        tauri::async_runtime::block_on(async {
            let cancel = Arc::new(AtomicBool::new(false));
            let marker =
                std::env::temp_dir().join(format!("luma-local-cancel-{}", uuid::Uuid::new_v4()));
            let mut command = tokio::process::Command::new("sh");
            command
                .args(["-c", "echo $$ > \"$1\"; exec sleep 30", "sh"])
                .arg(&marker)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            let trigger = cancel.clone();
            let task = tokio::spawn(async move {
                sleep(Duration::from_millis(100)).await;
                trigger.store(true, Ordering::SeqCst);
            });
            let start = tokio::time::Instant::now();
            let result = launch_llama_server(
                command,
                "http://127.0.0.1:1".to_string(),
                Path::new("fixture.gguf"),
                cancel,
            ).await;
            assert!(matches!(result, Err(JobError::Cancelled)));
            assert!(start.elapsed() < Duration::from_secs(2));
            task.await.unwrap();
            let pid = std::fs::read_to_string(&marker).unwrap();
            let alive = std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success();
            let _ = std::fs::remove_file(marker);
            assert!(
                !alive,
                "llama process must be reaped even when startup is cancelled"
            );
        });
    }

    #[test]
    fn drains_server_output_larger_than_pipe_capacity() {
        tauri::async_runtime::block_on(async {
            let child = tokio::process::Command::new("sh")
                .args([
                    "-c",
                    "head -c 262144 /dev/zero; head -c 262144 /dev/zero >&2",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let mut server = LocalLlamaServer::new(child, String::new(), Path::new("fixture.gguf"));
            let status = tokio::time::timeout(Duration::from_secs(3), server.child.wait())
                .await
                .unwrap()
                .unwrap();
            assert!(status.success());
            server.shutdown().await;
            assert!(server.readers.is_empty());
        });
    }

    #[test]
    fn dropping_server_stops_process_during_unwinding() {
        tauri::async_runtime::block_on(async {
            let child = tokio::process::Command::new("sleep")
                .arg("30")
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let pid = child.id().unwrap().to_string();
            drop(LocalLlamaServer::new(child, String::new(), Path::new("fixture.gguf")));
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                let alive = std::process::Command::new("kill")
                    .args(["-0", &pid])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .unwrap()
                    .success();
                if !alive {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "dropped server still running"
                );
                sleep(Duration::from_millis(10)).await;
            }
        });
    }

    #[test]
    fn local_request_cancels_without_per_cue_fallback() {
        use crate::translation::client::tests::{config, MockServer};
        tauri::async_runtime::block_on(async {
            let server = MockServer::new(
                vec![(200, "", "never finishes".to_string())],
                Duration::from_secs(10),
            );
            let cancel = Arc::new(AtomicBool::new(false));
            let trigger = cancel.clone();
            let task = tokio::spawn(async move {
                sleep(Duration::from_millis(80)).await;
                trigger.store(true, Ordering::SeqCst);
            });
            let segments = [
                SubtitleSegment {
                    id: 8,
                    start_ms: 0,
                    end_ms: 1000,
                    text: "one".to_string(),
                },
                SubtitleSegment {
                    id: 10,
                    start_ms: 1000,
                    end_ms: 2000,
                    text: "two".to_string(),
                },
            ];
            let result = translate_shard_via_local(
                &reqwest::Client::new(),
                &server.url,
                &config(&server.url),
                &segments,
                1,
                1,
                cancel,
            )
            .await;
            assert!(matches!(result, Err(JobError::Cancelled)));
            task.await.unwrap();
        });
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn startup_diagnostics_redact_paths_tokens_and_terminal_controls() {
        let path = Path::new(r"C:\Users\private-user\Models\my model.gguf");
        let mut stderr = StartupStderr::new(path);
        let lines = [
            format!("loading '{}' failed: out of memory\n", path.display()),
            "Authorization: Bearer credential-value\n".to_string(),
            "{\"api_key\": \"credential-value\"}\n".to_string(),
            "HF_TOKEN=credential-value\n".to_string(),
            "token: credential-value\n".to_string(),
            "TOKEN = credential-value\n".to_string(),
            "could not read /Users/private-user/cache/another.gguf\n".to_string(),
            "\x1b[31mCUDA allocation failed\x1b[0m\x1b]8;;https://private.example\x07\n".to_string(),
            "\u{202e}insufficient memory\u{2069}\n".to_string(),
        ];
        for line in lines {
            // Redaction must work across arbitrary pipe read boundaries.
            for chunk in line.as_bytes().chunks(3) {
                stderr.append(chunk);
            }
        }
        let diagnostic = stderr.diagnostic();
        assert!(diagnostic.contains("loading '[model]' failed: out of memory"));
        assert!(diagnostic.contains("CUDA allocation failed"));
        assert!(diagnostic.contains("insufficient memory"));
        for private in ["private-user", "my model.gguf", "credential-value", "private.example"] {
            assert!(!diagnostic.contains(private), "leaked {private}: {diagnostic}");
        }
        assert!(!diagnostic.contains('\x1b'));
        assert!(!diagnostic.contains('\u{202e}'));
        assert!(!diagnostic.contains('\u{2069}'));
    }

    #[test]
    fn startup_diagnostics_bound_lines_retention_and_unicode_output() {
        let mut stderr = StartupStderr::new(Path::new("model.gguf"));
        stderr.append(b"token=");
        stderr.append(&vec![b'x'; 100_000]);
        stderr.append(b"sensitive-tail\n");
        assert!(!stderr.diagnostic().contains("sensitive-tail"));
        for _ in 0..2_000 {
            stderr.append("llama_model_load: error 加载模型失败\n".as_bytes());
            assert!(stderr.retained_bytes <= STARTUP_STDERR_BYTES);
            assert!(stderr.pending.len() <= STARTUP_STDERR_LINE_BYTES);
        }
        stderr.append(b"CUDA_ERROR_OUT_OF_MEMORY");
        let diagnostic = stderr.diagnostic();
        assert!(diagnostic.len() <= STARTUP_DIAGNOSTIC_BYTES);
        assert!(diagnostic.ends_with("CUDA_ERROR_OUT_OF_MEMORY"));
        stderr.stop_capture();
        stderr.append(b"runtime subtitle text must not be retained\n");
        assert!(stderr.diagnostic().is_empty());
        assert!(stderr.pending.is_empty());
        assert_eq!(stderr.retained_bytes, 0);
    }

    #[test]
    fn startup_diagnostics_omit_model_inventory_templates_and_urls() {
        let mut stderr = StartupStderr::new(Path::new("model.gguf"));
        for line in [
            "llama_model_loader: - kv 1: general.description str = private-model-description\n",
            "llama_model_loader: - kv 2: tokenizer.chat_template str = private-template\n",
            "private-template-continuation: describe an error\n",
            "llama_model_load: failed https://example.invalid/model?signature=private-signature\n",
            "llama_model_loader: 42 tensors\n",
            "ggml_cuda_init: failed to initialize CUDA: driver mismatch\n",
            "llama_model_load: error: unsupported model architecture\n",
        ] {
            stderr.append(line.as_bytes());
        }
        let diagnostic = stderr.diagnostic();
        assert!(!diagnostic.contains("private-"));
        assert!(!diagnostic.contains("42 tensors"));
        assert!(diagnostic.contains("driver mismatch"));
        assert!(diagnostic.contains("unsupported model architecture"));
    }

    fn fixture(mode: &str, marker: &Path, model: &Path) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        hide_tokio_command_window(&mut command);
        command
            .args(["--exact", "translation::local::diagnostic_tests::stderr_child_fixture", "--nocapture"])
            .env("LUMA_LLAMA_DIAGNOSTIC_TEST", mode)
            .env("LUMA_LLAMA_DIAGNOSTIC_MARKER", marker)
            .env("LUMA_LLAMA_DIAGNOSTIC_MODEL", model)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }

    #[test]
    fn stderr_child_fixture() {
        let Ok(mode) = std::env::var("LUMA_LLAMA_DIAGNOSTIC_TEST") else { return };
        if mode == "hold-pipe" {
            std::thread::sleep(Duration::from_millis(1500));
            return;
        }
        let marker = PathBuf::from(std::env::var_os("LUMA_LLAMA_DIAGNOSTIC_MARKER").unwrap());
        let mut launches = std::fs::OpenOptions::new().create(true).append(true).open(marker).unwrap();
        writeln!(launches, "launched").unwrap();
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&vec![b'x'; 262_144]).unwrap();
        stdout.flush().unwrap();
        let mut stderr = std::io::stderr().lock();
        stderr.write_all(&vec![b'x'; 262_144]).unwrap();
        writeln!(stderr).unwrap();
        let model = std::env::var("LUMA_LLAMA_DIAGNOSTIC_MODEL").unwrap();
        writeln!(stderr, "failed to load '{model}': CUDA allocation failed").unwrap();
        writeln!(stderr, "Authorization: Bearer credential-value").unwrap();
        writeln!(stderr, "\x1b[31mCUDA_ERROR_OUT_OF_MEMORY\x1b[0m").unwrap();
        stderr.flush().unwrap();
        if mode == "inherited-pipe" {
            // A finite fixture retains inherited handles after its parent exits.
            // The server's bounded drain must not wait for this helper's EOF.
            let _child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "translation::local::diagnostic_tests::stderr_child_fixture", "--nocapture"])
                .env("LUMA_LLAMA_DIAGNOSTIC_TEST", "hold-pipe")
                .spawn().unwrap();
        }
        std::process::exit(17);
    }

    #[test]
    fn failed_startup_reports_bounded_stderr_status_and_does_not_retry() {
        tauri::async_runtime::block_on(async {
            let marker = std::env::temp_dir().join(format!("luma-llama-diagnostic-{}", uuid::Uuid::new_v4()));
            let model = Path::new("/private-user/Models/my model.gguf");
            let command = fixture("fail", &marker, model);
            let result = tokio::time::timeout(
                Duration::from_secs(8),
                launch_llama_server(
                    command,
                    "http://127.0.0.1:1".to_string(),
                    model,
                    Arc::new(AtomicBool::new(false)),
                ),
            ).await.expect("startup diagnostic pipe readers deadlocked");
            let Err(JobError::Failed(message)) = result else {
                panic!("fixture must fail at startup");
            };
            assert!(message.contains("exit code 17 (0x00000011)"), "{message}");
            assert!(message.contains("CUDA_ERROR_OUT_OF_MEMORY"), "{message}");
            assert!(!message.contains("private-user"));
            assert!(!message.contains("credential-value"));
            assert!(!message.contains('\x1b'));
            assert!(message.len() < STARTUP_DIAGNOSTIC_BYTES + 1024);
            assert_eq!(std::fs::read_to_string(&marker).unwrap(), "launched\n");
            std::fs::remove_file(marker).unwrap();
        });
    }

    #[test]
    fn diagnostic_drain_does_not_wait_for_inherited_pipe_eof() {
        tauri::async_runtime::block_on(async {
            let marker = std::env::temp_dir().join(format!("luma-llama-pipes-{}", uuid::Uuid::new_v4()));
            let model = Path::new("model.gguf");
            let child = fixture("inherited-pipe", &marker, model).spawn().unwrap();
            let mut server = LocalLlamaServer::new(child, String::new(), model);
            let status = tokio::time::timeout(Duration::from_secs(8), server.child.wait())
                .await.unwrap().unwrap();
            assert_eq!(status.code(), Some(17));
            let started = tokio::time::Instant::now();
            server.shutdown().await;
            assert!(started.elapsed() < Duration::from_secs(1));
            assert!(server.readers.is_empty());
            assert!(server.stderr.lock().diagnostic().contains("CUDA_ERROR_OUT_OF_MEMORY"));
            std::fs::remove_file(marker).unwrap();
        });
    }
}
