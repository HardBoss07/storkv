use crate::common;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use storkv_core::{DbOptions, ScopeHandle, ScopeOptions, StorkvError, WriteOptions};

fn l0_count(scope: &ScopeHandle, db: &storkv_core::Db) -> usize {
    db.inner()
        .scopes
        .read()
        .get(&scope.scope_id)
        .and_then(|scope| scope.sst_levels.read().get(&0).map(Vec::len))
        .unwrap_or(0)
}

fn flush(db: &storkv_core::Db) {
    db.flush().expect("flush scopes");
}

#[test]
fn l0_slowdown_adds_write_latency() {
    let temp = common::TempDb::open_with_options(
        "bp_slowdown",
        DbOptions {
            l0_slowdown_writes_trigger: 1,
            l0_stop_writes_trigger: 2,
            max_total_wal_bytes: u64::MAX,
            ..DbOptions::default()
        },
    );
    let db = temp.db();
    let scope = db
        .create_scope("slowdown", ScopeOptions::default())
        .unwrap();

    db.put(&scope, b"first", b"value").unwrap();
    flush(db);
    assert_eq!(l0_count(&scope, db), 1, "one L0 file must exist");

    let started = Instant::now();
    db.put(&scope, b"second", b"value").unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(10),
        "write must slow down at the L0 trigger, elapsed {elapsed:?}"
    );
}

#[test]
fn l0_stop_stalls_until_compaction() {
    let temp = common::TempDb::open_with_options(
        "bp_l0_stop",
        DbOptions {
            l0_slowdown_writes_trigger: 100,
            l0_stop_writes_trigger: 4,
            max_total_wal_bytes: u64::MAX,
            ..DbOptions::default()
        },
    );
    let db = temp.db();
    let scope = db.create_scope("stop", ScopeOptions::default()).unwrap();
    for batch in 0..4u64 {
        db.put(&scope, format!("k:{batch}"), b"value").unwrap();
        flush(db);
    }
    assert_eq!(l0_count(&scope, db), 4, "the stop trigger must be reached");

    let writer_db = db.clone();
    let writer_scope = scope.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = writer_db.put(&writer_scope, b"blocked", b"value");
        done_tx.send(()).unwrap();
        result
    });

    started_rx.recv().unwrap();
    assert!(
        done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "the write must stall at the L0 stop trigger"
    );

    db.compact().unwrap();
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the stalled write must resume after compaction");
    writer.join().unwrap().unwrap();
    assert_eq!(
        db.get(&scope, b"blocked").unwrap().as_deref(),
        Some(&b"value"[..])
    );
}

#[test]
fn wal_watermark_stalls_until_flush() {
    let temp = common::TempDb::open_with_options(
        "bp_wal_stop",
        DbOptions {
            l0_slowdown_writes_trigger: 100,
            l0_stop_writes_trigger: 100,
            max_total_wal_bytes: 1,
            ..DbOptions::default()
        },
    );
    let db = temp.db();
    let scope = db.create_scope("wal", ScopeOptions::default()).unwrap();
    db.put(&scope, b"first", b"value").unwrap();

    let writer_db = db.clone();
    let writer_scope = scope.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = writer_db.put(&writer_scope, b"blocked", b"value");
        done_tx.send(()).unwrap();
        result
    });

    started_rx.recv().unwrap();
    assert!(
        done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "the write must stall at the WAL byte watermark"
    );

    flush(db);
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the stalled write must resume after a flush");
    writer.join().unwrap().unwrap();
    assert_eq!(
        db.get(&scope, b"blocked").unwrap().as_deref(),
        Some(&b"value"[..])
    );
}

#[test]
fn non_blocking_reports_write_stalled_at_l0_stop() {
    let temp = common::TempDb::open_with_options(
        "bp_non_blocking_l0",
        DbOptions {
            l0_slowdown_writes_trigger: 100,
            l0_stop_writes_trigger: 2,
            max_total_wal_bytes: u64::MAX,
            ..DbOptions::default()
        },
    );
    let db = temp.db();
    let scope = db
        .create_scope("non_blocking_l0", ScopeOptions::default())
        .unwrap();
    for batch in 0..2u64 {
        db.put(&scope, format!("k:{batch}"), b"value").unwrap();
        flush(db);
    }

    let error = db
        .put_with_options(
            &scope,
            b"rejected",
            b"value",
            WriteOptions { non_blocking: true },
        )
        .unwrap_err();
    assert!(matches!(error, StorkvError::WriteStalled(_)), "{error:?}");
}

#[test]
fn non_blocking_reports_write_stalled_at_wal_limit() {
    let temp = common::TempDb::open_with_options(
        "bp_non_blocking_wal",
        DbOptions {
            max_total_wal_bytes: 1,
            ..DbOptions::default()
        },
    );
    let db = temp.db();
    let scope = db
        .create_scope("non_blocking_wal", ScopeOptions::default())
        .unwrap();
    db.put(&scope, b"first", b"value").unwrap();

    let error = db
        .put_with_options(
            &scope,
            b"rejected",
            b"value",
            WriteOptions { non_blocking: true },
        )
        .unwrap_err();
    assert!(matches!(error, StorkvError::WriteStalled(_)), "{error:?}");
}

#[test]
fn close_unblocks_stalled_writer() {
    let mut temp = common::TempDb::open_with_options(
        "bp_shutdown",
        DbOptions {
            l0_slowdown_writes_trigger: 100,
            l0_stop_writes_trigger: 2,
            max_total_wal_bytes: u64::MAX,
            ..DbOptions::default()
        },
    );
    let db = temp.db();
    let scope = db
        .create_scope("shutdown", ScopeOptions::default())
        .unwrap();
    for batch in 0..2u64 {
        db.put(&scope, format!("k:{batch}"), b"value").unwrap();
        flush(db);
    }

    let writer_db = db.clone();
    let writer_scope = scope.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        writer_db.put(&writer_scope, b"blocked", b"value")
    });
    started_rx.recv().unwrap();
    std::thread::sleep(Duration::from_millis(50));

    temp.db.take().unwrap().close().unwrap();
    let result = writer.join().unwrap();
    assert!(
        matches!(result, Err(StorkvError::InvalidOperation(_))),
        "a stalled writer must fail cleanly on close, got {result:?}"
    );
}
