use std::path::{Path, PathBuf};

use crate::error::{Result, StorkvError};

/// Join path components with `/` for a manifest entry.
pub fn manifest_path_string(relative_path: &Path) -> String {
    relative_path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// Convert a manifest path to a confined relative path.
///
/// The function rejects absolute paths, drive prefixes, and `..` segments.
///
/// # Errors
///
/// Returns [`StorkvError::BackupCorruption`] when the path can escape the
/// backup generation directory.
pub fn manifest_relative_path(value: &str) -> Result<PathBuf> {
    if value.is_empty() || value.starts_with('/') || value.starts_with('\\') || value.contains(':')
    {
        return Err(invalid_path(value, "path is absolute or empty"));
    }
    let mut path = PathBuf::new();
    for segment in value.split(['/', '\\']) {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(invalid_path(value, "path contains a parent segment"));
        }
        path.push(segment);
    }
    if path.as_os_str().is_empty() {
        return Err(invalid_path(value, "path has no segment"));
    }
    Ok(path)
}

/// Return the directory of a generation inside the backup root.
///
/// # Errors
///
/// Returns [`StorkvError::BackupCorruption`] when the identifier is unsafe.
pub fn generation_directory(root: &Path, generation_id: &str) -> Result<PathBuf> {
    validate_generation_id(generation_id)?;
    Ok(root.join("generations").join(generation_id))
}

/// Return the staging directory of a generation inside the backup root.
///
/// # Errors
///
/// Returns [`StorkvError::BackupCorruption`] when the identifier is unsafe.
pub fn staging_directory(root: &Path, generation_id: &str) -> Result<PathBuf> {
    validate_generation_id(generation_id)?;
    Ok(root.join(".tmp").join(generation_id))
}

/// Reject a generation identifier that can escape a directory.
///
/// # Errors
///
/// Returns [`StorkvError::BackupCorruption`] when the identifier is empty or
/// contains a separator, a drive prefix, or a parent segment.
pub(super) fn validate_generation_id(generation_id: &str) -> Result<()> {
    let unsafe_characters = generation_id
        .chars()
        .any(|character| matches!(character, '/' | '\\' | ':'));
    if generation_id.is_empty()
        || generation_id == "."
        || generation_id == ".."
        || unsafe_characters
    {
        return Err(invalid_path(generation_id, "invalid generation identifier"));
    }
    Ok(())
}

fn invalid_path(path: &str, detail: &str) -> StorkvError {
    StorkvError::BackupCorruption {
        path: path.to_string(),
        detail: detail.to_string(),
    }
}

/// Build a new full generation identifier.
pub fn new_generation_id(timestamp_ms: u64, sequence: u64) -> String {
    format!("full-{timestamp_ms}-{sequence:020}")
}

/// Build a new WAL generation identifier.
pub fn new_wal_generation_id(segment_number: u64) -> String {
    format!("wal-{segment_number:020}")
}

/// Return the current Unix epoch time in milliseconds.
pub fn now_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
