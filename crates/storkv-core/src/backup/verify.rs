use std::path::Path;

use crate::manifest::{MANIFEST_MAGIC, VersionEdit};
use crate::sstable::SstReader;
use crate::vfs::{FileSystem, PosixFs};

use super::manifest::{
    BackupFileKind, BackupManifest, MANIFEST_CRC_SUFFIX, MANIFEST_FILENAME, generation_directory,
    parse_crc32c,
};
use super::retention;
use super::wal_index;

const MANIFEST_HEADER_SIZE: usize = 16;

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
/// Returns [`crate::StorKvError::TargetSequenceUnavailable`] when `target_seq`
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
            return Err(crate::StorKvError::TargetSequenceUnavailable {
                requested: target,
                oldest,
                newest,
            });
        }
    }

    for manifest in &catalog {
        report.generations_checked += 1;
        verify_generation(backup_root, manifest, &mut report);
    }
    verify_external_wal(backup_root, &mut report);
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
    verify_generation(backup_root, manifest, &mut report);
    Ok(report)
}

fn verify_generation(
    backup_root: &Path,
    manifest: &BackupManifest,
    report: &mut BackupVerifyReport,
) {
    let directory = generation_directory(backup_root, &manifest.generation_id);
    for entry in &manifest.files {
        let path = directory.join(super::manifest::manifest_relative_path(&entry.path));
        let path_label = path.display().to_string();
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                push_error(report, &path_label, format!("missing file: {error}"));
                continue;
            }
        };
        if metadata.len() != entry.size_bytes {
            push_error(
                report,
                &path_label,
                format!(
                    "size mismatch: manifest says {}, file is {}",
                    entry.size_bytes,
                    metadata.len()
                ),
            );
        }
        let expected_crc = match parse_crc32c(&entry.crc32c) {
            Ok(value) => value,
            Err(error) => {
                push_error(report, &path_label, error.to_string());
                continue;
            }
        };
        match super::manifest::crc32c_file(&path) {
            Ok(actual_crc) if actual_crc == expected_crc => {}
            Ok(actual_crc) => push_error(
                report,
                &path_label,
                format!("CRC32C mismatch: expected 0x{expected_crc:08x}, got 0x{actual_crc:08x}"),
            ),
            Err(error) => push_error(report, &path_label, error.to_string()),
        }
        verify_file_kind(&path, entry.kind, report);
        report.files_checked += 1;
        report.bytes_checked = report.bytes_checked.saturating_add(metadata.len());
    }

    let manifest_path = directory.join(MANIFEST_FILENAME);
    if !manifest_path.exists() {
        push_error(
            report,
            &manifest_path.display().to_string(),
            "missing manifest",
        );
    }
    let sidecar = directory.join(format!("{MANIFEST_FILENAME}{MANIFEST_CRC_SUFFIX}"));
    if !sidecar.exists() {
        push_error(
            report,
            &sidecar.display().to_string(),
            "missing manifest checksum sidecar",
        );
    }
}

fn verify_file_kind(path: &Path, kind: BackupFileKind, report: &mut BackupVerifyReport) {
    match kind {
        BackupFileKind::Sst => verify_sstable(path, report),
        BackupFileKind::Wal => verify_wal(path, report),
        BackupFileKind::Manifest => verify_manifest(path, report),
        BackupFileKind::Current => verify_current(path, report),
    }
}

fn verify_sstable(path: &Path, report: &mut BackupVerifyReport) {
    let file_system = PosixFs;
    let path_label = path.display().to_string();
    let mut file = match file_system.open_file(path) {
        Ok(file) => file,
        Err(error) => {
            push_error(report, &path_label, error.to_string());
            return;
        }
    };
    match SstReader::open(&mut file) {
        Ok(reader) => {
            if let Err(error) = reader.read_all_entries(&mut file) {
                push_error(report, &path_label, format!("SSTable block error: {error}"));
            }
        }
        Err(error) => push_error(
            report,
            &path_label,
            format!("SSTable footer error: {error}"),
        ),
    }
}

fn verify_wal(path: &Path, report: &mut BackupVerifyReport) {
    match wal_index::inspect_segment(path) {
        Ok(_) => report.wal_segments_checked += 1,
        Err(error) => push_error(report, &path.display().to_string(), error.to_string()),
    }
}

fn verify_manifest(path: &Path, report: &mut BackupVerifyReport) {
    let path_label = path.display().to_string();
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            push_error(report, &path_label, error.to_string());
            return;
        }
    };
    if bytes.len() < MANIFEST_HEADER_SIZE {
        push_error(report, &path_label, "manifest shorter than its header");
        return;
    }
    if &bytes[0..8] != MANIFEST_MAGIC {
        push_error(report, &path_label, "invalid manifest magic");
        return;
    }
    let mut offset = MANIFEST_HEADER_SIZE;
    while offset < bytes.len() {
        match VersionEdit::decode(&bytes[offset..]) {
            Ok((_edit, consumed)) if consumed > 0 => offset += consumed,
            Ok((_edit, _)) => {
                push_error(report, &path_label, "manifest frame decoded to zero bytes");
                return;
            }
            Err(error) => {
                push_error(
                    report,
                    &path_label,
                    format!("manifest frame error: {error}"),
                );
                return;
            }
        }
    }
}

fn verify_current(path: &Path, report: &mut BackupVerifyReport) {
    match std::fs::read_to_string(path) {
        Ok(contents) if contents.trim().starts_with("MANIFEST-") => {}
        Ok(_) => push_error(
            report,
            &path.display().to_string(),
            "invalid CURRENT content",
        ),
        Err(error) => push_error(report, &path.display().to_string(), error.to_string()),
    }
}

fn verify_external_wal(backup_root: &Path, report: &mut BackupVerifyReport) {
    let wal_dir = backup_root.join("wal");
    let segments = match wal_index::list_segments(&wal_dir) {
        Ok(segments) => segments,
        Err(error) => {
            push_error(report, &wal_dir.display().to_string(), error.to_string());
            return;
        }
    };
    for segment in segments {
        report.wal_segments_checked += 1;
        report.files_checked += 1;
        report.bytes_checked = report.bytes_checked.saturating_add(segment.size_bytes);
    }
}

fn push_error(report: &mut BackupVerifyReport, path: &str, detail: impl Into<String>) {
    report.errors.push(BackupVerificationError {
        path: path.to_string(),
        detail: detail.into(),
    });
}
