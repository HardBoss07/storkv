//! Core and watch wire types for protocol responses.

use bytes::Bytes;

use super::admin::{
    WireAdminStats, WireBackupEntry, WireBackupReport, WireBackupStatus, WireBackupVerifyReport,
    WireTelemetrySnapshot,
};
use crate::error::StorkvWireError;

/// Visible-version metadata of a key, used by migration and TTL tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WireKeyMeta {
    /// Absolute Unix epoch milliseconds, `0` when the key is permanent.
    pub expires_at_ms: u64,
    /// Sequence number of the visible version.
    pub seq: u64,
}

/// Identifier and name of a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireScopeHandle {
    /// Opaque scope identifier used by data requests.
    pub scope_id: u32,
    /// Unique scope name.
    pub name: String,
}

/// Change event pushed to a watch subscriber.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireChangeEvent {
    /// A key received a new value.
    Put {
        /// Identifier of the scope.
        scope_id: u32,
        /// User key that changed.
        key: Bytes,
        /// New value bytes.
        value: Bytes,
        /// Commit sequence number of the change.
        seq: u64,
        /// Absolute Unix epoch milliseconds, `0` when the key is permanent.
        expires_at_ms: u64,
    },
    /// A key was deleted.
    Delete {
        /// Identifier of the scope.
        scope_id: u32,
        /// User key that was deleted.
        key: Bytes,
        /// Commit sequence number of the change.
        seq: u64,
    },
    /// A scope was dropped.
    DropScope {
        /// Identifier of the dropped scope.
        scope_id: u32,
        /// Commit sequence number of the change.
        seq: u64,
    },
    /// A scope needs a full cache invalidation.
    ///
    /// The server sends this event after a range delete or a merge.
    InvalidateScope {
        /// Identifier of the affected scope.
        scope_id: u32,
        /// Commit sequence number of the change.
        seq: u64,
    },
    /// A key reached its time to live deadline and expired.
    Expired {
        /// Identifier of the scope.
        scope_id: u32,
        /// User key that expired.
        key: Bytes,
        /// Commit sequence number of the expiration.
        seq: u64,
    },
}

/// Outcome of an atomic conditional write.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WirePutIfResult {
    /// The condition matched. The key now has this sequence number.
    Applied {
        /// Sequence number of the committed write.
        seq: u64,
    },
    /// The condition did not match.
    Conflict {
        /// Sequence number of the visible version, or `None` when absent.
        current_seq: Option<u64>,
        /// Value of the visible version, or `None` when absent.
        current_value: Option<Bytes>,
    },
}

/// Response message in the StorKV native protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Response {
    /// Reply to [`crate::Request::Ping`].
    Pong,
    /// Generic success with no payload.
    Empty,
    /// Reply to [`crate::Request::TxBegin`] with the read sequence number.
    TxStarted {
        /// Sequence number that the transaction must read and validate.
        read_seq: u64,
    },
    /// Reply to a read. `None` means the key does not exist.
    Value(Option<Bytes>),
    /// Reply to a range read as ordered key/value pairs.
    Scan(Vec<(Bytes, Bytes)>),
    /// Reply to a scope creation.
    Scope(WireScopeHandle),
    /// Reply to a telemetry query.
    Telemetry(WireTelemetrySnapshot),
    /// Reply to a health check.
    HealthOk,
    /// One watch event pushed on a subscribed connection.
    WatchEvent(WireChangeEvent),
    /// Reply to an admin statistics request.
    AdminStats(WireAdminStats),
    /// Reply to a backup status request.
    BackupStatus(WireBackupStatus),
    /// Reply to a backup trigger.
    BackupReport(WireBackupReport),
    /// Reply to a backup verification request.
    BackupVerifyReport(WireBackupVerifyReport),
    /// Reply to a backup catalog request.
    BackupCatalog(Vec<WireBackupEntry>),
    /// Reply to a key metadata request. `None` means the key does not exist.
    KeyMeta(Option<WireKeyMeta>),
    /// Reply to an atomic conditional write.
    PutIf(WirePutIfResult),
    /// One replication frame pushed on a subscribed connection.
    ReplicationFrame {
        /// Unix epoch milliseconds when the primary sent the frame.
        sent_at_ms: u64,
        /// Encoded WAL batch frame bytes.
        frame: Bytes,
    },
    /// Base snapshot for a replica that missed the retained WAL history.
    ///
    /// `snapshot_seq` is the consistent sequence at which the primary captured
    /// the snapshot. Each entry carries `(scope_id, key, value, expires_at_ms)`
    /// where `expires_at_ms` is `0` for permanent keys.
    ReplicationSnapshot {
        /// The consistent sequence number at which the primary captured the snapshot.
        snapshot_seq: u64,
        /// Every live scope id as of `snapshot_seq`, including empty scopes.
        scopes: Vec<u32>,
        /// All live key/value pairs as of `snapshot_seq`.
        entries: Vec<(u32, Bytes, Bytes, u64)>,
    },
    /// Operation failure.
    Error(StorkvWireError),
}
