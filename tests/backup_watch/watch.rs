use std::time::Duration;

use bytes::Bytes;
use storkv_core::watch::{ChangeEvent, MAX_REPLAY_EVENTS, WATCH_CHANNEL_CAPACITY, WatchRegistry};
use storkv_core::{Durability, ScopeOptions};

#[test]
fn test_scoped_cdc_watch_stream() {
    let mut temp = crate::common::TempDb::new("watch_stream");
    let db = temp.db();

    let scope = db
        .create_scope("watch_scope", ScopeOptions::default())
        .unwrap();

    let rx = db.watch().unwrap();

    db.put(&scope, b"watch_key", b"watch_val").unwrap();

    let event = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("Failed to receive watch event");

    match event {
        ChangeEvent::Put {
            scope_id,
            key,
            value,
            ..
        } => {
            assert_eq!(scope_id, scope.scope_id);
            assert_eq!(key.as_ref(), b"watch_key");
            assert_eq!(value.as_ref(), b"watch_val");
        }
        _ => panic!("Expected Put change event"),
    }

    temp.close();
}

#[test]
fn test_watch_from_replays_history_then_live_events() {
    let mut temp = crate::common::TempDb::new("watch_replay");
    let db = temp.db();
    let scope = db
        .create_scope("replay_scope", ScopeOptions::default())
        .unwrap();

    db.put(&scope, b"first", b"1").unwrap();
    db.put(&scope, b"second", b"2").unwrap();

    let rx = db.watch_from(0).unwrap();
    let first = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let second = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(first.seq(), 1);
    assert_eq!(second.seq(), 2);
    assert!(first.seq() < second.seq(), "replay must stay ordered");

    db.put(&scope, b"third", b"3").unwrap();
    let third = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(third.seq(), 3, "live events must follow the replay");
    assert_eq!(third.scope_id(), scope.scope_id);

    temp.close();
}

#[test]
fn test_watch_scope_from_filters_other_scopes() {
    let mut temp = crate::common::TempDb::new("watch_scope_replay");
    let db = temp.db();
    let first = db
        .create_scope("first_scope", ScopeOptions::default())
        .unwrap();
    let second = db
        .create_scope("second_scope", ScopeOptions::default())
        .unwrap();

    db.put(&first, b"a", b"1").unwrap();
    db.put(&second, b"b", b"2").unwrap();

    let rx = db.watch_scope_from(&first, 0).unwrap();
    let event = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(event.scope_id(), first.scope_id);
    assert!(
        rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "events from other scopes must not appear"
    );

    temp.close();
}

