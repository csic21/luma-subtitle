//! Extract only the pinned standalone Python tar.gz. Internal links are resolved
//! against archive members and copied as regular files; no filesystem links exist.
use super::{archive::{cancelled, relative_path}, store::{self, FileReceipt}};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet}, fs::{self, File, OpenOptions}, io::{Read, Write}, path::Path, sync::atomic::AtomicBool};

#[derive(Clone)]
enum Member { File(FileReceipt, u32), Link(String) }
pub(super) fn extract(source: &Path, root: &Path, byte_cap: u64, file_cap: usize, cancel: &AtomicBool) -> Result<Vec<FileReceipt>, String> {
    if byte_cap == 0 || byte_cap > super::catalog::MAX_INSTALLED_BYTES || file_cap == 0 || file_cap > 100_000 { return Err("Invalid private Python extraction bounds.".into()); }
    let raw_cap = byte_cap.checked_add((file_cap as u64) * 4096).and_then(|n| n.checked_add(1024 * 1024)).ok_or("Python tar size overflow")?;
    let decoder = GzDecoder::new(File::open(source).map_err(|e| e.to_string())?).take(raw_cap + 1);
    let mut archive = tar::Archive::new(decoder);
    let mut seen = HashSet::new(); let mut members = HashMap::new(); let mut total = 0u64; let mut count = 0usize;
    for entry in archive.entries().map_err(|e| format!("Invalid Python tar archive: {e}"))?.raw(true) {
        cancelled(cancel)?; count += 1;
        if count > file_cap.saturating_mul(4).saturating_add(1024) { return Err("Private Python tar exceeds its member count.".into()); }
        let mut entry = entry.map_err(|e| e.to_string())?;
        // Raw iteration prevents tar-rs from buffering GNU long-name/PAX data
        // or expanding sparse metadata before our limits/cancellation checks.
        let kind = entry.header().entry_type();
        if !matches!(kind.as_byte(), 0 | b'0' | b'1' | b'2' | b'5') { return Err("Private Python tar contains unsupported extension or sparse metadata.".into()); }
        let raw = entry.path_bytes();
        let name = std::str::from_utf8(&raw).map_err(|_| "Python tar paths must be UTF-8")?.trim_end_matches('/').to_owned();
        if name == "python" && kind.is_dir() {
            if entry.header().size().map_err(|e| e.to_string())? != 0 { return Err("Python tar root directory contains unexpected data.".into()); }
            continue;
        }
        let relative = name.strip_prefix("python/").ok_or("Python tar member is outside the pinned python/ root")?.to_owned();
        relative_path(&relative)?;
        if !seen.insert(relative.to_lowercase()) { return Err("Duplicate or case-aliased Python tar member.".into()); }
        let size = entry.header().size().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            if size != 0 { return Err("Python tar directory contains unexpected data.".into()); }
            store::create_private_dir(&root.join(&relative))?; continue;
        }
        if members.len() >= file_cap { return Err("Private Python tar exceeds its regular/link file count.".into()); }
        if kind.is_symlink() || kind.is_hard_link() {
            if size != 0 { return Err("Python tar link contains unexpected data.".into()); }
            let link = entry.link_name_bytes().ok_or("Python tar link has no target")?;
            let link = std::str::from_utf8(&link).map_err(|_| "Python tar link targets must be UTF-8")?;
            let target = resolve_link_name(&relative, link, kind.is_hard_link())?;
            members.insert(relative, Member::Link(target)); continue;
        }
        if !kind.is_file() { return Err("Private Python tar contains a special or unsupported file.".into()); }
        total = total.checked_add(size).ok_or("Python tar size overflow")?;
        if total > byte_cap { return Err("Private Python tar exceeds its unpacked byte cap.".into()); }
        let mode = entry.header().mode().map_err(|e| e.to_string())?;
        let receipt = copy_member(&mut entry, root, &relative, size, mode, cancel)?;
        members.insert(relative, Member::File(receipt, mode));
    }
    // Consume bounded trailing padding and gzip checksum. A compressed bomb
    // cannot hide behind tar's end-of-archive marker.
    let mut decoder = archive.into_inner(); let mut buffer = [0u8; 64 * 1024];
    loop { cancelled(cancel)?; let count = decoder.read(&mut buffer).map_err(|e| e.to_string())?; if count == 0 { break; } }
    if decoder.limit() == 0 { return Err("Private Python tar exceeds its decompressed stream limit.".into()); }
    let mut paths: Vec<_> = members.keys().cloned().collect(); paths.sort();
    let mut receipts = Vec::new();
    for path in paths {
        cancelled(cancel)?;
        match members.get(&path).ok_or("Missing Python tar member")? {
            Member::File(receipt, _) => receipts.push(receipt.clone()),
            Member::Link(_) => {
                let (original, mode) = resolve_regular(&path, &members, &mut HashSet::new(), 0)?;
                total = total.checked_add(original.bytes).ok_or("Expanded Python tar size overflow")?;
                if total > byte_cap { return Err("Expanded private Python links exceed the unpacked byte cap.".into()); }
                let source = store::checked_path(root, &original.path)?; store::regular_file(&source)?;
                let mut input = File::open(source).map_err(|e| e.to_string())?;
                let receipt = copy_member(&mut input, root, &path, original.bytes, mode, cancel)?;
                if receipt.sha256 != original.sha256 { return Err("Python tar link copy changed unexpectedly.".into()); }
                receipts.push(receipt);
            }
        }
    }
    if receipts.is_empty() { return Err("Private Python tar contains no regular files.".into()); }
    Ok(receipts)
}
fn copy_member(reader: &mut impl Read, root: &Path, relative: &str, size: u64, mode: u32, cancel: &AtomicBool) -> Result<FileReceipt, String> {
    let path = root.join(relative_path(relative)?);
    if let Some(parent) = path.parent() { store::create_private_dir(parent)?; }
    let mut options = OpenOptions::new(); options.create_new(true).write(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut output = options.open(&path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new(); let mut total = 0u64; let mut buffer = [0u8; 64 * 1024];
    loop {
        cancelled(cancel)?;
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        total = total.checked_add(count as u64).ok_or("Python tar entry size overflow")?;
        if total > size { return Err("Python tar member exceeded its exact size.".into()); }
        output.write_all(&buffer[..count]).map_err(|e| e.to_string())?; digest.update(&buffer[..count]);
    }
    if total != size { return Err("Truncated Python tar member.".into()); }
    output.sync_all().map_err(|e| e.to_string())?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; fs::set_permissions(&path, fs::Permissions::from_mode(if mode & 0o111 != 0 { 0o700 } else { 0o600 })).map_err(|e| e.to_string())?; }
    let _ = mode;
    Ok(FileReceipt { path: relative.into(), bytes: total, sha256: format!("{:x}", digest.finalize()) })
}
fn resolve_link_name(name: &str, link: &str, hard: bool) -> Result<String, String> {
    if link.is_empty() || link.len() > 2048 || link.starts_with('/') || link.contains('\\') || link.contains(':') || link.chars().any(|c| c.is_control()) { return Err("Unsafe private Python tar link target.".into()); }
    let mut parts: Vec<&str> = if hard { Vec::new() } else { name.split('/').collect() };
    if !hard { parts.pop(); }
    let link = if hard { link.strip_prefix("python/").ok_or("Python hardlink escapes its archive root")? } else { link };
    for part in link.split('/') {
        match part {
            "" => return Err("Ambiguous Python tar link target.".into()),
            "." => (),
            ".." => { if parts.pop().is_none() { return Err("Python tar link escapes its archive root.".into()); } },
            value => { relative_path(value)?; parts.push(value); }
        }
    }
    let normalized = parts.join("/"); relative_path(&normalized)?; Ok(normalized)
}
fn resolve_regular<'a>(name: &str, members: &'a HashMap<String, Member>, seen: &mut HashSet<String>, depth: usize) -> Result<(&'a FileReceipt, u32), String> {
    if depth > 16 || !seen.insert(name.to_owned()) { return Err("Cyclic or excessive-depth private Python tar links.".into()); }
    match members.get(name) {
        Some(Member::File(receipt, mode)) => Ok((receipt, *mode)),
        Some(Member::Link(target)) => resolve_regular(target, members, seen, depth + 1),
        None => Err("Python tar link targets a missing or non-regular archive member.".into()),
    }
}
