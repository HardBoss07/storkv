//! Watch and replication streams for the asynchronous client.
//!
//! Both streams share the client backend mutex. Do not read two streams from
//! one client at the same time.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::backend::Backend;
use crate::transport::ClientResult;

use super::AsyncClient;

impl AsyncClient {
    /// Open a watch subscription on transports that support streaming.
    ///
    /// Native and WebSocket connections receive `WatchEvent` frames on the same
    /// connection. The returned stream decodes them until it is dropped.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Unsupported`] when the transport cannot stream.
    /// Returns [`crate::ClientError::Protocol`] when the server rejects the subscribe.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn subscribe_watch(&self) -> ClientResult<WatchStream> {
        self.subscribe_watch_from(None).await
    }

    /// Resume a watch subscription after the sequence `from_seq`.
    ///
    /// Retained server events after the cursor arrive before live events.
    /// Native, WebSocket, and gRPC transports support cursor replay.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Unsupported`] when the transport cannot stream.
    /// Returns [`crate::ClientError::Protocol`] when the server rejects the subscribe.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn subscribe_watch_from(&self, from_seq: Option<u64>) -> ClientResult<WatchStream> {
        let backend = self.backend.clone();
        let request_id = self.next_request_id();
        {
            let mut backend = backend.lock().await;
            backend.start_watch(request_id, from_seq).await?;
        }
        Ok(WatchStream { backend })
    }

    /// Start a replication frame stream after the sequence `from_seq`.
    ///
    /// Only the native transport supports replication.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Unsupported`] when the transport cannot replicate.
    /// Returns [`crate::ClientError::Protocol`] when the server rejects the start.
    /// Returns [`crate::ClientError::Io`] when the send fails.
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
}

/// Stream of change events pushed by a native or WebSocket server connection.
pub struct WatchStream {
    backend: Arc<Mutex<Backend>>,
}

impl WatchStream {
    /// Receive the next watch event.
    ///
    /// Yields `None` when the connection ends.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Protocol`] when a frame is malformed.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the read fails.
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
    /// Receive the next replication response.
    ///
    /// Yields `None` when the connection ends.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Protocol`] when a frame is malformed.
    /// Returns [`crate::ClientError::Io`] when the read fails.
    pub async fn next(&mut self) -> ClientResult<Option<storkv_proto::Response>> {
        let mut backend = self.backend.lock().await;
        backend.next_replication().await
    }
}
