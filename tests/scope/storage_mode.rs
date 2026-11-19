use crate::common;
use std::sync::Arc;
use storkv_core::vfs::{AnyFs, FaultInjectingFs, PosixFs};
use storkv_core::{Db, DbOptions, ScopeOptions, StorageMode};

fn scope_options(storage_mode: StorageMode) -> ScopeOptions {
    ScopeOptions {
        storage_mode,
        ..ScopeOptions::default()
    }
}

#[test]
fn test_in_memory_database_keeps_state_only_in_ram() {
    let db = Db::open_in_memory(DbOptions::default()).unwrap();
    let scope = db
        .create_scope("ephemeral", ScopeOptions::default())
        .unwrap();
    db.put(&scope, b"key", b"value").unwrap();
    assert_eq!(
        db.get(&scope, b"key").unwrap().as_deref(),
        Some(&b"value"[..])
    );
    drop(db);

    let reopened = Db::open_in_memory(DbOptions::default()).unwrap();
    assert!(
        reopened
            .inner()
            .scope_names
            .read()
            .get("ephemeral")
            .is_none(),
        "a new in-memory database must start empty"
    );
}

#[test]
fn test_pure_in_memory_scope_is_not_replayed_from_disk() {
    let mut temp = common::TempDb::new("scope_memory_pure");
    let scope_id;
    {
        let db = temp.db();
        let scope = db
            .create_scope("pure_memory", scope_options(StorageMode::InMemory))
            .unwrap();
        scope_id = scope.scope_id;
        db.put(&scope, b"volatile", b"value").unwrap();
        assert_eq!(
            db.get(&scope, b"volatile").unwrap().as_deref(),
            Some(&b"value"[..])
        );
    }
    temp.close();

    let reopened = Db::open(&temp.path, DbOptions::default()).unwrap();
    let handle = storkv_core::ScopeHandle {
        scope_id,
        name: "pure_memory".into(),
    };
    assert!(
        reopened.get(&handle, b"volatile").unwrap().is_none(),
        "pure in-memory scopes must not survive a restart"
    );
}

#[test]
fn test_in_memory_persistent_scope_recovers_from_snapshot() {
    let mut temp = common::TempDb::new("scope_memory_persistent");
    let scope_id;
    let snapshot_path;
    {
        let db = temp.db();
        let scope = db
            .create_scope(
                "memory_persistent",
                scope_options(StorageMode::InMemoryPersistent),
            )
            .unwrap();
        scope_id = scope.scope_id;
        snapshot_path = temp
            .path
            .join("snapshots")
            .join(format!("{:08x}.snapshot", scope_id));
        for index in 0..32 {
            let key = format!("key-{index:02}");
            let value = format!("value-{index}");
            db.put(&scope, key.as_bytes(), value.as_bytes()).unwrap();
        }
        db.flush().unwrap();
        assert!(
            snapshot_path.exists(),
            "flush must write a recovery snapshot"
        );
    }
    temp.close();

    let scope_dir = temp.path.join("scopes").join(format!("{:08x}", scope_id));
    let disk_ssts: Vec<_> = std::fs::read_dir(&scope_dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().ends_with(".sst"))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        disk_ssts.is_empty(),
        "memory-backed scopes must not write SSTables to disk"
    );

    let reopened = Db::open(&temp.path, DbOptions::default()).unwrap();
    let handle = storkv_core::ScopeHandle {
        scope_id,
        name: "memory_persistent".into(),
    };
    for index in 0..32 {
        let key = format!("key-{index:02}");
        let value = format!("value-{index}");
        assert_eq!(
            reopened.get(&handle, key.as_bytes()).unwrap().as_deref(),
            Some(value.as_bytes())
        );
    }
}

#[test]
fn test_mixed_storage_modes_survive_restart() {
    let mut temp = common::TempDb::new("scope_memory_mixed");
    let memory_id;
    let disk_id;
    {
        let db = temp.db();
        let memory = db
            .create_scope(
                "mixed_memory",
                scope_options(StorageMode::InMemoryPersistent),
            )
            .unwrap();
        let disk = db
            .create_scope("mixed_disk", scope_options(StorageMode::Disk))
            .unwrap();
        memory_id = memory.scope_id;
        disk_id = disk.scope_id;
        db.put(&memory, b"memory-key", b"memory-value").unwrap();
        db.put(&disk, b"disk-key", b"disk-value").unwrap();
    }
    temp.close();

    let reopened = Db::open(&temp.path, DbOptions::default()).unwrap();
    let memory = storkv_core::ScopeHandle {
        scope_id: memory_id,
        name: "mixed_memory".into(),
    };
    let disk = storkv_core::ScopeHandle {
        scope_id: disk_id,
        name: "mixed_disk".into(),
    };
    assert_eq!(
        reopened.get(&memory, b"memory-key").unwrap().as_deref(),
        Some(&b"memory-value"[..])
    );
    assert_eq!(
        reopened.get(&disk, b"disk-key").unwrap().as_deref(),
        Some(&b"disk-value"[..])
    );
}

#[test]
fn test_fault_injected_wal_failure_rejects_memory_persistent_write() {
    let root =
        std::env::temp_dir().join(format!("storkv_scope_memory_fault_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let faulty = Arc::new(FaultInjectingFs::new(PosixFs));
    let fs = Arc::new(AnyFs::Disk(faulty.clone()));
    let inner =
        Arc::new(storkv_core::DbInner::open(fs, root.clone(), DbOptions::default()).unwrap());
    let scope = inner
        .create_scope(
            "fault_memory".into(),
            scope_options(StorageMode::InMemoryPersistent),
        )
        .unwrap();

    faulty.set_fail_writes(true);
    let mut transaction = inner.begin_tx();
    transaction.put(&scope, b"key", b"value").unwrap();
    let result = transaction.commit();
    faulty.set_fail_writes(false);

    assert!(result.is_err(), "a failed WAL write must reject the commit");
    let _ = std::fs::remove_dir_all(&root);
}
