use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

use tauri::{AppHandle, Manager};

use crate::{
    job_events::{publish_job_event, ExportedSubtitlePaths, JobEventDraft, StoredSubtitleResult},
    paths::path_to_string,
    state::{ensure_not_cancelled, AppState, JobError, JobResult, QueuedTaskOperation},
    subtitles::{parse_srt_file, write_srt_text},
    task_db::{self, TaskRecord},
    translation::{
        checkpoint::{
            checkpoint_path, clear_checkpoint, load_compatible_checkpoint, source_fingerprint,
        },
        TranslationProgressHook, TranslationResume,
    },
};

use super::{
    helpers::{
        display_file_name, operation_cancelled_message, operation_failed_message,
        validate_start_request, validate_translate_request,
    },
    requests::{JobRequest, TranslateSubtitlesRequest},
    single_job::{run_job, run_translation},
};

pub(super) async fn execute_task_operation(
    app: AppHandle,
    queued: QueuedTaskOperation,
    cancel: Arc<AtomicBool>,
) -> bool {
    let generation = task_db::require_task(&app, &queued.task_id)
        .ok()
        .map(|task| task.run_generation);
    let result = match queued.operation.as_str() {
        "transcribe" => run_transcribe_task(app.clone(), &queued.task_id, cancel).await,
        "translate" => run_translate_task(app.clone(), &queued.task_id, cancel, false).await,
        "resume_translate" => run_translate_task(app.clone(), &queued.task_id, cancel, true).await,
        "export" => run_export_task(app.clone(), &queued.task_id, cancel).await,
        _ => Err(JobError::failed("未知任务操作")),
    };

    let state = app.state::<AppState>();
    let _mutation = state.task_mutations.lock();
    if task_db::require_task(&app, &queued.task_id)
        .ok()
        .map(|task| task.run_generation)
        != generation
    {
        return false;
    }
    match result {
        Ok(()) => true,
        Err(JobError::Cancelled) => {
            publish_job_event(
                &app,
                JobEventDraft::cancelled(
                    &queued.task_id,
                    "cancelled",
                    operation_cancelled_message(&queued.operation),
                    "任务已取消",
                ),
            );
            false
        }
        Err(JobError::Failed(message)) => {
            publish_job_event(
                &app,
                JobEventDraft::failed(
                    &queued.task_id,
                    "failed",
                    operation_failed_message(&queued.operation),
                    message,
                ),
            );
            false
        }
    }
}

async fn run_transcribe_task(
    app: AppHandle,
    task_id: &str,
    cancel: Arc<AtomicBool>,
) -> JobResult<()> {
    let task = task_db::require_task(&app, task_id).map_err(JobError::failed)?;
    let media_path = transcription_media_path(&task)?;
    let request = JobRequest {
        media_path,
        source_type: task.source_type.clone(),
        output_dir: task.settings.output_dir.clone(),
        target_language: task.settings.target_language.clone(),
        whisper_model_path: task.settings.whisper_model_path.clone(),
        whisper_language: task.settings.whisper_language.clone(),
        base_url: task.settings.base_url.clone(),
        base_url_is_complete: task.settings.base_url_is_complete,
        model: task.settings.model.clone(),
        temperature: task.settings.temperature,
        translation_shard_size: Some(task.settings.translation_shard_size),
        translation_provider: Some(task.settings.translation_provider.clone()),
        translation_cli_tool: Some(task.settings.translation_cli_tool.clone()),
        translation_cli_command: Some(task.settings.translation_cli_command.clone()),
        translation_cli_model: Some(task.settings.translation_cli_model.clone()),
        translation_cli_args: Some(task.settings.translation_cli_args.clone()),
        translation_local_model_path: Some(task.settings.translation_local_model_path.clone()),
    };
    validate_start_request(&request).map_err(JobError::failed)?;

    publish_job_event(
        &app,
        JobEventDraft::running(task_id, "transcribe", "转写已开始", 0.0),
    );

    let outputs = run_job(app.clone(), task_id.to_string(), request, cancel.clone()).await?;
    let stored = app
        .state::<AppState>()
        .subtitle_results
        .lock()
        .get(task_id)
        .cloned()
        .ok_or_else(|| JobError::failed("转写结果未写入内存"))?;
    let work_dir = task_db::task_work_dir(&app, task_id).map_err(JobError::failed)?;
    let source_srt_path = work_dir.join(format!("source-{}.srt", uuid::Uuid::new_v4()));
    write_srt_text(&source_srt_path, &stored.source_srt).await?;
    let state = app.state::<AppState>();
    let _mutation = state.task_mutations.lock();
    if let Err(error) = ensure_not_cancelled(&cancel) {
        let _ = std::fs::remove_file(&source_srt_path);
        return Err(error);
    }
    let saved = task_db::set_subtitle_result(
        &app,
        &task,
        &cancel,
        path_to_string(source_srt_path.clone()),
        stored.source_file_name,
        stored.output_dir,
        stored.segments.len(),
    );
    if let Err(error) = saved {
        let _ = std::fs::remove_file(&source_srt_path);
        return Err(error);
    }
    publish_job_event(
        &app,
        JobEventDraft::completed(task_id, "completed", "SRT 已生成").with_outputs(outputs),
    );
    Ok(())
}

