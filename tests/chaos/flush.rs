use std::fs;
use std::sync::Arc;

use storkv_core::vfs::{FaultInjectingFs, PosixFs};
use storkv_core::{DbInner, DbOptions, ScopeHandle, ScopeOptions};

type ChaosFs = Arc<FaultInjectingFs<PosixFs>>;

fn chaos_fs() -> Arc<ChaosFs> {
    Arc::new(Arc::new(FaultInjectingFs::new(PosixFs)))
}

fn temp_root(label: &str) -> std::path::PathBuf {
    let root =
        std::env::temp_dir().join(format!("storkv_chaos_flush_{label}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A torn write while the MemTable is being flushed to a fresh SSTable must
/// surface as a structured error, leave the WAL authoritative, and recover
/// without silent data loss once the filesystem heals.
#[test]
fn torn_memtable_flush_is_reported_and_recoverable_from_wal() {
    let root = temp_root("torn");
    let faulty = chaos_fs();
    let db = Arc::new(
        DbInner::open(
            faulty.clone(),
            root.clone(),
            DbOptions {
                max_wal_pin_bytes: 1,
                ..DbOptions::default()
            },
        )
        .unwrap(),
    );
    let scope = db
        .create_scope("flush_scope".to_string(), ScopeOptions::default())
        .unwrap();
    let handle = ScopeHandle {
        scope_id: scope.scope_id,
        name: scope.name.clone(),
    };

    // Seed one healthy flush so the failure below targets a second rotation.
    let mut seed = db.begin_tx();
    seed.put(&handle, b"seed", b"seed-value").unwrap();
    seed.commit().unwrap();

    // WAL and MANIFEST descriptors snapshot the byte limit at open time, so
    // only the newly created SSTable is truncated to a few bytes.
    faulty.set_write_byte_limit(16);
    let mut doomed = db.begin_tx();
    doomed.put(&handle, b"doomed", b"doomed-value").unwrap();
    let error = doomed
        .commit()
        .expect_err("torn SSTable write must fail the commit");
    assert!(
        matches!(error, storkv_core::StorKvError::Io(_)),
        "unexpected error: {error:?}"
    );
    faulty.set_write_byte_limit(usize::MAX);

    drop(db);

    let healed = chaos_fs();
    let reopened = DbInner::open(healed, root.clone(), DbOptions::default())
        .expect("recovered database opens");
    assert_eq!(
        reopened.get(scope.scope_id, b"seed").unwrap().as_deref(),
        Some(b"seed-value".as_slice())
    );
    assert_eq!(
        reopened.get(scope.scope_id, b"doomed").unwrap().as_deref(),
        Some(b"doomed-value".as_slice()),
        "WAL replay must preserve the operation whose flush failed"
    );

    drop(reopened);
    let _ = fs::remove_dir_all(&root);
}
