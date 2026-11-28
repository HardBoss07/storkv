//! Event mapping and filtering for watch streams.

use bytes::Bytes;
use storkv_core::watch::ChangeEvent;
use storkv_proto::WireChangeEvent;

use super::super::{WatchEvent, WatchEventKind};

pub(super) fn map_core_event(
    event: ChangeEvent,
    scope_id: u32,
    prefix: &[u8],
) -> Option<WatchEvent> {
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

pub(super) fn map_wire_event(
    event: WireChangeEvent,
    scope_id: u32,
    prefix: &[u8],
) -> Option<WatchEvent> {
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
