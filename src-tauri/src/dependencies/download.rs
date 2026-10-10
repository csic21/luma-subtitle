use reqwest::{
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE, RANGE},
    StatusCode,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::{io::AsyncWriteExt, time::sleep};

use super::{
    events::{
        emit_dependency_install, emit_dependency_install_with_metrics, format_bytes,
        DownloadMetrics, DownloadUpdate,
    },
    trust::{self, Artifact},
    DOWNLOAD_MAX_ATTEMPTS,
};

pub(super) async fn download_dependency_archive(
    app: &AppHandle,
    item: &str,
    artifact: &Artifact,
    archive_path: &Path,
) -> Result<(), String> {
    emit_dependency_install(
        app,
        item,
        "running",
        format!("开始下载 {item}"),
        0.0,
        None,
        None,
    );
    let result = download_file_with_resume(
        artifact,
        archive_path,
        0.82,
        |update| {
            let metrics = update.metrics;
            let message = download_message(item, update);
            emit_dependency_install_with_metrics(
                app,
                item,
                "running",
                message,
                update.progress,
                None,
                None,
                metrics,
            );
        },
        |attempt, error, downloaded| {
            emit_dependency_install_with_metrics(
                app,
                item,
                "running",
                format!(
                    "下载中断，保留 {}，正在重试 {}/{}: {}",
                    format_bytes(downloaded),
                    attempt,
                    DOWNLOAD_MAX_ATTEMPTS,
                    error
                ),
                0.0,
                None,
                None,
                DownloadMetrics {
                    downloaded_bytes: Some(downloaded),
                    ..DownloadMetrics::default()
                },
            );
        },
    )
    .await;
    if let Err(message) = &result {
        emit_dependency_install(
            app,
            item,
            "failed",
            message,
            0.0,
            None,
            Some(message.clone()),
        );
    }
    result
}

/// A resumed prefix is never trusted: the complete file is rehashed before return.
pub(super) async fn download_file_with_resume<F, R>(
    artifact: &Artifact,
    path: &Path,
    progress_scale: f32,
    mut on_update: F,
    mut on_retry: R,
) -> Result<(), String>
where
    F: FnMut(DownloadUpdate),
    R: FnMut(usize, &str, u64),
{
    let client = trust::client(artifact)?;
    let mut last_error = String::new();
    for attempt in 1..=DOWNLOAD_MAX_ATTEMPTS {
        let existing_bytes = match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                crate::asr_components::legacy_regular_file(path)?.len()
            }
            Ok(_) => return Err("Refusing a non-regular dependency download destination".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.to_string()),
        };
        if existing_bytes >= artifact.bytes {
            if trust::verify_file(path, artifact).await.is_ok() {
                return Ok(());
            }
            tokio::fs::remove_file(path)
                .await
                .map_err(|e| e.to_string())?;
        }
        let existing_bytes = if existing_bytes >= artifact.bytes {
            0
        } else {
            existing_bytes
        };
        let mut request = client
            .get(&artifact.url)
            .header(ACCEPT_ENCODING, "identity");
        if existing_bytes > 0 {
            request = request.header(RANGE, format!("bytes={existing_bytes}-"));
        }
        let mut response = match request.send().await {
            Ok(response) => response,
            Err(_) => {
                last_error = "Cannot connect to the reviewed dependency source".into();
                retry_download(attempt, &last_error, existing_bytes, &mut on_retry).await;
                continue;
            }
        };
        let status = response.status();
        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
            last_error = format!("HTTP {status}");
            retry_download(attempt, &last_error, existing_bytes, &mut on_retry).await;
            continue;
        }
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
        };
        let resumed = match validate_response(
            status.as_u16(),
            existing_bytes,
            artifact.bytes,
            header(CONTENT_RANGE),
            response.content_length(),
            header(CONTENT_ENCODING),
        ) {
            Ok(resumed) => resumed,
            Err(error) => {
                let _ = tokio::fs::remove_file(path).await;
                return Err(error);
            }
        };
        let mut downloaded = if resumed { existing_bytes } else { 0 };
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true);
        if resumed {
            options.append(true);
        } else {
            if existing_bytes > 0 {
                tokio::fs::remove_file(path)
                    .await
                    .map_err(|e| e.to_string())?;
            } else if tokio::fs::symlink_metadata(path).await.is_ok() {
                tokio::fs::remove_file(path)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            options.create_new(true);
        }
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut file = options
            .open(path)
            .await
            .map_err(|e| format!("Cannot stage dependency: {e}"))?;
        let started_at = Instant::now();
        let started_bytes = downloaded;
        let mut last_emit_at = Instant::now();
        last_error.clear();
        loop {
            let chunk = match tokio::time::timeout(Duration::from_secs(60), response.chunk()).await
            {
                Ok(Ok(Some(chunk))) => chunk,
                Ok(Ok(None)) => break,
                _ => {
                    last_error = "Dependency download was interrupted or stalled".into();
                    break;
                }
            };
            downloaded = downloaded
                .checked_add(chunk.len() as u64)
                .ok_or("Dependency byte count overflow")?;
            if downloaded > artifact.bytes {
                drop(file);
                let _ = tokio::fs::remove_file(path).await;
                return Err("Dependency download exceeded its pinned size".into());
            }
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Cannot write dependency download: {e}"))?;
            if downloaded == artifact.bytes || last_emit_at.elapsed() >= Duration::from_secs(1) {
                last_emit_at = Instant::now();
                let speed = (downloaded - started_bytes) as f64
                    / started_at.elapsed().as_secs_f64().max(0.001);
                on_update(DownloadUpdate {
                    progress: (downloaded as f32 / artifact.bytes as f32 * progress_scale)
                        .clamp(0.0, progress_scale),
                    metrics: DownloadMetrics {
                        bytes_per_second: Some(speed),
                        eta_seconds: (speed > 0.0)
                            .then(|| ((artifact.bytes - downloaded) as f64 / speed).ceil() as u64),
                        downloaded_bytes: Some(downloaded),
                        total_bytes: Some(artifact.bytes),
                    },
                    attempt,
                    resumed,
                });
            }
        }
        file.flush().await.map_err(|e| e.to_string())?;
        file.sync_all().await.map_err(|e| e.to_string())?;
        drop(file);
        if last_error.is_empty() && downloaded != artifact.bytes {
            last_error = "Truncated dependency download".into();
        }
        if last_error.is_empty() {
            let verified = trust::verify_file(path, artifact).await;
            if verified.is_err() {
                let _ = tokio::fs::remove_file(path).await;
            }
            return verified;
        }
        retry_download(attempt, &last_error, downloaded, &mut on_retry).await;
    }
    Err(format!(
        "Download failed after {DOWNLOAD_MAX_ATTEMPTS} attempts: {last_error}"
    ))
}

