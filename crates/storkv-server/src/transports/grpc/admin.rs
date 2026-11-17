//! `StorKvAdmin` gRPC implementation.

use storkv_proto::{Request, Response};
use tonic::{Request as GrpcRequest, Response as GrpcResponse, Status};

use super::conversions::telemetry_reply;
use super::generated::stor_kv_admin_server::StorKvAdmin;
use super::{
    BackupEntry, GrpcService, ListBackupsReply, ScopeRef, TelemetryReply, VerifyBackupReply,
    VerifyBackupRequest, not_found,
};

#[tonic::async_trait]
impl StorKvAdmin for GrpcService {
    async fn admin_stats(
        &self,
        request: GrpcRequest<super::Empty>,
    ) -> Result<GrpcResponse<TelemetryReply>, Status> {
        self.dispatch(request, Request::AdminStats)?;
        Ok(GrpcResponse::new(telemetry_reply(&self.server)))
    }

    async fn checkpoint(
        &self,
        request: GrpcRequest<super::generated::CheckpointRequest>,
    ) -> Result<GrpcResponse<super::Empty>, Status> {
        let payload = Request::Checkpoint {
            dest_dir: request.get_ref().dest_dir.clone(),
        };
        self.ok(request, payload)
    }

    async fn compact(
        &self,
        request: GrpcRequest<super::Empty>,
    ) -> Result<GrpcResponse<super::Empty>, Status> {
        self.ok(request, Request::Compact)
    }

    async fn create_snapshot(
        &self,
        request: GrpcRequest<ScopeRef>,
    ) -> Result<GrpcResponse<super::Empty>, Status> {
        let name = request.get_ref().name.clone();
        self.ok(request, Request::CreateSnapshot { name })
    }

    async fn unpin_snapshot(
        &self,
        request: GrpcRequest<ScopeRef>,
    ) -> Result<GrpcResponse<super::Empty>, Status> {
        let name = request.get_ref().name.clone();
        self.ok(request, Request::UnpinSnapshot { name })
    }

    async fn trigger_backup(
        &self,
        request: GrpcRequest<super::generated::BackupRequest>,
    ) -> Result<GrpcResponse<super::generated::BackupReply>, Status> {
        let wait = request.get_ref().wait;
        match self.dispatch(request, Request::TriggerBackup { wait })? {
            Response::BackupReport(report) => {
                Ok(GrpcResponse::new(super::generated::BackupReply {
                    generation_id: report.generation_id,
                    seq: report.sequence,
                }))
            }
            _ => Err(not_found()),
        }
    }

    async fn backup_status(
        &self,
        request: GrpcRequest<super::Empty>,
    ) -> Result<GrpcResponse<super::generated::BackupStatusReply>, Status> {
        match self.dispatch(request, Request::BackupStatus)? {
            Response::BackupStatus(status) => {
                Ok(GrpcResponse::new(super::generated::BackupStatusReply {
                    enabled: status.enabled,
                    last_backup_timestamp_ms: status.last_backup_timestamp_ms,
                    last_backup_seq: status.last_backup_seq,
                    backups_completed_total: status.backups_completed_total,
                    backups_failed_total: status.backups_failed_total,
                    wal_segments_shipped_total: status.wal_segments_shipped_total,
                    last_error: status.last_error.unwrap_or_default(),
                    backup_root: status.backup_root.unwrap_or_default(),
                    next_full_backup_in_ms: status.next_full_backup_in_ms,
                    sink: status.sink,
                }))
            }
            _ => Err(not_found()),
        }
    }

    async fn verify_backup(
        &self,
        request: GrpcRequest<VerifyBackupRequest>,
    ) -> Result<GrpcResponse<VerifyBackupReply>, Status> {
        let target_seq = {
            let message = request.get_ref();
            if message.has_target_seq {
                Some(message.target_seq)
            } else {
                None
            }
        };
        match self.dispatch(request, Request::VerifyBackup { target_seq })? {
            Response::BackupVerifyReport(report) => Ok(GrpcResponse::new(VerifyBackupReply {
                generations_checked: report.generations_checked,
                files_checked: report.files_checked,
                bytes_checked: report.bytes_checked,
                errors: report
                    .errors
                    .into_iter()
                    .map(|error| format!("{}: {}", error.path, error.detail))
                    .collect(),
            })),
            _ => Err(not_found()),
        }
    }

    async fn list_backups(
        &self,
        request: GrpcRequest<super::Empty>,
    ) -> Result<GrpcResponse<ListBackupsReply>, Status> {
        match self.dispatch(request, Request::ListBackups)? {
            Response::BackupCatalog(entries) => Ok(GrpcResponse::new(ListBackupsReply {
                entries: entries
                    .into_iter()
                    .map(|entry| BackupEntry {
                        generation_id: entry.generation_id,
                        kind: match entry.kind {
                            storkv_proto::WireBackupKind::Full => 1,
                            storkv_proto::WireBackupKind::Wal => 2,
                            _ => 0,
                        },
                        created_at_ms: entry.created_at_ms,
                        min_seq: entry.min_seq,
                        max_seq: entry.max_seq,
                        total_bytes: entry.total_bytes,
                        files: entry.files,
                        compression: entry.compression,
                    })
                    .collect(),
            })),
            _ => Err(not_found()),
        }
    }
}
