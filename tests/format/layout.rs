use crate::common;
use std::fs;
use storkv_core::{Durability as EngineDurability, Retention, ScopeOptions};

#[test]
fn test_directory_structure_and_hex_naming() {
    let mut temp = common::TempDb::new("dir_format");
    let db = temp.db();

    let scope1 = db
        .create_scope(
            "users",
            ScopeOptions {
                durability: EngineDurability::StrictFsync,
                retention: Retention::Permanent,
                prefix_bloom_len: None,
            },
        )
        .unwrap();

    let scope2 = db
        .create_scope(
            "metrics",
            ScopeOptions {
                durability: EngineDurability::Buffered {
                    sync_interval: std::time::Duration::from_millis(50),
                },
                retention: Retention::Permanent,
                prefix_bloom_len: None,
            },
        )
        .unwrap();

    db.put(&scope1, b"user:1", b"val1").unwrap();
    db.put(&scope2, b"metric:1", b"val2").unwrap();

    let lock_path = temp.path.join("LOCK");
    assert!(lock_path.exists());

    let current_path = temp.path.join("CURRENT");
    assert!(current_path.exists());
    let current_content = fs::read_to_string(&current_path).unwrap();
    let manifest_name = current_content.trim();
    assert!(
        manifest_name.starts_with("MANIFEST-"),
        "Manifest name must start with MANIFEST-, got: {}",
        manifest_name
    );
    let hex_part = manifest_name.strip_prefix("MANIFEST-").unwrap();
    assert_eq!(hex_part.len(), 16, "Manifest num must be 16-hex characters");
    assert!(
        u64::from_str_radix(hex_part, 16).is_ok(),
        "Manifest num must be valid hex"
    );

    let wal_dir = temp.path.join("wal");
    assert!(wal_dir.exists());
    let wal_entries = fs::read_dir(&wal_dir).unwrap();
    let mut wal_count = 0;
    for entry in wal_entries {
        let entry = entry.unwrap();
        let file_name = entry.file_name().into_string().unwrap();
        assert!(
            file_name.ends_with(".wal"),
            "WAL file must end with .wal: {}",
            file_name
        );
        let number = file_name.strip_suffix(".wal").unwrap();
        assert_eq!(number.len(), 16, "WAL num must be 16-hex characters");
        assert!(u64::from_str_radix(number, 16).is_ok());
        wal_count += 1;
    }
    assert!(wal_count >= 1);

    let scopes_dir = temp.path.join("scopes");
    assert!(scopes_dir.exists());
    let scope_entries = fs::read_dir(&scopes_dir).unwrap();
    for entry in scope_entries {
        let entry = entry.unwrap();
        let directory_name = entry.file_name().into_string().unwrap();
        assert_eq!(
            directory_name.len(),
            8,
            "Scope dir must be 8-hex characters: {}",
            directory_name
        );
        assert!(u32::from_str_radix(&directory_name, 16).is_ok());
    }

    temp.close();
}
