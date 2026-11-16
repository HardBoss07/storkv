use crate::common;
use std::time::Duration;
use storkv_core::mvcc::MergeOp;
use storkv_core::{CompactionStyle, Durability, Retention, ScopeOptions};

#[test]
fn test_merge_operator_resolution() {
    let mut temp = common::TempDb::new("mvcc_merge");
    let db = temp.db();

    let scope = db
        .create_scope("counter_scope", ScopeOptions::default())
        .unwrap();

    db.put(&scope, b"page_views", b"10").unwrap();

    db.merge(&scope, b"page_views", MergeOp::CounterAdd(5))
        .unwrap();
    db.merge(&scope, b"page_views", MergeOp::CounterAdd(20))
        .unwrap();

    let final_value = db.get(&scope, b"page_views").unwrap().unwrap();
    assert_eq!(final_value.as_ref(), b"35");

    temp.close();
}

#[test]
fn test_wall_clock_ttl_read_invisibility() {
    let mut temp = common::TempDb::new("mvcc_ttl");
    let db = temp.db();

    let scope = db
        .create_scope(
            "ttl_scope",
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

    db.put_with_ttl(
        &scope,
        b"ephemeral",
        b"temp_data",
        Duration::from_millis(30),
    )
    .unwrap();

    let value_before = db.get(&scope, b"ephemeral").unwrap();
    assert!(value_before.is_some());

    std::thread::sleep(Duration::from_millis(60));

    let value_after = db.get(&scope, b"ephemeral").unwrap();
    assert!(
        value_after.is_none(),
        "Expired TTL entry must be completely invisible to reads"
    );

    temp.close();
}
