//! Replica task that streams WAL frames and snapshots from a primary.
//!
//! The task reconnects forever and applies each frame in order. It never
//! advances the applied sequence when the local engine rejects a frame.

use std::sync::Arc;
use std::time::Duration;

use storkv_client::{AsyncClient, ClientError};
use storkv_core::wal::WalBatchFrame;
use storkv_proto::{Response, StorkvWireError};
use tokio::task::JoinHandle;

use crate::Server;

const RECONNECT_DELAY: Duration = Duration::from_millis(100);

impl Server {
    /// Start the replica task when this node is a replica with a primary URI.
    ///
    /// The returned task is `None` for standalone and primary nodes. The task
    /// requires a running Tokio runtime.
    pub fn spawn_replica(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        let primary_uri = self.primary_uri()?.to_string();
        let server = self.clone();
        Some(tokio::spawn(async move {
            server.run_replica(primary_uri).await;
        }))
    }

    /// Connect to the primary and apply frames until the stream fails.
    pub async fn run_replica(&self, primary_uri: String) {
        let mut from_seq = self.last_applied_seq();
        loop {
            let client = match AsyncClient::connect_uri(&primary_uri).await {
                Ok(client) => client,
                Err(_) => {
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
            };
            let mut stream = match client.subscribe_replication_from(from_seq).await {
                Ok(stream) => stream,
                Err(ClientError::Server(StorkvWireError::WatchCursorExpired {
                    oldest_available,
                    ..
                })) => {
                    from_seq = oldest_available;
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
                Err(_) => {
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
            };
            while let Ok(Some(response)) = stream.next().await {
                match response {
                    Response::ReplicationFrame { sent_at_ms, frame } => {
                        from_seq = self.apply_frame(&frame, sent_at_ms);
                    }
                    Response::ReplicationSnapshot {
                        snapshot_seq,
                        scopes,
                        entries,
                    } => {
                        if self
                            .db()
                            .apply_replication_snapshot(snapshot_seq, &scopes, &entries)
                            .is_err()
                        {
                            continue;
                        }
                        self.last_applied_seq
                            .store(snapshot_seq, std::sync::atomic::Ordering::Relaxed);
                        from_seq = snapshot_seq;
                    }
                    _ => {}
                }
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    }

    fn apply_frame(&self, encoded: &[u8], sent_at_ms: u64) -> u64 {
        let Ok((frame, _)) = WalBatchFrame::decode(encoded) else {
            return self.last_applied_seq();
        };
        let applied = frame
            .batch_seq
            .saturating_add(frame.ops.len().saturating_sub(1) as u64);
        if self.db().apply_replication_frame(frame).is_err() {
            return self.last_applied_seq();
        }
        self.last_applied_seq
            .store(applied, std::sync::atomic::Ordering::Relaxed);
        self.last_frame_sent_at_ms
            .store(sent_at_ms, std::sync::atomic::Ordering::Relaxed);
        applied
    }
}
