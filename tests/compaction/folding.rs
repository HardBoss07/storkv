use crate::common;
use storkv_core::ScopeOptions;

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
    db.merge(&scope, b"counter", storkv_core::MergeOp::CounterAdd(7))
        .unwrap();
    db.merge(&scope, b"counter", storkv_core::MergeOp::CounterAdd(5))
        .unwrap();

    let scope_arc = db
        .inner()
        .scopes
        .read()
        .get(&scope.scope_id)
        .cloned()
        .unwrap();
    db.inner().flush_scope_memtable(&scope_arc).unwrap();
    db.compact().unwrap();

    assert_eq!(db.get(&scope, b"del_me").unwrap(), None);
    assert_eq!(db.get(&scope, b"counter").unwrap().unwrap().as_ref(), b"22");
}
