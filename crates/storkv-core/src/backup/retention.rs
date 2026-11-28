//! Backup catalog loading and retention planning.
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::Result;

use super::config::BackupRetention;
use super::manifest::{BackupKind, BackupManifest};
use super::sink::RetentionPlan;
use super::wal_index;

///
/// # Errors
///
/// Returns `StorkvError::Io` when the generations directory cannot be read.
/// Returns `StorkvError::BackupCorruption` when a manifest fails validation.
/// Load and schema-validate every full-generation manifest under `generations/`.
pub fn load_catalog(backup_root: &Path) -> Result<Vec<BackupManifest>> {
    let generations_dir = backup_root.join("generations");
    if !generations_dir.exists() {
        return Ok(Vec::new());
    }
    let mut manifests = Vec::new();
    for entry in std::fs::read_dir(&generations_dir)? {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join(super::manifest::MANIFEST_FILENAME);
        if !manifest_path.exists() {
            continue;
        }
        manifests.push(BackupManifest::read_from_dir(&path)?);
    }
    manifests.sort_by(|left, right| {
        left.max_seq
            .cmp(&right.max_seq)
            .then(left.created_at_ms.cmp(&right.created_at_ms))
    });
    Ok(manifests)
}

///
/// # Errors
///
/// Returns the errors of [`load_catalog`].
/// Returns `StorkvError::Io` when WAL segments cannot be inspected.
/// Produce a conservative prune plan.
///
/// A WAL segment is only removable when it is fully covered by the oldest
/// retained full generation, older than `max_wal_age`, and not listed as a
/// file inside any retained generation.
pub fn build_plan(backup_root: &Path, retention: &BackupRetention) -> Result<RetentionPlan> {
    let catalog = load_catalog(backup_root)?;
    let keep_count = retention.max_full_backups.max(1);
    let split_at = catalog.len().saturating_sub(keep_count);
    let (removable_generations, retained_generations) = catalog.split_at(split_at);

    let oldest_retained_seq = retained_generations
        .iter()
        .map(|manifest| manifest.max_seq)
        .min()
        .unwrap_or(u64::MAX);

    let mut referenced_segments = BTreeSet::new();
    for manifest in retained_generations {
        for file in &manifest.files {
            if file.kind == super::manifest::BackupFileKind::Wal {
                if let Some(file_name) = file.path.rsplit('/').next() {
                    if let Some(number) = super::wal_index::parse_file_number(Path::new(file_name))
                    {
                        referenced_segments.insert(number);
                    }
                }
            }
        }
    }

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let max_wal_age_secs = retention.max_wal_age.as_secs();
    let mut removable_segments = Vec::new();
    for segment in wal_index::list_segments(&backup_root.join("wal"))? {
        if referenced_segments.contains(&segment.file_num) {
            continue;
        }
        if segment.max_seq > oldest_retained_seq {
            continue;
        }
        if now_secs.saturating_sub(segment.created_at_sec as u64) < max_wal_age_secs {
            continue;
        }
        removable_segments.push(segment.file_num);
    }

    Ok(RetentionPlan {
        generations: removable_generations
            .iter()
            .filter(|manifest| manifest.kind == BackupKind::Full)
            .map(|manifest| manifest.generation_id.clone())
            .collect(),
        wal_segments: removable_segments,
        generations_removed: Some(removable_generations.len()),
    })
}

/// Return the newest sequence number that the archive holds.
///
/// # Errors
///
/// Returns an error when the catalog or the WAL index cannot be read.
pub fn max_archived_sequence(backup_root: &Path) -> Result<Option<u64>> {
    let mut newest = load_catalog(backup_root)?
        .into_iter()
        .map(|manifest| manifest.max_seq)
        .max();
    for segment in wal_index::list_segments(&backup_root.join("wal"))? {
        newest = Some(newest.unwrap_or_default().max(segment.max_seq));
    }
    Ok(newest)
}
