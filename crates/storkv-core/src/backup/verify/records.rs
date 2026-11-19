//! Format-level checks for one backed-up record file.

use std::path::Path;

use crate::manifest::{MANIFEST_MAGIC, VersionEdit};
use crate::sstable::SstReader;
use crate::vfs::{FileSystem, PosixFs};

use super::super::manifest::BackupFileKind;
use super::super::wal_index;
use super::{BackupVerifyReport, push_error};

const MANIFEST_HEADER_SIZE: usize = 16;

pub(super) fn verify_file_kind(path: &Path, kind: BackupFileKind, report: &mut BackupVerifyReport) {
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
