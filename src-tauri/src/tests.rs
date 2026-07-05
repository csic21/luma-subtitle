use crate::process_utils::windows_create_no_window_flag;
use crate::settings::normalize_language;
use crate::state::JobError;
use crate::subtitles::{
    collapse_repeated_vocalization, format_srt_time, parse_srt_text, parse_timestamp_ms,
    parse_whisper_json, summarize_repeated_vocalization, validate_whisper_repetition,
    SubtitleSegment,
};
use crate::translation::{attach_model_output, chat_endpoint, parse_translation_content};
use std::{fs, process};

#[test]
fn formats_srt_time() {
    assert_eq!(format_srt_time(3_723_045), "01:02:03,045");
}

#[test]
fn parses_whisper_timestamp() {
    assert_eq!(parse_timestamp_ms("00:01:02,500"), Some(62_500));
    assert_eq!(parse_timestamp_ms("00:01:02.500"), Some(62_500));
}

#[test]
fn normalizes_whisper_language_choices() {
    assert_eq!(normalize_language(""), "auto");
    assert_eq!(normalize_language("自动检测"), "auto");
    assert_eq!(normalize_language("简体中文"), "zh");
    assert_eq!(normalize_language("English"), "en");
}

#[test]
fn uses_windows_create_no_window_flag_for_child_processes() {
    assert_eq!(windows_create_no_window_flag(), 0x08000000);
}

#[test]
fn parses_whisper_json_with_lossy_text_decoding() {
    let path = std::env::temp_dir().join(format!(
        "luma-whisper-lossy-{}-{}.json",
        process::id(),
        "segments"
    ));
    let mut body = r#"{"segments":[{"start":0.0,"end":1.2,"text":"こんにちは"#
        .as_bytes()
        .to_vec();
    body.push(0xff);
    body.extend_from_slice(br#""}]}"#);
    fs::write(&path, body).expect("test json should be written");

    let parsed = parse_whisper_json(&path).expect("lossy json should parse");
    let _ = fs::remove_file(&path);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].text, "こんにちは�");
}

#[test]
fn caps_repeated_whisper_vocalization_text() {
    let path = std::env::temp_dir().join(format!(
        "luma-whisper-repeat-{}-{}.json",
        process::id(),
        "segments"
    ));
    let body = format!(
        r#"{{"segments":[{{"start":0.0,"end":1.2,"text":"{}"}}]}}"#,
        "啊".repeat(80)
    );
    fs::write(&path, body).expect("test json should be written");

    let parsed = parse_whisper_json(&path).expect("json should parse");
    let _ = fs::remove_file(&path);
    assert_eq!(parsed[0].text, "啊".repeat(20));
}

#[test]
fn parses_imported_srt_text() {
    let parsed = parse_srt_text(
        "\u{feff}7\r\n00:00:01,000 --> 00:00:02,500\r\nHello\r\nworld\r\n\r\n9\r\n00:00:03,000 --> 00:00:04,000\r\nBye",
    )
    .expect("srt should parse");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].id, 7);
    assert_eq!(parsed[0].start_ms, 1_000);
    assert_eq!(parsed[0].end_ms, 2_500);
    assert_eq!(parsed[0].text, "Hello world");
    assert_eq!(parsed[1].id, 9);
}

#[test]
fn parses_srt_time_with_position_suffix() {
    let parsed = parse_srt_text("1\n00:00:01,000 --> 00:00:02,000 position:50% line:80%\nHello")
        .expect("srt time suffix should parse");
    assert_eq!(parsed[0].end_ms, 2_000);
}

#[test]
fn rejects_repeated_whisper_hallucination_run() {
    let segments = (0..31)
        .map(|index| SubtitleSegment {
            id: index + 1,
            start_ms: index as u64 * 1_000,
            end_ms: index as u64 * 1_000 + 900,
            text: "うまくできてたんじゃないですか".to_string(),
        })
        .collect::<Vec<_>>();

    let error = validate_whisper_repetition(&segments).expect_err("long repeated run should fail");

    assert!(format!("{error:?}").contains("重复幻觉"));
}

#[test]
fn allows_short_filler_repetition() {
    let segments = (0..40)
        .map(|index| SubtitleSegment {
            id: index + 1,
            start_ms: index as u64 * 1_000,
            end_ms: index as u64 * 1_000 + 900,
            text: "はい".to_string(),
        })
        .collect::<Vec<_>>();

    validate_whisper_repetition(&segments).expect("short filler repeats should be allowed");
}

#[test]
fn rejects_global_whisper_repetition() {
    let mut segments = (0..500)
        .map(|index| SubtitleSegment {
            id: index + 1,
            start_ms: index as u64 * 1_000,
            end_ms: index as u64 * 1_000 + 900,
            text: format!("普通の字幕 {index}"),
        })
        .collect::<Vec<_>>();
    for segment in segments.iter_mut().step_by(4).take(110) {
        segment.text = "あなたのために私を愛しています。".to_string();
    }

    let error =
        validate_whisper_repetition(&segments).expect_err("global repeated text should fail");

    assert!(format!("{error:?}").contains("全片出现"));
}

