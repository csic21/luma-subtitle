use super::{archive::cancelled, cancellable, catalog::{allowed_redirect, validate_download, Source}};
use sha2::{Digest, Sha256};
use std::{fs::OpenOptions, io::Write, path::Path, sync::atomic::AtomicBool, time::{Duration, Instant}};

pub(super) fn validate_redirect(url: &url::Url, source: Source, previous_urls: usize) -> Result<(), &'static str> {
    // Preserve reqwest's existing history cutoff, including the original URL.
    if previous_urls >= 8 { return Err("Too many component download redirects"); }
    if !allowed_redirect(url, source) { return Err("Component redirect is outside the trusted HTTPS origin allowlist"); }
    Ok(())
}

pub(super) fn verify_digest(bytes: u64, digest: &str, expected_bytes: u64, expected_digest: &str) -> Result<(), String> {
    if bytes != expected_bytes { return Err("Component download did not match its pinned byte size.".into()); }
    if !digest.eq_ignore_ascii_case(expected_digest) { return Err("Component SHA-256 mismatch. The untrusted download was discarded; the previous component is unchanged.".into()); }
    Ok(())
}
struct Integrity { bytes: u64, expected_bytes: u64, expected_digest: String, digest: Sha256 }
impl Integrity {
    fn new(expected_bytes: u64, expected_digest: &str) -> Self { Self { bytes: 0, expected_bytes, expected_digest: expected_digest.into(), digest: Sha256::new() } }
    fn write_chunk(&mut self, output: &mut impl Write, chunk: &[u8], cancel: &AtomicBool) -> Result<(), String> {
        cancelled(cancel)?;
        self.bytes = self.bytes.checked_add(chunk.len() as u64).ok_or("Component download size overflow")?;
        if self.bytes > self.expected_bytes { return Err("Component download exceeded its pinned byte size.".into()); }
        output.write_all(chunk).map_err(|e| format!("Cannot save component download (check free disk space): {e}"))?;
        self.digest.update(chunk); Ok(())
    }
    fn finish(self) -> Result<(), String> { verify_digest(self.bytes, &format!("{:x}", self.digest.finalize()), self.expected_bytes, &self.expected_digest) }
}
/// Test transport accepts only local byte chunks; production never accepts a
/// caller-supplied URL, fixture origin, or unpinned override.
#[cfg(test)]
pub(super) fn fixture_download(chunks: &[&[u8]], expected_bytes: u64, expected_digest: &str, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    let mut output = Vec::new(); let mut integrity = Integrity::new(expected_bytes, expected_digest);
    for chunk in chunks { integrity.write_chunk(&mut output, chunk, cancel)?; }
    cancelled(cancel)?; integrity.finish()?; Ok(output)
}
pub(super) async fn fetch(url: &str, expected_bytes: u64, expected_digest: &str, source: Source, destination: &Path, cancel: &AtomicBool, mut progress: impl FnMut(u64)) -> Result<(), String> {
    validate_download(url, expected_bytes, expected_digest, source)?;
    cancelled(cancel)?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60 * 60 * 4))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            match validate_redirect(attempt.url(), source, attempt.previous().len()) {
                Ok(()) => attempt.follow(),
                Err(error) => attempt.error(error),
            }
        }))
        .build().map_err(|e| format!("Cannot create component download client: {e}"))?;
    // No credentials, auth tokens, ambient cookies, or remote catalog requests.
    let mut response = cancellable(client.get(url).header(reqwest::header::ACCEPT_ENCODING, "identity").header(reqwest::header::USER_AGENT, "Luma Subtitle verified component installer").send(), cancel).await?
        .map_err(|_| "The component download could not connect to its trusted source. Check the connection and retry.")?;
    if !response.status().is_success() { return Err(format!("Component source returned HTTP {}. Retry later.", response.status().as_u16())); }
    if response.content_length().is_some_and(|length| length != expected_bytes) { return Err("Component source Content-Length does not match the pinned byte size.".into()); }
    let mut options = OpenOptions::new(); options.create_new(true).write(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut output = options.open(destination).map_err(|e| format!("Cannot create staged download: {e}"))?;
    let mut integrity = Integrity::new(expected_bytes, expected_digest); let mut last = Instant::now();
    loop {
        let chunk = cancellable(tokio::time::timeout(Duration::from_secs(60), response.chunk()), cancel).await?
            .map_err(|_| "The component download stalled. Retry the verified source.")?
            .map_err(|_| "The component download was interrupted. Retry to start a fresh verified download.")?;
        let Some(chunk) = chunk else { break; };
        integrity.write_chunk(&mut output, &chunk, cancel)?;
        if last.elapsed() >= Duration::from_millis(200) { progress(integrity.bytes); last = Instant::now(); }
    }
    cancelled(cancel)?;
    integrity.finish()?;
    output.sync_all().map_err(|e| e.to_string())?;
    progress(expected_bytes);
    Ok(())
}

struct PartialDownload(std::path::PathBuf);
impl Drop for PartialDownload { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }
/// Caller holds the app-owned setup lease. Cache entries are content addressed,
/// fully rehashed before reuse, and never become executable active components.
pub(super) async fn cached(root: &Path, component_id: &str, artifact: &super::catalog::Archive, source: Source, cancel: &std::sync::Arc<AtomicBool>, mut progress: impl FnMut(u64)) -> Result<std::path::PathBuf, String> {
    validate_download(&artifact.url, artifact.bytes, &artifact.sha256, source)?;
    let cache = super::store::cache_directory(root, component_id)?;
    let path = cache.join(&artifact.sha256);
    if std::fs::symlink_metadata(&path).is_ok() {
        let metadata = super::store::regular_file(&path)?;
        let cached_path = path.clone(); let digest = artifact.sha256.clone();
        let expected_bytes = artifact.bytes;
        // Small cache verification reads occur here per chunk; task cancellation
        // is observed before each chunk even when the file is already cached.
        let cancellation = cancel.clone();
        let valid = if metadata.len() == expected_bytes {
            tauri::async_runtime::spawn_blocking(move || super::store::hash_file_checked(&cached_path, &cancellation).map(|hash| hash.eq_ignore_ascii_case(&digest))).await.map_err(|e| e.to_string())??
        } else { false };
        if valid { progress(artifact.bytes); return Ok(path); }
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    let mut last_error = String::new();
    for attempt in 0..3 {
        cancelled(cancel)?;
        let temporary = PartialDownload(cache.join(format!("{}.{}.partial", artifact.sha256, uuid::Uuid::new_v4())));
        match fetch(&artifact.url, artifact.bytes, &artifact.sha256, source, &temporary.0, cancel, &mut progress).await {
            Ok(()) => {
                cancelled(cancel)?;
                std::fs::rename(&temporary.0, &path).map_err(|e| e.to_string())?;
                return Ok(path);
            }
            Err(error) => {
                cancelled(cancel)?;
                let retryable = error.contains("could not connect") || error.contains("interrupted") || error.contains("stalled") || error.contains("HTTP 429") || error.contains("HTTP 5");
                last_error = error;
                if !retryable || attempt == 2 { break; }
                cancellable(tokio::time::sleep(Duration::from_secs(1 << attempt)), cancel).await?;
            }
        }
    }
    Err(last_error)
}
