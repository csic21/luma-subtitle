use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{state::AppState, subtitles::SubtitleSegment};

#[derive(Clone, Serialize)]
pub(crate) struct JobOutputs {
    pub(crate) source_file_name: String,
    pub(crate) translated_file_name: Option<String>,
    pub(crate) output_dir: String,
    pub(crate) segment_count: usize,
}

#[derive(Clone)]
pub(crate) struct StoredSubtitleResult {
    pub(crate) source_srt: String,
    pub(crate) translated_srt: Option<String>,
    pub(crate) segments: Vec<SubtitleSegment>,
    pub(crate) output_dir: String,
    pub(crate) source_file_name: String,
    pub(crate) translated_file_name: Option<String>,
}

#[derive(Clone, Serialize)]
pub(crate) struct ExportedSubtitlePaths {
    pub(crate) source_srt: String,
    pub(crate) translated_srt: Option<String>,
    pub(crate) output_dir: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum JobStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Serialize)]
pub(crate) struct JobEvent {
    pub(crate) job_id: String,
    pub(crate) stage: String,
    pub(crate) status: JobStatus,
    pub(crate) message: String,
    pub(crate) progress: f32,
    pub(crate) outputs: Option<JobOutputs>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Serialize)]
pub(crate) struct JobProgressSnapshot {
    event: Option<JobEvent>,
    logs: Vec<String>,
}

pub(crate) fn emit_job(
    app: &AppHandle,
    job_id: &str,
    stage: &str,
    status: JobStatus,
    message: impl Into<String>,
    progress: f32,
    outputs: Option<JobOutputs>,
    error: Option<String>,
) {
    let message = message.into();
    let event = JobEvent {
        job_id: job_id.to_string(),
        stage: stage.to_string(),
        status,
        message: message.clone(),
        progress,
        outputs,
        error,
    };

    let state = app.state::<AppState>();
    state
        .job_events
        .lock()
        .insert(job_id.to_string(), event.clone());
    state
        .job_logs
        .lock()
        .entry(job_id.to_string())
        .or_default()
        .push(format!("{stage} · {message}"));
    if let Some(error) = event.error.as_ref() {
        state
            .job_logs
            .lock()
            .entry(job_id.to_string())
            .or_default()
            .push(format!("error · {error}"));
    }

    let _ = crate::task_db::record_job_event(app, &event);
    let _ = app.emit("job-event", event);
}

#[tauri::command]
pub(crate) fn job_status(state: State<'_, AppState>, job_id: String) -> JobProgressSnapshot {
    JobProgressSnapshot {
        event: state.job_events.lock().get(&job_id).cloned(),
        logs: state
            .job_logs
            .lock()
            .get(&job_id)
            .cloned()
            .unwrap_or_default(),
    }
}
