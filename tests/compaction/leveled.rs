use crate::common;
use std::time::Duration;
use storkv_core::{Durability, Retention, ScopeOptions};

#[test]
fn test_leveled_compaction_over_10k_keys() {
    let temp = common::TempDb::new("comp_bulk");
    let db = temp.db();
    let scope = db
        .create_scope(
            "bulk_scope",
            ScopeOptions {
                durability: Durability::Buffered {
                    sync_interval: Duration::from_millis(10),
                },
                retention: Retention::Permanent,
                prefix_bloom_len: Some(8),
                storage_mode: storkv_core::StorageMode::Disk,
            },
        )
        .unwrap();

    for batch in 0..4u64 {
        for index in 0..2500u64 {
            let id = batch * 2500 + index;
            db.put(&scope, format!("k:{:05}", id), format!("v:{:05}", id))
                .unwrap();
        }
        let scope_arc = db
            .inner()
            .scopes
            .read()
            .get(&scope.scope_id)
            .cloned()
            .unwrap();
        db.inner().flush_scope_memtable(&scope_arc).unwrap();
    }

    let scope_arc = db
        .inner()
        .scopes
        .read()
        .get(&scope.scope_id)
        .cloned()
        .unwrap();
    assert_eq!(scope_arc.sst_levels.read().get(&0).map(Vec::len), Some(4));

    db.compact().unwrap();

    {
        let levels = scope_arc.sst_levels.read();
        assert_eq!(levels.get(&0).map(Vec::len), Some(0), "L0 must be drained");
        assert_eq!(
            levels.get(&1).map(Vec::len),
            Some(1),
            "L0 must merge into L1"
        );
    }

    let all_entries = db.scan_prefix(&scope, b"k:").unwrap();
    assert_eq!(all_entries.len(), 10_000);
    for id in [0u64, 1, 1234, 7777, 9999] {
        let key = format!("k:{:05}", id);
        let value = db.get(&scope, key.as_bytes()).unwrap().unwrap();
        assert_eq!(value.as_ref(), format!("v:{:05}", id).as_bytes());
    }
}