fn transcription_media_path(task: &TaskRecord) -> JobResult<String> {
    match task.source_type.as_str() {
        "video" => task
            .video_path
            .clone()
            .ok_or_else(|| JobError::failed("视频任务缺少视频文件")),
        "audio" => task
            .audio_path
            .clone()
            .ok_or_else(|| JobError::failed("音频任务缺少音频文件")),
        _ => Err(JobError::failed("该任务不是可转写的媒体任务")),
    }
}

async fn run_translate_task(
    app: AppHandle,
    task_id: &str,
    cancel: Arc<AtomicBool>,
    resume: bool,
) -> JobResult<()> {
    let task = task_db::require_task(&app, task_id).map_err(JobError::failed)?;
    let source_srt_path = task
        .source_srt_path
        .clone()
        .ok_or_else(|| JobError::failed("请先完成转写或导入 SRT"))?;
    let source_srt = tokio::fs::read_to_string(&source_srt_path)
        .await
        .map_err(|error| JobError::failed(format!("读取原文字幕失败: {error}")))?;
    let segments = parse_srt_file(Path::new(&source_srt_path))?;
    let source_file_name = task
        .source_file_name
        .clone()
        .unwrap_or_else(|| display_file_name(Path::new(&source_srt_path)));
    let output_dir = task
        .output_dir
        .clone()
        .or(task.settings.output_dir.clone())
        .unwrap_or_else(|| ".".to_string());
    let request = TranslateSubtitlesRequest {
        job_id: task_id.to_string(),
        target_language: task.settings.target_language.clone(),
        base_url: task.settings.base_url.clone(),
        base_url_is_complete: task.settings.base_url_is_complete,
        model: task.settings.model.clone(),
        temperature: task.settings.temperature,
        translation_shard_size: Some(task.settings.translation_shard_size),
        translation_provider: Some(task.settings.translation_provider.clone()),
        translation_cli_tool: Some(task.settings.translation_cli_tool.clone()),
        translation_cli_command: Some(task.settings.translation_cli_command.clone()),
        translation_cli_model: Some(task.settings.translation_cli_model.clone()),
        translation_cli_args: Some(task.settings.translation_cli_args.clone()),
        translation_local_model_path: Some(task.settings.translation_local_model_path.clone()),
    };
    validate_translate_request(&request).map_err(JobError::failed)?;
    let api_key = task_db::load_api_key(&app).map_err(JobError::failed)?;
    let work_dir = task_db::task_work_dir(&app, task_id).map_err(JobError::failed)?;
    let checkpoint_file = checkpoint_path(&work_dir);
    let fingerprint = source_fingerprint(&segments);
    let target_language = task.settings.target_language.clone();

    let resume_state = if resume {
        let completed =
            load_compatible_checkpoint(&checkpoint_file, &target_language, &fingerprint)?;
        if completed.is_empty() {
            return Err(JobError::failed(
                "没有可续翻的进度。请使用「翻译」重新开始，或确认原文与目标语言未变更",
            ));
        }
        task_db::set_translation_progress(&app, task_id, completed.len())
            .map_err(JobError::failed)?;
        Some(build_resume_state(
            &app,
            task_id,
            checkpoint_file.clone(),
            fingerprint.clone(),
            completed,
        ))
    } else {
        clear_checkpoint(&checkpoint_file)?;
        task_db::clear_translation_progress(&app, task_id).map_err(JobError::failed)?;
        Some(build_resume_state(
            &app,
            task_id,
            checkpoint_file.clone(),
            fingerprint.clone(),
            Vec::new(),
        ))
    };

    let stored = StoredSubtitleResult {
        source_srt,
        translated_srt: None,
        segments,
        output_dir,
        source_file_name,
        translated_file_name: None,
    };

    publish_job_event(
        &app,
        JobEventDraft::running(
            task_id,
            "preparing-translation",
            if resume {
                "正在读取续翻断点"
            } else {
                "正在读取翻译配置"
            },
            0.54,
        ),
    );
    let (stored, outputs) = run_translation(
        &app,
        &request,
        stored,
        api_key.as_deref(),
        cancel.clone(),
        resume_state,
    )
    .await?;
    let translated_srt = stored
        .translated_srt
        .clone()
        .ok_or_else(|| JobError::failed("翻译结果为空"))?;
    let translated_file_name = stored
        .translated_file_name
        .clone()
        .ok_or_else(|| JobError::failed("翻译文件名为空"))?;
    let translated_srt_path = work_dir.join(format!("translation-{}.srt", uuid::Uuid::new_v4()));
    write_srt_text(&translated_srt_path, &translated_srt).await?;
    let state = app.state::<AppState>();
    let _mutation = state.task_mutations.lock();
    if let Err(error) = ensure_not_cancelled(&cancel) {
        let _ = std::fs::remove_file(&translated_srt_path);
        return Err(error);
    }
    let saved = task_db::set_translation_result(
        &app,
        &task,
        &cancel,
        path_to_string(translated_srt_path.clone()),
        translated_file_name,
    );
    if let Err(error) = saved {
        let _ = std::fs::remove_file(&translated_srt_path);
        return Err(error);
    }
    app.state::<AppState>()
        .subtitle_results
        .lock()
        .insert(task_id.to_string(), stored);

    let _ = clear_checkpoint(&checkpoint_file);
    publish_job_event(
        &app,
        JobEventDraft::completed(task_id, "completed", "译文字幕已生成").with_outputs(outputs),
    );
    Ok(())
}

