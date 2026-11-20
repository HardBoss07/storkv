use crate::common;
use std::time::Duration;
use storkv_core::mvcc::MergeOp;
use storkv_core::{
    CompactionStyle, Db, DbOptions, Durability, Retention, ScopeHandle, ScopeOptions,
};

fn default_scope() -> ScopeHandle {
    ScopeHandle {
        scope_id: 0,
        name: "default".to_string(),
    }
}

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
fn merge_operands_apply_once_across_reopen() {
    let mut temp = common::TempDb::new("mvcc_merge_reopen");
    let scope = default_scope();
    {
        let db = temp.db();
        db.merge(&scope, b"counter", MergeOp::CounterAdd(7))
            .unwrap();
        db.merge(&scope, b"counter", MergeOp::CounterAdd(5))
            .unwrap();
        assert_eq!(db.get(&scope, b"counter").unwrap().unwrap().as_ref(), b"12");
    }

    temp.close();
    let db = Db::open(&temp.path, DbOptions::default()).unwrap();
    assert_eq!(
        db.get(&scope, b"counter").unwrap().unwrap().as_ref(),
        b"12",
        "reopening must not apply flushed merge operands a second time"
    );
}

#[test]
fn repeated_empty_flushes_do_not_duplicate_merge_operands() {
    let temp = common::TempDb::new("mvcc_empty_flush_merge");
    let db = temp.db();
    let scope = default_scope();

    for _ in 0..4 {
        db.flush().unwrap();
    }

    db.merge(&scope, b"counter", MergeOp::CounterAdd(2))
        .unwrap();
    db.flush().unwrap();
    for _ in 0..4 {
        db.flush().unwrap();
    }

    db.merge(&scope, b"counter", MergeOp::CounterAdd(3))
        .unwrap();
    db.flush().unwrap();

    assert_eq!(
        db.get(&scope, b"counter").unwrap().unwrap().as_ref(),
        b"5",
        "empty flushes must not leave a queued copy that double-counts merges"
    );
    assert_eq!(
        db.scan(&scope, None::<&[u8]>, None::<&[u8]>).unwrap(),
        vec![(
            bytes::Bytes::from_static(b"counter"),
            bytes::Bytes::from_static(b"5")
        )]
    );
}

#[test]
fn scans_resolve_uncompacted_merge_operands() {
    let temp = common::TempDb::new("mvcc_scan_merge");
    let db = temp.db();
    let scope = default_scope();

    db.merge(&scope, b"operand_only", MergeOp::CounterAdd(4))
        .unwrap();
    db.merge(&scope, b"operand_only", MergeOp::CounterAdd(6))
        .unwrap();
    db.put(&scope, b"with_base", b"10").unwrap();
    db.merge(&scope, b"with_base", MergeOp::CounterAdd(7))
        .unwrap();

    assert_eq!(
        db.get(&scope, b"operand_only").unwrap().unwrap().as_ref(),
        b"10"
    );
    assert_eq!(
        db.get(&scope, b"with_base").unwrap().unwrap().as_ref(),
        b"17"
    );

    let scanned: Vec<(Vec<u8>, Vec<u8>)> = db
        .scan(&scope, None::<&[u8]>, None::<&[u8]>)
        .unwrap()
        .into_iter()
        .map(|(key, value)| (key.to_vec(), value.to_vec()))
        .collect();
    assert_eq!(
        scanned,
        vec![
            (b"operand_only".to_vec(), b"10".to_vec()),
            (b"with_base".to_vec(), b"17".to_vec()),
        ],
        "full scans must resolve merge operands like point reads"
    );

    let prefixed: Vec<(Vec<u8>, Vec<u8>)> = db
        .scan_prefix(&scope, b"with_")
        .unwrap()
        .into_iter()
        .map(|(key, value)| (key.to_vec(), value.to_vec()))
        .collect();
    assert_eq!(
        prefixed,
        vec![(b"with_base".to_vec(), b"17".to_vec())],
        "prefix scans must resolve merge operands like point reads"
    );
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
                storage_mode: storkv_core::StorageMode::Disk,
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
