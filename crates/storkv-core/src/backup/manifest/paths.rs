use std::path::{Path, PathBuf};

pub fn manifest_path_string(relative_path: &Path) -> String {
    relative_path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn manifest_relative_path(value: &str) -> PathBuf {
    value
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "..")
        .fold(PathBuf::new(), |path, segment| path.join(segment))
}

pub fn generation_directory(root: &Path, generation_id: &str) -> PathBuf {
    root.join("generations").join(generation_id)
}

pub fn staging_directory(root: &Path, generation_id: &str) -> PathBuf {
    root.join(".tmp").join(generation_id)
}

pub fn new_generation_id(timestamp_ms: u64, sequence: u64) -> String {
    format!("full-{timestamp_ms}-{sequence:020}")
}

pub fn new_wal_generation_id(segment_number: u64) -> String {
    format!("wal-{segment_number:020}")
}

pub fn now_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
