use parking_lot::Mutex;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use crate::{
    dependencies::{DependencyInstallEvent, ModelDownloadEvent},
    job_events::StoredSubtitleResult,
};

#[derive(Default)]
pub(crate) struct AppState {
    pub(crate) tasks: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub(crate) queued_operations: Mutex<VecDeque<QueuedTaskOperation>>,
    pub(crate) running_operations: Mutex<HashMap<String, String>>,
    pub(crate) model_download: Mutex<Option<ModelDownloadEvent>>,
    pub(crate) dependency_install: Mutex<Option<DependencyInstallEvent>>,
    pub(crate) subtitle_results: Mutex<HashMap<String, StoredSubtitleResult>>,
}

#[derive(Clone)]
pub(crate) struct QueuedTaskOperation {
    pub(crate) task_id: String,
    pub(crate) operation: String,
}
#[derive(Debug)]
pub(crate) enum JobError {
    Cancelled,
    Failed(String),
}
impl JobError {
    pub(crate) fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
}
pub(crate) type JobResult<T> = Result<T, JobError>;

pub(crate) fn ensure_not_cancelled(cancel: &Arc<AtomicBool>) -> JobResult<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(JobError::Cancelled)
    } else {
        Ok(())
    }
}
