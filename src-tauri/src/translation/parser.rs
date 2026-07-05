use serde::Deserialize;
use std::collections::HashSet;

use crate::{
    state::{JobError, JobResult},
    subtitles::{summarize_repeated_vocalization, SubtitleSegment, TranslatedSegment},
};

#[derive(Deserialize)]
struct TranslationItem {
    id: usize,
    text: String,
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
                text: summarize_repeated_vocalization(item.as_str().unwrap_or_default()),
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
                text: summarize_repeated_vocalization(&item.text),
            })
            .collect());
    }
    if parsed_items.len() == segments.len() {
        return Ok(parsed_items
            .into_iter()
            .zip(segments.iter())
            .map(|(item, segment)| TranslatedSegment {
                id: segment.id,
                text: summarize_repeated_vocalization(&item.text),
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
