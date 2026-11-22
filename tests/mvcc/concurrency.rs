use crate::common;
use bytes::Bytes;
use std::sync::Arc;
use std::time::Duration;
use storkv_core::{Durability, Retention, ScopeOptions, StorkvError};

#[test]
fn test_50_worker_occ_contention_storm_converges() {
    let mut temp = common::TempDb::new("mvcc_storm");
    let db = Arc::new(temp.db.take().unwrap());
    let scope = db
        .create_scope(
            "storm_scope",
            ScopeOptions {
                durability: Durability::Buffered {
                    sync_interval: Duration::from_millis(5),
                },
                retention: Retention::Permanent,
                prefix_bloom_len: None,
                storage_mode: storkv_core::StorageMode::Disk,
                ..ScopeOptions::default()
            },
        )
        .unwrap();
    db.put(&scope, b"counter", b"0").unwrap();

    let workers = 50;
    let increments_per_worker = 5;
    let mut handles = Vec::new();
    for _ in 0..workers {
        let db = db.clone();
        let scope = scope.clone();
        handles.push(std::thread::spawn(move || {
            for _ in 0..increments_per_worker {
                let mut committed = false;
                for _ in 0..100_000 {
                    let mut transaction = db.begin_tx();
                    let current = transaction
                        .get(&scope, b"counter")
                        .unwrap()
                        .unwrap_or_else(|| Bytes::from_static(b"0"));
                    let value: i64 = std::str::from_utf8(&current).unwrap().parse().unwrap();
                    transaction
                        .put(&scope, b"counter", (value + 1).to_string())
                        .unwrap();
                    match transaction.commit() {
                        Ok(()) => {
                            committed = true;
                            break;
                        }
                        Err(StorkvError::WriteConflict { .. }) => continue,
                        Err(other) => panic!("unexpected commit error: {other:?}"),
                    }
                }
                assert!(committed, "OCC retry did not converge");
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    let final_value = db.get(&scope, b"counter").unwrap().unwrap();
    let expected = (workers * increments_per_worker).to_string();
    assert_eq!(final_value.as_ref(), expected.as_bytes());
}

#[test]
fn test_50_worker_blind_writes_never_conflict() {
    let mut temp = common::TempDb::new("mvcc_blind");
    let db = Arc::new(temp.db.take().unwrap());
    let scope = db
        .create_scope("blind_scope", ScopeOptions::default())
        .unwrap();

    let workers = 50usize;
    let keys_per_worker = 20usize;
    let mut handles = Vec::new();
    for worker in 0..workers {
        let db = db.clone();
        let scope = scope.clone();
        handles.push(std::thread::spawn(move || {
            for index in 0..keys_per_worker {
                let key = format!("blind:{:03}:{:03}", worker, index);
                db.put(&scope, key.as_bytes(), b"value").unwrap();
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    let all_entries = db.scan_prefix(&scope, b"blind:").unwrap();
    assert_eq!(all_entries.len(), workers * keys_per_worker);
}

#[test]
fn test_atomic_transaction_spanning_ten_scopes() {
    let temp = common::TempDb::new("mvcc_ten_scopes");
    let db = temp.db();

    let mut scopes = Vec::new();
    for index in 0..10 {
        scopes.push(
            db.create_scope(format!("tx_scope_{}", index), ScopeOptions::default())
                .unwrap(),
        );
    }

    let mut transaction = db.begin_tx();
    for (index, scope) in scopes.iter().enumerate() {
        transaction
            .put(scope, format!("key:{}", index).as_bytes(), b"committed")
            .unwrap();
    }
    transaction.commit().unwrap();

    for (index, scope) in scopes.iter().enumerate() {
        let value = db
            .get(scope, format!("key:{}", index).as_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(value.as_ref(), b"committed");
    }
}
