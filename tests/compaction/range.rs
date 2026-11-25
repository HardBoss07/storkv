use crate::common;
use bytes::Bytes;
use storkv_core::format::{InternalKey, ValueType};
use storkv_core::sstable::SstReader;
use storkv_core::vfs::{AnyFs, FileSystem, PosixFs};
use storkv_core::{ScopeOptions, StorkvError};

type TestScope = storkv_core::scope::Scope<AnyFs<PosixFs>>;

fn scope_arc(db: &storkv_core::Db, scope_id: u32) -> std::sync::Arc<TestScope> {
    db.inner()
        .scopes
        .read()
        .get(&scope_id)
        .cloned()
        .expect("live scope")
}

fn file_numbers(scope: &TestScope, level: u8) -> Vec<u64> {
    let mut numbers: Vec<u64> = scope
        .sst_levels
        .read()
        .get(&level)
        .map(|files| files.iter().map(|file| file.file_num).collect())
        .unwrap_or_default();
    numbers.sort_unstable();
    numbers
}

fn sst_entries(scope: &TestScope) -> Vec<(InternalKey, Bytes)> {
    let levels = scope.sst_levels.read();
    let mut entries = Vec::new();
    for files in levels.values() {
        for metadata in files {
            let path = scope
                .scope_dir
                .join(format!("{:016x}.sst", metadata.file_num));
            let mut file = scope.fs.open_file(&path).expect("open sstable");
            let reader = SstReader::open_cached(
                &mut file,
                &scope.block_cache,
                scope.scope_id,
                metadata.file_num,
                false,
            )
            .expect("open reader");
            for (encoded_key, value) in reader
                .read_all_entries_cached(
                    &mut file,
                    &scope.block_cache,
                    scope.scope_id,
                    metadata.file_num,
                    false,
                )
                .expect("read entries")
            {
                if let Some(internal_key) = InternalKey::decode(encoded_key) {
                    entries.push((internal_key, value));
                }
            }
        }
    }
    entries
}

#[test]
fn compact_range_leaves_disjoint_sstables_untouched() {
    let temp = common::TempDb::new("comp_range_disjoint");
    let db = temp.db();
    let scope = db
        .create_scope("range_only", ScopeOptions::default())
        .unwrap();
    for prefix in ["a", "m", "z"] {
        for index in 0..4 {
            db.put(&scope, format!("{prefix}:{index}"), "v").unwrap();
        }
        db.flush().unwrap();
    }

    let scope_arc = scope_arc(db, scope.scope_id);
    let l0_before = file_numbers(&scope_arc, 0);
    assert_eq!(l0_before.len(), 3);
    let mut selected = l0_before.clone();
    // The middle file holds the only keys in [m, z). The z file starts at
    // `z`, so the half-open end must exclude it.
    let z_file = *l0_before.last().unwrap();
    selected.retain(|file| *file != z_file);

    db.compact_range("range_only", Some(b"m"), Some(b"z"))
        .unwrap();

    let l0_after = file_numbers(&scope_arc, 0);
    assert_eq!(l0_after.len(), 2, "only the overlapping file leaves L0");
    assert!(
        l0_after.contains(&z_file),
        "the file at the exclusive bound stays"
    );
    assert_eq!(file_numbers(&scope_arc, 1).len(), 1);

    for prefix in ["a", "m", "z"] {
        for index in 0..4 {
            let key = format!("{prefix}:{index}");
            assert_eq!(
                db.get(&scope, key.as_bytes()).unwrap().as_deref(),
                Some(b"v".as_slice()),
                "{key} must survive"
            );
        }
    }
}

#[test]
fn compact_range_reclaims_delete_range_tombstones() {
    let temp = common::TempDb::new("comp_range_delete");
    let db = temp.db();
    let scope = db
        .create_scope("range_delete", ScopeOptions::default())
        .unwrap();
    for key in [b"a", b"b", b"c", b"d", b"e", b"f"] {
        db.put(&scope, key, b"value").unwrap();
    }
    db.flush().unwrap();
    db.delete_range(&scope, b"b", b"e").unwrap();
    db.flush().unwrap();

    let scope_arc = scope_arc(db, scope.scope_id);
    assert!(
        sst_entries(&scope_arc)
            .iter()
            .any(|(key, _)| key.value_type == ValueType::RangeTombstone)
    );

    db.compact_range("range_delete", Some(b"b"), Some(b"e"))
        .unwrap();

    assert_eq!(db.get(&scope, b"b").unwrap(), None);
    assert_eq!(db.get(&scope, b"c").unwrap(), None);
    assert_eq!(db.get(&scope, b"d").unwrap(), None);
    assert_eq!(
        db.get(&scope, b"a").unwrap().as_deref(),
        Some(b"value".as_slice())
    );
    assert_eq!(
        db.get(&scope, b"e").unwrap().as_deref(),
        Some(b"value".as_slice())
    );
    assert!(
        !sst_entries(&scope_arc)
            .iter()
            .any(|(key, _)| key.value_type == ValueType::RangeTombstone)
    );
}

#[test]
fn compact_range_merges_files_that_share_the_output_span() {
    let temp = common::TempDb::new("comp_range_span");
    let db = temp.db();
    let scope = db
        .create_scope("range_span", ScopeOptions::default())
        .unwrap();
    for key in [b"a", b"b", b"c"] {
        db.put(&scope, key, b"value").unwrap();
        db.flush().unwrap();
    }
    db.put(&scope, b"d", b"value").unwrap();
    db.flush().unwrap();
    db.compact().unwrap();

    let scope_arc = scope_arc(db, scope.scope_id);
    assert_eq!(file_numbers(&scope_arc, 1).len(), 1);

    // One wide L0 file spans [a, z], so the selected output reaches the L1
    // file that covers [a, d] even though the range itself starts at `m`.
    for key in [b"a", b"m", b"z"] {
        db.put(&scope, key, b"new").unwrap();
    }
    db.flush().unwrap();
    db.compact_range("range_span", Some(b"m"), Some(b"n"))
        .unwrap();

    assert_eq!(file_numbers(&scope_arc, 0).len(), 0);
    assert_eq!(file_numbers(&scope_arc, 1).len(), 1);
    assert_eq!(
        db.get(&scope, b"a").unwrap().as_deref(),
        Some(b"new".as_slice())
    );
    assert_eq!(
        db.get(&scope, b"d").unwrap().as_deref(),
        Some(b"value".as_slice())
    );
    assert_eq!(
        db.get(&scope, b"z").unwrap().as_deref(),
        Some(b"new".as_slice())
    );
}

#[test]
fn compact_range_validates_bounds_and_scope() {
    let temp = common::TempDb::new("comp_range_bounds");
    let db = temp.db();
    db.create_scope("range_bounds", ScopeOptions::default())
        .unwrap();

    assert!(matches!(
        db.compact_range("range_bounds", Some(b"z"), Some(b"z")),
        Err(StorkvError::InvalidOperation(_))
    ));
    assert!(matches!(
        db.compact_range("missing", None, None),
        Err(StorkvError::ScopeNotFound(_))
    ));
    db.compact_range("range_bounds", Some(b"z".as_slice()), None)
        .unwrap();
    db.compact_range("range_bounds", None, Some(b"a".as_slice()))
        .unwrap();
    db.compact_range("range_bounds", None, None).unwrap();
}
