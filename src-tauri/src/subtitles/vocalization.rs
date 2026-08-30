use unicode_segmentation::UnicodeSegmentation;

use super::normalize_subtitle_text;

// Leave natural, short repetitions alone; excessive runs retain the same short
// representation in Whisper output, translation requests, and translated text.
const MIN_REPETITIONS: usize = 6;
const MAX_UNIT_GRAPHEMES: usize = 4;
const SUMMARIZED_REPETITIONS: usize = 3;

pub(crate) fn collapse_repeated_vocalization(text: &str) -> String {
    summarize_repeated_vocalization(text)
}

pub(crate) fn summarize_repeated_vocalization(text: &str) -> String {
    let normalized = normalize_subtitle_text(text);
    let graphemes = normalized.graphemes(true).collect::<Vec<_>>();
    let mut summarized = String::with_capacity(normalized.len());
    let mut index = 0;

    while index < graphemes.len() {
        if let Some((kept_end, end)) = repeated_vocalization_run(&graphemes, index) {
            append_graphemes(&mut summarized, &graphemes[index..kept_end]);
            append_ellipsis(&mut summarized, &graphemes[end..]);
            index = end;
        } else if let Some((token, end)) = shortened_latin_vocalization(&graphemes, index) {
            summarized.push_str(&token);
            append_ellipsis(&mut summarized, &graphemes[end..]);
            index = end;
        } else {
            summarized.push_str(graphemes[index]);
            index += 1;
        }
    }

    summarized
}

// Return the end of the retained repetitions and the end of the entire run.
// Separators belong to the run only when another matching unit follows them,
// so final punctuation, whitespace, and surrounding dialogue stay intact.
fn repeated_vocalization_run(graphemes: &[&str], start: usize) -> Option<(usize, usize)> {
    for unit_len in 1..=MAX_UNIT_GRAPHEMES.min(graphemes.len() - start) {
        let unit = &graphemes[start..start + unit_len];
        let latin = unit.iter().all(|grapheme| is_latin_word_grapheme(grapheme));
        let dragged_vowel = unit.iter().all(|grapheme| is_length_mark(grapheme))
            && start > 0
            && is_vocalization_grapheme(graphemes[start - 1])
            && !is_length_mark(graphemes[start - 1]);
        if !(is_vocalization_unit(unit) || dragged_vowel)
            || (latin && start > 0 && is_word_grapheme(graphemes[start - 1]))
            || (latin && !matches_latin_token(graphemes, start, unit))
        {
            continue;
        }

        let mut end = start + unit_len;
        let mut kept_end = end;
        let mut repetitions = 1;
        loop {
            let mut next = end;
            while next < graphemes.len() && is_repetition_separator(graphemes, next) {
                next += 1;
            }
            // Isolated vowels can be articles, pronouns, or spelled letters.
            // Only shorten their contiguous dragged-out form ("aaaaaa").
            if latin && unit_len == 1 && is_latin_vowel(unit[0]) && next != end {
                break;
            }
            if next + unit_len > graphemes.len()
                || (latin && next > end && !matches_latin_token(graphemes, next, unit))
                || !unit
                    .iter()
                    .zip(&graphemes[next..next + unit_len])
                    .all(|(left, right)| left.eq_ignore_ascii_case(right))
            {
                break;
            }
            end = next + unit_len;
            repetitions += 1;
            if repetitions <= SUMMARIZED_REPETITIONS {
                kept_end = end;
            }
        }

        if repetitions >= MIN_REPETITIONS
            && !(latin && end < graphemes.len() && is_word_grapheme(graphemes[end]))
        {
            return Some((kept_end, end));
        }
    }
    None
}

// Check each Latin token before extending a run. Otherwise the "um" prefix of
// "umbrella" would invalidate every preceding filler and repeatedly rescan it.
fn matches_latin_token(graphemes: &[&str], start: usize, unit: &[&str]) -> bool {
    let mut end = start;
    while end < graphemes.len() && is_word_grapheme(graphemes[end]) {
        if !graphemes[end].eq_ignore_ascii_case(unit[(end - start) % unit.len()]) {
            return false;
        }
        end += 1;
    }
    end > start && (end - start) % unit.len() == 0
}

