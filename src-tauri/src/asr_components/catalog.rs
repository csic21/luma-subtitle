//! Trust comes only from the catalog compiled into the signed application.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use url::Url;

pub(super) const MAX_INSTALLED_BYTES: u64 = 40 * 1024 * 1024 * 1024;
pub(super) const MAX_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Archive {
    pub(crate) url: String,
    pub(crate) bytes: u64,
    pub(crate) sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelFile {
    pub(crate) path: String,
    pub(crate) url: String,
    pub(crate) bytes: u64,
    pub(crate) sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Runtime {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) label: String,
    pub(crate) platform: String,
    pub(crate) min_os_version: Option<String>,
    pub(crate) engine: String,
    pub(crate) backend: String,
    pub(crate) device: String,
    pub(crate) license: String,
    pub(crate) license_url: String,
    pub(crate) installed_bytes: u64,
    pub(crate) max_files: usize,
    pub(crate) entrypoint: String,
    pub(crate) archive: Option<Archive>,
    pub(crate) unavailable_reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Model {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) label: String,
    pub(crate) engine: String,
    pub(crate) backend: String,
    pub(crate) role: String,
    pub(crate) license: String,
    pub(crate) license_url: String,
    pub(crate) installed_bytes: u64,
    pub(crate) files: Vec<ModelFile>,
    pub(crate) unavailable_reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    pub(crate) schema: u32,
    #[serde(default)]
    pub(crate) platform: String,
    pub(crate) runtimes: Vec<Runtime>,
    pub(crate) models: Vec<Model>,
}
#[derive(Clone, Debug)]
pub(super) enum Component {
    Runtime(Runtime),
    Model(Model),
}
impl Component {
    pub(super) fn id(&self) -> &str { match self { Self::Runtime(v) => &v.id, Self::Model(v) => &v.id } }
    pub(super) fn version(&self) -> &str { match self { Self::Runtime(v) => &v.version, Self::Model(v) => &v.version } }
    pub(super) fn kind(&self) -> &str { match self { Self::Runtime(_) => "runtime", Self::Model(_) => "model" } }
    pub(super) fn installed_bytes(&self) -> u64 { match self { Self::Runtime(v) => v.installed_bytes, Self::Model(v) => v.installed_bytes } }
    pub(super) fn download_bytes(&self) -> u64 { match self { Self::Runtime(v) => v.archive.as_ref().map_or(0, |a| a.bytes), Self::Model(v) => v.files.iter().map(|f| f.bytes).sum() } }
    pub(super) fn availability(&self) -> Result<(), String> {
        match self {
            Self::Runtime(v) => {
                check_os(v.min_os_version.as_deref())?;
                if v.platform != platform() { return Err("This engine component is not available for this computer's platform.".into()); }
                if let Some(reason) = &v.unavailable_reason { return Err(reason.clone()); }
                if v.archive.is_none() { return Err("This engine component has not been published and verified yet.".into()); }
            }
            Self::Model(v) => {
                if let Some(reason) = &v.unavailable_reason { return Err(reason.clone()); }
                if v.files.is_empty() { return Err("This model has no verified download files.".into()); }
            }
        }
        Ok(())
    }
}
impl Catalog {
    pub(super) fn components(&self) -> Vec<Component> {
        self.runtimes.iter().cloned().map(Component::Runtime).chain(self.models.iter().cloned().map(Component::Model)).collect()
    }
    pub(super) fn find(&self, id: &str) -> Result<Component, String> {
        self.components().into_iter().find(|c| c.id() == id).ok_or_else(|| "Unknown managed ASR component.".into())
    }
}
pub(super) fn platform() -> &'static str {
    if cfg!(all(windows, target_arch = "x86_64")) { "windows-x64" }
    else if cfg!(all(target_os = "macos", target_arch = "aarch64")) { "macos-arm64" }
    else { "unsupported" }
}
pub(super) fn embedded() -> Result<Catalog, String> {
    let mut catalog = parse(include_str!("../../resources/asr/catalog.json"))?;
    for runtime in &mut catalog.runtimes {
        if runtime.platform == platform() {
            if let Err(reason) = check_os(runtime.min_os_version.as_deref()) { runtime.unavailable_reason = Some(reason); }
        }
    }
    Ok(catalog)
}
pub(super) fn parse(source: &str) -> Result<Catalog, String> {
    let mut catalog: Catalog = serde_json::from_str(source).map_err(|e| format!("Invalid embedded ASR catalog: {e}"))?;
    if catalog.schema != 1 { return Err("Unsupported embedded ASR catalog schema.".into()); }
    catalog.platform = platform().into();
    let mut ids = HashSet::new();
    for component in catalog.components() {
        if !safe_id(component.id()) || !safe_version(component.version()) || !ids.insert(component.id().to_owned()) {
            return Err("Invalid or duplicate embedded ASR component identity.".into());
        }
        if component.installed_bytes() > MAX_INSTALLED_BYTES { return Err("Embedded component exceeds installation safety limit.".into()); }
        match component {
            Component::Runtime(v) => {
                if !matches!(v.platform.as_str(), "windows-x64" | "macos-arm64") || !matches!(v.backend.as_str(), "faster-whisper" | "mlx-whisper" | "qwen3-asr") || !matches!(v.device.as_str(), "cpu" | "metal") || v.max_files > 100_000 {
                    return Err("Unsupported embedded ASR runtime.".into());
                }
                if v.min_os_version.as_deref().is_some_and(|v| version_numbers(v).is_none()) { return Err("Invalid runtime minimum OS version.".into()); }
                super::archive::relative_path(&v.entrypoint)?;
                if let Some(a) = v.archive {
                    validate_download(&a.url, a.bytes, &a.sha256, Source::Runtime)?;
                    if v.installed_bytes == 0 || v.max_files == 0 { return Err("Runtime extraction bounds are missing.".into()); }
                }
            }
            Component::Model(v) => {
                if !matches!(v.role.as_str(), "model" | "aligner") || v.files.len() > 10_000 { return Err("Invalid embedded model role/files.".into()); }
                let mut names = HashSet::new(); let mut bytes = 0u64;
                for file in v.files {
                    super::archive::relative_path(&file.path)?;
                    if !names.insert(file.path.to_ascii_lowercase()) { return Err("Duplicate embedded model file.".into()); }
                    validate_download(&file.url, file.bytes, &file.sha256, Source::Model)?;
                    bytes = bytes.checked_add(file.bytes).ok_or("Model size overflow")?;
                }
                if bytes != v.installed_bytes { return Err("Embedded model file sizes do not match the installation total.".into()); }
            }
        }
    }
    Ok(catalog)
}
pub(super) fn safe_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 96 && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn safe_version(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') && !value.starts_with('.')
}
#[derive(Clone, Copy)]
pub(super) enum Source { Runtime, Model }
pub(super) fn validate_download(raw: &str, bytes: u64, hash: &str, source: Source) -> Result<(), String> {
    if bytes == 0 || bytes > MAX_DOWNLOAD_BYTES || hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Embedded download is missing an exact size or SHA-256 digest.".into());
    }
    let url = Url::parse(raw).map_err(|_| "Invalid embedded download URL")?;
    secure_url(&url)?;
    if url.query().is_some() || url.fragment().is_some() { return Err("Embedded source URLs must be immutable and have no query or fragment.".into()); }
    let parts: Vec<_> = url.path().split('/').filter(|s| !s.is_empty()).collect();
    let trusted = match source {
        Source::Runtime => url.host_str() == Some("github.com") && parts.len() == 6 && parts[..4] == ["csic21", "luma-subtitle", "releases", "download"] && parts[4].starts_with("asr-components-") && parts[5].ends_with(".zip"),
        Source::Model => url.host_str() == Some("huggingface.co") && parts.len() >= 5 && matches!(parts[0], "Qwen" | "Systran" | "mobiuslabsgmbh" | "dropbox-dash" | "mlx-community") && parts[2] == "resolve" && parts[3].len() == 40 && parts[3].bytes().all(|b| b.is_ascii_hexdigit()),
    };
    if !trusted { return Err("Embedded download URL is outside the trusted immutable source allowlist.".into()); }
    Ok(())
}
fn secure_url(url: &Url) -> Result<(), String> {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.port().is_some_and(|p| p != 443) {
        return Err("Component downloads require HTTPS without URL credentials or custom ports.".into());
    }
    Ok(())
}
pub(super) fn allowed_redirect(url: &Url, source: Source) -> bool {
    if secure_url(url).is_err() { return false; }
    let host = url.host_str().unwrap_or("");
    match source {
        Source::Runtime => matches!(host, "github.com" | "release-assets.githubusercontent.com" | "objects.githubusercontent.com"),
        Source::Model => matches!(host, "huggingface.co" | "cdn-lfs.huggingface.co" | "cdn-lfs.hf.co" | "cdn-lfs-us-1.hf.co" | "cdn-lfs-eu-1.hf.co") || host.ends_with(".xethub.hf.co"),
    }
}

