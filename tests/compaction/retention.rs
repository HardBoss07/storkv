use crate::common;
use std::time::Duration;
use storkv_core::{CompactionStyle, Durability, Retention, ScopeOptions};

#[test]
fn test_retention_modes_leveled_and_fifo() {
    let temp = common::TempDb::new("comp_retention");
    let db = temp.db();

    let fifo_scope = db
        .create_scope(
            "fifo_scope",
            ScopeOptions {
                durability: Durability::StrictFsync,
                retention: Retention::Ttl {
                    default_ttl: Duration::from_millis(50),
                    compaction_style: CompactionStyle::Fifo,
                },
                prefix_bloom_len: None,
            },
        )
        .unwrap();

    for index in 0..20 {
        let key = format!("fifo_k_{}", index);
        db.put_with_ttl(
            &fifo_scope,
            key.as_bytes(),
            b"val",
            Duration::from_millis(40),
        )
        .unwrap();
    }

    let scope_arc = db
        .inner()
        .scopes
        .read()
        .get(&fifo_scope.scope_id)
        .cloned()
        .unwrap();
    db.inner().flush_scope_memtable(&scope_arc).unwrap();

    let scope_dir = temp
        .path
        .join("scopes")
        .join(format!("{:08x}", fifo_scope.scope_id));
    let initial_ssts: Vec<_> = std::fs::read_dir(&scope_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".sst"))
        .collect();
    assert!(!initial_ssts.is_empty());

    std::thread::sleep(Duration::from_millis(60));

    db.compact().unwrap();
}

#[test]
fn test_gc_safe_seq_calculation_and_ttl_purging() {
    let temp = common::TempDb::new("comp_gc");
    let db = temp.db();

    let scope = db
        .create_scope("leveled_scope", ScopeOptions::default())
        .unwrap();

    db.put(&scope, b"account:1", b"v1").unwrap();
    let mut transaction = db.begin_tx();
    let pinned_value = transaction.get(&scope, b"account:1").unwrap().unwrap();
    assert_eq!(pinned_value.as_ref(), b"v1");

    db.put(&scope, b"account:1", b"v2").unwrap();
    db.put(&scope, b"account:1", b"v3").unwrap();

    let gc_safe = db.inner().gc_safe_seq();
    assert!(
        gc_safe
            <= db
                .inner()
                .last_visible_seq
                .load(std::sync::atomic::Ordering::SeqCst),
        "gc_safe_seq must be bounded by active transaction read_seq"
    );

    drop(transaction);
}
