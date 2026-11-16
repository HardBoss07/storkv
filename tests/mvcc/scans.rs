use crate::common;
use bytes::Bytes;
use std::time::Duration;
use storkv_core::ScopeOptions;

#[test]
fn test_delete_range_and_scans() {
    let mut temp = common::TempDb::new("mvcc_scans");
    let db = temp.db();

    let scope = db
        .create_scope("range_scope", ScopeOptions::default())
        .unwrap();

    for index in 0..10 {
        let key = format!("user:{:02}", index);
        let value = format!("val:{:02}", index);
        db.put(&scope, key.as_bytes(), value.as_bytes()).unwrap();
    }

    let prefix_items = db.scan_prefix(&scope, b"user:").unwrap();
    assert_eq!(prefix_items.len(), 10);

    let range_items = db
        .scan(
            &scope,
            Some(b"user:02".as_slice()),
            Some(b"user:07".as_slice()),
        )
        .unwrap();
    assert_eq!(range_items.len(), 5);

    db.delete_range(&scope, b"user:03".as_slice(), b"user:06".as_slice())
        .unwrap();

    let remaining = db.scan_prefix(&scope, b"user:").unwrap();
    assert_eq!(remaining.len(), 7);

    temp.close();
}

#[test]
fn test_large_scan_with_interleaved_delete_range_and_ttl() {
    let temp = common::TempDb::new("mvcc_large_scan");
    let db = temp.db();
    let scope = db
        .create_scope("scan_scope", ScopeOptions::default())
        .unwrap();

    for index in 0..1_000 {
        db.put(
            &scope,
            format!("k:{:04}", index).as_bytes(),
            format!("v:{:04}", index).as_bytes(),
        )
        .unwrap();
    }
    for index in 0..100 {
        db.put_with_ttl(
            &scope,
            format!("t:{:04}", index).as_bytes(),
            b"ttl",
            Duration::from_millis(20),
        )
        .unwrap();
    }

    db.delete_range(&scope, b"k:0200", b"k:0400").unwrap();
    std::thread::sleep(Duration::from_millis(50));

    let remaining = db.scan_prefix(&scope, b"k:").unwrap();
    assert_eq!(remaining.len(), 800);
    assert_eq!(db.scan_prefix(&scope, b"t:").unwrap().len(), 0);
    assert!(db.get(&scope, b"k:0199").unwrap().is_some());
    assert!(db.get(&scope, b"k:0400").unwrap().is_some());
    let keys: Vec<Bytes> = remaining.into_iter().map(|(key, _)| key).collect();
    assert!(!keys.iter().any(|key| key.as_ref() == b"k:0200"));
    assert!(!keys.iter().any(|key| key.as_ref() == b"k:0399"));
}

#[test]
fn test_range_tombstone_boundary_semantics() {
    let temp = common::TempDb::new("mvcc_range_boundary");
    let db = temp.db();
    let scope = db
        .create_scope("range_boundary", ScopeOptions::default())
        .unwrap();

    for index in 0..10 {
        db.put(&scope, format!("a{}", index).as_bytes(), b"v")
            .unwrap();
    }
    db.delete_range(&scope, b"a3", b"a7").unwrap();

    let remaining = db.scan_prefix(&scope, b"a").unwrap();
    let keys: Vec<&[u8]> = remaining.iter().map(|(key, _)| key.as_ref()).collect();
    assert_eq!(remaining.len(), 6);
    assert_eq!(
        keys,
        vec![
            b"a0".as_slice(),
            b"a1".as_slice(),
            b"a2".as_slice(),
            b"a7".as_slice(),
            b"a8".as_slice(),
            b"a9".as_slice(),
        ]
    );
}
