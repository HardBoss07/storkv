use crate::common;
use std::time::Duration;
use storkv_core::{Durability as EngineDurability, Retention, ScopeOptions};

#[test]
fn test_durability_modes_and_strictest_precedence() {
    let mut temp = common::TempDb::new("wal_durability");
    let db = temp.db();

    let buffered_scope = db
        .create_scope(
            "buffered_scope",
            ScopeOptions {
                durability: EngineDurability::Buffered {
                    sync_interval: Duration::from_millis(100),
                },
                retention: Retention::Permanent,
                prefix_bloom_len: None,
                storage_mode: storkv_core::StorageMode::Disk,
            },
        )
        .unwrap();

    let strict_scope = db
        .create_scope(
            "strict_scope",
            ScopeOptions {
                durability: EngineDurability::StrictFsync,
                retention: Retention::Permanent,
                prefix_bloom_len: None,
                storage_mode: storkv_core::StorageMode::Disk,
            },
        )
        .unwrap();

    let fsyncs_before = db
        .telemetry()
        .fsyncs_total
        .load(std::sync::atomic::Ordering::Relaxed);
    db.put(&buffered_scope, b"buf_k", b"buf_v").unwrap();
    let fsyncs_after = db
        .telemetry()
        .fsyncs_total
        .load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        fsyncs_before, fsyncs_after,
        "Buffered write should not trigger immediate fsync"
    );

    db.put(&strict_scope, b"strict_k", b"strict_v").unwrap();
    let fsyncs_strict = db
        .telemetry()
        .fsyncs_total
        .load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        fsyncs_strict > fsyncs_after,
        "Strict write must increment fsyncs_total"
    );

    let fsyncs_before_tx = fsyncs_strict;
    let mut transaction = db.begin_tx();
    transaction
        .put(&buffered_scope, b"buf_k2", b"buf_v2")
        .unwrap();
    transaction
        .put(&strict_scope, b"strict_k2", b"strict_v2")
        .unwrap();
    transaction.commit().unwrap();

    let fsyncs_after_tx = db
        .telemetry()
        .fsyncs_total
        .load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        fsyncs_after_tx > fsyncs_before_tx,
        "Transaction touching StrictFsync must trigger fsync despite touching Buffered scope"
    );

    temp.close();
}