fn build_resume_state(
    app: &AppHandle,
    task_id: &str,
    checkpoint_file: PathBuf,
    fingerprint: String,
    completed: Vec<crate::subtitles::TranslatedSegment>,
) -> TranslationResume {
    let app_handle = app.clone();
    let task_id = task_id.to_string();
    let on_progress: TranslationProgressHook = std::sync::Arc::new(move |items| {
        task_db::set_translation_progress(&app_handle, &task_id, items.len())
            .map_err(JobError::failed)?;
        Ok(())
    });
    TranslationResume {
        completed,
        checkpoint_path: checkpoint_file,
        source_fingerprint: fingerprint,
        on_progress: Some(on_progress),
    }
}

async fn run_export_task(app: AppHandle, task_id: &str, cancel: Arc<AtomicBool>) -> JobResult<()> {
    ensure_not_cancelled(&cancel)?;
    let task = task_db::require_task(&app, task_id).map_err(JobError::failed)?;
    let source_srt_path = task
        .source_srt_path
        .clone()
        .ok_or_else(|| JobError::failed("没有可导出的原文字幕"))?;
    let source_file_name = task
        .source_file_name
        .clone()
        .unwrap_or_else(|| display_file_name(Path::new(&source_srt_path)));
    let output_dir = task
        .output_dir
        .clone()
        .or(task.settings.output_dir.clone())
        .ok_or_else(|| JobError::failed("无法确定导出目录"))?;
    let output_dir_path = Path::new(&output_dir);
    publish_job_event(
        &app,
        JobEventDraft::running(task_id, "exporting", "正在导出字幕", 0.92),
    );
    tokio::fs::create_dir_all(output_dir_path)
        .await
        .map_err(|error| JobError::failed(format!("创建导出目录失败: {error}")))?;

    let mut files = ExportFiles::default();
    let source_stage = files.stage_path(output_dir_path);
    let source = tokio::fs::read_to_string(&source_srt_path)
        .await
        .map_err(|error| JobError::failed(format!("读取原文字幕失败: {error}")))?;
    write_srt_text(&source_stage, &source).await?;
    ensure_not_cancelled(&cancel)?;
    let translated_stage =
        if let (Some(path), Some(name)) = (&task.translated_srt_path, &task.translated_file_name) {
            let body = tokio::fs::read_to_string(path)
                .await
                .map_err(|error| JobError::failed(format!("读取译文字幕失败: {error}")))?;
            let stage = files.stage_path(output_dir_path);
            write_srt_text(&stage, &body).await?;
            Some((stage, name.clone()))
        } else {
            None
        };

    let state = app.state::<AppState>();
    let _mutation = state.task_mutations.lock();
    task_db::set_exported(&app, &task, &cancel, || {
        let source_path =
            files.publish(output_dir_path, &source_file_name, task_id, &source_stage)?;
        let translated_path = if let Some((stage, name)) = &translated_stage {
            ensure_not_cancelled(&cancel)?;
            let path = files.publish(output_dir_path, name, task_id, stage)?;
            Some(path_to_string(path))
        } else {
            None
        };
        Ok(ExportedSubtitlePaths {
            source_srt: path_to_string(source_path),
            translated_srt: translated_path,
            output_dir: output_dir.clone(),
        })
    })?;
    files.keep_published = true;
    Ok(())
}

