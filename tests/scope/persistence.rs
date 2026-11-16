use crate::common;
use storkv_core::{Db, DbOptions, ScopeOptions};

#[test]
fn test_scopes_and_data_survive_reopen() {
    let mut temp = common::TempDb::new("scope_reopen");
    {
        let db = temp.db();
        let scope = db
            .create_scope("persistent_scope", ScopeOptions::default())
            .unwrap();
        db.put(&scope, b"persistent_key", b"persistent_value")
            .unwrap();

        let scope_arc = db
            .inner()
            .scopes
            .read()
            .get(&scope.scope_id)
            .cloned()
            .unwrap();
        db.inner().flush_scope_memtable(&scope_arc).unwrap();
    }
    temp.close();

    let reopened = Db::open(&temp.path, DbOptions::default()).unwrap();
    let scope_id = *reopened
        .inner()
        .scope_names
        .read()
        .get("persistent_scope")
        .expect("scope must survive reopen");
    let handle = storkv_core::ScopeHandle {
        scope_id,
        name: "persistent_scope".into(),
    };
    let value = reopened.get(&handle, b"persistent_key").unwrap().unwrap();
    assert_eq!(value.as_ref(), b"persistent_value");
}
