use std::{collections::HashMap, fs, path::Path};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use tauri::AppHandle;
use uuid::Uuid;

use crate::{
    jobs::SourceSubtitleEdit,
    paths::path_to_string,
    state::JobError,
    subtitles::{
        normalize_subtitle_text, parse_srt_text, render_srt, write_srt_text, SubtitleSegment,
    },
};

use super::{
    events::emit_task,
    schema::{connection, task_from_row},
    task_work_dir, TaskRecord,
};

const SAVED_MESSAGE: &str = "原文字幕已保存，请重新翻译或导出";

pub(crate) fn save_source_subtitles(
    app: &AppHandle,
    task_id: &str,
    original_source_srt: &str,
    edits: Vec<SourceSubtitleEdit>,
) -> Result<TaskRecord, String> {
    super::require_task(app, task_id)?;
    let mut conn = connection(app)?;
    let work_dir = task_work_dir(app, task_id)?;
    let task = save_in_connection(&mut conn, &work_dir, task_id, original_source_srt, edits)?;
    emit_task(app, task_id);
    Ok(task)
}

// The caller holds AppState::task_mutations through persistence and cache invalidation.
// A new source file makes the database commit the only switch to the edited revision.
fn save_in_connection(
    conn: &mut Connection,
    work_dir: &Path,
    task_id: &str,
    original_source_srt: &str,
    edits: Vec<SourceSubtitleEdit>,
) -> Result<TaskRecord, String> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let task = tx
        .query_row(
            "SELECT * FROM tasks WHERE id = ?1",
            params![task_id],
            task_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "没有找到任务".to_string())?;
    if matches!(task.status.as_str(), "queued" | "running") {
        return Err("任务正在运行或排队中，稍后再编辑原文".to_string());
    }
    let source_path = task
        .source_srt_path
        .as_deref()
        .ok_or_else(|| "没有找到可编辑的原文字幕".to_string())?;
    let current_source =
        fs::read_to_string(source_path).map_err(|error| format!("读取原文字幕失败: {error}"))?;
    if current_source != original_source_srt {
        return Err("原文字幕已发生变化，请重新打开预览后再编辑".to_string());
    }
    let segments = apply_text_edits(&current_source, edits)?;
    let source_srt = render_srt(&segments, None);
    if source_srt == current_source {
        return Ok(task);
    }

    let new_source_path = work_dir.join(format!("source-edit-{}.srt", Uuid::new_v4()));
    tauri::async_runtime::block_on(write_srt_text(&new_source_path, &source_srt))
        .map_err(job_error_message)?;

    let persist = || -> Result<TaskRecord, String> {
        let now = super::now_ts();
        tx.execute(
            "UPDATE tasks SET
                source_srt_path = ?1,
                segment_count = ?2,
                translated_srt_path = NULL,
                translated_file_name = NULL,
                exported_source_srt = NULL,
                exported_translated_srt = NULL,
                exported_output_dir = NULL,
                status = 'completed',
                stage = 'source-ready',
                message = ?3,
                progress = 1.0,
                error = NULL,
                updated_at = ?4
            WHERE id = ?5",
            params![
                path_to_string(new_source_path.clone()),
                segments.len() as i64,
                SAVED_MESSAGE,
                now,
                task_id
            ],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO task_logs(task_id, created_at, line) VALUES(?1, ?2, ?3)",
            params![task_id, now, format!("source-ready · {SAVED_MESSAGE}")],
        )
        .map_err(|error| error.to_string())?;
        let saved = tx
            .query_row(
                "SELECT * FROM tasks WHERE id = ?1",
                params![task_id],
                task_from_row,
            )
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(saved)
    };
    let result = persist();
    if result.is_err() {
        let _ = fs::remove_file(&new_source_path);
    }
    result
}

fn apply_text_edits(
    source_srt: &str,
    edits: Vec<SourceSubtitleEdit>,
) -> Result<Vec<SubtitleSegment>, String> {
    let mut segments = parse_srt_text(source_srt).map_err(job_error_message)?;
    if edits.len() != segments.len() {
        return Err("字幕条目数量已变化，请重新打开预览后再编辑".to_string());
    }
    let mut original_ids = std::collections::HashSet::new();
    if segments
        .iter()
        .any(|segment| !original_ids.insert(segment.id))
    {
        return Err("原文字幕包含重复编号，无法安全编辑".to_string());
    }
    let mut by_id = HashMap::new();
    for edit in edits {
        if !original_ids.contains(&edit.id) || by_id.contains_key(&edit.id) {
            return Err("字幕编号已变化或重复，请重新打开预览后再编辑".to_string());
        }
        let text = normalize_subtitle_text(&edit.text);
        if text.is_empty() {
            return Err(format!("第 {} 条字幕原文不能为空", edit.id));
        }
        by_id.insert(edit.id, text);
    }
    for segment in &mut segments {
        segment.text = by_id
            .remove(&segment.id)
            .expect("all original IDs were validated");
    }
    Ok(segments)
}

