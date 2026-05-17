use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashSet,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use tauri::AppHandle;

use crate::{
    job_events::{emit_job, JobStatus},
    state::{ensure_not_cancelled, JobError, JobResult},
    subtitles::{
        collapse_repeated_vocalization, format_srt_time, SubtitleSegment, TranslatedSegment,
    },
};

#[derive(Clone)]
pub(crate) struct TranslationConfig {
    pub(crate) target_language: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    pub(crate) shard_size: usize,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    usage: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[derive(Deserialize)]
struct TranslationItem {
    id: usize,
    text: String,
}

const MAX_CHAT_COMPLETION_TOKENS: u32 = 32_768;
pub(crate) const DEFAULT_TRANSLATION_SHARD_SIZE: usize = 200;
pub(crate) const MIN_TRANSLATION_SHARD_SIZE: usize = 1;
pub(crate) const MAX_TRANSLATION_SHARD_SIZE: usize = 1_000;
const MAX_CONCURRENT_SHARDS: usize = 4;

pub(crate) fn normalize_translation_shard_size(size: usize) -> usize {
    size.clamp(MIN_TRANSLATION_SHARD_SIZE, MAX_TRANSLATION_SHARD_SIZE)
}

pub(crate) async fn translate_with_single_request(
    app: &AppHandle,
    job_id: &str,
    config: &TranslationConfig,
    api_key: &str,
    segments: &[SubtitleSegment],
    _source_srt: &str,
    _source_file_name: &str,
    cancel: Arc<AtomicBool>,
) -> JobResult<Vec<TranslatedSegment>> {
    ensure_not_cancelled(&cancel)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(240))
        .build()
        .map_err(|error| JobError::failed(format!("创建 HTTP 客户端失败: {error}")))?;

    let shard_size = normalize_translation_shard_size(config.shard_size);
    emit_job(
        app,
        job_id,
        "translate-shards",
        JobStatus::Running,
        format!("正在按每片 {shard_size} 条字幕分片翻译（并发 {MAX_CONCURRENT_SHARDS}）"),
        0.58,
        None,
        None,
    );

    translate_shards(app, job_id, &client, config, api_key, segments, cancel).await
}

async fn translate_shards(
    app: &AppHandle,
    job_id: &str,
    client: &reqwest::Client,
    config: &TranslationConfig,
    api_key: &str,
    segments: &[SubtitleSegment],
    cancel: Arc<AtomicBool>,
) -> JobResult<Vec<TranslatedSegment>> {
    let shard_size = normalize_translation_shard_size(config.shard_size);
    let shards = segments
        .chunks(shard_size)
        .map(|chunk| chunk.to_vec())
        .collect::<Vec<_>>();
    let total_shards = shards.len().max(1);
    let mut translated = Vec::with_capacity(segments.len());

    for (group_index, group) in shards.chunks(MAX_CONCURRENT_SHARDS).enumerate() {
        ensure_not_cancelled(&cancel)?;
        let mut handles = Vec::with_capacity(group.len());
        for (offset, shard) in group.iter().cloned().enumerate() {
            let shard_index = group_index * MAX_CONCURRENT_SHARDS + offset + 1;
            let progress = shard_progress(shard_index.saturating_sub(1), total_shards);
            emit_job(
                app,
                job_id,
                "translate-shard",
                JobStatus::Running,
                format!(
                    "分片 {shard_index}/{total_shards} 已提交（{} 条字幕）",
                    shard.len()
                ),
                progress,
                None,
                None,
            );

            let client = client.clone();
            let config = (*config).clone();
            let api_key = api_key.to_string();
            handles.push(tauri::async_runtime::spawn(async move {
                translate_shard_once(
                    &client,
                    &config,
                    &api_key,
                    &shard,
                    shard_index,
                    total_shards,
                )
                .await
                .map(|items| (shard_index, items))
                .map_err(|error| prefix_shard_error(error, shard_index, total_shards))
            }));
        }

        for handle in handles {
            ensure_not_cancelled(&cancel)?;
            let (shard_index, mut items) = handle
                .await
                .map_err(|error| JobError::failed(format!("翻译分片任务失败: {error}")))??;
            translated.append(&mut items);
            emit_job(
                app,
                job_id,
                "translate-shard",
                JobStatus::Running,
                format!("分片 {shard_index}/{total_shards} 已完成"),
                shard_progress(shard_index, total_shards),
                None,
                None,
            );
        }
    }

    translated.sort_by_key(|item| item.id);
    Ok(translated)
}

