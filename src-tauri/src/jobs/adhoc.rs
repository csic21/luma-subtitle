use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::{
    job_events::{emit_job, ExportedSubtitlePaths, JobOutputs, JobStatus, StoredSubtitleResult},
    paths::path_to_string,
    state::{AppState, JobError},
    subtitles::{parse_srt_file, render_srt, write_srt_text},
    task_db,
};

use super::{
    helpers::{job_error_to_string, validate_start_request, validate_translate_request},
    requests::{
        ExportSubtitlesRequest, ImportSourceSrtRequest, JobRequest, SubtitlePreview,
        TranslateSubtitlesRequest,
    },
    single_job::{resolve_translation_inputs, run_job, run_translation},
};

#[tauri::command]
pub(crate) fn import_source_srt(
    app: AppHandle,
    request: ImportSourceSrtRequest,
) -> Result<String, String> {
    let srt_path = PathBuf::from(request.srt_path.trim());
    let segments = parse_srt_file(&srt_path).map_err(job_error_to_string)?;
    let source_srt = render_srt(&segments, None);
    let source_file_name = srt_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| "imported.source.srt".to_string());
    let output_dir = request
        .output_dir
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            srt_path
                .parent()
                .map(|path| path_to_string(path.to_path_buf()))
        })
        .unwrap_or_else(|| ".".to_string());
    let job_id = Uuid::new_v4().to_string();
    let segment_count = segments.len();

    app.state::<AppState>().subtitle_results.lock().insert(
        job_id.clone(),
        StoredSubtitleResult {
            source_srt,
            translated_srt: None,
            segments,
            output_dir: output_dir.clone(),
            source_file_name: source_file_name.clone(),
            translated_file_name: None,
        },
    );

    emit_job(
        &app,
        &job_id,
        "completed",
        JobStatus::Completed,
        "已导入原文 SRT",
        1.0,
        Some(JobOutputs {
            source_file_name,
            translated_file_name: None,
            output_dir,
            segment_count,
        }),
        None,
    );

    Ok(job_id)
}

#[tauri::command]
pub(crate) fn start_job(
    app: AppHandle,
    state: State<'_, AppState>,
    request: JobRequest,
) -> Result<String, String> {
    validate_start_request(&request)?;
    let job_id = Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    state.tasks.lock().insert(job_id.clone(), cancel.clone());
    emit_job(
        &app,
        &job_id,
        "queued",
        JobStatus::Running,
        "任务已创建",
        0.0,
        None,
        None,
    );
    let app_handle = app.clone();
    let spawned_job_id = job_id.clone();
    tauri::async_runtime::spawn(async move {
        let result = run_job(app_handle.clone(), spawned_job_id.clone(), request, cancel).await;
        match result {
            Ok(outputs) => emit_job(
                &app_handle,
                &spawned_job_id,
                "completed",
                JobStatus::Completed,
                "SRT 已生成",
                1.0,
                Some(outputs),
                None,
            ),
            Err(JobError::Cancelled) => emit_job(
                &app_handle,
                &spawned_job_id,
                "cancelled",
                JobStatus::Cancelled,
                "任务已取消",
                0.0,
                None,
                Some("任务已取消".to_string()),
            ),
            Err(JobError::Failed(message)) => emit_job(
                &app_handle,
                &spawned_job_id,
                "failed",
                JobStatus::Failed,
                "任务失败",
                0.0,
                None,
                Some(message),
            ),
        }
        app_handle
            .state::<AppState>()
            .tasks
            .lock()
            .remove(&spawned_job_id);
    });

    Ok(job_id)
}

#[tauri::command]
pub(crate) fn cancel_job(state: State<'_, AppState>, job_id: String) -> bool {
    if let Some(cancel) = state.tasks.lock().get(&job_id) {
        cancel.store(true, Ordering::SeqCst);
        true
    } else {
        false
    }
}

