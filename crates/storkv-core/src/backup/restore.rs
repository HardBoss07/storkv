//! Backup restore planning and replay.
use std::path::Path;

use crate::error::{Result, StorkvError};

use super::config::BackupConfig;
use super::retention;
use super::verify::verify_manifest_generation;

mod replay;
mod staging;

/// Result of a point-in-time restore.
#[derive(Debug, Clone, Default)]
pub struct RestoreReport {
    /// Sequence number of the restored state.
    pub restored_seq: u64,
    /// Number of files verified before the restore.
    pub files_verified: usize,
    /// Number of WAL segments replayed.
    pub wal_segments_replayed: usize,
    /// Number of bytes restored.
    pub bytes_restored: u64,
    /// Identifier of the base generation.
    pub generation_id: String,
    /// Non-fatal warnings.
    pub warnings: Vec<String>,
}

/// Restore a database to `target_seq`, or to the newest archived state.
///
/// # Errors
///
/// Returns `StorkvError::TargetSequenceUnavailable` when the target is out of
/// range, `StorkvError::BackupCorruption` on a failed verification, or
/// `StorkvError::RestoreTargetNotEmpty` when the target directory is not empty.
pub fn restore_from_backup(
    backup_root: &Path,
    target_data_dir: &Path,
    target_seq: Option<u64>,
) -> Result<RestoreReport> {
    let catalog = retention::load_catalog(backup_root)?;
    if catalog.is_empty() {
        return Err(StorkvError::BackupGenerationNotFound(
            backup_root.display().to_string(),
        ));
    }
    let newest_archived = retention::max_archived_sequence(backup_root)?.unwrap_or_default();
    let effective_target = target_seq.unwrap_or(newest_archived);
    if effective_target > newest_archived {
        return Err(StorkvError::TargetSequenceUnavailable {
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
        .ok_or_else(|| StorkvError::TargetSequenceUnavailable {
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
        return Err(StorkvError::BackupCorruption {
            path: base.generation_id.clone(),
            detail,
        });
    }

    staging::prepare_target(target_data_dir)?;
    let staging_dir = staging::staging_target(target_data_dir);
    std::fs::DirBuilder::new().create(&staging_dir)?;

    let restore_result = staging::materialize(
        backup_root,
        &base,
        &staging_dir,
        effective_target,
        verification.files_checked,
    );
    let report = match restore_result {
        Ok(report) => report,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging_dir);
            return Err(error);
        }
    };

    if target_data_dir.exists() {
        std::fs::remove_dir_all(target_data_dir)?;
    }
    std::fs::rename(&staging_dir, target_data_dir)?;
    let restored = crate::Db::open(target_data_dir, crate::DbOptions::default())?;
    drop(restored);

    Ok(RestoreReport {
        generation_id: base.generation_id,
        restored_seq: effective_target,
        ..report
    })
}

///
/// # Errors
///
/// Returns the errors of [`restore_from_backup`].
/// Helper for callers that want to restore using a previously-built config root.
pub fn restore_from_config(
    config: &BackupConfig,
    target_data_dir: &Path,
    target_seq: Option<u64>,
) -> Result<RestoreReport> {
    restore_from_backup(&config.backup_root, target_data_dir, target_seq)
}
