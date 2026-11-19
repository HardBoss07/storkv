use std::fs;
use storkv_core::{Db, DbOptions, StorkvError};

#[test]
fn test_single_process_lock() {
    let temp = std::env::temp_dir().join(format!("storkv_flock_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    let db1 = Db::open(&temp, DbOptions::default()).expect("First open should succeed");

    let db2_result = Db::open(&temp, DbOptions::default());
    assert!(
        matches!(db2_result, Err(StorkvError::DatabaseLocked(_))),
        "Second open must fail with DatabaseLocked error"
    );

    drop(db1);

    let db3_result = Db::open(&temp, DbOptions::default());
    assert!(db3_result.is_ok(), "Opening after drop must succeed");

    let _ = fs::remove_dir_all(&temp);
}