fn version_numbers(value: &str) -> Option<Vec<u32>> {
    let mut values = value.split('.').map(str::parse::<u32>).collect::<Result<Vec<_>, _>>().ok()?;
    if values.is_empty() || values.len() > 3 { return None; }
    values.resize(3, 0); Some(values)
}
pub(super) fn version_supported(actual: &str, required: &str) -> bool {
    match (version_numbers(actual), version_numbers(required)) { (Some(a), Some(r)) => a >= r, _ => false }
}
pub(super) fn check_os(minimum: Option<&str>) -> Result<(), String> {
    let Some(minimum) = minimum else { return Ok(()); };
    let actual = host_os_version().ok_or("Cannot verify this computer's OS version. This component cannot be safely installed.")?;
    if !version_supported(&actual, minimum) { return Err(format!("This engine component requires OS {minimum} or later; this computer is running {actual}. The original Whisper engine remains available.")); }
    Ok(())
}
#[cfg(target_os = "macos")]
fn host_os_version() -> Option<String> {
    let output = std::process::Command::new("/usr/bin/sw_vers").arg("-productVersion").output().ok()?;
    if !output.status.success() || output.stdout.len() > 64 { return None; }
    Some(String::from_utf8(output.stdout).ok()?.trim().into())
}
#[cfg(windows)]
fn host_os_version() -> Option<String> {
    #[repr(C)] struct Version { size: u32, major: u32, minor: u32, build: u32, platform: u32, service_pack: [u16; 128] }
    #[link(name = "ntdll")] extern "system" { fn RtlGetVersion(version: *mut Version) -> i32; }
    let mut version = Version { size: std::mem::size_of::<Version>() as u32, major: 0, minor: 0, build: 0, platform: 0, service_pack: [0; 128] };
    // Official read-only Windows version query; unlike GetVersionEx this does
    // not report a compatibility-manifest version. The structure is initialized.
    if unsafe { RtlGetVersion(&mut version) } < 0 { return None; }
    Some(format!("{}.{}.{}", version.major, version.minor, version.build))
}
#[cfg(not(any(windows, target_os = "macos")))]
fn host_os_version() -> Option<String> { None }
