use crate::common;
use bytes::Bytes;
use std::time::Duration;
use storkv_core::mvcc::{DefaultMergeOperator, MergeOp, MergeOperator};
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
fn extended_merge_operators_resolve_and_persist() {
    let mut temp = common::TempDb::new("mvcc_extended_merge");
    let scope = default_scope();
    {
        let db = temp.db();

        // AppendBytes with max_len bound
        db.merge(
            &scope,
            b"log",
            MergeOp::AppendBytes {
                data: Bytes::from_static(b"line1\n"),
                max_len: Some(12),
            },
        )
        .unwrap();
        db.flush().unwrap();
        db.merge(
            &scope,
            b"log",
            MergeOp::AppendBytes {
                data: Bytes::from_static(b"line2\n"),
                max_len: Some(12),
            },
        )
        .unwrap();
        db.merge(
            &scope,
            b"log",
            MergeOp::AppendBytes {
                data: Bytes::from_static(b"line3\n"),
                max_len: Some(12),
            },
        )
        .unwrap();
        assert_eq!(
            db.get(&scope, b"log").unwrap().as_deref(),
            Some(&b"line2\nline3\n"[..])
        );

        // Unbounded AppendBytes
        db.put(&scope, b"stream", b"head-").unwrap();
        db.merge(
            &scope,
            b"stream",
            MergeOp::AppendBytes {
                data: Bytes::from_static(b"mid-"),
                max_len: None,
            },
        )
        .unwrap();
        db.merge(
            &scope,
            b"stream",
            MergeOp::AppendBytes {
                data: Bytes::from_static(b"tail"),
                max_len: None,
            },
        )
        .unwrap();
        assert_eq!(
            db.get(&scope, b"stream").unwrap().as_deref(),
            Some(&b"head-mid-tail"[..])
        );

        // MaxI64 & MinI64
        db.merge(&scope, b"high", MergeOp::MaxI64(-10)).unwrap();
        db.flush().unwrap();
        db.merge(&scope, b"high", MergeOp::MaxI64(25)).unwrap();
        db.merge(&scope, b"high", MergeOp::MaxI64(12)).unwrap();
        assert_eq!(
            db.get(&scope, b"high").unwrap().as_deref(),
            Some(&b"25"[..])
        );

        db.merge(&scope, b"low", MergeOp::MinI64(50)).unwrap();
        db.flush().unwrap();
        db.merge(&scope, b"low", MergeOp::MinI64(-7)).unwrap();
        db.merge(&scope, b"low", MergeOp::MinI64(3)).unwrap();
        assert_eq!(db.get(&scope, b"low").unwrap().as_deref(), Some(&b"-7"[..]));

        // BitOr & BitAnd
        db.merge(&scope, b"flags", MergeOp::BitOr(0b0101)).unwrap();
        db.flush().unwrap();
        db.merge(&scope, b"flags", MergeOp::BitOr(0b1010)).unwrap();
        db.merge(&scope, b"flags", MergeOp::BitAnd(0b1100)).unwrap();
        assert_eq!(
            db.get(&scope, b"flags").unwrap().as_deref(),
            Some(&b"12"[..])
        );
    }

    temp.close();
    let db = Db::open(&temp.path, DbOptions::default()).unwrap();
    assert_eq!(
        db.get(&scope, b"log").unwrap().as_deref(),
        Some(&b"line2\nline3\n"[..])
    );
    assert_eq!(
        db.get(&scope, b"stream").unwrap().as_deref(),
        Some(&b"head-mid-tail"[..])
    );
    assert_eq!(
        db.get(&scope, b"high").unwrap().as_deref(),
        Some(&b"25"[..])
    );
    assert_eq!(db.get(&scope, b"low").unwrap().as_deref(), Some(&b"-7"[..]));
    assert_eq!(
        db.get(&scope, b"flags").unwrap().as_deref(),
        Some(&b"12"[..])
    );
}

#[test]
fn partial_merge_matches_full_merge_for_all_associative_variants() {
    let operator = DefaultMergeOperator;
    let cases: Vec<(Option<&[u8]>, MergeOp, MergeOp)> = vec![
        (None, MergeOp::CounterAdd(5), MergeOp::CounterAdd(-2)),
        (Some(b"10"), MergeOp::CounterAdd(5), MergeOp::CounterAdd(-2)),
        (None, MergeOp::MaxI64(-20), MergeOp::MaxI64(-5)),
        (Some(b"15"), MergeOp::MaxI64(8), MergeOp::MaxI64(12)),
        (None, MergeOp::MinI64(-5), MergeOp::MinI64(-20)),
        (Some(b"-30"), MergeOp::MinI64(-5), MergeOp::MinI64(-20)),
        (None, MergeOp::BitOr(0b0101), MergeOp::BitOr(0b0011)),
        (Some(b"8"), MergeOp::BitOr(0b0101), MergeOp::BitOr(0b0011)),
        (None, MergeOp::BitAnd(0b1100), MergeOp::BitAnd(0b1010)),
        (
            Some(b"15"),
            MergeOp::BitAnd(0b1100),
            MergeOp::BitAnd(0b1010),
        ),
        (
            Some(b"ab"),
            MergeOp::AppendBytes {
                data: Bytes::from_static(b"cd"),
                max_len: None,
            },
            MergeOp::AppendBytes {
                data: Bytes::from_static(b"ef"),
                max_len: Some(4),
            },
        ),
        (
            Some(b"abcdef"),
            MergeOp::AppendBytes {
                data: Bytes::from_static(b"gh"),
                max_len: Some(6),
            },
            MergeOp::AppendBytes {
                data: Bytes::from_static(b"ij"),
                max_len: Some(4),
            },
        ),
    ];

    for (base, left, right) in cases {
        let mut temp = common::TempDb::new("partial_encode");
        let db = temp.db();
        let scope = default_scope();
        db.merge(&scope, b"l", left.clone()).unwrap();
        db.merge(&scope, b"r", right.clone()).unwrap();
        let left_bytes = db
            .inner()
            .scopes
            .read()
            .get(&0)
            .unwrap()
            .get_versions(b"l", u64::MAX.into(), 0.into())
            .unwrap()[0]
            .1
            .clone();
        let right_bytes = db
            .inner()
            .scopes
            .read()
            .get(&0)
            .unwrap()
            .get_versions(b"r", u64::MAX.into(), 0.into())
            .unwrap()[0]
            .1
            .clone();
        temp.close();

        let expected = operator
            .full_merge(b"k", base, &[left_bytes.clone(), right_bytes.clone()])
            .unwrap();
        let folded = operator
            .partial_merge(b"k", &left_bytes, &right_bytes)
            .expect("associative operands must fold");
        let actual = operator.full_merge(b"k", base, &[folded]).unwrap();
        assert_eq!(actual, expected, "mismatch for {left:?} + {right:?}");
    }
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
                ..ScopeOptions::default()
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