#[test]
fn collapses_long_repeated_vocalization() {
    assert_eq!(
        collapse_repeated_vocalization(&"あー".repeat(40)),
        "あー".repeat(10)
    );
    assert_eq!(
        collapse_repeated_vocalization(&"啊".repeat(80)),
        "啊".repeat(20)
    );
    assert_eq!(
        collapse_repeated_vocalization("你好你好你好你好你好你好"),
        "你好你好你好你好你好你好"
    );
}

#[test]
fn summarizes_repeated_translation_vocalization() {
    assert_eq!(
        summarize_repeated_vocalization(&"啊".repeat(80)),
        "啊啊啊..."
    );
    assert_eq!(
        summarize_repeated_vocalization(&"あー".repeat(40)),
        "あーあーあー..."
    );
}

#[test]
fn parses_translation_json_with_fence() {
    let batch = vec![SubtitleSegment {
        id: 7,
        start_ms: 0,
        end_ms: 1,
        text: "hello".to_string(),
    }];
    let parsed = parse_translation_content(
        "```json\n{\"items\":[{\"id\":7,\"text\":\"你好\"}]}\n```",
        &batch,
    )
    .expect("translation should parse");
    assert_eq!(parsed[0].id, 7);
    assert_eq!(parsed[0].text, "你好");
}

#[test]
fn maps_translation_by_order_when_model_renumbers_ids() {
    let segments = vec![
        SubtitleSegment {
            id: 7,
            start_ms: 0,
            end_ms: 1,
            text: "hello".to_string(),
        },
        SubtitleSegment {
            id: 9,
            start_ms: 1,
            end_ms: 2,
            text: "world".to_string(),
        },
    ];
    let parsed = parse_translation_content(
        "{\"items\":[{\"id\":1,\"text\":\"你好\"},{\"id\":2,\"text\":\"世界\"}]}",
        &segments,
    )
    .expect("same-count translation should map by order");
    assert_eq!(parsed[0].id, 7);
    assert_eq!(parsed[0].text, "你好");
    assert_eq!(parsed[1].id, 9);
    assert_eq!(parsed[1].text, "世界");
}

#[test]
fn parses_compact_translation_array_by_order() {
    let segments = vec![
        SubtitleSegment {
            id: 7,
            start_ms: 0,
            end_ms: 1,
            text: "hello".to_string(),
        },
        SubtitleSegment {
            id: 9,
            start_ms: 1,
            end_ms: 2,
            text: "world".to_string(),
        },
    ];
    let parsed = parse_translation_content("[\"你好\",\"世界\"]", &segments)
        .expect("compact array should parse");
    assert_eq!(parsed[0].id, 7);
    assert_eq!(parsed[0].text, "你好");
    assert_eq!(parsed[1].id, 9);
    assert_eq!(parsed[1].text, "世界");
}

#[test]
fn parses_compact_items_string_array_by_order() {
    let segments = vec![SubtitleSegment {
        id: 7,
        start_ms: 0,
        end_ms: 1,
        text: "hello".to_string(),
    }];
    let parsed = parse_translation_content("{\"items\":[\"你好\"]}", &segments)
        .expect("compact items array should parse");
    assert_eq!(parsed[0].id, 7);
    assert_eq!(parsed[0].text, "你好");
}

#[test]
fn collapses_repetitive_translation_output() {
    let segments = vec![SubtitleSegment {
        id: 1360,
        start_ms: 2_544_900,
        end_ms: 2_574_900,
        text: "あー".repeat(60),
    }];
    let output = format!(
        "{{\"items\":[{{\"id\":1360,\"text\":\"{}\"}}]}}",
        "啊".repeat(100)
    );

    let parsed =
        parse_translation_content(&output, &segments).expect("repetitive translation should parse");

    assert_eq!(parsed[0].text, "啊啊啊...");
}

#[test]
fn rejects_translation_when_item_count_differs() {
    let segments = vec![
        SubtitleSegment {
            id: 1,
            start_ms: 0,
            end_ms: 1,
            text: "hello".to_string(),
        },
        SubtitleSegment {
            id: 2,
            start_ms: 1,
            end_ms: 2,
            text: "world".to_string(),
        },
    ];
    let error = parse_translation_content("{\"items\":[{\"id\":1,\"text\":\"你好\"}]}", &segments)
        .expect_err("missing item should fail");
    assert!(format!("{error:?}").contains("请求 2 条，返回 1 条"));
}

#[test]
fn parse_error_includes_model_output_preview() {
    let usage = serde_json::json!({ "completion_tokens": 32768 });
    let error = attach_model_output(
        JobError::failed("翻译 JSON 解析失败: EOF"),
        "{\"items\":[",
        Some("length"),
        Some(&usage),
    );
    let JobError::Failed(message) = error else {
        panic!("expected failed error");
    };
    assert!(message.contains("模型返回文本"));
    assert!(message.contains("finish_reason: length"));
    assert!(message.contains("completion_tokens"));
    assert!(message.contains("{\"items\":["));
}

#[test]
fn translation_endpoint_appends_chat_path_by_default() {
    assert_eq!(
        chat_endpoint("https://api.openai.com/", false),
        "https://api.openai.com/v1/chat/completions"
    );
}

#[test]
fn translation_endpoint_uses_complete_url_when_enabled() {
    assert_eq!(
        chat_endpoint("https://example.test/custom/chat", true),
        "https://example.test/custom/chat"
    );
}
