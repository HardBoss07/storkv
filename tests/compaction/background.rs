use crate::common;
use std::time::{Duration, Instant};
use storkv_core::compaction::RateLimiter;
use storkv_core::{DbOptions, Durability, ScopeOptions};

fn buffered_scope() -> ScopeOptions {
    ScopeOptions {
        durability: Durability::Buffered {
            sync_interval: Duration::from_millis(10),
        },
        ..ScopeOptions::default()
    }
}

#[test]
fn background_scheduler_compacts_flushed_l0_files() {
    let temp = common::TempDb::open_with_options(
        "bg_compaction",
        DbOptions {
            enable_background_compaction: true,
            l0_compaction_trigger: 1,
            max_background_compactions: 1,
            ..DbOptions::default()
        },
    );
    let db = temp.db();
    let scope = db
        .create_scope("background", storkv_core::ScopeOptions::default())
        .unwrap();

    for batch in 0..4u64 {
        for index in 0..100u64 {
            let id = batch * 100 + index;
            db.put(&scope, format!("k:{id:04}"), format!("v:{id:04}"))
                .unwrap();
        }
        let scope_arc = db
            .inner()
            .scopes
            .read()
            .get(&scope.scope_id)
            .cloned()
            .unwrap();
        db.inner().flush_scope_memtable(&scope_arc).unwrap();
    }

    let scope_arc = db
        .inner()
        .scopes
        .read()
        .get(&scope.scope_id)
        .cloned()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let levels = scope_arc.sst_levels.read();
        let l0 = levels.get(&0).map(Vec::len).unwrap_or(0);
        let l1 = levels.get(&1).map(Vec::len).unwrap_or(0);
        if l0 == 0 && l1 >= 1 {
            break;
        }
        drop(levels);
        assert!(
            Instant::now() < deadline,
            "background compaction did not run: L0={l0} L1={l1}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(db.scan_prefix(&scope, b"k:").unwrap().len(), 400);
}

#[test]
fn disabled_scheduler_makes_flush_trigger_a_noop() {
    let temp = common::TempDb::new("bg_disabled");
    let db = temp.db();
    let scope = db
        .create_scope("no_background", storkv_core::ScopeOptions::default())
        .unwrap();

    db.put(&scope, b"key", b"value").unwrap();
    let scope_arc = db
        .inner()
        .scopes
        .read()
        .get(&scope.scope_id)
        .cloned()
        .unwrap();
    db.inner().flush_scope_memtable(&scope_arc).unwrap();

    assert_eq!(
        scope_arc.sst_levels.read().get(&0).map(Vec::len),
        Some(1),
        "flush must not block on compaction when the scheduler is disabled"
    );
}

#[test]
fn background_scheduler_drains_multi_scope_backlog() {
    let temp = common::TempDb::open_with_options(
        "bg_backlog",
        DbOptions {
            enable_background_compaction: true,
            l0_compaction_trigger: 4,
            max_background_compactions: 2,
            ..DbOptions::default()
        },
    );
    let db = temp.db();
    // Build a backlog without waking the worker after each flush.
    db.inner().shutdown_compaction_scheduler();

    let mut scope_arcs = Vec::new();
    for scope_index in 0..4u64 {
        let scope = db
            .create_scope(format!("backlog_{scope_index}"), buffered_scope())
            .unwrap();
        for batch in 0..4u64 {
            for index in 0..16u64 {
                let id = batch * 16 + index;
                db.put(
                    &scope,
                    format!("k:{scope_index}:{id:04}"),
                    format!("v:{id:04}"),
                )
                .unwrap();
            }
            let scope_arc = db
                .inner()
                .scopes
                .read()
                .get(&scope.scope_id)
                .cloned()
                .unwrap();
            db.inner().flush_scope_memtable(&scope_arc).unwrap();
        }
        scope_arcs.push(
            db.inner()
                .scopes
                .read()
                .get(&scope.scope_id)
                .cloned()
                .unwrap(),
        );
    }

    for scope_arc in &scope_arcs {
        assert_eq!(
            scope_arc.sst_levels.read().get(&0).map(Vec::len),
            Some(4),
            "backlog must hold four L0 files per scope"
        );
    }

    db.inner().start_compaction_scheduler().unwrap();
    db.inner().trigger_compaction_from_flush().unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let drained = scope_arcs.iter().all(|scope_arc| {
            let levels = scope_arc.sst_levels.read();
            levels.get(&0).map(Vec::len).unwrap_or(0) == 0
                && levels.get(&1).map(Vec::len).unwrap_or(0) >= 1
        });
        if drained {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "one wake must drain every due scope, not only max_background_compactions"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn rate_limited_compaction_paces_writes_by_bytes() {
    let temp = common::TempDb::new("bg_rate_limit");
    let db = temp.db();
    let scope = db.create_scope("rate_limited", buffered_scope()).unwrap();

    let value = vec![b'v'; 128];
    for batch in 0..4u64 {
        for index in 0..1000u64 {
            let id = batch * 1000 + index;
            db.put(&scope, format!("k:{id:05}"), &value).unwrap();
        }
        let scope_arc = db
            .inner()
            .scopes
            .read()
            .get(&scope.scope_id)
            .cloned()
            .unwrap();
        db.inner().flush_scope_memtable(&scope_arc).unwrap();
    }

    let scope_arc = db
        .inner()
        .scopes
        .read()
        .get(&scope.scope_id)
        .cloned()
        .unwrap();
    let rate = 32 * 1024u64;
    let limiter = RateLimiter::new(rate);
    let started = Instant::now();
    let compacted = db
        .inner()
        .run_due_compactions(4, 1, Some(&limiter))
        .unwrap();
    let elapsed = started.elapsed();

    assert_eq!(compacted, 1, "the scope must compact exactly once");
    let l1_bytes: u64 = scope_arc
        .sst_levels
        .read()
        .get(&1)
        .map(|files| files.iter().map(|meta| meta.file_size).sum())
        .unwrap_or(0);
    assert!(
        l1_bytes > rate,
        "compacted output must exceed one second of budget, got {l1_bytes}"
    );
    let expected = Duration::from_secs_f64((l1_bytes - rate) as f64 / rate as f64);
    assert!(
        elapsed >= expected.mul_f64(0.5),
        "compaction must pause for its debt: elapsed {elapsed:?}, expected at least {expected:?}"
    );
}
