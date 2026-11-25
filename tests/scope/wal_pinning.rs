use crate::common;
use std::time::Duration;
use storkv_core::{DbOptions, ScopeOptions};

#[test]
fn test_wal_pinning_byte_and_time_watermarks() {
    let mut temp = common::TempDb::open_with_options(
        "wal_pin",
        DbOptions {
            max_wal_pin_bytes: 4096,
            max_wal_pin_age: Duration::from_millis(50),
            ..DbOptions::default()
        },
    );
    let db = temp.db();

    let scope = db
        .create_scope("watermark_scope", ScopeOptions::default())
        .unwrap();

    let payload = vec![b'p'; 512];
    for index in 0..10 {
        let key = format!("k_{}", index);
        db.put(&scope, key.as_bytes(), &payload).unwrap();
    }

    let scope_dir = temp
        .path
        .join("scopes")
        .join(format!("{:08x}", scope.scope_id));
    assert!(scope_dir.exists());

    let sst_files: Vec<_> = std::fs::read_dir(&scope_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".sst"))
        .collect();

    assert!(
        !sst_files.is_empty(),
        "WAL pinning watermark should have scheduled and produced at least one SSTable"
    );

    temp.close();
}
