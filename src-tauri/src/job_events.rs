use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::subtitles::SubtitleSegment;

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

    if let Err(error) = crate::task_db::record_job_event(app, &event) {
        eprintln!("持久化任务事件失败 [{}]: {error}", event.job_id);
    }
    let _ = app.emit("job-event", event);
}
