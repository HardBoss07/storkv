use crate::common;
use storkv_core::ScopeOptions;

#[test]
fn test_scope_storage_isolation() {
    let mut temp = common::TempDb::new("scope_iso");
    let db = temp.db();

    let scope_a = db.create_scope("scope_a", ScopeOptions::default()).unwrap();
    let scope_b = db.create_scope("scope_b", ScopeOptions::default()).unwrap();

    let key = b"isolation_test_key";
    db.put(&scope_a, key, b"val_scope_a").unwrap();
    db.put(&scope_b, key, b"val_scope_b").unwrap();

    let value_a = db.get(&scope_a, key).unwrap().unwrap();
    let value_b = db.get(&scope_b, key).unwrap().unwrap();

    assert_eq!(value_a.as_ref(), b"val_scope_a");
    assert_eq!(value_b.as_ref(), b"val_scope_b");

    temp.close();
}

#[test]
fn test_fifty_scopes_isolated_with_drop_compaction() {
    let temp = common::TempDb::new("scope_many");
    let db = temp.db();

    let mut scopes = Vec::new();
    for index in 0..50 {
        let scope = db
            .create_scope(format!("isolated_{:02}", index), ScopeOptions::default())
            .unwrap();
        let value = format!("value_{:02}", index);
        db.put(&scope, b"same_key", value.as_bytes()).unwrap();
        scopes.push(scope);
    }

    for (index, scope) in scopes.iter().enumerate() {
        let value = db.get(scope, b"same_key").unwrap().unwrap();
        assert_eq!(value.as_ref(), format!("value_{:02}", index).as_bytes());
    }

    for index in (0..50).step_by(2) {
        db.drop_scope(format!("isolated_{:02}", index)).unwrap();
    }

    for (index, scope) in scopes.iter().enumerate() {
        let value = db.get(scope, b"same_key").unwrap();
        if index % 2 == 0 {
            assert!(value.is_none(), "dropped scope {index} must be empty");
        } else {
            assert!(value.is_some(), "live scope {index} must retain data");
        }
    }
}
