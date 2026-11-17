use std::path::{Path, PathBuf};

use crate::error::{Result, StorKvError};
use crate::wal::{WAL_HEADER_SIZE, WAL_MAGIC, WalBatchFrame};

use super::config::BackupConfig;
use super::manifest::{BackupManifest, generation_directory, write_synced};
use super::retention;
use super::verify::verify_manifest_generation;
use super::wal_index;

/// Result of a point-in-time restore.
#[derive(Debug, Clone, Default)]
pub struct RestoreReport {
    /// Identifier of the base generation.
    pub generation_id: String,
    /// Sequence number of the restored state.
    pub restored_seq: u64,
    /// Number of files verified before the restore.
    pub files_verified: usize,
    /// Number of WAL segments replayed.
    pub wal_segments_replayed: usize,
    /// Number of bytes restored.
    pub bytes_restored: u64,
    /// Non-fatal warnings.
    pub warnings: Vec<String>,
}

/// Restore a database to `target_seq`, or to the newest archived state.
///
/// # Errors
///
/// Returns [`StorKvError::TargetSequenceUnavailable`] when the target is out of
/// range, [`StorKvError::BackupCorruption`] on a failed verification, or
/// [`StorKvError::RestoreTargetNotEmpty`] when the target directory is not empty.
pub fn restore_from_backup(
    backup_root: &Path,
    target_data_dir: &Path,
    target_seq: Option<u64>,
) -> Result<RestoreReport> {
    let catalog = retention::load_catalog(backup_root)?;
    if catalog.is_empty() {
        return Err(StorKvError::BackupGenerationNotFound(
            backup_root.display().to_string(),
        ));
    }
    let newest_archived = retention::max_archived_sequence(backup_root)?.unwrap_or_default();
    let effective_target = target_seq.unwrap_or(newest_archived);
    if effective_target > newest_archived {
        return Err(StorKvError::TargetSequenceUnavailable {
            requested: effective_target,
            oldest: catalog
                .first()
                .map(|manifest| manifest.min_seq)
                .unwrap_or(0),
            newest: newest_archived,
        });
    }

    let base = catalog
        .iter()
        .rev()
        .find(|manifest| manifest.max_seq <= effective_target)
        .cloned()
        .ok_or_else(|| StorKvError::TargetSequenceUnavailable {
            requested: effective_target,
            oldest: catalog
                .first()
                .map(|manifest| manifest.min_seq)
                .unwrap_or(0),
            newest: newest_archived,
        })?;

    let verification = verify_manifest_generation(backup_root, &base)?;
    if !verification.is_ok() {
        let detail = verification
            .errors
            .iter()
            .map(|error| format!("{}: {}", error.path, error.detail))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(StorKvError::BackupCorruption {
            path: base.generation_id.clone(),
            detail,
        });
    }

    prepare_target(target_data_dir)?;
    let staging = staging_target(target_data_dir);
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;

    let restore_result = materialize(
        backup_root,
        &base,
        &staging,
        effective_target,
        verification.files_checked,
    );
    let report = match restore_result {
        Ok(report) => report,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };

    if target_data_dir.exists() {
        std::fs::remove_dir_all(target_data_dir)?;
    }
    std::fs::rename(&staging, target_data_dir)?;
    let restored = crate::Db::open(target_data_dir, crate::DbOptions::default())?;
    drop(restored);

    Ok(RestoreReport {
        generation_id: base.generation_id,
        restored_seq: effective_target,
        ..report
    })
}

fn materialize(
    backup_root: &Path,
    base: &BackupManifest,
    staging: &Path,
    target_seq: u64,
    files_verified: usize,
) -> Result<RestoreReport> {
    let base_directory = generation_directory(backup_root, &base.generation_id);
    copy_tree(&base_directory, staging)?;

    let base_wal_directory = staging.join("wal");
    let base_segments = wal_index::list_segments(&base_wal_directory)?;
    let max_base_segment = base_segments
        .iter()
        .map(|segment| segment.file_num)
        .max()
        .unwrap_or(0);

    let mut wal_segments_replayed = 0usize;
    for segment in wal_index::list_segments(&backup_root.join("wal"))? {
        if segment.file_num <= max_base_segment {
            continue;
        }
        if segment.min_seq > target_seq {
            continue;
        }
        if replay_segment_filtered(&segment.path, &base_wal_directory, target_seq)? {
            wal_segments_replayed += 1;
        }
    }

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

fn replay_segment_filtered(source: &Path, destination_wal: &Path, target_seq: u64) -> Result<bool> {
    let bytes = std::fs::read(source)?;
    if bytes.len() < WAL_HEADER_SIZE || &bytes[0..8] != WAL_MAGIC {
        return Err(StorKvError::BackupCorruption {
            path: source.display().to_string(),
            detail: "invalid WAL header".into(),
        });
    }
    std::fs::create_dir_all(destination_wal)?;

    let mut output = wal_header(&bytes[12..16]);
    let mut offset = WAL_HEADER_SIZE;
    let mut kept_any = false;
    while offset < bytes.len() {
        let (frame, consumed) = WalBatchFrame::decode(&bytes[offset..]).map_err(|error| {
            StorKvError::BackupCorruption {
                path: source.display().to_string(),
                detail: error.to_string(),
            }
        })?;
        offset += consumed;
        let kept_ops = frame
            .ops
            .iter()
            .enumerate()
            .take_while(|(index, _)| frame.batch_seq.saturating_add(*index as u64) <= target_seq)
            .count();
        if kept_ops == 0 {
            continue;
        }
        let filtered = WalBatchFrame::new(frame.batch_seq, frame.ops[..kept_ops].to_vec());
        output.extend_from_slice(&filtered.encode(false)?);
        kept_any = true;
        if kept_ops < frame.ops.len() {
            break;
        }
    }

    if !kept_any {
        return Ok(false);
    }
    let file_name = source
        .file_name()
        .ok_or_else(|| StorKvError::InvalidOperation("WAL segment has no file name".into()))?;
    write_synced(&destination_wal.join(file_name), &output)?;
    Ok(true)
}

fn wal_header(created_at_sec: &[u8]) -> Vec<u8> {
    let mut header = Vec::with_capacity(WAL_HEADER_SIZE);
    header.extend_from_slice(WAL_MAGIC);
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&0u16.to_le_bytes());
    if created_at_sec.len() == 4 {
        header.extend_from_slice(created_at_sec);
    } else {
        header.extend_from_slice(&0u32.to_le_bytes());
    }
    header
}

fn prepare_target(target: &Path) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(target)?;
    if entries.next().is_some() {
        return Err(StorKvError::RestoreTargetNotEmpty(target.to_path_buf()));
    }
    std::fs::remove_dir_all(target)?;
    Ok(())
}

fn staging_target(target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("storkv-restore");
    target.with_file_name(format!(".{file_name}.restoring-{}", std::process::id()))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let path = entry?.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .ok_or_else(|| StorKvError::InvalidOperation("directory has no name".into()))?;
            copy_tree(&path, &destination.join(name))?;
            continue;
        }
        let name = path
            .file_name()
            .ok_or_else(|| StorKvError::InvalidOperation("file has no name".into()))?;
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

/// Helper for callers that want to restore using a previously-built config root.
pub fn restore_from_config(
    config: &BackupConfig,
    target_data_dir: &Path,
    target_seq: Option<u64>,
) -> Result<RestoreReport> {
    restore_from_backup(&config.backup_root, target_data_dir, target_seq)
}
