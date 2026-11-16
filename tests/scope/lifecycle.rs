use crate::common;
use storkv_core::ScopeOptions;

#[test]
fn test_atomic_cross_scope_transactions() {
    let mut temp = common::TempDb::new("scope_tx");
    let db = temp.db();

    let scope1 = db
        .create_scope("tx_scope_1", ScopeOptions::default())
        .unwrap();
    let scope2 = db
        .create_scope("tx_scope_2", ScopeOptions::default())
        .unwrap();

    let mut transaction = db.begin_tx();
    transaction.put(&scope1, b"key1", b"val1").unwrap();
    transaction.put(&scope2, b"key2", b"val2").unwrap();
    transaction.commit().unwrap();

    assert_eq!(db.get(&scope1, b"key1").unwrap().unwrap().as_ref(), b"val1");
    assert_eq!(db.get(&scope2, b"key2").unwrap().unwrap().as_ref(), b"val2");

    temp.close();
}

#[test]
fn test_drop_scope_hiding_and_reclamation() {
    let mut temp = common::TempDb::new("scope_drop");
    let db = temp.db();

    let scope = db
        .create_scope("ephemeral_scope", ScopeOptions::default())
        .unwrap();

    db.put(&scope, b"secret_k", b"secret_v").unwrap();
    assert!(db.get(&scope, b"secret_k").unwrap().is_some());

    db.drop_scope("ephemeral_scope").unwrap();

    let read_result = db.get(&scope, b"secret_k");
    assert!(
        read_result.is_ok_and(|value| value.is_none()),
        "Dropped scope keys must be hidden immediately"
    );

    let new_scope = db
        .create_scope("ephemeral_scope", ScopeOptions::default())
        .unwrap();
    assert_ne!(new_scope.scope_id, scope.scope_id);
    assert!(db.get(&new_scope, b"secret_k").unwrap().is_none());

    temp.close();
}

#[test]
fn test_drop_scope_while_reader_holds_arc() {
    let temp = common::TempDb::new("scope_drop_reader");
    let db = temp.db();

    let scope = db
        .create_scope("live_reader", ScopeOptions::default())
        .unwrap();
    db.put(&scope, b"k", b"v").unwrap();

    let scope_arc = db
        .inner()
        .scopes
        .read()
        .get(&scope.scope_id)
        .cloned()
        .unwrap();

    db.drop_scope("live_reader").unwrap();
    assert!(scope_arc.is_dropped());
    assert!(db.get(&scope, b"k").unwrap().is_none());

    let fresh = db
        .create_scope("live_reader", ScopeOptions::default())
        .unwrap();
    assert_ne!(fresh.scope_id, scope.scope_id);
    assert!(db.get(&fresh, b"k").unwrap().is_none());
    assert!(db.get(&scope, b"k").unwrap().is_none());
}
