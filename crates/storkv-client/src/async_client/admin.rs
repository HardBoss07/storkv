//! Admin and metadata calls for the asynchronous client.
//!
//! Each method maps one request to one expected response variant.

use storkv_proto::{
    Request, Response, WireAdminStats, WireBackupEntry, WireBackupReport, WireBackupStatus,
    WireBackupVerifyReport, WireTelemetrySnapshot,
};

use crate::transaction::AsyncTransaction;
use crate::transport::{ClientError, ClientResult, ScopeHandle};

use super::{AsyncClient, expect_variant, unexpected_response};

impl AsyncClient {
    /// Run a health check and require a healthy reply.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Protocol` when the reply is not a health status.
    /// Returns `ClientError::Io` or `ClientError::Http` when the send fails.
    pub async fn health_check(&self) -> ClientResult<()> {
        expect_variant(
            self.send_request(Request::HealthCheck).await?,
            Response::HealthOk,
        )
    }

    /// Read runtime counters.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Protocol` when the reply is not a telemetry snapshot.
    /// Returns `ClientError::Io` or `ClientError::Http` when the send fails.
    pub async fn telemetry(&self) -> ClientResult<WireTelemetrySnapshot> {
        match self.send_request(Request::TelemetryQuery).await? {
            Response::Telemetry(snapshot) => Ok(snapshot),
            other => Err(unexpected_response(other)),
        }
    }

    /// Fetch admin statistics.
    ///
    /// The reply holds the L0..L6 SSTable distribution, per-scope counters,
    /// and the observed WAL commit latency percentiles.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Protocol` when the reply is not an admin snapshot.
    /// Returns `ClientError::Io` or `ClientError::Http` when the send fails.
    pub async fn admin_stats(&self) -> ClientResult<WireAdminStats> {
        match self.send_request(Request::AdminStats).await? {
            Response::AdminStats(stats) => Ok(stats),
            Response::Telemetry(snapshot) => Ok(WireAdminStats {
                backups_completed_total: snapshot.backups_completed_total,
                backups_failed_total: snapshot.backups_failed_total,
                last_backup_timestamp_ms: snapshot.last_backup_timestamp_ms,
                last_backup_seq: snapshot.last_backup_seq,
                wal_segments_shipped_total: snapshot.wal_segments_shipped_total,
                ..WireAdminStats::default()
            }),
            other => Err(unexpected_response(other)),
        }
    }

    /// Run a full backup now.
    ///
    /// With `wait == false` the server only wakes its background worker and
    /// returns an empty report.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Server` when the backup fails.
    /// Returns `ClientError::Protocol` when the reply is not a backup report.
    /// Returns `ClientError::Io` or `ClientError::Http` when the send fails.
    pub async fn trigger_backup(&self, wait: bool) -> ClientResult<WireBackupReport> {
        match self.send_request(Request::TriggerBackup { wait }).await? {
            Response::BackupReport(report) => Ok(report),
            other => Err(unexpected_response(other)),
        }
    }

    /// Fetch the live autonomous backup status, if configured.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Protocol` when the reply is not a backup status.
    /// Returns `ClientError::Io` or `ClientError::Http` when the send fails.
    pub async fn backup_status(&self) -> ClientResult<WireBackupStatus> {
        match self.send_request(Request::BackupStatus).await? {
            Response::BackupStatus(status) => Ok(status),
            other => Err(unexpected_response(other)),
        }
    }

    /// Verify archived backup generations, optionally as of `target_seq`.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Server` when the verification runs and fails.
    /// Returns `ClientError::Protocol` when the reply is not a verify report.
    /// Returns `ClientError::Io` or `ClientError::Http` when the send fails.
    pub async fn verify_backup(
        &self,
        target_seq: Option<u64>,
    ) -> ClientResult<WireBackupVerifyReport> {
        match self
            .send_request(Request::VerifyBackup { target_seq })
            .await?
        {
            Response::BackupVerifyReport(report) => Ok(report),
            other => Err(unexpected_response(other)),
        }
    }

    /// List archived backup generations from oldest to newest.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Protocol` when the reply is not a backup catalog.
    /// Returns `ClientError::Io` or `ClientError::Http` when the send fails.
    pub async fn list_backups(&self) -> ClientResult<Vec<WireBackupEntry>> {
        match self.send_request(Request::ListBackups).await? {
            Response::BackupCatalog(entries) => Ok(entries),
            other => Err(unexpected_response(other)),
        }
    }

    /// Fetch the visible-version metadata of a key.
    ///
    /// The reply holds the absolute expiry and the sequence number. `None`
    /// means the key does not exist.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Protocol` when the reply is not key metadata.
    /// Returns `ClientError::Io` or `ClientError::Http` when the send fails.
    pub async fn key_meta(
        &self,
        scope: &ScopeHandle,
        key: impl AsRef<[u8]>,
    ) -> ClientResult<Option<storkv_proto::WireKeyMeta>> {
        match self
            .send_request(Request::KeyMeta {
                scope_id: scope.scope_id,
                key: bytes::Bytes::copy_from_slice(key.as_ref()),
            })
            .await?
        {
            Response::KeyMeta(meta) => Ok(meta),
            other => Err(unexpected_response(other)),
        }
    }

    /// Begin a transaction and receive its read sequence number.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Protocol` when the reply is not a transaction start.
    /// Returns `ClientError::Io` or `ClientError::Http` when the send fails.
    pub async fn begin_tx(&self) -> ClientResult<AsyncTransaction> {
        let read_sequence = match self.send_request(Request::TxBegin).await? {
            Response::TxStarted { read_seq } => read_seq,
            other => {
                return Err(ClientError::Protocol(format!(
                    "Unexpected response for TxBegin: {:?}",
                    other
                )));
            }
        };

        Ok(AsyncTransaction::new(self.clone(), read_sequence))
    }
}
