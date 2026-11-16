use crate::common;
use storkv_core::ScopeOptions;

#[test]
fn test_snapshot_isolation_and_read_your_own_writes() {
    let mut temp = common::TempDb::new("mvcc_ryow");
    let db = temp.db();

    let scope = db
        .create_scope("ryow_scope", ScopeOptions::default())
        .unwrap();

    db.put(&scope, b"shared_key", b"initial_val").unwrap();

    let mut transaction = db.begin_tx();
    transaction.put(&scope, b"shared_key", b"tx1_val").unwrap();
    let local_read = transaction.get(&scope, b"shared_key").unwrap().unwrap();
    assert_eq!(
        local_read.as_ref(),
        b"tx1_val",
        "Transaction must read its own uncommitted write"
    );

    let read_value = db.get(&scope, b"shared_key").unwrap().unwrap();
    assert_eq!(
        read_value.as_ref(),
        b"initial_val",
        "Outside reads must not see uncommitted writes"
    );

    transaction.commit().unwrap();

    let committed_value = db.get(&scope, b"shared_key").unwrap().unwrap();
    assert_eq!(committed_value.as_ref(), b"tx1_val");

    temp.close();
}
