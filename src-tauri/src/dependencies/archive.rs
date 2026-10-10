//! Bounded tar.gz extraction. Links are copied only from validated archive members;
//! no archive-controlled symlink or hardlink is ever created on disk.
use super::trust::Artifact;
use crate::asr_components::{legacy_private_dir, legacy_relative_path};
use flate2::read::GzDecoder;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
#[derive(Clone)]
enum Member {
    File(u64),
    Link(String),
}

pub(super) fn extract_tar(source: &Path, root: &Path, artifact: &Artifact) -> Result<(), String> {
    let byte_cap = artifact.unpacked_bytes;
    let file_cap = artifact.file_limit;
    if byte_cap == 0 || byte_cap > 4 * 1024 * 1024 * 1024 || file_cap == 0 || file_cap > 100_000 {
        return Err("Invalid dependency extraction limits".into());
    }
    let raw_cap = byte_cap
        .checked_add(file_cap as u64 * 4096)
        .and_then(|n| n.checked_add(1024 * 1024))
        .ok_or("Tar limit overflow")?;
    let decoder = GzDecoder::new(File::open(source).map_err(|e| e.to_string())?).take(raw_cap + 1);
    let mut archive = tar::Archive::new(decoder);
    let mut seen = HashSet::new();
    let mut members = HashMap::new();
    let mut total = 0u64;
    let mut count = 0usize;
    for entry in archive.entries().map_err(|e| e.to_string())?.raw(true) {
        count += 1;
        if count > file_cap.saturating_mul(4).saturating_add(1024) {
            return Err("Dependency tar exceeds its entry limit".into());
        }
        let mut entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.header().entry_type();
        let size = entry.header().size().map_err(|e| e.to_string())?;
        // GitHub source archives carry only this global commit comment. Never
        // accept PAX path/size overrides, GNU long names, sparse files or devices.
        if kind.as_byte() == b'g' {
            if size > 1024 {
                return Err("Oversized tar metadata".into());
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            let expected = format!("52 comment={}\n", artifact.version);
            if artifact.source != super::trust::Source::Codeload || bytes != expected.as_bytes() {
                return Err("Unsupported tar metadata".into());
            }
            continue;
        }
        if !matches!(kind.as_byte(), 0 | b'0' | b'1' | b'2' | b'5') {
            return Err("Unsupported dependency tar entry type".into());
        }
        let raw = entry.path_bytes();
        let name = std::str::from_utf8(&raw)
            .map_err(|_| "Tar filenames must be UTF-8")?
            .trim_end_matches('/')
            .to_owned();
        legacy_relative_path(&name)?;
        if !seen.insert(name.to_lowercase()) {
            return Err("Duplicate or case-aliased tar path".into());
        }
        if kind.is_dir() {
            if size != 0 {
                return Err("Tar directory has unexpected data".into());
            }
            legacy_private_dir(&root.join(&name))?;
            continue;
        }
        if members.len() >= file_cap {
            return Err("Dependency tar exceeds its file limit".into());
        }
        if kind.is_symlink() || kind.is_hard_link() {
            if size != 0 {
                return Err("Tar link has unexpected data".into());
            }
            let raw = entry.link_name_bytes().ok_or("Tar link has no target")?;
            let link = std::str::from_utf8(&raw).map_err(|_| "Tar link targets must be UTF-8")?;
            members.insert(
                name.clone(),
                Member::Link(resolve_link(&name, link, kind.is_hard_link())?),
            );
            continue;
        }
        total = total.checked_add(size).ok_or("Tar size overflow")?;
        if total > byte_cap {
            return Err("Dependency tar exceeds its unpacked byte limit".into());
        }
        let mode = entry.header().mode().map_err(|e| e.to_string())?;
        copy_member(&mut entry, root, &name, size, mode)?;
        members.insert(name, Member::File(size));
    }
    // Read bounded trailing bytes as well, to verify gzip CRC and prohibit bombs
    // hidden after the tar end marker.
    let mut decoder = archive.into_inner();
    let mut buffer = [0u8; 64 * 1024];
    while decoder.read(&mut buffer).map_err(|e| e.to_string())? != 0 {}
    if decoder.limit() == 0 {
        return Err("Dependency tar exceeded its decompressed stream limit".into());
    }
    if members.is_empty() {
        return Err("Dependency tar contains no files".into());
    }
    for (name, member) in &members {
        if matches!(member, Member::Link(_)) {
            let (original, bytes) = resolve_regular(name, &members, &mut HashSet::new(), 0)?;
            total = total
                .checked_add(bytes)
                .ok_or("Expanded tar size overflow")?;
            if total > byte_cap {
                return Err("Materialized tar links exceed the unpacked byte limit".into());
            }
            let source = root.join(legacy_relative_path(original)?);
            let metadata = fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err("Tar link source is not a regular file".into());
            }
            let mut input = File::open(&source).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode()
            };
            #[cfg(not(unix))]
            let mode = 0;
            copy_member(&mut input, root, name, bytes, mode)?;
        }
    }
    Ok(())
}
fn copy_member(
    input: &mut impl Read,
    root: &Path,
    name: &str,
    size: u64,
    mode: u32,
) -> Result<(), String> {
    let path = root.join(legacy_relative_path(name)?);
    if let Some(parent) = path.parent() {
        legacy_private_dir(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(&path).map_err(|e| e.to_string())?;
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or("Tar entry size overflow")?;
        if total > size {
            return Err("Tar entry exceeded declared size".into());
        }
        output
            .write_all(&buffer[..read])
            .map_err(|e| e.to_string())?;
    }
    if total != size {
        return Err("Truncated tar member".into());
    }
    output.sync_all().map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if mode & 0o111 != 0 { 0o700 } else { 0o600 }),
        )
        .map_err(|e| e.to_string())?;
    }
    let _ = mode;
    Ok(())
}
pub(super) fn resolve_link(name: &str, link: &str, hard: bool) -> Result<String, String> {
    if link.is_empty()
        || link.len() > 2048
        || link.starts_with('/')
        || link.contains('\\')
        || link.contains(':')
        || link.chars().any(|c| c.is_control())
    {
        return Err("Unsafe tar link target".into());
    }
    let mut parts: Vec<_> = if hard {
        Vec::new()
    } else {
        name.split('/').collect()
    };
    if !hard {
        parts.pop();
    }
    for part in link.split('/') {
        match part {
            "" => return Err("Ambiguous tar link target".into()),
            "." => (),
            ".." => {
                if parts.pop().is_none() {
                    return Err("Tar link escaped its archive root".into());
                }
            }
            value => {
                legacy_relative_path(value)?;
                parts.push(value);
            }
        }
    }
    let normalized = parts.join("/");
    legacy_relative_path(&normalized)?;
    Ok(normalized)
}
fn resolve_regular<'a>(
    name: &'a str,
    members: &'a HashMap<String, Member>,
    seen: &mut HashSet<String>,
    depth: usize,
) -> Result<(&'a str, u64), String> {
    if depth > 16 || !seen.insert(name.into()) {
        return Err("Cyclic or excessive-depth tar links".into());
    }
    match members.get(name) {
        Some(Member::File(bytes)) => Ok((name, *bytes)),
        Some(Member::Link(target)) => resolve_regular(target, members, seen, depth + 1),
        None => Err("Tar link targets a missing or non-regular member".into()),
    }
}
