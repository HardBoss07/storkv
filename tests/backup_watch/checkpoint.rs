use std::fs;

use storkv_core::ScopeOptions;

#[test]
fn test_zero_copy_checkpoint_hard_links() {
    let mut temp = crate::common::TempDb::new("cp_hardlinks");
    let db = temp.db();

    let scope = db
        .create_scope("checkpoint_scope", ScopeOptions::default())
        .unwrap();

    for index in 0..50 {
        let k = format!("cp_key_{}", index);
        let v = format!("cp_val_{}", index);
        db.put(&scope, k.as_bytes(), v.as_bytes()).unwrap();
    }

    // Force flush
    let scope_arc = db
        .inner()
        .scopes
        .read()
        .get(&scope.scope_id)
        .cloned()
        .unwrap();
    db.inner().flush_scope_memtable(&scope_arc).unwrap();

    let cp_dir = temp.path.join("checkpoints").join("cp1");
    db.checkpoint(&cp_dir).unwrap();

    assert!(cp_dir.exists());
    assert!(cp_dir.join("CURRENT").exists());

    // Verify SSTables exist in checkpoint
    let cp_scope_dir = cp_dir
        .join("scopes")
        .join(format!("{:08x}", scope.scope_id));
    assert!(cp_scope_dir.exists());

    let original_scope_dir = temp
        .path
        .join("scopes")
        .join(format!("{:08x}", scope.scope_id));
    let orig_entries = fs::read_dir(&original_scope_dir).unwrap();
    for entry in orig_entries {
        let entry = entry.unwrap();
        let fname = entry.file_name();
        if fname.to_string_lossy().ends_with(".sst") {
            let cp_sst = cp_scope_dir.join(&fname);
            assert!(cp_sst.exists());
        }
    }

    temp.close();
}

#[test]
fn test_named_snapshots_and_unpin() {
    let mut temp = crate::common::TempDb::new("snap_test");
    let db = temp.db();

    let scope = db
        .create_scope("snap_scope", ScopeOptions::default())
        .unwrap();

    db.put(&scope, b"key1", b"val1").unwrap();

    db.create_snapshot("release_1.0").unwrap();
    assert!(
        db.inner()
            .manifest_mgr
            .state()
            .pinned_snapshots
            .contains_key("release_1.0"),
        "Named snapshot must be recorded in manifest"
    );

    db.unpin_snapshot("release_1.0").unwrap();
    assert!(
        !db.inner()
            .manifest_mgr
            .state()
            .pinned_snapshots
            .contains_key("release_1.0"),
        "Unpinned snapshot must be removed from manifest"
    );

    temp.close();
}