pub(super) fn validate_response(
    status: u16,
    existing: u64,
    expected: u64,
    content_range: Option<&str>,
    length: Option<u64>,
    encoding: Option<&str>,
) -> Result<bool, String> {
    if encoding.is_some_and(|value| !value.eq_ignore_ascii_case("identity")) {
        return Err("Encoded dependency responses are not supported".into());
    }
    match status {
        200 => {
            if content_range.is_some() || length.is_some_and(|n| n != expected) {
                return Err("Dependency response differs from its pinned size".into());
            }
            Ok(false) // A server ignoring Range must replace, never append.
        }
        206 if existing > 0 && existing < expected => {
            let expected_range = format!("bytes {existing}-{}/{expected}", expected - 1);
            if content_range != Some(expected_range.as_str())
                || length.is_some_and(|n| n != expected - existing)
            {
                return Err("Invalid dependency Content-Range or resumed length".into());
            }
            Ok(true)
        }
        // 416 is never completion. A fully cached file is verified before HTTP.
        _ => Err(format!("Unexpected dependency HTTP status {status}")),
    }
}

async fn retry_download<R>(attempt: usize, error: &str, downloaded: u64, on_retry: &mut R)
where
    R: FnMut(usize, &str, u64),
{
    if attempt < DOWNLOAD_MAX_ATTEMPTS {
        let next_attempt = attempt + 1;
        on_retry(next_attempt, error, downloaded);
        sleep(Duration::from_millis(700 * attempt as u64)).await;
    }
}

pub(super) fn download_message(label: &str, update: DownloadUpdate) -> String {
    let downloaded = update
        .metrics
        .downloaded_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "0 KiB".to_string());
    let total = update
        .metrics
        .total_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "未知大小".to_string());
    let prefix = if update.resumed {
        "正在续传"
    } else if update.attempt > 1 {
        "正在重试"
    } else {
        "正在下载"
    };
    format!("{prefix} {label} {downloaded} / {total}")
}

/// Unique per-operation staging files are removed after failure or activation.
pub(super) struct PartialFile(pub std::path::PathBuf);
impl Drop for PartialFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
