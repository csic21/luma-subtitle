use reqwest::{header::RANGE, StatusCode};
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
    DOWNLOAD_MAX_ATTEMPTS, HTTP_USER_AGENT,
};

pub(super) async fn download_dependency_archive(
    app: &AppHandle,
    item: &str,
    url: &str,
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
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30 * 60))
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|error| format!("创建下载客户端失败: {error}"))?;
    let result = download_file_with_resume(
        &client,
        url,
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

pub(super) async fn download_file_with_resume<F, R>(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    progress_scale: f32,
    mut on_update: F,
    mut on_retry: R,
) -> Result<(), String>
where
    F: FnMut(DownloadUpdate),
    R: FnMut(usize, &str, u64),
{
    let mut last_error = String::new();
    for attempt in 1..=DOWNLOAD_MAX_ATTEMPTS {
        let existing_bytes = file_len(path).await.unwrap_or(0);
        let mut request = client.get(url);
        if existing_bytes > 0 {
            request = request.header(RANGE, format!("bytes={existing_bytes}-"));
        }
        let mut response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                last_error = error.to_string();
                retry_download(attempt, &last_error, existing_bytes, &mut on_retry).await;
                continue;
            }
        };
        let status = response.status();
        if status == StatusCode::RANGE_NOT_SATISFIABLE && existing_bytes > 0 {
            return Ok(());
        }
        if !(status.is_success() || status == StatusCode::PARTIAL_CONTENT) {
            last_error = format!("HTTP {status}");
            retry_download(attempt, &last_error, existing_bytes, &mut on_retry).await;
            continue;
        }
        last_error.clear();
        let resumed = existing_bytes > 0 && status == StatusCode::PARTIAL_CONTENT;
        let mut downloaded = if resumed { existing_bytes } else { 0 };
        let total = response.content_length().and_then(|remaining| {
            if resumed {
                existing_bytes.checked_add(remaining)
            } else {
                Some(remaining)
            }
        });
        let mut file = if resumed {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .await
                .map_err(|error| format!("打开续传文件失败: {error}"))?
        } else {
            tokio::fs::File::create(path)
                .await
                .map_err(|error| format!("创建下载文件失败: {error}"))?
        };
        let started_at = Instant::now();
        let started_bytes = downloaded;
        let mut last_emit_progress = -1.0f32;
        let mut last_emit_at = Instant::now();
        loop {
            let chunk = match response.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(error) => {
                    last_error = error.to_string();
                    break;
                }
            };
            file.write_all(&chunk)
                .await
                .map_err(|error| format!("写入下载文件失败: {error}"))?;
            downloaded += chunk.len() as u64;
            let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
            let speed = (downloaded.saturating_sub(started_bytes)) as f64 / elapsed;
            let eta_seconds = total.and_then(|total| {
                (speed > 0.0 && total > downloaded)
                    .then(|| ((total - downloaded) as f64 / speed).ceil() as u64)
            });
            let progress = total
                .map(|total| {
                    (downloaded as f32 / total as f32 * progress_scale).clamp(0.0, progress_scale)
                })
                .unwrap_or(0.0);
            if progress >= progress_scale
                || progress - last_emit_progress >= 0.01
                || last_emit_at.elapsed() >= Duration::from_secs(1)
            {
                last_emit_progress = progress;
                last_emit_at = Instant::now();
                on_update(DownloadUpdate {
                    progress,
                    metrics: DownloadMetrics {
                        bytes_per_second: Some(speed),
                        eta_seconds,
                        downloaded_bytes: Some(downloaded),
                        total_bytes: total,
                    },
                    attempt,
                    resumed,
                });
            }
        }
        file.flush()
            .await
            .map_err(|error| format!("刷新下载文件失败: {error}"))?;
        if last_error.is_empty() {
            return Ok(());
        }
        retry_download(attempt, &last_error, downloaded, &mut on_retry).await;
    }
    Err(format!(
        "下载失败，已重试 {DOWNLOAD_MAX_ATTEMPTS} 次: {last_error}"
    ))
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

async fn file_len(path: &Path) -> Option<u64> {
    tokio::fs::metadata(path)
        .await
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
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
