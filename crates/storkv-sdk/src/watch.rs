//! Watch event types and the change stream.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;
use tokio::sync::mpsc;

use crate::error::Result;

mod invalidator;
mod stream;

pub(crate) use invalidator::{spawn_embedded_cache_watch, spawn_networked_cache_watch};
pub(crate) use stream::{spawn_embedded, spawn_networked};

/// Kind of change carried by a [`WatchEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEventKind {
    /// A key received a new value.
    Put,
    /// A delete event for one key.
    Delete,
    /// A drop event for one scope.
    DropScope,
    /// A key reached its time to live deadline and expired.
    Expired,
}

/// One change delivered by a watch stream.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    /// Identifier of the scope that changed.
    pub scope_id: u32,
    /// Commit sequence number of the change.
    pub seq: u64,
    /// Kind of change.
    pub kind: WatchEventKind,
    /// User key that changed.
    pub key: Bytes,
    /// New value. Present only for [`WatchEventKind::Put`].
    pub value: Option<Bytes>,
    /// Absolute Unix epoch milliseconds, `None` for permanent keys or
    /// non-put events.
    pub expires_at_ms: Option<u64>,
}

impl WatchEvent {
    /// Return `true` when the event carries a new value.
    #[must_use]
    pub fn is_put(&self) -> bool {
        matches!(self.kind, WatchEventKind::Put)
    }

    /// Return `true` when the event reports a delete.
    #[must_use]
    pub fn is_delete(&self) -> bool {
        matches!(self.kind, WatchEventKind::Delete)
    }
}

/// Stream of change events for one scope or key prefix.
pub struct WatchStream {
    receiver: mpsc::Receiver<Result<WatchEvent>>,
    ready: Option<tokio::sync::oneshot::Receiver<()>>,
}

impl WatchStream {
    pub(crate) fn new(receiver: mpsc::Receiver<Result<WatchEvent>>) -> Self {
        Self {
            receiver,
            ready: None,
        }
    }

    pub(crate) fn with_ready(
        receiver: mpsc::Receiver<Result<WatchEvent>>,
        ready: tokio::sync::oneshot::Receiver<()>,
    ) -> Self {
        Self {
            receiver,
            ready: Some(ready),
        }
    }

    /// Wait until the underlying subscription is ready.
    ///
    /// Networked streams only observe changes written after this completes.
    pub(crate) async fn wait_ready(&mut self) {
        if let Some(ready) = self.ready.take() {
            let _ = ready.await;
        }
    }

    /// Receive the next event.
    ///
    /// # Errors
    ///
    /// Returns `Some(Err(..))` when the backend reports a watch failure.
    pub async fn next(&mut self) -> Option<Result<WatchEvent>> {
        self.wait_ready().await;
        self.receiver.recv().await
    }
}

impl Stream for WatchStream {
    type Item = Result<WatchEvent>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(ready) = self.ready.as_mut()
            && Pin::new(ready).poll(context).is_pending()
        {
            return Poll::Pending;
        }
        self.ready.take();
        self.receiver.poll_recv(context)
    }
}
