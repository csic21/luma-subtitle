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
            catalog::validate_download(&wheel.url, wheel.bytes, &wheel.sha256, Source::Wheel)?;
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
