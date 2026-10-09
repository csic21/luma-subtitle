//! Structured, bounded translation work. Futures stay owned by the caller, so
//! dropping a translation can never detach a shard into the background.
use std::{
    future::{poll_fn, Future},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::Poll,
    time::Duration,
};

use crate::state::{ensure_not_cancelled, JobError, JobResult};

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(super) async fn cancellable<T>(
    cancel: &Arc<AtomicBool>,
    future: impl Future<Output = JobResult<T>>,
) -> JobResult<T> {
    ensure_not_cancelled(cancel)?;
    let mut future = std::pin::pin!(future);
    let mut tick = std::pin::pin!(tokio::time::sleep(CANCEL_POLL_INTERVAL));
    poll_fn(|cx| {
        if cancel.load(Ordering::SeqCst) {
            return Poll::Ready(Err(JobError::Cancelled));
        }
        if let Poll::Ready(result) = future.as_mut().poll(cx) {
            return Poll::Ready(result);
        }
        if tick.as_mut().poll(cx).is_ready() {
            tick.as_mut()
                .reset(tokio::time::Instant::now() + CANCEL_POLL_INTERVAL);
            // Register the timer for the next cancellation check.
            let _ = tick.as_mut().poll(cx);
        }
        Poll::Pending
    })
    .await
}

async fn next_finished<F: Future>(active: &mut Vec<Pin<Box<F>>>) -> F::Output {
    let (index, result) = poll_fn(|cx| {
        for (index, future) in active.iter_mut().enumerate() {
            if let Poll::Ready(result) = future.as_mut().poll(cx) {
                return Poll::Ready((index, result));
            }
        }
        Poll::Pending
    })
    .await;
    drop(active.remove(index));
    result
}

