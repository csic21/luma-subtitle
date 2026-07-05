use unicode_segmentation::UnicodeSegmentation;

use super::normalize_subtitle_text;

const MIN_REPETITIONS: usize = 6;
const MAX_UNIT_GRAPHEMES: usize = 4;
const MAX_REPEATED_VOCALIZATION_GRAPHEMES: usize = 20;
const SUMMARIZED_REPETITIONS: usize = 3;

pub(crate) fn collapse_repeated_vocalization(text: &str) -> String {
    let normalized = normalize_subtitle_text(text);
    let collapsed = collapse_whole_repeated_vocalization(&normalized).unwrap_or(normalized);
    collapse_repeated_vocalization_runs(&collapsed)
}

pub(crate) fn summarize_repeated_vocalization(text: &str) -> String {
    let normalized = normalize_subtitle_text(text);
    let summarized = summarize_whole_repeated_vocalization(&normalized).unwrap_or(normalized);
    summarize_repeated_vocalization_runs(&summarized)
}

fn collapse_whole_repeated_vocalization(text: &str) -> Option<String> {
    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    if graphemes.len() < MIN_REPETITIONS {
        return None;
    }

    let core_len = trim_trailing_sentence_punctuation(&graphemes);
    let core = &graphemes[..core_len];
    if core.len() < MIN_REPETITIONS {
        return None;
    }

    if repeated_vocalization_unit_len(core).is_some()
        && core.len() > MAX_REPEATED_VOCALIZATION_GRAPHEMES
    {
        return Some(
            core.iter()
                .take(MAX_REPEATED_VOCALIZATION_GRAPHEMES)
                .copied()
                .collect(),
        );
    }

    None
}

fn summarize_whole_repeated_vocalization(text: &str) -> Option<String> {
    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    if graphemes.len() < MIN_REPETITIONS {
        return None;
    }

    let core_len = trim_trailing_sentence_punctuation(&graphemes);
    let core = &graphemes[..core_len];
    if core.len() < MIN_REPETITIONS {
        return None;
    }

    let unit_len = repeated_vocalization_unit_len(core)?;
    let unit = &core[..unit_len];
    let mut summarized = String::new();
    for _ in 0..SUMMARIZED_REPETITIONS {
        summarized.push_str(&unit.concat());
    }
    summarized.push_str("...");
    Some(summarized)
}

fn collapse_repeated_vocalization_runs(text: &str) -> String {
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
        if run_len > MAX_REPEATED_VOCALIZATION_GRAPHEMES && is_vocalization_grapheme(current) {
            for _ in 0..MAX_REPEATED_VOCALIZATION_GRAPHEMES {
                collapsed.push_str(current);
            }
        } else {
            for grapheme in &graphemes[index..end] {
                collapsed.push_str(grapheme);
            }
        }

        index = end;
    }

    collapsed
}

fn summarize_repeated_vocalization_runs(text: &str) -> String {
    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    let mut summarized = String::new();
    let mut index = 0;

    while index < graphemes.len() {
        let current = graphemes[index];
        let mut end = index + 1;
        while end < graphemes.len() && graphemes[end] == current {
            end += 1;
        }

        let run_len = end - index;
        if run_len >= MIN_REPETITIONS && is_vocalization_grapheme(current) {
            for _ in 0..SUMMARIZED_REPETITIONS {
                summarized.push_str(current);
            }
            summarized.push_str("...");
        } else {
            for grapheme in &graphemes[index..end] {
                summarized.push_str(grapheme);
            }
        }

        index = end;
    }

    summarized
}

fn repeated_vocalization_unit_len(core: &[&str]) -> Option<usize> {
    for unit_len in 1..=MAX_UNIT_GRAPHEMES.min(core.len()) {
        let repetitions = core.len().div_ceil(unit_len);
        if repetitions >= MIN_REPETITIONS
            && matches_repeated_unit(core, unit_len)
            && is_vocalization_unit(&core[..unit_len])
        {
            return Some(unit_len);
        }
    }
    None
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
