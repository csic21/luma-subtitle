use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};
use unicode_segmentation::UnicodeSegmentation;

use crate::state::{JobError, JobResult};

#[derive(Clone, Debug)]
pub(crate) struct SubtitleSegment {
    pub(crate) id: usize,
    pub(crate) start_ms: u64,
    pub(crate) end_ms: u64,
    pub(crate) text: String,
}
#[derive(Clone, Debug)]
pub(crate) struct TranslatedSegment {
    pub(crate) id: usize,
    pub(crate) text: String,
}

pub(crate) fn render_srt(
    segments: &[SubtitleSegment],
    translations: Option<&[TranslatedSegment]>,
) -> String {
    let translated = translations.map(|items| {
        items
            .iter()
            .map(|item| (item.id, item.text.as_str()))
            .collect::<HashMap<_, _>>()
    });
    let mut body = String::new();
    for segment in segments {
        let text = translated
            .as_ref()
            .and_then(|map| map.get(&segment.id).copied())
            .unwrap_or(segment.text.as_str());
        body.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            segment.id,
            format_srt_time(segment.start_ms),
            format_srt_time(segment.end_ms),
            normalize_subtitle_text(text)
        ));
    }
    body
}

pub(crate) async fn write_srt_text(path: &Path, body: &str) -> JobResult<()> {
    tokio::fs::write(path, body)
        .await
        .map_err(|error| JobError::failed(format!("写入 SRT 失败: {error}")))
}

pub(crate) fn parse_srt_file(path: &Path) -> JobResult<Vec<SubtitleSegment>> {
    let body = read_text_lossy(path)
        .map_err(|error| JobError::failed(format!("读取 SRT 失败: {error}")))?;
    parse_srt_text(&body)
}

pub(crate) fn parse_srt_text(body: &str) -> JobResult<Vec<SubtitleSegment>> {
    let normalized = body
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut segments = Vec::new();
    for block in normalized.split("\n\n") {
        let lines = block
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        if lines.len() < 3 {
            continue;
        }
        let Some(id) = lines[0].parse::<usize>().ok() else {
            continue;
        };
        let Some((start_ms, end_ms)) = parse_srt_time_range(lines[1]) else {
            continue;
        };
        let text = normalize_subtitle_text(&lines[2..].join(" "));
        if text.is_empty() {
            continue;
        }
        segments.push(SubtitleSegment {
            id,
            start_ms,
            end_ms: end_ms.max(start_ms + 1),
            text,
        });
    }
    if segments.is_empty() {
        return Err(JobError::failed("SRT 中没有可导入的字幕条目"));
    }
    Ok(segments)
}

