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

pub(crate) struct JobEventDraft {
    job_id: String,
    stage: String,
    status: JobStatus,
    message: String,
    progress: f32,
    outputs: Option<JobOutputs>,
    error: Option<String>,
}

impl JobEventDraft {
    pub(crate) fn running(
        job_id: impl Into<String>,
        stage: impl Into<String>,
        message: impl Into<String>,
        progress: f32,
    ) -> Self {
        Self::new(job_id, stage, JobStatus::Running, message, progress)
    }

    pub(crate) fn completed(
        job_id: impl Into<String>,
        stage: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(job_id, stage, JobStatus::Completed, message, 1.0)
    }

    pub(crate) fn failed(
        job_id: impl Into<String>,
        stage: impl Into<String>,
        message: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self::new(job_id, stage, JobStatus::Failed, message, 0.0).with_error(error)
    }

    pub(crate) fn cancelled(
        job_id: impl Into<String>,
        stage: impl Into<String>,
        message: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self::new(job_id, stage, JobStatus::Cancelled, message, 0.0).with_error(error)
    }

    pub(crate) fn with_outputs(mut self, outputs: JobOutputs) -> Self {
        self.outputs = Some(outputs);
        self
    }

    pub(crate) fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    fn new(
        job_id: impl Into<String>,
        stage: impl Into<String>,
        status: JobStatus,
        message: impl Into<String>,
        progress: f32,
    ) -> Self {
        Self {
            job_id: job_id.into(),
            stage: stage.into(),
            status,
            message: message.into(),
            progress,
            outputs: None,
            error: None,
        }
    }
}

pub(crate) fn publish_job_event(app: &AppHandle, draft: JobEventDraft) {
    let event = JobEvent {
        job_id: draft.job_id,
        stage: draft.stage,
        status: draft.status,
        message: draft.message,
        progress: draft.progress,
        outputs: draft.outputs,
        error: draft.error,
    };

    let state = app.state::<AppState>();
    state
        .job_events
        .lock()
        .insert(event.job_id.clone(), event.clone());
    state
        .job_logs
        .lock()
        .entry(event.job_id.clone())
        .or_default()
        .push(format!("{} · {}", event.stage, event.message));
    if let Some(error) = event.error.as_ref() {
        state
            .job_logs
            .lock()
            .entry(event.job_id.clone())
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
