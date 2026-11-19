//! Admin-plane request handling for the gRPC backend.

use storkv_proto::{
    Request, Response, WireBackupEntry, WireBackupKind, WireBackupReport, WireBackupStatus,
    WireBackupVerifyError, WireBackupVerifyReport,
};

use super::GrpcBackend;
use super::telemetry;
use crate::grpc::{self, Empty, ScopeRef};
use crate::transport::{ClientError, ClientResult};

impl GrpcBackend {
    pub(super) async fn handle_admin(&mut self, request: Request) -> ClientResult<Response> {
        let response = match request {
            Request::CreateSnapshot { name } => {
                self.admin
                    .create_snapshot(ScopeRef { scope_id: 0, name })
                    .await?;
                Response::Empty
            }
            Request::UnpinSnapshot { name } => {
                self.admin
                    .unpin_snapshot(ScopeRef { scope_id: 0, name })
                    .await?;
                Response::Empty
            }
            Request::Checkpoint { dest_dir } => {
                self.admin
                    .checkpoint(grpc::CheckpointRequest { dest_dir })
                    .await?;
                Response::Empty
            }
            Request::Compact => {
                self.admin.compact(Empty {}).await?;
                Response::Empty
            }
            Request::AdminStats => {
                let reply = self.admin.admin_stats(Empty {}).await?.into_inner();
                Response::Telemetry(telemetry(reply))
            }
            Request::TriggerBackup { wait } => {
                let reply = self
                    .admin
                    .trigger_backup(grpc::BackupRequest { wait })
                    .await?
                    .into_inner();
                Response::BackupReport(WireBackupReport {
                    generation_id: reply.generation_id,
                    sequence: reply.seq,
                    ..WireBackupReport::default()
                })
            }
            Request::BackupStatus => {
                let reply = self.admin.backup_status(Empty {}).await?.into_inner();
                Response::BackupStatus(WireBackupStatus {
                    enabled: reply.enabled,
                    last_backup_timestamp_ms: reply.last_backup_timestamp_ms,
                    last_backup_seq: reply.last_backup_seq,
                    backups_completed_total: reply.backups_completed_total,
                    backups_failed_total: reply.backups_failed_total,
                    wal_segments_shipped_total: reply.wal_segments_shipped_total,
                    last_error: (!reply.last_error.is_empty()).then_some(reply.last_error),
                    backup_root: (!reply.backup_root.is_empty()).then_some(reply.backup_root),
                    next_full_backup_in_ms: reply.next_full_backup_in_ms,
                    sink: reply.sink,
                })
            }
            Request::VerifyBackup { target_seq } => {
                let reply = self
                    .admin
                    .verify_backup(grpc::VerifyBackupRequest {
                        has_target_seq: target_seq.is_some(),
                        target_seq: target_seq.unwrap_or(0),
                    })
                    .await?
                    .into_inner();
                Response::BackupVerifyReport(WireBackupVerifyReport {
                    generations_checked: reply.generations_checked,
                    files_checked: reply.files_checked,
                    bytes_checked: reply.bytes_checked,
                    wal_segments_checked: 0,
                    errors: reply
                        .errors
                        .into_iter()
                        .map(|detail| WireBackupVerifyError {
                            path: String::new(),
                            detail,
                        })
                        .collect(),
                })
            }
            Request::ListBackups => {
                let reply = self.admin.list_backups(Empty {}).await?.into_inner();
                Response::BackupCatalog(
                    reply
                        .entries
                        .into_iter()
                        .map(|entry| WireBackupEntry {
                            generation_id: entry.generation_id,
                            kind: match entry.kind {
                                2 => WireBackupKind::Wal,
                                _ => WireBackupKind::Full,
                            },
                            created_at_ms: entry.created_at_ms,
                            min_seq: entry.min_seq,
                            max_seq: entry.max_seq,
                            total_bytes: entry.total_bytes,
                            files: entry.files,
                            compression: entry.compression,
                        })
                        .collect(),
                )
            }
            _ => {
                return Err(ClientError::Unsupported(
                    "request is not supported over gRPC".into(),
                ));
            }
        };
        Ok(response)
    }
}
