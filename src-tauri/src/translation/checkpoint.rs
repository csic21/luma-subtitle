use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    state::{JobError, JobResult},
    subtitles::{SubtitleSegment, TranslatedSegment},
};

pub(crate) const CHECKPOINT_FILE_NAME: &str = "translation-checkpoint.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct TranslationCheckpoint {
    pub(crate) version: u32,
    pub(crate) target_language: String,
    pub(crate) source_fingerprint: String,
    pub(crate) translations: Vec<TranslatedCue>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct TranslatedCue {
    pub(crate) id: usize,
    pub(crate) text: String,
}

impl From<&TranslatedSegment> for TranslatedCue {
    fn from(value: &TranslatedSegment) -> Self {
        Self {
            id: value.id,
            text: value.text.clone(),
        }
    }
}

impl From<&TranslatedCue> for TranslatedSegment {
    fn from(value: &TranslatedCue) -> Self {
        TranslatedSegment {
            id: value.id,
            text: value.text.clone(),
        }
    }
}

pub(crate) fn checkpoint_path(work_dir: &Path) -> PathBuf {
    work_dir.join(CHECKPOINT_FILE_NAME)
}

pub(crate) fn source_fingerprint(segments: &[SubtitleSegment]) -> String {
    let mut hasher = DefaultHasher::new();
    segments.len().hash(&mut hasher);
    for segment in segments {
        segment.id.hash(&mut hasher);
        segment.start_ms.hash(&mut hasher);
        segment.end_ms.hash(&mut hasher);
        segment.text.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

pub(crate) fn load_compatible_checkpoint(
    path: &Path,
    target_language: &str,
    fingerprint: &str,
) -> JobResult<Vec<TranslatedSegment>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let body = std::fs::read_to_string(path)
        .map_err(|error| JobError::failed(format!("读取翻译断点失败: {error}")))?;
    let checkpoint: TranslationCheckpoint = serde_json::from_str(&body)
        .map_err(|error| JobError::failed(format!("解析翻译断点失败: {error}")))?;
    if checkpoint.version != 1 {
        return Ok(Vec::new());
    }
    if checkpoint.target_language.trim() != target_language.trim() {
        return Ok(Vec::new());
    }
    if checkpoint.source_fingerprint != fingerprint {
        return Ok(Vec::new());
    }
    let mut by_id = HashMap::new();
    for cue in checkpoint.translations {
        if cue.text.trim().is_empty() {
            continue;
        }
        by_id.insert(cue.id, TranslatedSegment {
            id: cue.id,
            text: cue.text,
        });
    }
    let mut translations = by_id.into_values().collect::<Vec<_>>();
    translations.sort_by_key(|item| item.id);
    Ok(translations)
}

pub(crate) fn save_checkpoint(
    path: &Path,
    target_language: &str,
    fingerprint: &str,
    translations: &[TranslatedSegment],
) -> JobResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| JobError::failed(format!("创建翻译断点目录失败: {error}")))?;
    }
    let mut by_id = HashMap::new();
    for item in translations {
        if item.text.trim().is_empty() {
            continue;
        }
        by_id.insert(item.id, TranslatedCue::from(item));
    }
    let mut cues = by_id.into_values().collect::<Vec<_>>();
    cues.sort_by_key(|item| item.id);
    let checkpoint = TranslationCheckpoint {
        version: 1,
        target_language: target_language.to_string(),
        source_fingerprint: fingerprint.to_string(),
        translations: cues,
    };
    let body = serde_json::to_string_pretty(&checkpoint)
        .map_err(|error| JobError::failed(format!("序列化翻译断点失败: {error}")))?;
    let temporary = path.with_extension(format!(
        "tmp-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&temporary, body)
        .map_err(|error| JobError::failed(format!("写入翻译断点失败: {error}")))?;
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        JobError::failed(format!("替换翻译断点失败: {error}"))
    })?;
    Ok(())
}

pub(crate) fn clear_checkpoint(path: &Path) -> JobResult<()> {
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|error| JobError::failed(format!("清除翻译断点失败: {error}")))?;
    }
    Ok(())
}

pub(crate) fn remaining_segments<'a>(
    segments: &'a [SubtitleSegment],
    completed: &[TranslatedSegment],
) -> Vec<&'a SubtitleSegment> {
    let done = completed
        .iter()
        .filter(|item| !item.text.trim().is_empty())
        .map(|item| item.id)
        .collect::<HashSet<_>>();
    segments
        .iter()
        .filter(|segment| !done.contains(&segment.id))
        .collect()
}

pub(crate) fn merge_translations(
    existing: &[TranslatedSegment],
    newly: &[TranslatedSegment],
) -> Vec<TranslatedSegment> {
    let mut by_id = HashMap::new();
    for item in existing.iter().chain(newly.iter()) {
        if item.text.trim().is_empty() {
            continue;
        }
        by_id.insert(item.id, item.clone());
    }
    let mut merged = by_id.into_values().collect::<Vec<_>>();
    merged.sort_by_key(|item| item.id);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process, time::{SystemTime, UNIX_EPOCH}};

    fn sample_segments() -> Vec<SubtitleSegment> {
        vec![
            SubtitleSegment {
                id: 1,
                start_ms: 0,
                end_ms: 1000,
                text: "hello".to_string(),
            },
            SubtitleSegment {
                id: 2,
                start_ms: 1000,
                end_ms: 2000,
                text: "world".to_string(),
            },
            SubtitleSegment {
                id: 3,
                start_ms: 2000,
                end_ms: 3000,
                text: "again".to_string(),
            },
        ]
    }

    #[test]
    fn checkpoint_round_trip_keeps_compatible_translations() {
        let dir = temp_dir("round-trip");
        let path = checkpoint_path(&dir);
        let segments = sample_segments();
        let fingerprint = source_fingerprint(&segments);
        let translations = vec![
            TranslatedSegment {
                id: 1,
                text: "你好".to_string(),
            },
            TranslatedSegment {
                id: 2,
                text: "世界".to_string(),
            },
        ];
        save_checkpoint(&path, "简体中文", &fingerprint, &translations).unwrap();
        let loaded = load_compatible_checkpoint(&path, "简体中文", &fingerprint).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].text, "你好");
        let remaining = remaining_segments(&segments, &loaded);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn incompatible_target_or_source_is_ignored() {
        let dir = temp_dir("incompatible");
        let path = checkpoint_path(&dir);
        let segments = sample_segments();
        let fingerprint = source_fingerprint(&segments);
        save_checkpoint(
            &path,
            "简体中文",
            &fingerprint,
            &[TranslatedSegment {
                id: 1,
                text: "你好".to_string(),
            }],
        )
        .unwrap();
        assert!(load_compatible_checkpoint(&path, "English", &fingerprint)
            .unwrap()
            .is_empty());
        assert!(load_compatible_checkpoint(&path, "简体中文", "deadbeef")
            .unwrap()
            .is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "luma-checkpoint-{name}-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
