//! Stateful model oracle: run random operation sequences against a real `Db`
//! and compare every visible read with a plain `BTreeMap` model.

use std::collections::BTreeMap;
use std::time::Duration;

use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use storkv_core::{Db, DbOptions, MergeOp, Result, ScopeHandle, ScopeOptions};

use crate::common;

const PREFIXES: [&[u8]; 4] = [b"a:", b"b:", b"c:", b"d:"];
const PROBE_PREFIXES: [&[u8]; 4] = [b"a:", b"b:", b"c:", b"d:"];

/// One model operation. `Reopen` closes and reopens the database directory.
#[derive(Debug, Clone)]
enum Operation {
    Put { key: Vec<u8>, value: Vec<u8> },
    PutWithTtl { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
    DeleteRange { start: Vec<u8>, end: Vec<u8> },
    Merge { key: Vec<u8>, delta: i64 },
    Flush,
    Compact,
    Reopen,
}

fn key_strategy() -> impl Strategy<Value = Vec<u8>> {
    (0..PREFIXES.len(), 0u8..8).prop_map(|(prefix, suffix)| {
        let mut key = PREFIXES[prefix].to_vec();
        key.push(b'0' + suffix);
        key
    })
}

fn value_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..16)
}

fn operation_strategy() -> impl Strategy<Value = Operation> {
    prop_oneof![
        4 => (key_strategy(), value_strategy())
            .prop_map(|(key, value)| Operation::Put { key, value }),
        1 => (key_strategy(), value_strategy())
            .prop_map(|(key, value)| Operation::PutWithTtl { key, value }),
        2 => key_strategy().prop_map(|key| Operation::Delete { key }),
        // The engine defines the range as `[start, end)`, so keep the bounds
        // ordered and never pass an inverted range.
        1 => (key_strategy(), key_strategy()).prop_map(|(start, end)| {
            let (start, end) = if start <= end { (start, end) } else { (end, start) };
            Operation::DeleteRange { start, end }
        }),
        2 => (key_strategy(), -20i64..20)
            .prop_map(|(key, delta)| Operation::Merge { key, delta }),
        1 => Just(Operation::Flush),
        1 => Just(Operation::Compact),
        1 => Just(Operation::Reopen),
    ]
}

fn signed_counter(bytes: &[u8]) -> i64 {
    if let Ok(array) = <[u8; 8]>::try_from(bytes) {
        return i64::from_le_bytes(array);
    }
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.parse::<i64>().ok())
        .unwrap_or(0)
}

fn apply(db: &Db, scope: &ScopeHandle, model: &mut BTreeMap<Vec<u8>, Vec<u8>>, op: &Operation) {
    match op {
        Operation::Put { key, value } => {
            db.put(scope, key, value).expect("model put");
            model.insert(key.clone(), value.clone());
        }
        Operation::PutWithTtl { key, value } => {
            db.put_with_ttl(scope, key, value, Duration::from_secs(3600))
                .expect("model put with ttl");
            model.insert(key.clone(), value.clone());
        }
        Operation::Delete { key } => {
            db.delete(scope, key).expect("model delete");
            model.remove(key);
        }
        Operation::DeleteRange { start, end } => {
            db.delete_range(scope, start, end)
                .expect("model delete range");
            model.retain(|key, _| key < start || key >= end);
        }
        Operation::Merge { key, delta } => {
            db.merge(scope, key, MergeOp::CounterAdd(*delta))
                .expect("model merge");
            let total = signed_counter(model.get(key).map(Vec::as_slice).unwrap_or_default())
                .wrapping_add(*delta);
            model.insert(key.clone(), total.to_string().into_bytes());
        }
        Operation::Flush => db.flush().expect("model flush"),
        Operation::Compact => db.compact().expect("model compact"),
        Operation::Reopen => {}
    }
}

fn reopen(temp: &mut common::TempDb) {
    temp.close();
    let db = Db::open(&temp.path, DbOptions::default()).expect("reopen database");
    temp.db = Some(db);
}

fn assert_reads(db: &Db, scope: &ScopeHandle, model: &BTreeMap<Vec<u8>, Vec<u8>>) {
    for (key, expected) in model {
        let actual = db.get(scope, key).expect("model get");
        assert_eq!(actual.as_deref(), Some(expected.as_slice()), "key {key:?}");
    }
    let all = db
        .scan(scope, None::<&[u8]>, None::<&[u8]>)
        .expect("model scan");
    let actual_all: Vec<(Vec<u8>, Vec<u8>)> = all
        .into_iter()
        .map(|(key, value)| (key.to_vec(), value.to_vec()))
        .collect();
    let expected_all: Vec<(Vec<u8>, Vec<u8>)> = model
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    assert_eq!(actual_all, expected_all, "full scan");
    for prefix in PROBE_PREFIXES {
        let actual = db.scan_prefix(scope, prefix).expect("model prefix scan");
        let actual: Vec<(Vec<u8>, Vec<u8>)> = actual
            .into_iter()
            .map(|(key, value)| (key.to_vec(), value.to_vec()))
            .collect();
        let expected: Vec<(Vec<u8>, Vec<u8>)> = model
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        assert_eq!(actual, expected, "prefix scan {prefix:?}");
    }
}

fn run_sequence(operations: &[Operation]) {
    let mut temp = common::TempDb::new("proptest_oracle");
    let scope = temp
        .db()
        .create_scope("oracle", ScopeOptions::default())
        .expect("create oracle scope");
    let mut model = BTreeMap::new();

    for operation in operations {
        if matches!(operation, Operation::Reopen) {
            reopen(&mut temp);
        } else {
            apply(temp.db(), &scope, &mut model, operation);
        }
        assert_reads(temp.db(), &scope, &model);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn random_operations_match_the_btreemap_oracle(
        operations in prop::collection::vec(operation_strategy(), 1..24)
    ) {
        run_sequence(&operations);
    }
}

#[test]
fn ttl_expiry_matches_the_oracle() -> Result<()> {
    let temp = common::TempDb::new("proptest_oracle_ttl");
    let db = temp.db().clone();
    let scope = db.create_scope("ttl_oracle", ScopeOptions::default())?;

    db.put_with_ttl(&scope, b"ttl:short", b"value", Duration::from_millis(20))?;
    db.put(&scope, b"ttl:permanent", b"value")?;
    assert_eq!(db.get(&scope, b"ttl:short")?, Some("value".into()));

    std::thread::sleep(Duration::from_millis(60));

    assert_eq!(db.get(&scope, b"ttl:short")?, None, "expired key is hidden");
    assert_eq!(db.get(&scope, b"ttl:permanent")?, Some("value".into()));
    Ok(())
}
