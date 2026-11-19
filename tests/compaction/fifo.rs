use crate::common;
use std::time::Duration;
use storkv_core::{CompactionStyle, Durability, Retention, ScopeOptions};

#[test]
fn test_fifo_compaction_reclaims_expired_sstables_on_disk() {
    let temp = common::TempDb::new("comp_fifo_reclaim");
    let db = temp.db();
    let scope = db
        .create_scope(
            "fifo_reclaim",
            ScopeOptions {
                durability: Durability::Buffered {
                    sync_interval: Duration::from_millis(10),
                },
                retention: Retention::Ttl {
                    default_ttl: Duration::from_millis(10),
                    compaction_style: CompactionStyle::Fifo,
                },
                prefix_bloom_len: None,
                storage_mode: storkv_core::StorageMode::Disk,
            },
        )
        .unwrap();

    for index in 0..50 {
        db.put_with_ttl(
            &scope,
            format!("expire:{}", index).as_bytes(),
            b"value",
            Duration::from_millis(10),
        )
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

    std::thread::sleep(Duration::from_millis(30));
    db.compact().unwrap();

    assert_eq!(scope_arc.sst_levels.read().get(&0).map(Vec::len), Some(0));
    let scope_dir = temp
        .path
        .join("scopes")
        .join(format!("{:08x}", scope.scope_id));
    let remaining = std::fs::read_dir(&scope_dir)
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".sst"))
        .count();
    assert_eq!(remaining, 0, "expired FIFO SSTables must be unlinked");
}
