use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use serde::Deserialize;
use serde_json::json;
use tauri::AppHandle;
use tokio::{process::Child, time::sleep};

use crate::{
    job_events::{publish_job_event, JobEventDraft},
    paths::{is_existing_file, locate_binary, path_to_string},
    process_utils::hide_tokio_command_window,
    state::{ensure_not_cancelled, JobError, JobResult},
    subtitles::{collapse_repeated_vocalization, SubtitleSegment, TranslatedSegment},
};

use super::{
    parser::parse_delimited_translation_content, TranslationConfig,
    DEFAULT_LOCAL_TRANSLATION_SHARD_SIZE, MAX_LOCAL_TRANSLATION_SHARD_SIZE,
};

const LOCAL_CUE_DELIMITER: &str = "<<<CUE>>>";
const SERVER_START_TIMEOUT_SECS: u64 = 120;
const SHARD_TIMEOUT_SECS: u64 = 600;

struct LocalLlamaServer {
    child: Child,
    base_url: String,
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

    let _ = server.child.start_kill();
    Ok(progress.into_completed())
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
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|error| JobError::failed(format!("启动 llama-server 失败: {error}")))?;
    let base_url = format!("http://127.0.0.1:{port}");
    wait_for_server(&base_url, &mut child, cancel).await?;
    Ok(LocalLlamaServer { child, base_url })
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
        if let Ok(Some(status)) = child.try_wait() {
            return Err(JobError::failed(format!(
                "llama-server 在模型加载完成前退出: {status}"
            )));
        }
        if let Ok(response) = client.get(&health).send().await {
            if response.status().is_success() {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = child.start_kill();
            return Err(JobError::failed(
                "llama-server 启动超时。请确认模型文件完整，并检查本机内存是否足够",
            ));
        }
        sleep(Duration::from_millis(400)).await;
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
    match translate_shard_once(client, base_url, config, shard).await {
        Ok(items) => Ok(items),
        Err(error) => {
            if shard.len() <= 1 {
                return Err(error);
            }
            let mut items = Vec::with_capacity(shard.len());
            for (offset, segment) in shard.iter().enumerate() {
                ensure_not_cancelled(&cancel)?;
                let one = translate_shard_once(client, base_url, config, std::slice::from_ref(segment))
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
) -> JobResult<Vec<TranslatedSegment>> {
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
    let response = client
        .post(format!("{base_url}/v1/chat/completions"))
        .json(&payload)
        .send()
        .await
        .map_err(|error| JobError::failed(format!("本地翻译请求失败: {error}")))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| JobError::failed(format!("本地翻译响应读取失败: {error}")))?;
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
    parse_delimited_translation_content(content, shard)
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
    use crate::translation::{DEFAULT_LOCAL_TRANSLATION_SHARD_SIZE, MAX_LOCAL_TRANSLATION_SHARD_SIZE};

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
        let prompt = local_translation_prompt(
            "English",
            &[segment(1, "你好"), segment(2, "世界")],
        );
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
