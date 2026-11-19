use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use storkv_proto::{
    Consistency, Request, Response, WireAdminStats, WireBackupEntry, WireBackupReport,
    WireBackupStatus, WireBackupVerifyReport, WireTelemetrySnapshot,
};
use tokio::sync::Mutex;

use crate::backend::Backend;
use crate::transaction::AsyncTransaction;
use crate::transport::{ClientError, ClientResult, ScopeHandle};
use crate::uri::TransportKind;

mod scope_ops;

/// Asynchronous client for the StorKV protocol.
#[derive(Clone)]
pub struct AsyncClient {
    backend: Arc<Mutex<Backend>>,
    request_counter: Arc<AtomicU64>,
}

impl AsyncClient {
    /// Connect over TCP (`host:port`).
    pub async fn connect(address: &str) -> ClientResult<Self> {
        Self::from_backend(Backend::connect(TransportKind::NativeTcp, address).await?)
    }

    /// Connect to either a TCP endpoint or a Unix-domain socket path. Addresses
    /// using the `unix:` scheme or filesystem-like paths are treated as UDS.
    pub async fn connect_any(address: &str) -> ClientResult<Self> {
        let (kind, target) = TransportKind::parse_uri(address)?;
        match kind {
            TransportKind::NativeTcp | TransportKind::NativeUnix => {
                Self::from_backend(Backend::connect(kind, &target).await?)
            }
            other => Self::from_backend(Backend::connect(other, &target).await?),
        }
    }

    /// Connect using any supported scheme: `storkv://`, `unix://`, `http://`,
    /// `ws://`, or `grpc://`. A bare `host:port` remains native TCP.
    pub async fn connect_uri(uri: &str) -> ClientResult<Self> {
        let (kind, target) = TransportKind::parse_uri(uri)?;
        Self::from_backend(Backend::connect(kind, &target).await?)
    }

    fn from_backend(backend: Backend) -> ClientResult<Self> {
        Ok(Self {
            backend: Arc::new(Mutex::new(backend)),
            request_counter: Arc::new(AtomicU64::new(1)),
        })
    }

    fn next_request_id(&self) -> u64 {
        self.request_counter.fetch_add(1, Ordering::SeqCst)
    }

    /// Send one request and receive its response.
    pub async fn send_request(&self, request: Request) -> ClientResult<Response> {
        let request_id = self.next_request_id();
        let mut backend = self.backend.lock().await;
        backend.send(request_id, request).await
    }

    /// Open a watch subscription on transports that support streaming.
    ///
    /// Native and WebSocket connections receive `WatchEvent` frames on the same
    /// connection; the returned stream decodes them until it is dropped.
    pub async fn subscribe_watch(&self) -> ClientResult<WatchStream> {
        self.subscribe_watch_from(None).await
    }

    /// Resume a watch subscription after the sequence `from_seq`.
    ///
    /// Retained server events after the cursor arrive before live events.
    /// Native, WebSocket, and gRPC transports support cursor replay.
    pub async fn subscribe_watch_from(&self, from_seq: Option<u64>) -> ClientResult<WatchStream> {
        let backend = self.backend.clone();
        let request_id = self.next_request_id();
        {
            let mut backend = backend.lock().await;
            backend.start_watch(request_id, from_seq).await?;
        }
        Ok(WatchStream { backend })
    }

    /// Send a ping and require a pong reply.
    pub async fn ping(&self) -> ClientResult<()> {
        expect_variant(self.send_request(Request::Ping).await?, Response::Pong)
    }

    /// Select the consistency level for later data operations on this server.
    pub async fn set_consistency(&self, consistency: Consistency) -> ClientResult<()> {
        expect_variant(
            self.send_request(Request::ConfigureConsistency { consistency })
                .await?,
            Response::Empty,
        )
    }

    /// Start a replication frame stream after the sequence `from_seq`.
    ///
    /// Only the native transport supports replication.
    pub async fn subscribe_replication_from(
        &self,
        from_seq: u64,
    ) -> ClientResult<ReplicationStream> {
        let backend = self.backend.clone();
        let request_id = self.next_request_id();
        {
            let mut backend = backend.lock().await;
            backend.start_replication(request_id, from_seq).await?;
        }
        Ok(ReplicationStream { backend })
    }

    /// Run a health check and require a healthy reply.
    pub async fn health_check(&self) -> ClientResult<()> {
        expect_variant(
            self.send_request(Request::HealthCheck).await?,
            Response::HealthOk,
        )
    }

    /// Read runtime counters.
    pub async fn telemetry(&self) -> ClientResult<WireTelemetrySnapshot> {
        match self.send_request(Request::TelemetryQuery).await? {
            Response::Telemetry(snapshot) => Ok(snapshot),
            other => Err(unexpected_response(other)),
        }
    }

    /// Fetch admin statistics (L0..L6 SSTable distribution, per-scope counters,
    /// and observed WAL commit latency percentile).
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

    /// Run a full backup now. With `wait == false` the server only wakes its
    /// background worker and returns an empty report.
    pub async fn trigger_backup(&self, wait: bool) -> ClientResult<WireBackupReport> {
        match self.send_request(Request::TriggerBackup { wait }).await? {
            Response::BackupReport(report) => Ok(report),
            other => Err(unexpected_response(other)),
        }
    }

    /// Fetch the live autonomous backup status, if configured.
    /// Fetch the live autonomous backup status, if configured.
    pub async fn backup_status(&self) -> ClientResult<WireBackupStatus> {
        match self.send_request(Request::BackupStatus).await? {
            Response::BackupStatus(status) => Ok(status),
            other => Err(unexpected_response(other)),
        }
    }

    /// Verify archived backup generations, optionally as of `target_seq`.
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
    pub async fn list_backups(&self) -> ClientResult<Vec<WireBackupEntry>> {
        match self.send_request(Request::ListBackups).await? {
            Response::BackupCatalog(entries) => Ok(entries),
            other => Err(unexpected_response(other)),
        }
    }

    /// Fetch the visible-version metadata (absolute expiry and sequence) of a
    /// key. `None` means the key does not exist.
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

/// Stream of change events pushed by a native or WebSocket server connection.
pub struct WatchStream {
    backend: Arc<Mutex<Backend>>,
}

impl WatchStream {
    /// Receive the next watch event. Yields `None` when the connection ends.
    pub async fn next(&mut self) -> ClientResult<Option<storkv_proto::WireChangeEvent>> {
        let mut backend = self.backend.lock().await;
        backend.next_watch_event().await
    }
}

/// Stream of replication frames pushed by a primary server.
pub struct ReplicationStream {
    backend: Arc<Mutex<Backend>>,
}

impl ReplicationStream {
    /// Receive the next replication response. Yields `None` when the
    /// connection ends.
    pub async fn next(&mut self) -> ClientResult<Option<Response>> {
        let mut backend = self.backend.lock().await;
        backend.next_replication().await
    }
}

pub(super) fn expect_empty(response: Response) -> ClientResult<()> {
    expect_variant(response, Response::Empty)
}

pub(super) fn expect_variant(response: Response, expected: Response) -> ClientResult<()> {
    if response == expected {
        Ok(())
    } else {
        Err(unexpected_response(response))
    }
}

pub(super) fn unexpected_response(response: Response) -> ClientError {
    ClientError::Protocol(format!("Unexpected response: {:?}", response))
}
