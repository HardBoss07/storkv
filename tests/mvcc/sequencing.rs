use crate::common;
use storkv_core::ScopeOptions;

#[test]
fn test_monotonic_sequence_assignment() {
    let mut temp = common::TempDb::new("mvcc_seq");
    let db = temp.db();

    let scope = db.create_scope("monoseq", ScopeOptions::default()).unwrap();

    let sequence_before = db
        .inner()
        .last_visible_seq
        .load(std::sync::atomic::Ordering::SeqCst);

    let mut transaction = db.begin_tx();
    transaction.put(&scope, b"k1", b"v1").unwrap();
    transaction.put(&scope, b"k2", b"v2").unwrap();
    transaction.put(&scope, b"k3", b"v3").unwrap();
    transaction.commit().unwrap();

    let sequence_after = db
        .inner()
        .last_visible_seq
        .load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        sequence_after,
        sequence_before + 3,
        "Batch of 3 writes should increment seq by exactly 3"
    );

    temp.close();
}
