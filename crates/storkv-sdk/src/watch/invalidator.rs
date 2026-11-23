//! Cache invalidation workers that follow backend watch events.

use std::time::Duration;

use storkv_client::{AsyncClient, ClientError};
use storkv_core::Db;
use storkv_proto::{StorkvWireError, WireChangeEvent};

use crate::cache::ScopeCache;

const WATCH_IDLE_POLL: Duration = Duration::from_millis(200);
const RECONNECT_DELAY: Duration = Duration::from_millis(250);

pub(crate) fn spawn_embedded_cache_watch(
    db: Db,
    scope_id: u32,
    cache: std::sync::Weak<ScopeCache>,
) {
    tokio::task::spawn_blocking(move || {
        let mut next_seq: u64 = 0;
        loop {
            let Some(current) = cache.upgrade() else {
                break;
            };
            let receiver = match db.inner().watch_scope_from(scope_id, next_seq) {
                Ok(receiver) => receiver,
                Err(_) => {
                    current.clear();
                    match db.watch() {
                        Ok(receiver) => receiver,
                        Err(_) => break,
                    }
                }
            };
            drop(current);
            loop {
                match receiver.recv_timeout(WATCH_IDLE_POLL) {
                    Ok(event) => {
                        next_seq = event.seq().saturating_add(1);
                        if let Some(current) = cache.upgrade() {
                            current.apply_event(&event, scope_id);
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if cache.upgrade().is_none() {
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        if let Some(current) = cache.upgrade() {
                            current.clear();
                        }
                        break;
                    }
                }
            }
        }
    });
}

pub(crate) fn spawn_networked_cache_watch(
    uri: String,
    scope_id: u32,
    cache: std::sync::Weak<ScopeCache>,
) {
    tokio::spawn(async move {
        let mut next_seq: u64 = 0;
        let mut cleared_for_expiry = false;
        loop {
            if cache.upgrade().is_none() {
                return;
            }
            let client = match AsyncClient::connect_uri(&uri).await {
                Ok(client) => client,
                Err(_) => {
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
            };
            let mut stream = match client.subscribe_watch_from(Some(next_seq)).await {
                Ok(stream) => stream,
                Err(ClientError::Server(StorkvWireError::WatchCursorExpired { .. })) => {
                    next_seq = 0;
                    if !cleared_for_expiry {
                        if let Some(current) = cache.upgrade() {
                            current.clear();
                        }
                        cleared_for_expiry = true;
                    }
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
                Err(_) => {
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
            };
            cleared_for_expiry = false;
            while let Ok(Some(event)) = stream.next().await {
                next_seq = wire_seq(&event).saturating_add(1);
                let Some(current) = cache.upgrade() else {
                    return;
                };
                apply_wire_event(&current, &event, scope_id);
            }
            if cache.upgrade().is_none() {
                return;
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    });
}

fn wire_seq(event: &WireChangeEvent) -> u64 {
    match event {
        WireChangeEvent::Put { seq, .. }
        | WireChangeEvent::Delete { seq, .. }
        | WireChangeEvent::DropScope { seq, .. }
        | WireChangeEvent::InvalidateScope { seq, .. }
        | WireChangeEvent::Expired { seq, .. } => *seq,
        _ => 0,
    }
}

fn apply_wire_event(cache: &ScopeCache, event: &WireChangeEvent, scope_id: u32) {
    match event {
        WireChangeEvent::Put {
            scope_id: event_scope,
            key,
            value,
            expires_at_ms,
            ..
        } if *event_scope == scope_id => {
            cache.store_event(key, Some(value.clone()), *expires_at_ms)
        }
        WireChangeEvent::Delete {
            scope_id: event_scope,
            key,
            ..
        }
        | WireChangeEvent::Expired {
            scope_id: event_scope,
            key,
            ..
        } if *event_scope == scope_id => cache.store(key, None),
        WireChangeEvent::DropScope {
            scope_id: event_scope,
            ..
        }
        | WireChangeEvent::InvalidateScope {
            scope_id: event_scope,
            ..
        } if *event_scope == scope_id => cache.clear(),
        _ => {}
    }
}
