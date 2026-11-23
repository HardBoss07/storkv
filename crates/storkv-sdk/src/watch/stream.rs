//! Subscription workers and event filtering for watch streams.

use std::time::Duration;

use bytes::Bytes;
use storkv_client::AsyncClient;
use storkv_core::Db;
use storkv_core::watch::ChangeEvent;
use storkv_proto::WireChangeEvent;
use tokio::sync::mpsc;

use crate::error::Result;

use super::{WatchEvent, WatchEventKind, WatchStream};

const WATCH_BUFFER: usize = 256;
const IDLE_POLL: Duration = Duration::from_millis(100);
const RECONNECT_MIN: Duration = Duration::from_millis(25);
const RECONNECT_MAX: Duration = Duration::from_secs(2);

pub(crate) fn spawn_embedded(
    db: Db,
    scope_id: u32,
    prefix: Bytes,
    from_seq: Option<u64>,
) -> Result<WatchStream> {
    let (sender, receiver) = mpsc::channel(WATCH_BUFFER);
    let events = match from_seq {
        Some(seq) => db.watch_from(seq)?,
        None => db.watch()?,
    };
    tokio::task::spawn_blocking(move || {
        loop {
            if sender.is_closed() {
                break;
            }
            match events.recv_timeout(IDLE_POLL) {
                Ok(event) => {
                    if let Some(mapped) = map_core_event(event, scope_id, &prefix)
                        && sender.blocking_send(Ok(mapped)).is_err()
                    {
                        break;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    Ok(WatchStream::new(receiver))
}

pub(crate) fn spawn_networked(
    uri: String,
    scope_id: u32,
    prefix: Bytes,
    from_seq: Option<u64>,
) -> WatchStream {
    let (sender, receiver) = mpsc::channel(WATCH_BUFFER);
    let (ready_sender, ready_receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let mut ready_sender = Some(ready_sender);
        let mut delay = RECONNECT_MIN;
        loop {
            if sender.is_closed() {
                return;
            }
            let client = match AsyncClient::connect_uri(&uri).await {
                Ok(client) => client,
                Err(_) => {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(RECONNECT_MAX);
                    continue;
                }
            };
            let mut stream = match client.subscribe_watch_from(from_seq).await {
                Ok(stream) => stream,
                Err(_) => {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(RECONNECT_MAX);
                    continue;
                }
            };
            if let Some(sender) = ready_sender.take() {
                let _ = sender.send(());
            }
            loop {
                match stream.next().await {
                    Ok(Some(event)) => {
                        if let Some(mapped) = map_wire_event(event, scope_id, &prefix)
                            && sender.send(Ok(mapped)).await.is_err()
                        {
                            return;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            if sender.is_closed() {
                return;
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(RECONNECT_MAX);
        }
    });
    WatchStream::with_ready(receiver, ready_receiver)
}

fn map_core_event(event: ChangeEvent, scope_id: u32, prefix: &[u8]) -> Option<WatchEvent> {
    match event {
        ChangeEvent::Put {
            value,
            key,
            seq,
            scope_id: event_scope,
            expires_at_ms,
        } => {
            if event_scope != scope_id || !key.starts_with(prefix) {
                return None;
            }
            Some(WatchEvent {
                kind: WatchEventKind::Put,
                scope_id: event_scope,
                seq,
                key,
                value: Some(value),
                expires_at_ms: (expires_at_ms != 0).then_some(expires_at_ms),
            })
        }
        ChangeEvent::Delete {
            key,
            seq,
            scope_id: event_scope,
        } => {
            if event_scope != scope_id || !key.starts_with(prefix) {
                return None;
            }
            Some(WatchEvent {
                kind: WatchEventKind::Delete,
                scope_id: event_scope,
                seq,
                key,
                value: None,
                expires_at_ms: None,
            })
        }
        ChangeEvent::DropScope {
            seq,
            scope_id: event_scope,
        } => {
            if event_scope != scope_id || !prefix.is_empty() {
                return None;
            }
            Some(WatchEvent {
                kind: WatchEventKind::DropScope,
                scope_id: event_scope,
                seq,
                key: Bytes::new(),
                value: None,
                expires_at_ms: None,
            })
        }
        ChangeEvent::Expired {
            key,
            seq,
            scope_id: event_scope,
        } => {
            if event_scope != scope_id || !key.starts_with(prefix) {
                return None;
            }
            Some(WatchEvent {
                kind: WatchEventKind::Expired,
                scope_id: event_scope,
                seq,
                key,
                value: None,
                expires_at_ms: None,
            })
        }
        _ => None,
    }
}

fn map_wire_event(event: WireChangeEvent, scope_id: u32, prefix: &[u8]) -> Option<WatchEvent> {
    match event {
        WireChangeEvent::Put {
            scope_id: event_scope,
            key,
            value,
            seq,
            expires_at_ms,
        } => {
            if event_scope != scope_id || !key.starts_with(prefix) {
                return None;
            }
            Some(WatchEvent {
                kind: WatchEventKind::Put,
                scope_id: event_scope,
                seq,
                key,
                value: Some(value),
                expires_at_ms: (expires_at_ms != 0).then_some(expires_at_ms),
            })
        }
        WireChangeEvent::Delete {
            scope_id: event_scope,
            key,
            seq,
        } => {
            if event_scope != scope_id || !key.starts_with(prefix) {
                return None;
            }
            Some(WatchEvent {
                kind: WatchEventKind::Delete,
                scope_id: event_scope,
                seq,
                key,
                value: None,
                expires_at_ms: None,
            })
        }
        WireChangeEvent::DropScope {
            scope_id: event_scope,
            seq,
        } => {
            if event_scope != scope_id || !prefix.is_empty() {
                return None;
            }
            Some(WatchEvent {
                kind: WatchEventKind::DropScope,
                scope_id: event_scope,
                seq,
                key: Bytes::new(),
                value: None,
                expires_at_ms: None,
            })
        }
        WireChangeEvent::Expired {
            scope_id: event_scope,
            key,
            seq,
        } => {
            if event_scope != scope_id || !key.starts_with(prefix) {
                return None;
            }
            Some(WatchEvent {
                kind: WatchEventKind::Expired,
                scope_id: event_scope,
                seq,
                key,
                value: None,
                expires_at_ms: None,
            })
        }
        _ => None,
    }
}
