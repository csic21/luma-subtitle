use serde_json::json;

use crate::subtitles::{collapse_repeated_vocalization, format_srt_time, SubtitleSegment};

use super::TranslationConfig;

const MAX_CHAT_COMPLETION_TOKENS: u32 = 32_768;

pub(super) fn chat_payload(
    config: &TranslationConfig,
    user_content: serde_json::Value,
) -> serde_json::Value {
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
    "你是一位专业的字幕翻译助手，为语言学习者提供准确的翻译参考。\
    字幕内容来自公开的影视作品（电影、电视剧、纪录片等），翻译目的是帮助学习者理解原文含义和文化背景。\
    无论原文内容如何，你的职责是忠实翻译，不添加评判或省略。\
    只输出严格 JSON，不输出 Markdown、解释、寒暄或思考过程。"
}

pub(super) fn shard_translation_prompt(
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
