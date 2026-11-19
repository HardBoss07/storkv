use std::path::PathBuf;

use storkv_core::backup::{self, BackupKind, BackupReport, BackupStatus, BackupVerifyReport};
use storkv_proto::{
    Response, StorkvWireError, WireBackupEntry, WireBackupKind, WireBackupReport, WireBackupStatus,
    WireBackupVerifyReport,
};

use super::conversions::core_error_to_wire;
use crate::server::Server;

impl Server {
    pub(super) fn handle_trigger_backup(&self, wait: bool) -> Response {
        match self.db.trigger_backup(wait) {
            Ok(report) => Response::BackupReport(wire_backup_report(report)),
            Err(error) => Response::Error(core_error_to_wire(error)),
        }
    }

    pub(super) fn handle_backup_status(&self) -> Response {
        match self.db.backup_status() {
            Some(status) => Response::BackupStatus(wire_backup_status(status)),
            None => Response::BackupStatus(WireBackupStatus::default()),
        }
    }

    pub(super) fn handle_verify_backup(&self, target_seq: Option<u64>) -> Response {
        let Some(root) = self.backup_root() else {
            return not_configured();
        };
        match backup::verify_backup(&root, target_seq) {
            Ok(report) => Response::BackupVerifyReport(wire_verify_report(report)),
            Err(error) => Response::Error(core_error_to_wire(error)),
        }
    }

    pub(super) fn handle_list_backups(&self) -> Response {
        let Some(root) = self.backup_root() else {
            return not_configured();
        };
        match backup::load_catalog(&root) {
            Ok(catalog) => Response::BackupCatalog(catalog.iter().map(wire_backup_entry).collect()),
            Err(error) => Response::Error(core_error_to_wire(error)),
        }
    }

    fn backup_root(&self) -> Option<PathBuf> {
        self.db
            .backup_status()
            .and_then(|status| status.backup_root)
            .map(PathBuf::from)
    }
}

fn not_configured() -> Response {
    Response::Error(StorkvWireError::InvalidOperation(
        "auto-backup is not configured".into(),
    ))
}

fn wire_backup_report(report: BackupReport) -> WireBackupReport {
    WireBackupReport {
        generation_id: report.generation_id,
        sequence: report.sequence,
        files: report.files as u32,
        bytes: report.bytes,
        generations_removed: report.prune.generations_removed as u32,
        wal_segments_removed: report.prune.wal_segments_removed as u32,
        bytes_freed: report.prune.bytes_freed,
    }
}

fn wire_backup_status(status: BackupStatus) -> WireBackupStatus {
    WireBackupStatus {
        enabled: status.enabled,
        backup_root: status.backup_root,
        last_backup_timestamp_ms: status.last_backup_timestamp_ms,
        last_backup_seq: status.last_backup_seq,
        backups_completed_total: status.backups_completed_total,
        backups_failed_total: status.backups_failed_total,
        wal_segments_shipped_total: status.wal_segments_shipped_total,
        next_full_backup_in_ms: status.next_full_backup_in_ms,
        last_error: status.last_error,
        sink: status.sink,
    }
}

fn wire_verify_report(report: BackupVerifyReport) -> WireBackupVerifyReport {
    WireBackupVerifyReport {
        generations_checked: report.generations_checked as u32,
        files_checked: report.files_checked as u64,
        bytes_checked: report.bytes_checked,
        wal_segments_checked: report.wal_segments_checked as u64,
        errors: report
            .errors
            .into_iter()
            .map(|error| storkv_proto::WireBackupVerifyError {
                path: error.path,
                detail: error.detail,
            })
            .collect(),
    }
}

fn wire_backup_entry(manifest: &backup::BackupManifest) -> WireBackupEntry {
    WireBackupEntry {
        generation_id: manifest.generation_id.clone(),
        kind: match manifest.kind {
            BackupKind::Full => WireBackupKind::Full,
            BackupKind::Wal => WireBackupKind::Wal,
        },
        created_at_ms: manifest.created_at_ms,
        min_seq: manifest.min_seq,
        max_seq: manifest.max_seq,
        total_bytes: manifest.total_bytes,
        files: manifest.files.len() as u32,
        compression: manifest.compression.clone(),
    }
}
