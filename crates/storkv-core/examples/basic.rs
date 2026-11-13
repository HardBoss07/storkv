use std::time::Duration;
use storkv_core::{
    CompactionStyle, Db, DbOptions, Durability, MergeOp, Result, Retention, ScopeOptions,
};

fn main() -> Result<()> {
    let temp = std::env::temp_dir().join(format!("storkv_example_basic_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);

    let db = Db::open(&temp, DbOptions::default())?;

    let sessions = db.create_scope(
        "sessions",
        ScopeOptions {
            durability: Durability::Buffered {
                sync_interval: Duration::from_millis(100),
            },
            retention: Retention::Ttl {
                default_ttl: Duration::from_secs(900),
                compaction_style: CompactionStyle::Fifo,
            },
            prefix_bloom_len: Some(8),
        },
    )?;

    let accounts = db.create_scope(
        "accounts",
        ScopeOptions {
            durability: Durability::StrictFsync,
            retention: Retention::Permanent,
            prefix_bloom_len: None,
        },
    )?;

    let mut tx = db.begin_tx();
    tx.put(&accounts, b"user:42:balance", b"5000")?;
    tx.put_with_ttl(
        &sessions,
        b"sess:abc987",
        b"user:42",
        Duration::from_secs(1800),
    )?;
    tx.commit()?;

    let mut read_tx = db.begin_tx();
    if let Some(session_owner) = read_tx.get(&sessions, b"sess:abc987")? {
        assert_eq!(session_owner.as_ref(), b"user:42");
    }

    db.merge(&accounts, b"user:42:login_count", MergeOp::CounterAdd(1))?;

    db.create_snapshot("pre-migration-v2")?;
    let cp_dir = temp.join("backups").join("cp-2026-11-13");
    db.checkpoint(&cp_dir)?;

    let _ = std::fs::remove_dir_all(&temp);
    println!("StorKV basic example completed successfully!");
    Ok(())
}
