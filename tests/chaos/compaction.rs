use std::fs;
use std::sync::Arc;

use storkv_core::vfs::{FaultInjectingFs, PosixFs};
use storkv_core::{DbInner, DbOptions, ScopeHandle, ScopeOptions};

type ChaosFs = Arc<FaultInjectingFs<PosixFs>>;

fn chaos_fs() -> Arc<ChaosFs> {
    Arc::new(Arc::new(FaultInjectingFs::new(PosixFs)))
}

fn temp_root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "storkv_chaos_compaction_{label}_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn flush_batch(db: &Arc<DbInner<ChaosFs>>, handle: &ScopeHandle, from: usize) {
    for index in from..from + 50 {
        let mut tx = db.begin_tx();
        tx.put(handle, format!("k:{index:04}"), format!("v:{index:04}"))
            .unwrap();
        tx.commit().unwrap();
    }
    let scope = db.scopes.read().get(&handle.scope_id).cloned().unwrap();
    db.flush_scope_memtable(&scope).unwrap();
}

/// Injecting a write failure midway through L0 -> L1 compaction must abort the
/// compaction cleanly, preserve the inputs, and allow a later retry to finish.
#[test]
fn failed_leveled_compaction_preserves_inputs_and_can_be_retried() {
    let root = temp_root("retry");
    let faulty = chaos_fs();
    let db = Arc::new(
        DbInner::open(
            faulty.clone(),
            root.clone(),
            DbOptions {
                max_wal_pin_bytes: usize::MAX,
                ..DbOptions::default()
            },
        )
        .unwrap(),
    );
    let scope = db
        .create_scope("compact_scope".to_string(), ScopeOptions::default())
        .unwrap();
    let handle = ScopeHandle {
        scope_id: scope.scope_id,
        name: scope.name.clone(),
    };

    flush_batch(&db, &handle, 0);
    flush_batch(&db, &handle, 50);
    flush_batch(&db, &handle, 100);
    flush_batch(&db, &handle, 150);
    let scope_arc = db.scopes.read().get(&scope.scope_id).cloned().unwrap();
    assert_eq!(scope_arc.sst_levels.read().get(&0).map(Vec::len), Some(4));

    faulty.set_fail_writes(true);
    let error = db
        .run_compaction()
        .expect_err("injected write failure must abort compaction");
    assert!(
        matches!(error, storkv_core::StorkvError::Io(_)),
        "unexpected error: {error:?}"
    );
    faulty.set_fail_writes(false);

    assert_eq!(
        scope_arc.sst_levels.read().get(&0).map(Vec::len),
        Some(4),
        "failed compaction must not drop its inputs"
    );
    assert_eq!(
        db.get(scope.scope_id, b"k:0007").unwrap().as_deref(),
        Some(b"v:0007".as_slice())
    );

    db.run_compaction().expect("compaction retry succeeds");
    assert_eq!(
        scope_arc.sst_levels.read().get(&0).map(Vec::len),
        Some(0),
        "retry must drain L0"
    );
    assert_eq!(
        db.get(scope.scope_id, b"k:0199").unwrap().as_deref(),
        Some(b"v:0199".as_slice())
    );

    drop(db);
    let healed = chaos_fs();
    let reopened = DbInner::open(healed, root.clone(), DbOptions::default()).unwrap();
    assert_eq!(
        reopened.get(scope.scope_id, b"k:0042").unwrap().as_deref(),
        Some(b"v:0042".as_slice())
    );

    drop(reopened);
    let _ = fs::remove_dir_all(&root);
}