fn prefix_shard_error(error: JobError, shard_index: usize, total_shards: usize) -> JobError {
    match error {
        JobError::Cancelled => JobError::Cancelled,
        JobError::Failed(message) => {
            JobError::failed(format!("分片 {shard_index}/{total_shards} 失败: {message}"))
        }
    }
}

async fn translate_shard_once(
    client: &reqwest::Client,
    config: &TranslationConfig,
    api_key: &str,
    shard: &[SubtitleSegment],
    shard_index: usize,
    total_shards: usize,
) -> JobResult<Vec<TranslatedSegment>> {
    let payload = chat_payload(
        config,
        json!(shard_translation_prompt(
            &config.target_language,
            shard,
            shard_index,
            total_shards
        )),
    );
    post_chat_translation(client, config, api_key, shard, payload).await
}

fn shard_progress(completed_shards: usize, total_shards: usize) -> f32 {
    0.58 + (completed_shards as f32 / total_shards.max(1) as f32) * 0.36
}

fn chat_payload(config: &TranslationConfig, user_content: serde_json::Value) -> serde_json::Value {
    let mut payload = json!({
        "model": &config.model,
        "temperature": config.temperature.clamp(0.0, 1.0),
        "max_tokens": MAX_CHAT_COMPLETION_TOKENS,
        "max_completion_tokens": MAX_CHAT_COMPLETION_TOKENS,
        "messages": [
            {
                "role": "system",
                "content": translation_system_prompt()
            },
            {
                "role": "user",
                "content": user_content
            }
        ]
    });
    apply_thinking_controls(config, &mut payload);
    payload
}

fn apply_thinking_controls(config: &TranslationConfig, payload: &mut serde_json::Value) {
    let marker = format!(
        "{} {}",
        config.model.to_ascii_lowercase(),
        config.base_url.to_ascii_lowercase()
    );
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    if marker.contains("qwen") || marker.contains("dashscope") || marker.contains("aliyun") {
        object.insert("enable_thinking".to_string(), json!(false));
    }
    if marker.contains("deepseek") {
        object.insert("thinking".to_string(), json!({ "type": "disabled" }));
    }
}

fn translation_system_prompt() -> &'static str {
    "你是专业字幕翻译器。你只执行翻译任务，只输出严格 JSON，不输出 Markdown、解释、寒暄或思考过程。"
}

async fn post_chat_translation(
    client: &reqwest::Client,
    config: &TranslationConfig,
    api_key: &str,
    segments: &[SubtitleSegment],
    payload: serde_json::Value,
) -> JobResult<Vec<TranslatedSegment>> {
    let endpoint = format!(
        "{}/v1/chat/completions",
        config.base_url.trim_end_matches('/')
    );
    let response = post_chat_request(client, &endpoint, api_key, &payload).await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        if body.contains("max_completion_tokens") {
            let mut fallback_payload = payload.clone();
            if let Some(object) = fallback_payload.as_object_mut() {
                object.remove("max_completion_tokens");
            }
            let retry = post_chat_request(client, &endpoint, api_key, &fallback_payload).await?;
            let retry_status = retry.status();
            if retry_status.is_success() {
                return parse_chat_response(retry, segments).await;
            }
            let retry_body = retry.text().await.unwrap_or_default();
            return Err(JobError::failed(format!(
                "翻译失败: HTTP {retry_status}: {}",
                trim_error_body(&retry_body)
            )));
        }
        return Err(JobError::failed(format!(
            "翻译失败: HTTP {status}: {}",
            trim_error_body(&body)
        )));
    }

    parse_chat_response(response, segments).await
}