#[tauri::command]
pub(crate) fn subtitle_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<SubtitlePreview, String> {
    if let Some(preview) = state
        .subtitle_results
        .lock()
        .get(&job_id)
        .cloned()
        .map(|result| SubtitlePreview {
            source_srt: result.source_srt,
            translated_srt: result.translated_srt,
            source_file_name: result.source_file_name,
            translated_file_name: result.translated_file_name,
        })
    {
        return Ok(preview);
    }

    let task = task_db::require_task(&app, &job_id)?;
    let source_srt_path = task
        .source_srt_path
        .as_ref()
        .ok_or_else(|| "没有找到可预览的字幕结果".to_string())?;
    let source_srt = std::fs::read_to_string(source_srt_path).map_err(|error| error.to_string())?;
    let translated_srt = task
        .translated_srt_path
        .as_ref()
        .map(std::fs::read_to_string)
        .transpose()
        .map_err(|error| error.to_string())?;
    Ok(SubtitlePreview {
        source_srt,
        translated_srt,
        source_file_name: task
            .source_file_name
            .unwrap_or_else(|| "source.srt".to_string()),
        translated_file_name: task.translated_file_name,
    })
}

#[tauri::command]
pub(crate) fn translate_subtitles(
    app: AppHandle,
    state: State<'_, AppState>,
    request: TranslateSubtitlesRequest,
) -> Result<(), String> {
    validate_translate_request(&request)?;
    emit_job(
        &app,
        &request.job_id,
        "preparing-translation",
        JobStatus::Running,
        "正在读取翻译配置",
        0.54,
        None,
        None,
    );
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .tasks
        .lock()
        .insert(request.job_id.clone(), cancel.clone());
    let app_handle = app.clone();
    let job_id = request.job_id.clone();
    tauri::async_runtime::spawn(async move {
        let result = resolve_translation_inputs(&app_handle, &request).map_err(JobError::failed);
        let result = match result {
            Ok((stored, api_key)) => {
                run_translation(&app_handle, &request, stored, &api_key, cancel).await
            }
            Err(error) => Err(error),
        };
        app_handle.state::<AppState>().tasks.lock().remove(&job_id);
        match result {
            Ok((stored, outputs)) => {
                app_handle
                    .state::<AppState>()
                    .subtitle_results
                    .lock()
                    .insert(job_id.clone(), stored);
                emit_job(
                    &app_handle,
                    &job_id,
                    "completed",
                    JobStatus::Completed,
                    "译文字幕已生成",
                    1.0,
                    Some(outputs),
                    None,
                );
            }
            Err(JobError::Cancelled) => emit_job(
                &app_handle,
                &job_id,
                "cancelled",
                JobStatus::Cancelled,
                "翻译已取消",
                0.0,
                None,
                Some("翻译已取消".to_string()),
            ),
            Err(JobError::Failed(message)) => emit_job(
                &app_handle,
                &job_id,
                "failed",
                JobStatus::Failed,
                "翻译失败",
                0.0,
                None,
                Some(message),
            ),
        }
    });

    Ok(())
}

#[tauri::command]
pub(crate) async fn export_subtitles(
    app: AppHandle,
    request: ExportSubtitlesRequest,
) -> Result<ExportedSubtitlePaths, String> {
    let result = app
        .state::<AppState>()
        .subtitle_results
        .lock()
        .get(&request.job_id)
        .cloned()
        .ok_or_else(|| "没有找到可导出的字幕结果".to_string())?;

    let output_dir = request
        .output_dir
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(result.output_dir);
    let output_dir_path = PathBuf::from(output_dir);
    tokio::fs::create_dir_all(&output_dir_path)
        .await
        .map_err(|error| format!("创建导出目录失败: {error}"))?;

    let source_path = output_dir_path.join(&result.source_file_name);
    write_srt_text(&source_path, &result.source_srt)
        .await
        .map_err(job_error_to_string)?;
    let translated_srt = if let (Some(translated_file_name), Some(translated_srt)) =
        (result.translated_file_name, result.translated_srt)
    {
        let translated_path = output_dir_path.join(translated_file_name);
        write_srt_text(&translated_path, &translated_srt)
            .await
            .map_err(job_error_to_string)?;
        Some(path_to_string(translated_path))
    } else {
        None
    };

    Ok(ExportedSubtitlePaths {
        source_srt: path_to_string(source_path),
        translated_srt,
        output_dir: path_to_string(output_dir_path),
    })
}