// Handle a single stretched interjection such as "uhhhhhhhh", but never trim
// repeated letters inside ordinary words ("coooooool"), numbers, or identifiers.
fn shortened_latin_vocalization(graphemes: &[&str], start: usize) -> Option<(String, usize)> {
    if !is_latin_word_grapheme(graphemes[start])
        || (start > 0 && is_word_grapheme(graphemes[start - 1]))
    {
        return None;
    }
    let mut end = start;
    while end < graphemes.len() && is_latin_word_grapheme(graphemes[end]) {
        end += 1;
    }
    if end < graphemes.len() && is_word_grapheme(graphemes[end]) {
        return None;
    }

    let mut compact = String::new();
    let mut shortened = String::new();
    let mut changed = false;
    let mut index = start;
    while index < end {
        let mut run_end = index + 1;
        while run_end < end && graphemes[index].eq_ignore_ascii_case(graphemes[run_end]) {
            run_end += 1;
        }
        compact.push_str(&graphemes[index].to_ascii_lowercase());
        let run_len = run_end - index;
        let kept = if run_len >= MIN_REPETITIONS {
            changed = true;
            SUMMARIZED_REPETITIONS
        } else {
            run_len
        };
        append_graphemes(&mut shortened, &graphemes[index..index + kept]);
        index = run_end;
    }
    (changed && is_latin_vocalization(&compact)).then_some((shortened, end))
}

fn is_vocalization_unit(unit: &[&str]) -> bool {
    if unit.iter().all(|grapheme| is_latin_word_grapheme(grapheme)) {
        let mut letters = unit.concat().to_ascii_lowercase().into_bytes();
        letters.dedup();
        return is_latin_vocalization(std::str::from_utf8(&letters).unwrap_or_default());
    }
    if matches!(
        unit.concat().as_str(),
        "えっと" | "えーと" | "えと" | "あの" | "あのー"
    ) {
        return true;
    }
    unit.iter()
        .all(|grapheme| is_vocalization_grapheme(grapheme))
        && unit.iter().any(|grapheme| !is_length_mark(grapheme))
}

fn is_latin_vocalization(unit: &str) -> bool {
    matches!(
        unit,
        "a" | "e"
            | "i"
            | "o"
            | "u"
            | "h"
            | "m"
            | "ah"
            | "eh"
            | "oh"
            | "uh"
            | "um"
            | "er"
            | "erm"
            | "ha"
            | "heh"
            | "hm"
            | "hmm"
            | "mm"
            | "mhm"
            | "huh"
            | "sh"
    )
}

fn is_latin_vowel(grapheme: &str) -> bool {
    matches!(
        grapheme,
        "a" | "A" | "e" | "E" | "i" | "I" | "o" | "O" | "u" | "U"
    )
}

fn is_latin_word_grapheme(grapheme: &str) -> bool {
    grapheme.len() == 1 && grapheme.as_bytes()[0].is_ascii_alphabetic()
}

fn is_word_grapheme(grapheme: &str) -> bool {
    grapheme
        .chars()
        .any(|value| value.is_alphanumeric() || value == '_')
}

fn is_repetition_separator(graphemes: &[&str], index: usize) -> bool {
    let grapheme = graphemes[index];
    // An ellipsis is a boundary, including the ones this cleanup inserts. This
    // keeps separate runs separate when saved text is cleaned again for an API.
    if grapheme == "." {
        return (index == 0 || graphemes[index - 1] != ".")
            && (index + 1 == graphemes.len() || graphemes[index + 1] != ".");
    }
    grapheme.chars().all(char::is_whitespace)
        || matches!(
            grapheme,
            "," | "，" | "、" | "。" | "!" | "！" | "?" | "？" | ";" | "；" | "-" | "—" | "–"
        )
}

fn append_graphemes(output: &mut String, graphemes: &[&str]) {
    for grapheme in graphemes {
        output.push_str(grapheme);
    }
}

fn append_ellipsis(output: &mut String, remaining: &[&str]) {
    if !remaining.starts_with(&["…"]) && !remaining.starts_with(&[".", "."]) {
        output.push_str("...");
    }
}

