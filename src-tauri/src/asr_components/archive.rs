//! Strict ZIP extraction into a new, private staging directory only.
use super::store::FileReceipt;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs::{self, File, OpenOptions}, io::{Read, Seek, SeekFrom, Write}, path::{Path, PathBuf}, sync::atomic::{AtomicBool, Ordering}};

pub(super) fn relative_path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.len() > 2048 || name.starts_with('/') || name.contains('\\') || name.contains(':') || name.chars().any(|c| c.is_control()) {
        return Err("Unsafe component archive path.".into());
    }
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    let parts: Vec<_> = trimmed.split('/').collect();
    if parts.len() > 32 || parts.iter().any(|part| {
        if part.is_empty() || *part == "." || *part == ".." || part.ends_with('.') || part.ends_with(' ') { return true; }
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$") || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit())
    }) { return Err("Unsafe or platform-ambiguous component archive path.".into()); }
    Ok(parts.iter().collect())
}
pub(super) fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) { Err("Component setup cancelled.".into()) } else { Ok(()) }
}
pub(super) fn validate_extra(mut bytes: &[u8]) -> Result<(), String> {
    while !bytes.is_empty() {
        if bytes.len() < 4 { return Err("Malformed ZIP extra metadata.".into()); }
        let kind = u16::from_le_bytes([bytes[0], bytes[1]]);
        let size = u16::from_le_bytes([bytes[2], bytes[3]]) as usize;
        if size > bytes.len() - 4 { return Err("Malformed ZIP extra metadata.".into()); }
        // Only ZIP64 and timestamps. In particular reject PKWARE/ASi Unix link
        // metadata (0x000d/0x756e), aliases and all unrecognized extensions.
        if !matches!(kind, 0x0001 | 0x000a | 0x5455) { return Err("Unsupported ZIP metadata; link/alias extensions are not allowed.".into()); }
        bytes = &bytes[4 + size..];
    }
    Ok(())
}
pub(super) fn extract(zip_path: &Path, destination: &Path, byte_limit: u64, file_limit: usize, cancel: &AtomicBool) -> Result<Vec<FileReceipt>, String> {
    let input = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(input).map_err(|e| format!("Invalid component ZIP: {e}"))?;
    if zip.len() == 0 || zip.len() > file_limit || file_limit > 100_000 { return Err("Component ZIP exceeds its file-count limit or is empty.".into()); }
    let mut local_headers = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut seen = HashSet::new(); let mut declared = 0u64;
    // Validate every entry before writing any data.
    for index in 0..zip.len() {
        cancelled(cancel)?;
        let file = zip.by_index(index).map_err(|e| e.to_string())?;
        let path = relative_path(file.name())?;
        if file.encrypted() { return Err("Encrypted component ZIP entries are not allowed.".into()); }
        validate_local_header(&mut local_headers, file.header_start(), file.name())?;
        let normalized = path.to_string_lossy().replace('\\', "/").to_lowercase();
        if !seen.insert(normalized) { return Err("Duplicate or case-aliased ZIP path.".into()); }
        if let Some(extra) = file.extra_data() { validate_extra(extra)?; }
        let mode = file.unix_mode().unwrap_or(0);
        let file_type = mode & 0o170000;
        if file_type != 0 && file_type != 0o100000 && file_type != 0o040000 { return Err("Component ZIP contains a link or special file.".into()); }
        if (file_type == 0o040000) != file.is_dir() && file_type != 0 { return Err("Component ZIP has inconsistent file metadata.".into()); }
        if file.is_dir() && file.size() != 0 { return Err("ZIP directory contains unexpected data.".into()); }
        declared = declared.checked_add(file.size()).ok_or("ZIP size overflow")?;
        if declared > byte_limit { return Err("Component ZIP exceeds its unpacked-byte limit.".into()); }
    }
    let mut actual = 0u64; let mut receipts = Vec::new();
    for index in 0..zip.len() {
        cancelled(cancel)?;
        let mut file = zip.by_index(index).map_err(|e| e.to_string())?;
        let relative = relative_path(file.name())?;
        let target = destination.join(&relative);
        if file.is_dir() { super::store::create_private_dir(&target)?; continue; }
        if let Some(parent) = target.parent() { super::store::create_private_dir(parent)?; }
        let mut options = OpenOptions::new(); options.write(true).create_new(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        let mut output = options.open(&target).map_err(|e| format!("Cannot create staged component file: {e}"))?;
        let mut digest = Sha256::new(); let mut size = 0u64; let mut buffer = [0u8; 64 * 1024];
        loop {
            cancelled(cancel)?;
            let count = file.read(&mut buffer).map_err(|e| format!("Cannot extract component file: {e}"))?;
            if count == 0 { break; }
            size = size.checked_add(count as u64).ok_or("ZIP size overflow")?;
            actual = actual.checked_add(count as u64).ok_or("ZIP size overflow")?;
            if size > file.size() || actual > byte_limit { return Err("Component ZIP exceeded its declared size while extracting.".into()); }
            output.write_all(&buffer[..count]).map_err(|e| e.to_string())?;
            digest.update(&buffer[..count]);
        }
        if size != file.size() { return Err("Component ZIP entry is truncated.".into()); }
        output.sync_all().map_err(|e| e.to_string())?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            // Never preserve setuid/setgid/world-write bits from archives.
            let executable = file.unix_mode().unwrap_or(0) & 0o111 != 0;
            fs::set_permissions(&target, fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 })).map_err(|e| e.to_string())?;
        }
        receipts.push(FileReceipt { path: relative.to_string_lossy().replace('\\', "/"), bytes: size, sha256: format!("{:x}", digest.finalize()) });
    }
    if receipts.is_empty() { return Err("Component ZIP contains no files.".into()); }
    Ok(receipts)
}

fn validate_local_header(input: &mut File, offset: u64, expected_name: &str) -> Result<(), String> {
    input.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut header = [0u8; 30]; input.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..4] != b"PK\x03\x04" { return Err("Invalid ZIP local header.".into()); }
    let name_len = u16::from_le_bytes([header[26], header[27]]) as usize;
    let extra_len = u16::from_le_bytes([header[28], header[29]]) as usize;
    if name_len == 0 || name_len > 2048 { return Err("Unsafe ZIP local filename length.".into()); }
    let mut name = vec![0u8; name_len]; input.read_exact(&mut name).map_err(|e| e.to_string())?;
    if name != expected_name.as_bytes() { return Err("ZIP local and central filenames differ.".into()); }
    relative_path(std::str::from_utf8(&name).map_err(|_| "ZIP paths must be UTF-8")?)?;
    let mut extra = vec![0u8; extra_len]; input.read_exact(&mut extra).map_err(|e| e.to_string())?;
    validate_extra(&extra)
}