pub(crate) fn parse_whisper_json(path: &Path) -> JobResult<Vec<SubtitleSegment>> {
    let body = read_text_lossy(path)
        .map_err(|error| JobError::failed(format!("读取 Whisper JSON 失败: {error}")))?;
    let value = serde_json::from_str::<Value>(&body)
        .map_err(|error| JobError::failed(format!("解析 Whisper JSON 失败: {error}")))?;
    if let Some(items) = value.get("transcription").and_then(Value::as_array) {
        return parse_transcription_array(items);
    }
    if let Some(items) = value.get("segments").and_then(Value::as_array) {
        return parse_segments_array(items);
    }
    Err(JobError::failed(
        "Whisper JSON 缺少 transcription 或 segments 字段",
    ))
}
fn read_text_lossy(path: &Path) -> std::io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(decode_text_lossy(&bytes))
}
fn decode_text_lossy(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(&bytes[3..]).to_string();
    }
    if bytes.starts_with(&[0xff, 0xfe]) {
        return decode_utf16_lossy(&bytes[2..], true);
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        return decode_utf16_lossy(&bytes[2..], false);
    }

    let even_nuls = bytes.iter().step_by(2).filter(|byte| **byte == 0).count();
    let odd_nuls = bytes
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|byte| **byte == 0)
        .count();
    let pairs = bytes.len() / 2;
    if pairs > 0 && odd_nuls > pairs / 3 && odd_nuls > even_nuls * 2 {
        return decode_utf16_lossy(bytes, true);
    }
    if pairs > 0 && even_nuls > pairs / 3 && even_nuls > odd_nuls * 2 {
        return decode_utf16_lossy(bytes, false);
    }

    String::from_utf8_lossy(bytes).to_string()
}
fn decode_utf16_lossy(bytes: &[u8], little_endian: bool) -> String {
    let words = bytes
        .chunks_exact(2)
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16_lossy(&words)
}
fn parse_transcription_array(items: &[Value]) -> JobResult<Vec<SubtitleSegment>> {
    let mut segments = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .map(normalize_subtitle_text)
            .unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        let (start_ms, end_ms) = item
            .get("offsets")
            .and_then(|offsets| {
                Some((
                    parse_offset_ms(offsets.get("from")?)?,
                    parse_offset_ms(offsets.get("to")?)?,
                ))
            })
            .or_else(|| {
                item.get("timestamps").and_then(|timestamps| {
                    Some((
                        parse_timestamp_ms(timestamps.get("from")?.as_str()?)?,
                        parse_timestamp_ms(timestamps.get("to")?.as_str()?)?,
                    ))
                })
            })
            .ok_or_else(|| JobError::failed("Whisper transcription 字段缺少时间戳"))?;
        segments.push(SubtitleSegment {
            id: index + 1,
            start_ms,
            end_ms: end_ms.max(start_ms + 1),
            text,
        });
    }
    Ok(segments)
}
fn parse_segments_array(items: &[Value]) -> JobResult<Vec<SubtitleSegment>> {
    let mut segments = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .map(normalize_subtitle_text)
            .unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        let start_ms = parse_seconds_ms(
            item.get("start")
                .ok_or_else(|| JobError::failed("Whisper segment 缺少 start"))?,
        )?;
        let end_ms = parse_seconds_ms(
            item.get("end")
                .ok_or_else(|| JobError::failed("Whisper segment 缺少 end"))?,
        )?;
        segments.push(SubtitleSegment {
            id: index + 1,
            start_ms,
            end_ms: end_ms.max(start_ms + 1),
            text,
        });
    }
    Ok(segments)
}
fn parse_offset_ms(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|number| number.round().max(0.0) as u64))
}
fn parse_seconds_ms(value: &Value) -> JobResult<u64> {
    value
        .as_f64()
        .map(|seconds| (seconds * 1000.0).round().max(0.0) as u64)
        .ok_or_else(|| JobError::failed("Whisper segment 时间戳不是数字"))
}
pub(crate) fn parse_timestamp_ms(timestamp: &str) -> Option<u64> {
    let normalized = timestamp.replace(',', ".");
    let parts = normalized.split(':').collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }
    let hours = parts[0].parse::<u64>().ok()?;
    let minutes = parts[1].parse::<u64>().ok()?;
    let seconds = parts[2].parse::<f64>().ok()?;
    Some(((hours * 3600 + minutes * 60) as f64 * 1000.0 + seconds * 1000.0).round() as u64)
}

fn parse_srt_time_range(line: &str) -> Option<(u64, u64)> {
    let (start, end) = line.split_once("-->")?;
    Some((parse_srt_timestamp(start)?, parse_srt_timestamp(end)?))
}

fn parse_srt_timestamp(value: &str) -> Option<u64> {
    parse_timestamp_ms(value.trim().split_whitespace().next()?)
}

pub(crate) fn format_srt_time(ms: u64) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1_000;
    let millis = ms % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}
pub(crate) fn normalize_subtitle_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn collapse_repeated_vocalization(text: &str) -> String {
    let normalized = normalize_subtitle_text(text);
    let collapsed = collapse_whole_repeated_vocalization(&normalized).unwrap_or(normalized);
    collapse_repeated_vocalization_runs(&collapsed)
}