fn is_length_mark(grapheme: &str) -> bool {
    matches!(grapheme, "ー" | "ｰ" | "〜" | "～" | "~")
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

#[cfg(test)]
mod tests {
    use super::collapse_repeated_vocalization as clean;

    #[test]
    fn cleans_repeated_filler_units_inside_dialogue() {
        for (input, expected) in [
            (
                format!("他说：{}继续。", "啊".repeat(80)),
                "他说：啊啊啊...继续。",
            ),
            (
                format!("前文{}后文", "あー".repeat(40)),
                "前文あーあーあー...后文",
            ),
            (
                format!("前文{}后文", "嗯哼".repeat(20)),
                "前文嗯哼嗯哼嗯哼...后文",
            ),
            (
                format!("「{}！」", "えっと、".repeat(6).trim_end_matches('、')),
                "「えっと、えっと、えっと...！」",
            ),
            (
                "We uh uh uh uh uh uh should go.".into(),
                "We uh uh uh... should go.",
            ),
            ("Oh, OH oh, Oh OH oh!".into(), "Oh, OH oh...!"),
            ("Well, ha-ha-ha-ha-ha-ha!".into(), "Well, ha-ha-ha...!"),
        ] {
            assert_eq!(clean(&input), expected, "input: {input}");
        }
    }

    #[test]
    fn handles_varying_separators_and_preserves_final_punctuation() {
        for (input, expected) in [
            (
                "嗯，嗯 嗯、嗯, 嗯，嗯，我们继续。",
                "嗯，嗯 嗯...，我们继续。",
            ),
            ("啊啊啊啊啊啊！", "啊啊啊...！"),
            ("嗯嗯嗯嗯嗯嗯？", "嗯嗯嗯...？"),
            ("uh. uh. uh. uh. uh. uh?", "uh. uh. uh...?"),
            ("啊啊啊啊啊啊...", "啊啊啊..."),
            ("啊啊啊啊啊啊…", "啊啊啊…"),
        ] {
            assert_eq!(clean(input), expected, "input: {input}");
        }
    }

    #[test]
    fn preserves_words_that_start_with_the_repeated_filler() {
        for (input, expected) in [
            ("um um um um um um umbrella", "um um um... umbrella"),
            ("ha ha ha ha ha ha happy", "ha ha ha... happy"),
            ("hmmm hmmm hmmm hmmm hmmm hmmm?", "hmmm hmmm hmmm...?"),
            ("ohhh ohhh ohhh ohhh ohhh ohhh!", "ohhh ohhh ohhh...!"),
        ] {
            assert_eq!(clean(input), expected, "input: {input}");
        }
        let long_run = format!("{}happy", "ha ".repeat(10_000));
        assert_eq!(clean(&long_run), "ha ha ha... happy");
    }

    #[test]
    fn shortens_stretched_interjections_without_changing_words() {
        for (input, expected) in [
            ("Well, uhhhhhhhh!", "Well, uhhh...!"),
            ("HMMMMMMMM?", "HMMM...?"),
            ("Aaaaaaaa!", "Aaa...!"),
            ("あーーーーーーーー！", "あーーー...！"),
            ("~~~~~~~~", "~~~~~~~~"),
            ("coooooool", "coooooool"),
            ("Muhahahahahahaha", "Muhahahahahahaha"),
            ("hahahahahahello", "hahahahahahello"),
            ("id_aaaaaaaa_123", "id_aaaaaaaa_123"),
            ("aaaaaa42", "aaaaaa42"),
        ] {
            assert_eq!(clean(input), expected, "input: {input}");
        }
    }

    #[test]
    fn preserves_short_repeats_regular_words_and_separate_phrases() {
        for input in [
            "啊啊啊啊啊",
            "uh uh uh uh uh",
            "I I I I I I",
            "a a a a a a",
            "你好你好你好你好你好你好",
            "home home home home home home",
            "mama mama mama mama mama mama",
            "no no no no no no",
            "嗯，我知道，嗯，继续，嗯，好的，嗯，明白，嗯，再见，嗯。",
            "áááááá 👩‍👩‍👧‍👦👩‍👩‍👧‍👦",
            "111111111111111111111111",
        ] {
            assert_eq!(clean(input), input);
        }
        let long_dialogue = "这是需要完整翻译的正常台词。".repeat(100);
        assert_eq!(clean(&long_dialogue), long_dialogue);
    }

    #[test]
    fn cleanup_is_stable_across_transcription_request_and_response() {
        for input in [
            "啊".repeat(10_000),
            "前文あーあーあーあーあーあー后文".into(),
            "嗯，嗯，嗯，嗯，嗯，嗯...嗯，嗯，嗯，嗯，嗯，嗯！".into(),
            "uhhhhhhhhh... uhhhhhhhh!".into(),
            "あーーーーーーーー！".into(),
            "啊啊啊啊啊啊…".into(),
        ] {
            let once = clean(&input);
            assert_eq!(clean(&once), once, "input: {input}");
        }
        assert_eq!(clean(&"啊".repeat(10_000)), "啊啊啊...");
    }
}