#[test]
fn test_watch_from_reports_expired_cursor_without_wal_history() {
    let mut temp = crate::common::TempDb::new("watch_expired");
    let scope_id;
    {
        let db = temp.db();
        let scope = db
            .create_scope("expired_scope", ScopeOptions::default())
            .unwrap();
        scope_id = scope.scope_id;
        db.put(&scope, b"key", b"value").unwrap();
        db.flush().unwrap();
    }
    temp.close();

    let wal_dir = temp.path.join("wal");
    std::fs::remove_dir_all(&wal_dir).unwrap();

    let db = storkv_core::Db::open(&temp.path, storkv_core::DbOptions::default()).unwrap();
    let error = db
        .watch_from(0)
        .expect_err("missing WAL history must expire");
    match error {
        storkv_core::StorkvError::WatchCursorExpired {
            requested,
            oldest_available,
        } => {
            assert_eq!(requested, 0);
            assert!(oldest_available > 0);
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let handle = storkv_core::ScopeHandle {
        scope_id,
        name: "expired_scope".into(),
    };
    let _ = db.watch_scope_from(&handle, 0);
    temp.close();
}

#[test]
fn test_slow_watch_subscriber_is_disconnected_without_stalling_broadcast() {
    let registry = WatchRegistry::new();
    let receiver = registry.subscribe().expect("subscribe watch");

    for seq in 0..WATCH_CHANNEL_CAPACITY as u64 {
        registry.broadcast(ChangeEvent::Delete {
            key: Bytes::from_static(b"slow"),
            seq,
            scope_id: 1,
        });
    }

    let started = std::time::Instant::now();
    registry.broadcast(ChangeEvent::Delete {
        key: Bytes::from_static(b"overflow"),
        seq: WATCH_CHANNEL_CAPACITY as u64,
        scope_id: 1,
    });
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a full subscriber queue must not block the writer"
    );

    let mut received = 0;
    while receiver.recv_timeout(Duration::from_millis(100)).is_ok() {
        received += 1;
    }
    assert_eq!(
        received, WATCH_CHANNEL_CAPACITY,
        "the slow subscriber must observe its queued budget and then a disconnect"
    );
}

#[test]
fn test_range_delete_and_merge_emit_scope_invalidation() {
    let mut temp = crate::common::TempDb::new("watch_invalidate_scope");
    let db = temp.db();
    let scope = db
        .create_scope("invalidate_scope", ScopeOptions::default())
        .unwrap();
    let receiver = db.watch().unwrap();

    db.delete_range(&scope, b"a", b"z").unwrap();
    match receiver.recv_timeout(Duration::from_secs(1)).unwrap() {
        ChangeEvent::InvalidateScope { scope_id, .. } => {
            assert_eq!(scope_id, scope.scope_id);
        }
        other => panic!("expected InvalidateScope, got {other:?}"),
    }

    db.merge(&scope, b"counter", storkv_core::MergeOp::CounterAdd(1))
        .unwrap();
    match receiver.recv_timeout(Duration::from_secs(1)).unwrap() {
        ChangeEvent::InvalidateScope { scope_id, .. } => {
            assert_eq!(scope_id, scope.scope_id);
        }
        other => panic!("expected InvalidateScope, got {other:?}"),
    }

    temp.close();
}

#[test]
fn test_replication_replay_budget_rejects_oversized_history() {
    const REPLAY_LIMIT: usize = 1024;
    let mut temp = crate::common::TempDb::new("replication_replay_budget");
    let db = temp.db();
    let scope = db
        .create_scope("replication_budget", ScopeOptions::default())
        .unwrap();

    for index in 0..=REPLAY_LIMIT {
        let key = format!("replication:{index}");
        db.put(&scope, key.as_bytes(), b"value")
            .expect("write replication frame");
    }

    match db.inner().replication_frames_from(0) {
        Err(storkv_core::StorkvError::ReplayBudgetExceeded { events, .. }) => {
            assert!(events > REPLAY_LIMIT);
        }
        Err(other) => panic!("expected ReplayBudgetExceeded, got {other:?}"),
        Ok(_) => panic!("expected ReplayBudgetExceeded, got a receiver"),
    }

    temp.close();
}

#[test]
fn test_watch_replay_budget_rejects_oversized_history() {
    let mut temp = crate::common::TempDb::new("watch_replay_budget");
    let db = temp.db();
    let options = ScopeOptions {
        durability: Durability::Buffered {
            sync_interval: Duration::from_millis(1),
        },
        ..ScopeOptions::default()
    };
    let scope = db
        .create_scope("budget_scope", options)
        .expect("create scope");

    for index in 0..=MAX_REPLAY_EVENTS {
        let key = format!("budget:{index}");
        db.put(&scope, key.as_bytes(), b"value")
            .expect("write replay event");
    }

    match db.watch_from(0) {
        Err(storkv_core::StorkvError::ReplayBudgetExceeded { events, .. }) => {
            assert!(events > MAX_REPLAY_EVENTS);
        }
        Err(other) => panic!("expected ReplayBudgetExceeded, got {other:?}"),
        Ok(_) => panic!("expected ReplayBudgetExceeded, got a receiver"),
    }

    temp.close();
}
