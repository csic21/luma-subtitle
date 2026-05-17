use serde::Deserialize;
use serde_json::json;

use crate::{
    state::{JobError, JobResult},
    subtitles::{SubtitleSegment, TranslatedSegment},
};

use super::{
    parser::{attach_model_output, parse_translation_content},
    prompt::{chat_payload, shard_translation_prompt},
    TranslationConfig,
};

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

pub(super) async fn translate_shard_once(
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

fn trim_error_body(body: &str) -> String {
    const MAX_ERROR_BODY: usize = 1_500;
    let trimmed = body.trim();
    if trimmed.chars().count() <= MAX_ERROR_BODY {
        return trimmed.to_string();
    }
    let preview = trimmed.chars().take(MAX_ERROR_BODY).collect::<String>();
    format!("{preview}...")
}