fn collapse_whole_repeated_vocalization(text: &str) -> Option<String> {
    const MIN_REPETITIONS: usize = 6;
    const MAX_UNIT_GRAPHEMES: usize = 4;
    const COLLAPSED_REPETITIONS: usize = 3;

    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    if graphemes.len() < MIN_REPETITIONS {
        return None;
    }

    let core_len = trim_trailing_sentence_punctuation(&graphemes);
    let core = &graphemes[..core_len];
    if core.len() < MIN_REPETITIONS {
        return None;
    }

    for unit_len in 1..=MAX_UNIT_GRAPHEMES.min(core.len()) {
        let repetitions = (core.len() + unit_len - 1) / unit_len;
        if repetitions < MIN_REPETITIONS || !matches_repeated_unit(core, unit_len) {
            continue;
        }

        let unit = &core[..unit_len];
        if !is_vocalization_unit(unit) {
            continue;
        }

        let mut collapsed = String::new();
        for _ in 0..COLLAPSED_REPETITIONS.min(repetitions) {
            collapsed.push_str(&unit.concat());
        }
        collapsed.push_str("...");
        return Some(collapsed);
    }

    None
}

fn collapse_repeated_vocalization_runs(text: &str) -> String {
    const MIN_RUN: usize = 8;
    const COLLAPSED_RUN: usize = 3;

    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    let mut collapsed = String::new();
    let mut index = 0;

    while index < graphemes.len() {
        let current = graphemes[index];
        let mut end = index + 1;
        while end < graphemes.len() && graphemes[end] == current {
            end += 1;
        }

        let run_len = end - index;
        if run_len >= MIN_RUN && is_vocalization_grapheme(current) {
            for _ in 0..COLLAPSED_RUN {
                collapsed.push_str(current);
            }
            collapsed.push_str("...");
        } else {
            for grapheme in &graphemes[index..end] {
                collapsed.push_str(grapheme);
            }
        }

        index = end;
    }

    collapsed
}

fn trim_trailing_sentence_punctuation(graphemes: &[&str]) -> usize {
    let mut end = graphemes.len();
    while end > 0 && is_sentence_punctuation(graphemes[end - 1]) {
        end -= 1;
    }
    end
}

fn matches_repeated_unit(graphemes: &[&str], unit_len: usize) -> bool {
    graphemes
        .iter()
        .enumerate()
        .all(|(index, grapheme)| *grapheme == graphemes[index % unit_len])
}

fn is_vocalization_unit(unit: &[&str]) -> bool {
    let mut has_voice = false;
    for grapheme in unit {
        if is_repetition_separator(grapheme) {
            continue;
        }
        if !is_vocalization_grapheme(grapheme) {
            return false;
        }
        has_voice = true;
    }
    has_voice
}

fn is_repetition_separator(grapheme: &str) -> bool {
    grapheme.chars().all(char::is_whitespace)
}

fn is_vocalization_grapheme(grapheme: &str) -> bool {
    grapheme.chars().all(is_vocalization_char)
}

fn is_vocalization_char(value: char) -> bool {
    matches!(
        value,
        '啊' | '呀'
            | '哈'
            | '呵'
            | '嘿'
            | '哼'
            | '嗯'
            | '唔'
            | '呜'
            | '哇'
            | '哎'
            | '唉'
            | '诶'
            | '欸'
            | '噢'
            | '哦'
            | '喔'
            | '呃'
            | '呐'
            | '啦'
            | '嗨'
            | '咦'
            | '嘘'
            | '咿'
            | '吼'
            | 'あ'
            | 'ぁ'
            | 'ア'
            | 'ァ'
            | 'い'
            | 'ぃ'
            | 'イ'
            | 'ィ'
            | 'う'
            | 'ぅ'
            | 'ウ'
            | 'ゥ'
            | 'え'
            | 'ぇ'
            | 'エ'
            | 'ェ'
            | 'お'
            | 'ぉ'
            | 'オ'
            | 'ォ'
            | 'ん'
            | 'ン'
            | 'ー'
            | 'ｰ'
            | '〜'
            | '～'
            | '~'
            | 'a'
            | 'A'
            | 'e'
            | 'E'
            | 'i'
            | 'I'
            | 'o'
            | 'O'
            | 'u'
            | 'U'
            | 'h'
            | 'H'
            | 'm'
            | 'M'
    )
}

fn is_sentence_punctuation(grapheme: &str) -> bool {
    matches!(
        grapheme,
        "." | "。" | "!" | "！" | "?" | "？" | "," | "，" | "、" | "…"
    )
}