pub(super) async fn run_bounded<I, T, F, Fut, C>(
    inputs: impl IntoIterator<Item = I>,
    limit: usize,
    cancel: Arc<AtomicBool>,
    mut start: F,
    mut completed: C,
) -> JobResult<()>
where
    F: FnMut(I, Arc<AtomicBool>) -> Fut,
    Fut: Future<Output = JobResult<T>>,
    C: FnMut(T) -> JobResult<()>,
{
    ensure_not_cancelled(&cancel)?;
    let stop = Arc::new(AtomicBool::new(false));
    let mut inputs = inputs.into_iter();
    let mut active = Vec::new();
    loop {
        while active.len() < limit.max(1) && !cancel.load(Ordering::SeqCst) {
            let Some(input) = inputs.next() else { break };
            active.push(Box::pin(start(input, stop.clone())));
        }
        if active.is_empty() {
            return ensure_not_cancelled(&cancel);
        }
        let result = cancellable(&cancel, next_finished(&mut active))
            .await
            .and_then(&mut completed);
        if let Err(error) = result {
            // Give subprocess shards a chance to kill/reap their children and
            // join pipe readers before returning. HTTP futures cancel promptly.
            stop.store(true, Ordering::SeqCst);
            while !active.is_empty() {
                if let Ok(item) = next_finished(&mut active).await {
                    // Preserve other already completed shards on a partial failure.
                    let _ = completed(item);
                }
            }
            return Err(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn window_refills_and_reports_completion_order() {
        tauri::async_runtime::block_on(async {
            let active = Arc::new(AtomicUsize::new(0));
            let peak = Arc::new(AtomicUsize::new(0));
            let mut finished = Vec::new();
            let last_started = Arc::new(tokio::sync::Notify::new());
            run_bounded(
                0..4,
                2,
                Arc::new(AtomicBool::new(false)),
                |id, _| {
                    let active = active.clone();
                    let peak = peak.clone();
                    let last_started = last_started.clone();
                    async move {
                        let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(count, Ordering::SeqCst);
                        if id == 0 {
                            last_started.notified().await;
                        } else if id == 3 {
                            last_started.notify_one();
                        }
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok(id)
                    }
                },
                |id| {
                    finished.push(id);
                    Ok(())
                },
            )
            .await
            .unwrap();
            assert_eq!(finished, vec![1, 2, 3, 0]);
            assert_eq!(peak.load(Ordering::SeqCst), 2);
        });
    }

    #[test]
    fn failure_cancels_and_drains_other_shards() {
        tauri::async_runtime::block_on(async {
            let cleaned = Arc::new(AtomicBool::new(false));
            let result = run_bounded(
                0..3,
                2,
                Arc::new(AtomicBool::new(false)),
                |id, stop| {
                    let cleaned = cleaned.clone();
                    async move {
                        if id == 0 {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            return Err(JobError::failed("mock failure"));
                        }
                        let result =
                            cancellable(&stop, std::future::pending::<JobResult<()>>()).await;
                        cleaned.store(true, Ordering::SeqCst);
                        result
                    }
                },
                |_| Ok(()),
            )
            .await;
            assert!(matches!(result, Err(JobError::Failed(_))));
            assert!(cleaned.load(Ordering::SeqCst));
        });
    }

    #[test]
    fn user_cancellation_drains_active_work() {
        tauri::async_runtime::block_on(async {
            let cancel = Arc::new(AtomicBool::new(false));
            let cleaned = Arc::new(AtomicBool::new(false));
            let result = run_bounded(
                0..2,
                1,
                cancel.clone(),
                |_, stop| {
                    let cancel = cancel.clone();
                    let cleaned = cleaned.clone();
                    async move {
                        cancel.store(true, Ordering::SeqCst);
                        let result =
                            cancellable(&stop, std::future::pending::<JobResult<()>>()).await;
                        cleaned.store(true, Ordering::SeqCst);
                        result
                    }
                },
                |_| Ok(()),
            )
            .await;
            assert!(matches!(result, Err(JobError::Cancelled)));
            assert!(cleaned.load(Ordering::SeqCst));
        });
    }

    #[test]
    fn completed_later_shard_is_checkpointed_before_slow_shard_fails() {
        use crate::{
            subtitles::{SubtitleSegment, TranslatedSegment},
            translation::{
                checkpoint, client::tests::config, TranslationProgress, TranslationResume,
            },
        };
        tauri::async_runtime::block_on(async {
            let segments = [7, 9]
                .into_iter()
                .map(|id| SubtitleSegment {
                    id,
                    start_ms: 0,
                    end_ms: 1000,
                    text: format!("cue {id}"),
                })
                .collect::<Vec<_>>();
            let path =
                std::env::temp_dir().join(format!("luma-window-{}.json", uuid::Uuid::new_v4()));
            let fingerprint = checkpoint::source_fingerprint(&segments);
            let progress = TranslationProgress::new(
                &config(""),
                &segments,
                Some(TranslationResume {
                    completed: Vec::new(),
                    checkpoint_path: path.clone(),
                    source_fingerprint: fingerprint.clone(),
                    on_progress: None,
                }),
            )
            .unwrap();
            let result = run_bounded(
                0..2,
                2,
                Arc::new(AtomicBool::new(false)),
                |id, _| async move {
                    if id == 0 {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        Err(JobError::failed("slow shard failed"))
                    } else {
                        Ok(vec![TranslatedSegment {
                            id: 9,
                            text: "已完成".to_string(),
                        }])
                    }
                },
                |items| progress.append_and_persist(items),
            )
            .await;
            assert!(matches!(result, Err(JobError::Failed(_))));
            let saved =
                checkpoint::load_compatible_checkpoint(&path, "简体中文", &fingerprint).unwrap();
            assert_eq!(saved.len(), 1);
            assert_eq!(saved[0].id, 9);
            assert_eq!(checkpoint::remaining_segments(&segments, &saved)[0].id, 7);
            let _ = std::fs::remove_file(path);
        });
    }
}
