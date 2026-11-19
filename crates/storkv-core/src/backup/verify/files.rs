//! Manifest and file checks for a backup generation.

use std::path::Path;

use crate::backup::manifest::{
    BackupManifest, MANIFEST_CRC_SUFFIX, MANIFEST_FILENAME, generation_directory,
    manifest_relative_path, parse_crc32c,
};
use crate::backup::wal_index;

use super::records::verify_file_kind;
use super::{BackupVerifyReport, push_error};

pub(super) fn verify_generation(
    backup_root: &Path,
    manifest: &BackupManifest,
    report: &mut BackupVerifyReport,
) {
    if let Err(error) = manifest.validate() {
        push_error(report, &manifest.generation_id, error.to_string());
        return;
    }
    let directory = match generation_directory(backup_root, &manifest.generation_id) {
        Ok(directory) => directory,
        Err(error) => {
            push_error(report, &manifest.generation_id, error.to_string());
            return;
        }
    };
    for entry in &manifest.files {
        let relative = match manifest_relative_path(&entry.path) {
            Ok(relative) => relative,
            Err(error) => {
                push_error(report, &entry.path, error.to_string());
                continue;
            }
        };
        let path = directory.join(relative);
        let path_label = path.display().to_string();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                push_error(report, &path_label, format!("missing file: {error}"));
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            push_error(report, &path_label, "file entry is a symbolic link");
            continue;
        }
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
        match crate::backup::manifest::crc32c_file(&path) {
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

pub(super) fn verify_external_wal(backup_root: &Path, report: &mut BackupVerifyReport) {
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
