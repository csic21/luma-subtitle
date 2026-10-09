//! Fixed, embedded direct-source recipes. This is not a package resolver.
use super::{archive::relative_path, catalog::{self, Runtime, Source}, ComponentRequest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PythonArchive {
    pub url: String, pub bytes: u64, pub sha256: String,
    pub installed_bytes: u64, pub max_files: usize,
    pub entrypoint: String, pub site_packages: String, pub pip_version: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Wheel {
    pub name: String, pub version: String, pub filename: String,
    pub url: String, pub bytes: u64, pub sha256: String,
    pub installed_bytes: u64, pub max_files: usize,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Term {
    pub id: String, pub version: String, pub sha256: String, pub url: String, pub text: String,
    pub raw_sha256: Option<String>, pub source_encoding: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recipe {
    pub schema: u32, pub python: PythonArchive, pub wheels: Vec<Wheel>, pub terms: Vec<Term>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_crt: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(crate) struct TermAcknowledgement { pub id: String, pub version: String, pub sha256: String }
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Consent {
    pub component_id: String, pub plan_sha256: String, pub terms: Vec<TermAcknowledgement>,
}
impl Recipe {
    pub(super) fn download_bytes(&self) -> u64 { self.python.bytes + self.wheels.iter().map(|w| w.bytes).sum::<u64>() + if self.windows_crt.is_some() { super::direct_crt::DOWNLOAD_BYTES } else { 0 } }
    pub(super) fn acknowledgements(&self) -> Vec<TermAcknowledgement> {
        let mut terms: Vec<_> = self.terms.iter().map(|t| TermAcknowledgement { id: t.id.clone(), version: t.version.clone(), sha256: t.sha256.clone() }).collect();
        terms.sort(); terms
    }
    pub(super) fn validate(&self, runtime: &Runtime) -> Result<(), String> {
        if self.schema != 1 || self.wheels.is_empty() || self.wheels.len() > 128 || self.terms.is_empty() || self.terms.len() > 256 {
            return Err("Embedded direct-source recipe has invalid schema or input counts.".into());
        }
        super::direct_crt::validate_recipe(self, runtime)?;
        let python = &self.python;
        catalog::validate_download(&python.url, python.bytes, &python.sha256, Source::Python)?;
        if python.entrypoint != runtime.entrypoint || python.pip_version != "26.2.1" {
            return Err("The recipe interpreter or private pip is not the reviewed version.".into());
        }
        let expected_site = if runtime.platform == "windows-x64" { "Lib/site-packages" } else { "lib/python3.12/site-packages" };
        if python.site_packages != expected_site { return Err("The private site-packages path does not match this platform.".into()); }
        relative_path(&python.entrypoint)?; relative_path(&python.site_packages)?;
        bounds(python.installed_bytes, python.max_files)?;
        let mut total = python.installed_bytes + if self.windows_crt.is_some() { super::direct_crt::INSTALLED_BOUND } else { 0 }; let mut total_download = python.bytes + if self.windows_crt.is_some() { super::direct_crt::DOWNLOAD_BYTES } else { 0 };
        let mut names = HashSet::new(); let mut filenames = HashSet::new();
        for wheel in &self.wheels {
            if wheel.name.is_empty() || !wheel.name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) || wheel.version.is_empty() || !wheel.version.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+!-".contains(&b)) {
                return Err("Invalid pinned wheel identity.".into());
            }
            if !names.insert(wheel.name.to_ascii_lowercase().replace('_', "-").replace('.', "-")) || !filenames.insert(wheel.filename.to_ascii_lowercase()) {
                return Err("Duplicate pinned wheel identity or filename.".into());
            }
            if relative_path(&wheel.filename)?.components().count() != 1 || !wheel.filename.ends_with(".whl") { return Err("Recipe contains an unsupported wheel filename.".into()); }
            wheel_source(wheel, &runtime.platform)?;
            if !url::Url::parse(&wheel.url).map_err(|e| e.to_string())?.path().ends_with(&format!("/{}", wheel.filename)) { return Err("Pinned wheel URL filename differs from the recipe.".into()); }
            bounds(wheel.installed_bytes, wheel.max_files)?;
            total = total.checked_add(wheel.installed_bytes).ok_or("Recipe extraction size overflow")?;
            total_download = total_download.checked_add(wheel.bytes).ok_or("Recipe download size overflow")?;
        }
        if total > catalog::MAX_INSTALLED_BYTES || total_download > catalog::MAX_INSTALLED_BYTES || runtime.installed_bytes == 0 || runtime.max_files == 0 {
            return Err("Recipe exceeds component safety limits or has no final-tree bounds.".into());
        }
        let mut terms = HashSet::new(); let mut text_bytes = 0usize;
        for term in &self.terms {
            if !catalog::safe_id(&term.id) || term.version.is_empty() || term.version.len() > 128 || !terms.insert(term.id.clone()) || term.text.is_empty() || term.text.len() > 1024 * 1024 {
                return Err("Invalid embedded upstream terms identity or text length.".into());
            }
            if !valid_hash(&term.sha256) || format!("{:x}", Sha256::digest(term.text.as_bytes())) != term.sha256 {
                return Err("Embedded upstream terms do not match their displayed UTF-8 SHA-256.".into());
            }
            if term.raw_sha256.as_deref().is_some_and(|h| !valid_hash(h)) || term.source_encoding.as_deref().is_some_and(|e| !matches!(e, "utf-8" | "cp1252" | "ascii" | "rtf")) {
                return Err("Invalid upstream terms source provenance.".into());
            }
            let url = url::Url::parse(&term.url).map_err(|_| "Invalid upstream terms URL")?;
            if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.port().is_some_and(|p| p != 443) { return Err("Upstream terms require a trusted HTTPS source link.".into()); }
            text_bytes = text_bytes.checked_add(term.text.len()).ok_or("Terms size overflow")?;
        }
        if text_bytes > 8 * 1024 * 1024 { return Err("Embedded upstream terms are too large.".into()); }
        Ok(())
    }
}
/// One audited CPU-only wheel is delivered from the app's exact release asset.
/// Every other wheel retains the existing PyPI-only origin policy.
pub(super) fn wheel_source(wheel: &Wheel, platform: &str) -> Result<Source, String> {
    let normalized_name = wheel.name.to_ascii_lowercase().replace('_', "-").replace('.', "-");
    if platform == "windows-x64" && normalized_name == "ctranslate2" && wheel.url != catalog::CPU_WHEEL_URL {
        return Err("Managed Windows CTranslate2 requires the reviewed CPU-only wheel, not the upstream vendor wheel.".into());
    }
    let source = if wheel.url == catalog::CPU_WHEEL_URL {
        if platform != "windows-x64" || wheel.name != "ctranslate2" || wheel.version != "4.8.2" || wheel.filename != catalog::CPU_WHEEL_FILENAME {
            return Err("The fixed CPU-only wheel source does not match its reviewed package or platform.".into());
        }
        Source::CpuWheel
    } else { Source::Wheel };
    catalog::validate_download(&wheel.url, wheel.bytes, &wheel.sha256, source)?;
    Ok(source)
}
fn bounds(bytes: u64, files: usize) -> Result<(), String> {
    if bytes == 0 || bytes > catalog::MAX_INSTALLED_BYTES || files == 0 || files > 100_000 { return Err("Missing or excessive artifact extraction bounds.".into()); }
    Ok(())
}
pub(super) fn valid_hash(hash: &str) -> bool { hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }
pub(super) fn plan_hash(runtime: &Runtime) -> Result<String, String> {
    let mut snapshot = runtime.clone(); snapshot.plan_sha256 = None;
    // Availability is evaluated for the local OS separately; it is not assent.
    snapshot.unavailable_reason = None;
    let bytes = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new(); digest.update(bytes);
    if runtime.recipe.as_ref().is_some_and(|r| r.windows_crt.is_some()) { digest.update(super::direct_crt::CONTRACT.as_bytes()); }
    Ok(format!("{:x}", digest.finalize()))
}
pub(super) fn validate_acknowledgement(runtime: &Runtime, request: &ComponentRequest) -> Result<Option<Consent>, String> {
    let Some(recipe) = &runtime.recipe else { return Ok(None); };
    let plan = plan_hash(runtime)?;
    if request.plan_sha256.as_deref() != Some(plan.as_str()) { return Err("Review this exact engine setup plan and upstream terms before installing. The displayed plan is missing or has changed.".into()); }
    let mut terms = request.acknowledged_terms.clone(); terms.sort();
    if terms != recipe.acknowledgements() { return Err("Explicit acceptance of every displayed upstream terms ID, version and SHA-256 is required before any download.".into()); }
    Ok(Some(Consent { component_id: runtime.id.clone(), plan_sha256: plan, terms }))
}


#[cfg(test)]
mod cpu_source_tests {
    use super::*;
    fn wheel() -> Wheel {
        Wheel { name: "ctranslate2".into(), version: "4.8.2".into(), filename: catalog::CPU_WHEEL_FILENAME.into(), url: catalog::CPU_WHEEL_URL.into(), bytes: 4, sha256: "a".repeat(64), installed_bytes: 4, max_files: 1 }
    }
    #[test]
    fn own_cpu_wheel_requires_the_exact_identity_and_windows_platform() {
        let pin = wheel();
        assert!(matches!(wheel_source(&pin, "windows-x64").unwrap(), Source::CpuWheel));
        for platform in ["macos-arm64", "windows-arm64", "unsupported"] { assert!(wheel_source(&pin, platform).is_err()); }
        for field in ["name", "version", "filename"] {
            let mut value = serde_json::to_value(&pin).unwrap(); value[field] = serde_json::json!("changed");
            let changed: Wheel = serde_json::from_value(value).unwrap(); assert!(wheel_source(&changed, "windows-x64").is_err(), "accepted changed {field}");
        }
        let mut changed = pin.clone(); changed.bytes = 0; assert!(wheel_source(&changed, "windows-x64").is_err());
        let mut changed = pin; changed.sha256.clear(); assert!(wheel_source(&changed, "windows-x64").is_err());
    }
    #[test]
    fn original_vendor_ctranslate2_wheel_is_rejected_for_managed_windows() {
        // Genuine original PyPI lock entry: trusted transport alone is not the
        // reviewed CPU-only build identity, including case-normalized aliases.
        let mut vendor = wheel();
        vendor.filename = "ctranslate2-4.8.2-cp312-cp312-win_amd64.whl".into();
        vendor.url = "https://files.pythonhosted.org/packages/4e/23/e3b5322ff7368fcbed181ea4c209149416e7940b5b04971d5ee4084afe1a/ctranslate2-4.8.2-cp312-cp312-win_amd64.whl".into();
        vendor.bytes = 19_222_069;
        vendor.sha256 = "d94421d565d0de61c032998f737a18942b0f2bef40c0424b1846ec6f67300105".into();
        assert!(catalog::validate_download(&vendor.url, vendor.bytes, &vendor.sha256, Source::Wheel).is_ok());
        for name in ["ctranslate2", "CTranslate2", "CTRANSLATE2"] {
            vendor.name = name.into(); assert!(wheel_source(&vendor, "windows-x64").is_err());
        }
    }
    #[test]
    fn own_cpu_source_rejects_release_owner_tag_asset_and_url_drift() {
        for url in [
            catalog::CPU_WHEEL_URL.replace("csic21", "other-owner"),
            catalog::CPU_WHEEL_URL.replace("luma-subtitle/", "other-repository/"),
            catalog::CPU_WHEEL_URL.replace("asr-ct2-cpu-4.8.2-1/", "asr-ct2-cpu-4.8.2-2/"),
            catalog::CPU_WHEEL_URL.replace("download/asr-ct2-cpu-4.8.2-1", "latest/download"),
            catalog::CPU_WHEEL_URL.replace("1lumacpu", "2lumacpu"),
            catalog::CPU_WHEEL_URL.replace("https://", "http://"),
            catalog::CPU_WHEEL_URL.replace("github.com", "github.com.evil.invalid"),
            catalog::CPU_WHEEL_URL.replace("https://", "https://user@"),
            format!("{}?asset=changed", catalog::CPU_WHEEL_URL),
            format!("{}#changed", catalog::CPU_WHEEL_URL),
        ] {
            let mut pin = wheel(); pin.url = url;
            assert!(wheel_source(&pin, "windows-x64").is_err(), "accepted {}", pin.url);
            assert!(catalog::validate_download(&pin.url, pin.bytes, &pin.sha256, Source::CpuWheel).is_err());
        }
    }
    #[test]
    fn other_wheels_remain_pypi_only_and_cpu_redirects_stay_separate() {
        let mut pin = wheel(); pin.name = "example".into(); pin.version = "1.0".into(); pin.filename = "example-1.0-py3-none-any.whl".into();
        pin.url = format!("https://files.pythonhosted.org/packages/aa/bb/{}/{}", "a".repeat(64), pin.filename);
        assert!(matches!(wheel_source(&pin, "macos-arm64").unwrap(), Source::Wheel));
        assert!(catalog::validate_download(&pin.url, pin.bytes, &pin.sha256, Source::CpuWheel).is_err());
        assert!(catalog::validate_download(catalog::CPU_WHEEL_URL, pin.bytes, &pin.sha256, Source::Wheel).is_err());
        for raw in ["https://github.com/csic21/luma-subtitle/releases/download/asr-ct2-cpu-4.8.2-1/asset", "https://release-assets.githubusercontent.com/exact-asset?signed=1"] {
            let url = url::Url::parse(raw).unwrap(); assert!(catalog::allowed_redirect(&url, Source::CpuWheel)); assert!(!catalog::allowed_redirect(&url, Source::Wheel));
        }
        for raw in ["https://files.pythonhosted.org/asset", "https://objects.githubusercontent.com/asset", "https://raw.githubusercontent.com/asset", "http://release-assets.githubusercontent.com/asset", "https://release-assets.githubusercontent.com.evil.invalid/asset", "https://user@release-assets.githubusercontent.com/asset", "https://release-assets.githubusercontent.com:444/asset"] {
            assert!(!catalog::allowed_redirect(&url::Url::parse(raw).unwrap(), Source::CpuWheel), "accepted {raw}");
        }
    }
}
