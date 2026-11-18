use std::time::Duration;

use storkv_core::ScopeOptions;
use storkv_core::watch::ChangeEvent;

#[test]
fn test_scoped_cdc_watch_stream() {
    let mut temp = crate::common::TempDb::new("watch_stream");
    let db = temp.db();

    let scope = db
        .create_scope("watch_scope", ScopeOptions::default())
        .unwrap();

    let rx = db.watch();

    db.put(&scope, b"watch_key", b"watch_val").unwrap();

    let event = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("Failed to receive watch event");

    match event {
        ChangeEvent::Put {
            scope_id,
            key,
            value,
            ..
        } => {
            assert_eq!(scope_id, scope.scope_id);
            assert_eq!(key.as_ref(), b"watch_key");
            assert_eq!(value.as_ref(), b"watch_val");
        }
        _ => panic!("Expected Put change event"),
    }

    temp.close();
}
