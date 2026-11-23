use crate::common;
use bytes::Bytes;
use storkv_core::{MergeOp, ScopeOptions};

#[test]
fn test_merge_and_tombstone_folding_across_compaction() {
    let temp = common::TempDb::new("comp_merge_fold");
    let db = temp.db();
    let scope = db
        .create_scope("fold_scope", ScopeOptions::default())
        .unwrap();

    db.put(&scope, b"del_me", b"gone").unwrap();
    db.put(&scope, b"counter", b"10").unwrap();
    db.delete(&scope, b"del_me").unwrap();
    db.merge(&scope, b"counter", MergeOp::CounterAdd(7))
        .unwrap();
    db.merge(&scope, b"counter", MergeOp::CounterAdd(5))
        .unwrap();

    db.put(&scope, b"log", b"a").unwrap();
    db.merge(
        &scope,
        b"log",
        MergeOp::AppendBytes {
            data: Bytes::from_static(b"b"),
            max_len: Some(3),
        },
    )
    .unwrap();
    db.flush().unwrap();
    db.merge(
        &scope,
        b"log",
        MergeOp::AppendBytes {
            data: Bytes::from_static(b"cd"),
            max_len: Some(3),
        },
    )
    .unwrap();

    db.put(&scope, b"max_k", b"10").unwrap();
    db.merge(&scope, b"max_k", MergeOp::MaxI64(42)).unwrap();
    db.flush().unwrap();
    db.merge(&scope, b"max_k", MergeOp::MaxI64(21)).unwrap();

    db.put(&scope, b"min_k", b"10").unwrap();
    db.merge(&scope, b"min_k", MergeOp::MinI64(-4)).unwrap();
    db.flush().unwrap();
    db.merge(&scope, b"min_k", MergeOp::MinI64(3)).unwrap();

    db.put(&scope, b"mask", b"12").unwrap();
    db.merge(&scope, b"mask", MergeOp::BitOr(0b0011)).unwrap();
    db.flush().unwrap();
    db.merge(&scope, b"mask", MergeOp::BitAnd(0b1010)).unwrap();

    db.flush().unwrap();
    db.compact().unwrap();

    assert_eq!(db.get(&scope, b"del_me").unwrap(), None);
    assert_eq!(db.get(&scope, b"counter").unwrap().unwrap().as_ref(), b"22");
    assert_eq!(db.get(&scope, b"log").unwrap().unwrap().as_ref(), b"bcd");
    assert_eq!(db.get(&scope, b"max_k").unwrap().unwrap().as_ref(), b"42");
    assert_eq!(db.get(&scope, b"min_k").unwrap().unwrap().as_ref(), b"-4");
    assert_eq!(db.get(&scope, b"mask").unwrap().unwrap().as_ref(), b"10");
}
