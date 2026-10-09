use std::{fs, path::Path};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use tauri::AppHandle;
use uuid::Uuid;

use crate::{
    jobs::SourceSubtitleEdit,
    paths::path_to_string,
    subtitles::{parse_srt_text, render_srt, write_srt_text},
};

use super::{
    events::emit_task,
    schema::{connection, task_from_row},
    source_edit::apply_text_edits,
    task_work_dir, TaskRecord,
};

pub(crate) fn save_translated_subtitles(
    app: &AppHandle,
    task_id: &str,
    original_source_srt: &str,
    original_translated_srt: &str,
    edits: Vec<SourceSubtitleEdit>,
) -> Result<TaskRecord, String> {
    let mut conn = connection(app)?;
    let work_dir = task_work_dir(app, task_id)?;
    let task = save_in_connection(
        &mut conn,
        &work_dir,
        task_id,
        original_source_srt,
        original_translated_srt,
        edits,
    )?;
    emit_task(app, task_id);
    Ok(task)
}

// The command holds task_mutations. New files plus a transaction preserve the
// previous revision (and the user's existing exports) on every failure path.
fn save_in_connection(
    conn: &mut Connection,
    work_dir: &Path,
    task_id: &str,
    original_source_srt: &str,
    original_translated_srt: &str,
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
        return Err("任务正在运行或排队中，稍后再编辑译文".to_string());
    }
    let source_path = task
        .source_srt_path
        .as_deref()
        .ok_or_else(|| "没有找到原文字幕".to_string())?;
    let translated_path = task
        .translated_srt_path
        .as_deref()
        .ok_or_else(|| "没有找到可编辑的译文字幕".to_string())?;
    let source =
        fs::read_to_string(source_path).map_err(|error| format!("读取原文字幕失败: {error}"))?;
    let translated = fs::read_to_string(translated_path)
        .map_err(|error| format!("读取译文字幕失败: {error}"))?;
    if source != original_source_srt || translated != original_translated_srt {
        return Err("原文或译文字幕已发生变化，请重新打开预览后再编辑".to_string());
    }
    let source_segments = parse_srt_text(&source).map_err(|error| format!("{error:?}"))?;
    let edited = apply_text_edits(&translated, edits)?;
    if source_segments.len() != edited.len()
        || source_segments.iter().zip(&edited).any(|(source, target)| {
            (source.id, source.start_ms, source.end_ms)
                != (target.id, target.start_ms, target.end_ms)
        })
    {
        return Err("原文与译文的编号或时间轴不一致，请重新翻译后再编辑".to_string());
    }
    let body = render_srt(&edited, None);
    if body == translated {
        return Ok(task);
    }
    let new_path = work_dir.join(format!("translation-edit-{}.srt", Uuid::new_v4()));
    tauri::async_runtime::block_on(write_srt_text(&new_path, &body))
        .map_err(|error| format!("{error:?}"))?;
    let persist = || -> Result<TaskRecord, String> {
        tx.execute(
            "UPDATE tasks SET translated_srt_path = ?1, result_revision = result_revision + 1,
             exported_source_srt = NULL, exported_translated_srt = NULL, exported_output_dir = NULL,
             status = 'completed', stage = 'completed', message = '译文字幕已保存，请重新导出',
             progress = 1.0, error = NULL, updated_at = ?2 WHERE id = ?3",
            params![path_to_string(new_path.clone()), super::now_ts(), task_id],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO task_logs(task_id, created_at, line) VALUES(?1, ?2, ?3)",
            params![
                task_id,
                super::now_ts(),
                "translation-edit · 译文字幕已保存"
            ],
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
        let _ = fs::remove_file(&new_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_db::schema::migrate;
    use std::path::PathBuf;

    const SOURCE: &str =
        "4\n00:00:01,250 --> 00:00:02,750\nHello\n\n9\n00:00:03,000 --> 00:00:04,500\nWorld\n\n";
    const TRANSLATED: &str =
        "4\n00:00:01,250 --> 00:00:02,750\n你好\n\n9\n00:00:03,000 --> 00:00:04,500\n世界\n\n";

    struct Fixture {
        conn: Connection,
        dir: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("luma-translation-edit-{}", Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("source.srt"), SOURCE).unwrap();
            fs::write(dir.join("translated.srt"), TRANSLATED).unwrap();
            let conn = Connection::open_in_memory().unwrap();
            migrate(&conn).unwrap();
            conn.execute(
                "INSERT INTO tasks(id, source_type, file_name, status, stage, message, progress,
                 settings_json, source_srt_path, translated_srt_path, translated_file_name,
                 exported_translated_srt, created_at, updated_at)
                 VALUES('task', 'srt', 'clip.srt', 'exported', 'exported', '', 1, ?1, ?2, ?3,
                 'clip.zh.srt', 'existing-export.srt', 1, 1)",
                params![r#"{"output_dir":null,"target_language":"简体中文","whisper_model_path":"","whisper_language":"auto","base_url":"","model":"test","temperature":0.2}"#,
                    dir.join("source.srt").to_str(), dir.join("translated.srt").to_str()],
            ).unwrap();
            Self { conn, dir }
        }
        fn save(&mut self, source: &str, translated: &str) -> Result<TaskRecord, String> {
            save_in_connection(
                &mut self.conn,
                &self.dir,
                "task",
                source,
                translated,
                vec![
                    SourceSubtitleEdit {
                        id: 9,
                        text: "新世界".into(),
                    },
                    SourceSubtitleEdit {
                        id: 4,
                        text: "您好".into(),
                    },
                ],
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn translation_edits_keep_source_alignment_and_preserve_previous_files() {
        let mut fixture = Fixture::new();
        let saved = fixture.save(SOURCE, TRANSLATED).unwrap();
        let edited =
            parse_srt_text(&fs::read_to_string(saved.translated_srt_path.unwrap()).unwrap())
                .unwrap();
        assert_eq!(
            (edited[0].id, edited[0].start_ms, edited[0].end_ms),
            (4, 1250, 2750)
        );
        assert_eq!(edited[0].text, "您好");
        assert_eq!(saved.result_revision, 1);
        assert!(saved.exported_translated_srt.is_none());
        assert_eq!(saved.translated_file_name.as_deref(), Some("clip.zh.srt"));
        assert_eq!(
            fs::read_to_string(fixture.dir.join("source.srt")).unwrap(),
            SOURCE
        );
        assert_eq!(
            fs::read_to_string(fixture.dir.join("translated.srt")).unwrap(),
            TRANSLATED
        );
    }

    #[test]
    fn rejects_stale_source_translation_and_busy_tasks() {
        let mut fixture = Fixture::new();
        assert!(fixture
            .save("stale", TRANSLATED)
            .err()
            .unwrap()
            .contains("已发生变化"));
        assert!(fixture
            .save(SOURCE, "stale")
            .err()
            .unwrap()
            .contains("已发生变化"));
        fixture
            .conn
            .execute("UPDATE tasks SET status = 'running'", [])
            .unwrap();
        assert!(fixture
            .save(SOURCE, TRANSLATED)
            .err()
            .unwrap()
            .contains("正在运行"));
        assert_eq!(fs::read_dir(&fixture.dir).unwrap().count(), 2);
    }

    #[test]
    fn database_failure_leaves_previous_revision_intact() {
        let mut fixture = Fixture::new();
        fixture.conn.execute_batch("CREATE TRIGGER fail_log BEFORE INSERT ON task_logs BEGIN SELECT RAISE(ABORT, 'no log'); END;").unwrap();
        assert!(fixture.save(SOURCE, TRANSLATED).is_err());
        let revision: i64 = fixture
            .conn
            .query_row("SELECT result_revision FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(revision, 0);
        assert_eq!(fs::read_dir(&fixture.dir).unwrap().count(), 2);
    }

    #[test]
    fn rejects_misaligned_translation() {
        let mut fixture = Fixture::new();
        let moved = TRANSLATED.replace("00:00:01,250", "00:00:01,500");
        fs::write(fixture.dir.join("translated.srt"), &moved).unwrap();
        assert!(fixture
            .save(SOURCE, &moved)
            .err()
            .unwrap()
            .contains("时间轴不一致"));
    }
}
