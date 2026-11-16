use crate::common;
use storkv_core::{ScopeOptions, SequenceNumber, TimestampMs};

#[test]
fn test_snapshot_pin_bounds_gc_safe_seq_and_preserves_old_version() {
    let temp = common::TempDb::new("comp_snapshot_pin");
    let db = temp.db();
    let scope = db
        .create_scope("pinned_scope", ScopeOptions::default())
        .unwrap();

    db.put(&scope, b"pinned_key", b"old_value").unwrap();
    let pinned_seq = db
        .inner()
        .last_visible_seq
        .load(std::sync::atomic::Ordering::SeqCst);
    db.create_snapshot("release-pin").unwrap();

    db.put(&scope, b"pinned_key", b"new_value").unwrap();
    db.compact().unwrap();

    assert!(db.inner().gc_safe_seq() <= pinned_seq);

    let scope_arc = db
        .inner()
        .scopes
        .read()
        .get(&scope.scope_id)
        .cloned()
        .unwrap();
    let versions = scope_arc
        .get_versions(
            b"pinned_key",
            SequenceNumber::new(pinned_seq),
            TimestampMs::now(),
        )
        .unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].1.as_ref(), b"old_value");
}