#[derive(Default)]
pub(crate) struct ExportFiles {
    staged: Vec<PathBuf>,
    published: Vec<PathBuf>,
    keep_published: bool,
}

impl ExportFiles {
    pub(crate) fn publish(
        &mut self,
        output_dir: &Path,
        file_name: &str,
        task_id: &str,
        staged: &Path,
    ) -> JobResult<PathBuf> {
        let path = publish_export_file(output_dir, file_name, task_id, staged)?;
        self.published.push(path.clone());
        Ok(path)
    }

    pub(crate) fn stage_path(&mut self, output_dir: &Path) -> PathBuf {
        let path = output_dir.join(format!(".luma-export-{}.tmp", uuid::Uuid::new_v4()));
        self.staged.push(path.clone());
        path
    }
}

impl Drop for ExportFiles {
    fn drop(&mut self) {
        for path in &self.staged {
            let _ = std::fs::remove_file(path);
        }
        if !self.keep_published {
            for path in &self.published {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn publish_export_file(
    output_dir: &Path,
    file_name: &str,
    task_id: &str,
    staged: &Path,
) -> JobResult<PathBuf> {
    for _ in 0..10_000 {
        // Never overwrite any existing export, including an earlier revision
        // belonging to this task. create_new also closes external filename races.
        let target = resolve_export_path(output_dir, file_name, task_id, None)?;
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(JobError::failed(format!("创建导出文件失败: {error}"))),
        };
        let result = (|| -> std::io::Result<()> {
            let mut source = std::fs::File::open(staged)?;
            std::io::copy(&mut source, &mut file)?;
            file.sync_all()
        })();
        drop(file);
        if let Err(error) = result {
            let _ = std::fs::remove_file(&target);
            return Err(JobError::failed(format!("导出字幕失败: {error}")));
        }
        return Ok(target);
    }
    Err(JobError::failed("导出文件名持续冲突，请选择其他导出目录"))
}

fn resolve_export_path(
    output_dir: &Path,
    file_name: &str,
    task_id: &str,
    previous_export: Option<&str>,
) -> JobResult<PathBuf> {
    let preferred = output_dir.join(file_name);
    if export_path_is_available(&preferred, previous_export) {
        return Ok(preferred);
    }

    let task_file_name = file_name_with_task_suffix(file_name, task_id);
    let alternative = output_dir.join(&task_file_name);
    if export_path_is_available(&alternative, previous_export) {
        return Ok(alternative);
    }

    // Source edits invalidate prior exports while keeping those files for the user.
    for revision in 2..=10_000 {
        let candidate = output_dir.join(file_name_with_task_suffix(
            &task_file_name,
            &revision.to_string(),
        ));
        if export_path_is_available(&candidate, previous_export) {
            return Ok(candidate);
        }
    }

    Err(JobError::failed(format!(
        "导出文件名已被占用，请选择其他导出目录: {}",
        alternative.display()
    )))
}

fn export_path_is_available(path: &Path, previous_export: Option<&str>) -> bool {
    !path.exists() || previous_export.is_some_and(|previous| path == Path::new(previous))
}

fn file_name_with_task_suffix(file_name: &str, task_id: &str) -> String {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(file_name);
    let short_id = task_id
        .split('-')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("task");
    match path.extension().and_then(|value| value.to_str()) {
        Some(extension) => format!("{stem}.{short_id}.{extension}"),
        None => format!("{stem}.{short_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_export_path;
    use std::{
        fs,
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn source_and_translation_with_identical_preferred_names_never_overwrite() {
        let dir = temp_test_dir("same-preferred-name");
        {
            let mut files = super::ExportFiles::default();
            let source_stage = files.stage_path(&dir);
            let translated_stage = files.stage_path(&dir);
            fs::write(&source_stage, "source subtitle").unwrap();
            fs::write(&translated_stage, "translated subtitle").unwrap();
            let source = files
                .publish(&dir, "clip.srt", "task-id", &source_stage)
                .unwrap();
            let translated = files
                .publish(&dir, "clip.srt", "task-id", &translated_stage)
                .unwrap();
            assert_ne!(source, translated);
            assert_eq!(fs::read_to_string(&source).unwrap(), "source subtitle");
            assert_eq!(
                fs::read_to_string(&translated).unwrap(),
                "translated subtitle"
            );
        }
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            0,
            "rollback removes only this attempt's files"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_path_keeps_the_original_name_when_available() {
        let dir = temp_test_dir("available");

        let path = resolve_export_path(&dir, "clip.source.srt", "12345678-task", None)
            .expect("available name should be used");

        assert_eq!(path, dir.join("clip.source.srt"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_path_uses_a_stable_task_suffix_for_conflicts() {
        let dir = temp_test_dir("conflict");
        let original = dir.join("clip.source.srt");
        fs::write(&original, "other task").expect("conflicting file should exist");

        let fallback = resolve_export_path(&dir, "clip.source.srt", "12345678-task", None)
            .expect("task suffix should avoid the conflict");
        assert_eq!(fallback, dir.join("clip.source.12345678.srt"));

        fs::write(&fallback, "this task").expect("task fallback should exist");
        let repeated =
            resolve_export_path(&dir, "clip.source.srt", "12345678-task", fallback.to_str())
                .expect("task should reuse its own fallback");
        assert_eq!(repeated, fallback);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_path_preserves_previous_revisions_and_uses_a_numbered_name() {
        let dir = temp_test_dir("foreign-fallback");
        fs::write(dir.join("clip.source.srt"), "other task")
            .expect("conflicting file should exist");
        fs::write(dir.join("clip.source.12345678.srt"), "another task")
            .expect("fallback file should exist");

        let next = resolve_export_path(&dir, "clip.source.srt", "12345678-task", None)
            .expect("a new revision should get a free name");
        assert_eq!(next, dir.join("clip.source.12345678.2.srt"));
        fs::write(&next, "second revision").expect("second revision should exist");
        let third = resolve_export_path(&dir, "clip.source.srt", "12345678-task", None)
            .expect("a third revision should get a free name");
        assert_eq!(third, dir.join("clip.source.12345678.3.srt"));
        assert_eq!(fs::read_to_string(next).unwrap(), "second revision");
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "luma-export-path-{name}-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&dir).expect("test directory should be created");
        dir
    }
}
