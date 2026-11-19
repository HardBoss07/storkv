//! Target validation, staging directory creation, and base image copy.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::backup::manifest::{BackupManifest, generation_directory};
use crate::error::{Result, StorkvError};

use super::RestoreReport;
use super::replay;

pub(super) fn materialize(
    backup_root: &Path,
    base: &BackupManifest,
    staging: &Path,
    target_seq: u64,
    files_verified: usize,
) -> Result<RestoreReport> {
    let base_directory = generation_directory(backup_root, &base.generation_id)?;
    copy_tree(&base_directory, staging)?;

    let base_wal_directory = staging.join("wal");
    let wal_segments_replayed =
        replay::replay_segments(backup_root, &base_wal_directory, target_seq)?;

    let bytes_restored = directory_size(staging)?;
    Ok(RestoreReport {
        generation_id: base.generation_id.clone(),
        restored_seq: target_seq,
        files_verified,
        wal_segments_replayed,
        bytes_restored,
        warnings: Vec::new(),
    })
}

pub(super) fn prepare_target(target: &Path) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(target)?;
    if entries.next().is_some() {
        return Err(StorkvError::RestoreTargetNotEmpty(target.to_path_buf()));
    }
    std::fs::remove_dir_all(target)?;
    Ok(())
}

pub(super) fn staging_target(target: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("storkv-restore");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let suffix = format!(
        ".{file_name}.restoring-{}-{nanos}-{count}",
        std::process::id()
    );
    target.with_file_name(suffix)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let path = entry?.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(StorkvError::BackupCorruption {
                path: path.display().to_string(),
                detail: "symbolic link in backup tree".into(),
            });
        }
        let name = path
            .file_name()
            .ok_or_else(|| StorkvError::InvalidOperation("entry has no name".into()))?;
        if file_type.is_dir() {
            copy_tree(&path, &destination.join(name))?;
            continue;
        }
        if !file_type.is_file() {
            return Err(StorkvError::BackupCorruption {
                path: path.display().to_string(),
                detail: "unsupported file type in backup tree".into(),
            });
        }
        std::fs::copy(&path, destination.join(name))?;
    }
    Ok(())
}

fn directory_size(directory: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            total = total.saturating_add(directory_size(&path)?);
        } else {
            total = total.saturating_add(std::fs::metadata(&path)?.len());
        }
    }
    Ok(total)
}
