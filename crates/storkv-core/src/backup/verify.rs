//! Backup generation verification.
use std::path::Path;

use super::manifest::BackupManifest;
use super::retention;

mod files;
mod records;

/// One backup verification failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupVerificationError {
    /// Path of the file that failed verification.
    pub path: String,
    /// Failure detail.
    pub detail: String,
}

/// Result of a backup verification pass.
#[derive(Debug, Clone, Default)]
pub struct BackupVerifyReport {
    /// Number of generations checked.
    pub generations_checked: usize,
    /// Number of files checked.
    pub files_checked: usize,
    /// Number of bytes read and verified.
    pub bytes_checked: u64,
    /// Number of WAL segments checked.
    pub wal_segments_checked: usize,
    /// Verification failures. An empty list means success.
    pub errors: Vec<BackupVerificationError>,
}

impl BackupVerifyReport {
    /// Return true when the report contains no errors.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Verify every archived generation and external WAL segment.
///
/// # Errors
///
/// Returns `crate::StorkvError::TargetSequenceUnavailable` when `target_seq`
/// is outside the archived range, or a catalog read error.
pub fn verify_backup(
    backup_root: &Path,
    target_seq: Option<u64>,
) -> crate::Result<BackupVerifyReport> {
    let catalog = retention::load_catalog(backup_root)?;
    let mut report = BackupVerifyReport::default();

    if let Some(target) = target_seq {
        let oldest = catalog
            .first()
            .map(|manifest| manifest.min_seq)
            .unwrap_or(0);
        let newest = retention::max_archived_sequence(backup_root)?.unwrap_or(0);
        let reachable = catalog.iter().any(|manifest| manifest.max_seq <= target);
        if (!reachable || target > newest) && !catalog.is_empty() {
            return Err(crate::StorkvError::TargetSequenceUnavailable {
                requested: target,
                oldest,
                newest,
            });
        }
    }

    for manifest in &catalog {
        report.generations_checked += 1;
        files::verify_generation(backup_root, manifest, &mut report);
    }
    files::verify_external_wal(backup_root, &mut report);
    Ok(report)
}

pub(crate) fn verify_manifest_generation(
    backup_root: &Path,
    manifest: &BackupManifest,
) -> crate::Result<BackupVerifyReport> {
    let mut report = BackupVerifyReport {
        generations_checked: 1,
        ..BackupVerifyReport::default()
    };
    files::verify_generation(backup_root, manifest, &mut report);
    Ok(report)
}

pub(super) fn push_error(report: &mut BackupVerifyReport, path: &str, detail: impl Into<String>) {
    report.errors.push(BackupVerificationError {
        path: path.to_string(),
        detail: detail.into(),
    });
}