async fn post_chat_request(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    payload: &serde_json::Value,
) -> JobResult<reqwest::Response> {
    client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(payload)
        .send()
        .await
        .map_err(|error| JobError::failed(format!("翻译请求失败: {error}")))
}

async fn parse_chat_response(
    response: reqwest::Response,
    segments: &[SubtitleSegment],
) -> JobResult<Vec<TranslatedSegment>> {
    let chat = response
        .json::<ChatResponse>()
        .await
        .map_err(|error| JobError::failed(format!("翻译响应解析失败: {error}")))?;
    let choice = chat
        .choices
        .first()
        .ok_or_else(|| JobError::failed("翻译接口没有返回 choices"))?;
    let content = choice.message.content.as_str();
    parse_translation_content(content, segments).map_err(|error| {
        attach_model_output(
            error,
            content,
            choice.finish_reason.as_deref(),
            chat.usage.as_ref(),
        )
    })
}

fn shard_translation_prompt(
    target_language: &str,
    segments: &[SubtitleSegment],
    shard_index: usize,
    total_shards: usize,
) -> String {
    let items = segments
        .iter()
        .map(|segment| {
            json!({
                "id": segment.id,
                "time": format!(
                    "{} --> {}",
                    format_srt_time(segment.start_ms),
                    format_srt_time(segment.end_ms)
                ),
                "text": collapse_repeated_vocalization(&segment.text)
            })
        })
        .collect::<Vec<_>>();
    format!(
        "任务：把字幕分片翻译成{target_language}。\n\
        当前分片：{shard_index}/{total_shards}\n\
        当前分片字幕条数：{}\n\n\
        严格规则：\n\
        1. 只翻译 text，不要修改 id。\n\
        2. 不得跳过、合并、拆分或重排条目。\n\
        3. 只输出严格 JSON，格式必须是：{{\"items\":[{{\"id\":1,\"text\":\"译文\"}}]}}。\n\
        4. items 数量必须等于当前分片字幕条数。\n\
        5. 不要输出时间轴、原文、Markdown、解释、寒暄、思考过程或 <think> 内容。\n\
        6. 保持人名、术语、称呼、语气和上下文一致。\n\
        7. 如果原文包含多行字幕，把对应译文合并为自然的一条字幕文本。\n\
        8. 遇到长时间重复的语气词、拟声词或拖音（如“あーあー…”、“啊啊啊…”），只输出简短自然译文，可用省略号表示持续，不要逐字扩写重复。\n\n\
        输入 JSON：\n{}",
        segments.len(),
        serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
    )
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

pub(crate) fn attach_model_output(
    error: JobError,
    content: &str,
    finish_reason: Option<&str>,
    usage: Option<&serde_json::Value>,
) -> JobError {
    match error {
        JobError::Cancelled => JobError::Cancelled,
        JobError::Failed(message) => JobError::failed(format!(
            "{message}\n\nfinish_reason: {}\nusage: {}\n模型返回文本（{} 字符）：\n{}",
            finish_reason.unwrap_or("未知"),
            usage
                .map(serde_json::Value::to_string)
                .unwrap_or_else(|| "未知".to_string()),
            content.chars().count(),
            preview_model_output(content)
        )),
    }
}

fn preview_model_output(content: &str) -> String {
    const MAX_MODEL_OUTPUT_CHARS: usize = 20_000;
    const HALF_MODEL_OUTPUT_CHARS: usize = MAX_MODEL_OUTPUT_CHARS / 2;
    let char_count = content.chars().count();
    if char_count <= MAX_MODEL_OUTPUT_CHARS {
        return content.to_string();
    }
    let head = content
        .chars()
        .take(HALF_MODEL_OUTPUT_CHARS)
        .collect::<String>();
    let tail = content
        .chars()
        .skip(char_count.saturating_sub(HALF_MODEL_OUTPUT_CHARS))
        .collect::<String>();
    format!(
        "{head}\n\n... 中间省略 {} 字符 ...\n\n{tail}",
        char_count - MAX_MODEL_OUTPUT_CHARS
    )
}

pub(crate) fn parse_translation_content(
    content: &str,
    segments: &[SubtitleSegment],
) -> JobResult<Vec<TranslatedSegment>> {
    let json_text = extract_json_value(content)
        .ok_or_else(|| JobError::failed("翻译接口返回内容不是 JSON 对象或数组"))?;
    let value = serde_json::from_str::<serde_json::Value>(&json_text)
        .map_err(|error| JobError::failed(format!("翻译 JSON 解析失败: {error}")))?;
    parse_translation_value(value, segments)
}

fn parse_translation_value(
    value: serde_json::Value,
    segments: &[SubtitleSegment],
) -> JobResult<Vec<TranslatedSegment>> {
    let items = translation_items_value(&value)
        .ok_or_else(|| JobError::failed("翻译 JSON 缺少 items 数组或译文数组"))?;
    if items.iter().all(serde_json::Value::is_string) {
        if items.len() != segments.len() {
            return Err(JobError::failed(format!(
                "翻译返回的字幕数量与请求不一致：请求 {} 条，返回 {} 条",
                segments.len(),
                items.len()
            )));
        }
        return Ok(items
            .iter()
            .zip(segments.iter())
            .map(|(item, segment)| TranslatedSegment {
                id: segment.id,
                text: collapse_repeated_vocalization(item.as_str().unwrap_or_default()),
            })
            .collect());
    }

    let parsed_items = items
        .iter()
        .cloned()
        .map(serde_json::from_value::<TranslationItem>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| JobError::failed(format!("翻译 JSON 条目解析失败: {error}")))?;
    let expected_ids = segments
        .iter()
        .map(|segment| segment.id)
        .collect::<HashSet<_>>();
    let actual_ids = parsed_items
        .iter()
        .map(|item| item.id)
        .collect::<HashSet<_>>();
    if expected_ids == actual_ids {
        return Ok(parsed_items
            .into_iter()
            .map(|item| TranslatedSegment {
                id: item.id,
                text: collapse_repeated_vocalization(&item.text),
            })
            .collect());
    }
    if parsed_items.len() == segments.len() {
        return Ok(parsed_items
            .into_iter()
            .zip(segments.iter())
            .map(|(item, segment)| TranslatedSegment {
                id: segment.id,
                text: collapse_repeated_vocalization(&item.text),
            })
            .collect());
    }
    Err(JobError::failed(format!(
        "翻译返回的字幕数量或 id 与请求不一致：请求 {} 条，返回 {} 条",
        segments.len(),
        parsed_items.len()
    )))
}

fn translation_items_value(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    if let Some(items) = value.as_array() {
        return Some(items);
    }
    value
        .get("items")
        .or_else(|| value.get("translations"))
        .and_then(serde_json::Value::as_array)
}

fn extract_json_value(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if is_wrapped_json_value(trimmed) {
        return Some(trimmed.to_string());
    }
    let without_fences = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if is_wrapped_json_value(without_fences) {
        return Some(without_fences.to_string());
    }
    let start = [trimmed.find('{'), trimmed.find('[')]
        .into_iter()
        .flatten()
        .min()?;
    let end = [trimmed.rfind('}'), trimmed.rfind(']')]
        .into_iter()
        .flatten()
        .max()?;
    (end > start).then(|| trimmed[start..=end].to_string())
}

fn is_wrapped_json_value(value: &str) -> bool {
    (value.starts_with('{') && value.ends_with('}'))
        || (value.starts_with('[') && value.ends_with(']'))
}
