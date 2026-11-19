use crate::common;
use storkv_core::{ScopeOptions, StorkvError};

#[test]
fn test_occ_conflict_detection_and_blind_writes() {
    let mut temp = common::TempDb::new("mvcc_occ");
    let db = temp.db();

    let scope = db
        .create_scope("occ_scope", ScopeOptions::default())
        .unwrap();

    db.put(&scope, b"account_balance", b"100").unwrap();

    let mut transaction1 = db.begin_tx();
    let mut transaction2 = db.begin_tx();

    let balance1 = transaction1
        .get(&scope, b"account_balance")
        .unwrap()
        .unwrap();
    let balance2 = transaction2
        .get(&scope, b"account_balance")
        .unwrap()
        .unwrap();
    assert_eq!(balance1.as_ref(), b"100");
    assert_eq!(balance2.as_ref(), b"100");

    transaction1
        .put(&scope, b"account_balance", b"150")
        .unwrap();
    transaction1.commit().unwrap();

    transaction2
        .put(&scope, b"account_balance", b"200")
        .unwrap();
    let result = transaction2.commit();
    assert!(
        matches!(result, Err(StorkvError::WriteConflict { .. })),
        "tx2 must abort with WriteConflict because ReadSet was concurrently modified"
    );

    let mut transaction3 = db.begin_tx();
    transaction3
        .put(&scope, b"blind_key", b"blind_val")
        .unwrap();
    assert!(
        transaction3.commit().is_ok(),
        "Blind writes must succeed without ReadSet conflicts"
    );

    temp.close();
}
