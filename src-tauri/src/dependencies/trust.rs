//! Embedded, reviewed trust roots. Network metadata never chooses executable bytes.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use tokio::io::AsyncReadExt;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Source {
    Github,
    Codeload,
    Ffmpeg,
    Model,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Artifact {
    pub id: String,
    pub version: String,
    pub file_name: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
    pub source: Source,
    pub unpacked_bytes: u64,
    pub file_limit: usize,
}
pub(super) fn catalog() -> Result<Vec<Artifact>, String> {
    serde_json::from_str(include_str!("artifacts.json"))
        .map_err(|e| format!("Invalid embedded dependency catalog: {e}"))
}
pub(super) fn artifact(id: &str) -> Result<Artifact, String> {
    let item = catalog()?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("No reviewed dependency artifact for {id}"))?;
    validate_artifact(&item)?;
    Ok(item)
}
pub(super) fn validate_artifact(item: &Artifact) -> Result<(), String> {
    let url = url::Url::parse(&item.url).map_err(|e| e.to_string())?;
    if !safe_https(&url)
        || url.query().is_some()
        || item.version.is_empty()
        || matches!(item.version.as_str(), "latest" | "main" | "master" | "HEAD")
        || item.bytes == 0
        || item.bytes > 8 * 1024 * 1024 * 1024
        || item.sha256.len() != 64
        || !item.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid pinned dependency URL, SHA-256, or byte size".into());
    }
    crate::asr_components::legacy_relative_path(&item.file_name)?;
    if item.file_name.contains('/') {
        return Err("Invalid dependency filename".into());
    }
    let parts: Vec<_> = url.path().trim_start_matches('/').split('/').collect();
    let origin = match item.source {
        Source::Github => {
            url.host_str() == Some("github.com")
                && parts.len() == 6
                && matches!(
                    (parts[0], parts[1]),
                    ("ggml-org", "whisper.cpp" | "llama.cpp") | ("GyanD", "codexffmpeg")
                )
                && parts[2..4] == ["releases", "download"]
                && parts[4] == item.version
                && parts[5] == item.file_name
        }
        Source::Codeload => {
            url.host_str() == Some("codeload.github.com")
                && parts.len() == 4
                && parts[..3] == ["ggml-org", "whisper.cpp", "tar.gz"]
                && parts[3] == item.version
                && hex_revision(&item.version)
        }
        Source::Ffmpeg => {
            url.host_str() == Some("ffmpeg.org")
                && url.path() == format!("/releases/ffmpeg-{}.tar.gz", item.version)
        }
        Source::Model => {
            url.host_str() == Some("huggingface.co")
                && parts.len() == 5
                && matches!(
                    (parts[0], parts[1]),
                    ("ggerganov", "whisper.cpp")
                        | ("ggml-org", "whisper-vad")
                        | ("tencent", "Hy-MT2-1.8B-GGUF" | "Hy-MT2-7B-GGUF")
                )
                && parts[2] == "resolve"
                && parts[3] == item.version
                && hex_revision(&item.version)
                && parts[4] == item.file_name
        }
    };
    if !origin || item.unpacked_bytes > 4 * 1024 * 1024 * 1024 || item.file_limit > 100_000 {
        return Err(
            "Dependency artifact is outside its reviewed source or extraction bounds".into(),
        );
    }
    Ok(())
}
fn hex_revision(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn safe_https(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.port_or_known_default() == Some(443)
}
pub(super) fn allowed_redirect(url: &url::Url, source: Source, previous: usize) -> bool {
    if previous >= 8 || !safe_https(url) {
        return false;
    }
    let host = url.host_str().unwrap_or("");
    match source {
        Source::Github => matches!(
            host,
            "github.com" | "release-assets.githubusercontent.com" | "objects.githubusercontent.com"
        ),
        Source::Codeload => host == "codeload.github.com",
        Source::Ffmpeg => host == "ffmpeg.org",
        Source::Model => {
            matches!(
                host,
                "huggingface.co"
                    | "cdn-lfs.huggingface.co"
                    | "cdn-lfs.hf.co"
                    | "cdn-lfs-us-1.hf.co"
                    | "cdn-lfs-eu-1.hf.co"
                    | "us.aws.cdn.hf.co"
                    | "us.gcp.cdn.hf.co"
            ) || host.ends_with(".xethub.hf.co")
        }
    }
}
pub(super) fn client(item: &Artifact) -> Result<reqwest::Client, String> {
    validate_artifact(item)?;
    let source = item.source;
    reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(4 * 60 * 60))
        .user_agent(super::HTTP_USER_AGENT)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if allowed_redirect(attempt.url(), source, attempt.previous().len()) {
                attempt.follow()
            } else {
                attempt.error("Dependency redirect is outside its trusted HTTPS origins")
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}
pub(super) async fn verify_file(path: &Path, item: &Artifact) -> Result<(), String> {
    let metadata = crate::asr_components::legacy_regular_file(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != item.bytes {
        return Err("Dependency does not match its pinned regular-file size".into());
    }
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(read as u64)
            .ok_or("Dependency size overflow")?;
        if bytes > item.bytes {
            return Err("Dependency exceeded its pinned byte size".into());
        }
        digest.update(&buffer[..read]);
    }
    verify_digest(bytes, &format!("{:x}", digest.finalize()), item)
}
pub(super) fn verify_digest(bytes: u64, digest: &str, item: &Artifact) -> Result<(), String> {
    if bytes != item.bytes || !digest.eq_ignore_ascii_case(&item.sha256) {
        return Err(
            "Dependency SHA-256 or byte-size mismatch; untrusted bytes will not be activated"
                .into(),
        );
    }
    Ok(())
}