fn job_error_message(error: JobError) -> String {
    match error {
        JobError::Cancelled => "保存原文字幕已取消".to_string(),
        JobError::Failed(message) => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_db::{schema::migrate, TaskSettingsSnapshot};
    use std::path::PathBuf;

    const ORIGINAL: &str = "4\n00:00:01,250 --> 00:00:02,750\nOriginal first line\n\n9\n00:00:03,000 --> 00:00:04,500\nOriginal second line\n\n";

    struct Fixture {
        conn: Connection,
        dir: PathBuf,
        work_dir: PathBuf,
        original: PathBuf,
        translated: PathBuf,
        exported_source: PathBuf,
        exported_translation: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("luma-source-edit-{}", Uuid::new_v4()));
            let work_dir = dir.join("task-work");
            fs::create_dir_all(&work_dir).unwrap();
            let original = dir.join("imported-original.srt");
            let translated = work_dir.join("translated.srt");
            let exported_source = dir.join("exported-source.srt");
            let exported_translation = dir.join("exported-translated.srt");
            for (path, text) in [
                (&original, ORIGINAL),
                (&translated, "previous translation"),
                (&exported_source, ORIGINAL),
                (&exported_translation, "previous exported translation"),
            ] {
                fs::write(path, text).unwrap();
            }
            let conn = Connection::open_in_memory().unwrap();
            migrate(&conn).unwrap();
            let settings = serde_json::to_string(&TaskSettingsSnapshot {
                output_dir: Some(path_to_string(dir.clone())),
                target_language: "简体中文".to_string(),
                whisper_model_path: "model.bin".to_string(),
                whisper_language: "auto".to_string(),
                base_url: "https://example.test".to_string(),
                base_url_is_complete: false,
                model: "model".to_string(),
                temperature: 0.2,
                translation_shard_size: 120,
                translation_provider: "api".to_string(),
                translation_cli_tool: "opencode".to_string(),
                translation_cli_command: "opencode".to_string(),
                translation_cli_model: String::new(),
                translation_cli_args: String::new(),
            })
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (
                    id, source_type, srt_path, file_name, status, stage, message, progress,
                    settings_json, source_srt_path, translated_srt_path, source_file_name,
                    translated_file_name, output_dir, segment_count, exported_source_srt,
                    exported_translated_srt, exported_output_dir, error, created_at, updated_at
                ) VALUES (
                    'task-1', 'srt', ?1, 'original.srt', 'exported', 'exported', 'exported', 1.0,
                    ?2, ?1, ?3, 'original.srt', 'translated.srt', ?4, 2, ?5, ?6, ?4, 'old error', 1, 1
                )",
                params![path_to_string(original.clone()), settings, path_to_string(translated.clone()),
                    path_to_string(dir.clone()), path_to_string(exported_source.clone()),
                    path_to_string(exported_translation.clone())],
            ).unwrap();
            Self {
                conn,
                dir,
                work_dir,
                original,
                translated,
                exported_source,
                exported_translation,
            }
        }

        fn save(
            &mut self,
            original_source: &str,
            edits: Vec<SourceSubtitleEdit>,
        ) -> Result<TaskRecord, String> {
            save_in_connection(
                &mut self.conn,
                &self.work_dir,
                "task-1",
                original_source,
                edits,
            )
        }

        fn task(&self) -> TaskRecord {
            self.conn
                .query_row("SELECT * FROM tasks WHERE id = 'task-1'", [], task_from_row)
                .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn edited_lines() -> Vec<SourceSubtitleEdit> {
        vec![
            SourceSubtitleEdit {
                id: 9,
                text: " 嗯嗯嗯嗯嗯嗯嗯嗯 ".to_string(),
            },
            SourceSubtitleEdit {
                id: 4,
                text: " Fixed\n first\t line ".to_string(),
            },
        ]
    }

    #[test]
    fn edits_persist_without_changing_times_or_overwriting_original_and_exports() {
        let mut fixture = Fixture::new();
        let saved = fixture.save(ORIGINAL, edited_lines()).unwrap();
        let path = saved.source_srt_path.as_ref().unwrap();
        let persisted = fs::read_to_string(path).unwrap();
        let segments = parse_srt_text(&persisted).unwrap();
        assert_ne!(Path::new(path), fixture.original);
        assert!(Path::new(path).starts_with(&fixture.work_dir));
        assert_eq!(
            (segments[0].id, segments[0].start_ms, segments[0].end_ms),
            (4, 1250, 2750)
        );
        assert_eq!(
            (segments[1].id, segments[1].start_ms, segments[1].end_ms),
            (9, 3000, 4500)
        );
        assert_eq!(segments[0].text, "Fixed first line");
        assert_eq!(segments[1].text, "嗯嗯嗯嗯嗯嗯嗯嗯");
        assert_eq!(fs::read_to_string(&fixture.original).unwrap(), ORIGINAL);
        assert_eq!(
            fs::read_to_string(&fixture.translated).unwrap(),
            "previous translation"
        );
        assert_eq!(
            fs::read_to_string(&fixture.exported_source).unwrap(),
            ORIGINAL
        );
        assert_eq!(
            fs::read_to_string(&fixture.exported_translation).unwrap(),
            "previous exported translation"
        );

        assert_eq!(saved.status, "completed");
        assert_eq!(saved.stage, "source-ready");
        assert_eq!(saved.message, SAVED_MESSAGE);
        assert_eq!(saved.segment_count, Some(2));
        assert_eq!(saved.source_file_name.as_deref(), Some("original.srt"));
        assert!(saved.translated_srt_path.is_none());
        assert!(saved.translated_file_name.is_none());
        assert!(saved.exported_source_srt.is_none());
        assert!(saved.exported_translated_srt.is_none());
        assert!(saved.exported_output_dir.is_none());
        assert!(saved.error.is_none());
        assert_eq!(fixture.task().source_srt_path, saved.source_srt_path);
    }

    #[test]
    fn rejects_stale_source_after_a_previous_save() {
        let mut fixture = Fixture::new();
        let saved = fixture.save(ORIGINAL, edited_lines()).unwrap();
        let error = fixture.save(ORIGINAL, edited_lines()).err().unwrap();
        assert!(error.contains("已发生变化"));
        assert_eq!(fixture.task().source_srt_path, saved.source_srt_path);
    }

    #[test]
    fn rejects_queued_or_running_tasks_without_writing_files() {
        let mut fixture = Fixture::new();
        for status in ["queued", "running"] {
            fixture
                .conn
                .execute("UPDATE tasks SET status = ?1", params![status])
                .unwrap();
            let error = fixture.save(ORIGINAL, edited_lines()).err().unwrap();
            assert!(error.contains("正在运行或排队"));
            assert_eq!(
                fixture.task().source_srt_path.as_deref(),
                fixture.original.to_str()
            );
            assert_eq!(fs::read_dir(&fixture.work_dir).unwrap().count(), 1);
        }
    }

    #[test]
    fn rejects_missing_unknown_duplicate_ids_and_empty_text() {
        let missing = vec![SourceSubtitleEdit {
            id: 4,
            text: "only one".to_string(),
        }];
        let unknown = vec![
            SourceSubtitleEdit {
                id: 4,
                text: "first".to_string(),
            },
            SourceSubtitleEdit {
                id: 10,
                text: "second".to_string(),
            },
        ];
        let duplicate = vec![
            SourceSubtitleEdit {
                id: 4,
                text: "first".to_string(),
            },
            SourceSubtitleEdit {
                id: 4,
                text: "second".to_string(),
            },
        ];
        let empty = vec![
            SourceSubtitleEdit {
                id: 4,
                text: " \n\t ".to_string(),
            },
            SourceSubtitleEdit {
                id: 9,
                text: "second".to_string(),
            },
        ];
        for edits in [missing, unknown, duplicate, empty] {
            assert!(apply_text_edits(ORIGINAL, edits).is_err());
        }
        let duplicate_source = ORIGINAL.replace("9\n00:00:03", "4\n00:00:03");
        assert!(apply_text_edits(&duplicate_source, edited_lines())
            .unwrap_err()
            .contains("重复编号"));
    }

    #[test]
    fn database_failure_leaves_previous_source_translation_and_exports_intact() {
        let mut fixture = Fixture::new();
        fixture
            .conn
            .execute_batch(
                "CREATE TRIGGER reject_task_logs BEFORE INSERT ON task_logs
             BEGIN SELECT RAISE(ABORT, 'log write rejected'); END;",
            )
            .unwrap();
        let error = fixture.save(ORIGINAL, edited_lines()).err().unwrap();
        assert!(error.contains("log write rejected"));
        let task = fixture.task();
        assert_eq!(task.source_srt_path.as_deref(), fixture.original.to_str());
        assert_eq!(
            task.translated_srt_path.as_deref(),
            fixture.translated.to_str()
        );
        assert_eq!(
            task.exported_source_srt.as_deref(),
            fixture.exported_source.to_str()
        );
        assert_eq!(task.status, "exported");
        assert_eq!(fs::read_to_string(&fixture.original).unwrap(), ORIGINAL);
        assert_eq!(fs::read_dir(&fixture.work_dir).unwrap().count(), 1);
    }

    #[test]
    fn unchanged_edit_keeps_existing_translation_and_exports() {
        let mut fixture = Fixture::new();
        let edits = parse_srt_text(ORIGINAL)
            .unwrap()
            .into_iter()
            .map(|segment| SourceSubtitleEdit {
                id: segment.id,
                text: segment.text,
            })
            .collect();
        let task = fixture.save(ORIGINAL, edits).unwrap();
        assert_eq!(task.status, "exported");
        assert_eq!(
            task.translated_srt_path.as_deref(),
            fixture.translated.to_str()
        );
        assert_eq!(task.source_srt_path.as_deref(), fixture.original.to_str());
        assert_eq!(fs::read_dir(&fixture.work_dir).unwrap().count(), 1);
    }
}
