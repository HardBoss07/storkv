# StorKV

A scoped, crash-hardened MVCC key-value storage engine and operational observability suite built in Rust.

StorKV embeds directly as a zero-overhead Rust library (`storkv-core`) or runs as a standalone networked daemon (`storkvd`) with live `pg_top`-style terminal diagnostics (`storkv-top`) and operational tooling (`storkvctl`).

---

## Architecture Overview

```text
+-------------------------------------------------------------------------+
|                           Client / Embedded API                         |
|   Db::open()  |  ScopeHandle  |  MVCC Transaction (OCC)  | Watch Stream |
+------------------------------------+------------------------------------+
                                     |
            +------------------------+------------------------+
            v                                                 v
+-----------------------+                         +-----------------------+
|   Scope 0 ("default") |                         |  Scope 1 ("sessions") |
|  Retention::Permanent |                         |  Retention::Ttl(15m)  |
|  Durability::Strict   |                         |  Durability::Buffered |
+-----------+-----------+                         +-----------+-----------+
            |                                                 |
            v                                                 v
   +-----------------+                               +-----------------+
   | Active MemTable |                               | Active MemTable |
   |  (SkipList)     |                               |  (SkipList)     |
   +--------+--------+                               +--------+--------+
            |                                                 |
            +------------------------+------------------------+
                                     |
                                     v
                  +-------------------------------------+
                  |      Shared Atomic Group-Commit     |
                  |     Write-Ahead Log (CRC32C WAL)    |
                  +------------------+------------------+
                                     |
            +------------------------+------------------------+
            v                                                 v
   +------------------+                               +------------------+
   |  L0..L6 SSTables |                              |  L0..L6 SSTables |
   | (Prefix Bloom +  |                              |  (FIFO / TTL     |
   |  LZ4 Blocks)     |                              |   Compaction)    |
   +------------------+                               +------------------+
```

---

## Target Public API (`storkv-core`)

```rust
use std::time::Duration;
use storkv_core::{
    Db, DbOptions, Durability, MergeOp, Retention, ScopeOptions, CompactionStyle, Result,
};

fn main() -> Result<()> {
    let db = Db::open("./data/storkv", DbOptions::default())?;

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
    db.checkpoint("./backups/cp-2026-11-13")?;

    Ok(())
}
```

---

## Repository Layout

```text
storkv/
├── Cargo.toml                          # Workspace manifest
├── SPEC.md                             # Architecture, MVCC rules, and crash guarantees
├── FORMAT.md                           # On-disk binary layouts (WAL, SSTable, MANIFEST)
├── crates/
│   ├── storkv-core/
│   │   └── src/
│   │       ├── lib.rs                  # Public Db, ScopeHandle, and Transaction API
│   │       ├── error.rs                # Structured StorKvError definitions
│   │       ├── vfs/                    # FileSystem trait (PosixFs + FaultInjectingFs)
│   │       ├── wal/                    # Group-commit WAL writer, CRC32C framing, recovery
│   │       ├── memtable/               # Lock-free skip list & immutable flush queue
│   │       ├── sstable/                # Prefix-compressed block builder, Bloom filter, reader
│   │       ├── scope/                  # Isolated Column Families, presets, WAL-pin scheduler
│   │       ├── mvcc/                   # Snapshot isolation, OCC read-set validation, MergeOps
│   │       ├── manifest/               # Atomic VersionEdit log & CURRENT pointer management
│   │       ├── compaction/             # Leveled & FIFO background compaction + TTL pruning
│   │       ├── watch/                  # Scoped Change-Data-Capture (CDC) event stream
│   │       └── telemetry/              # Lock-free atomic metrics exported to admin socket
│   ├── storkv-proto/                   # Length-prefixed binary wire protocol
│   ├── storkv-server/                  # Standalone TCP/UDS server (`storkvd`)
│   ├── storkv-client/                  # Async/sync Rust client SDK
│   ├── storkv-tui/
│   │   └── src/
│   │       └── bin/
│   │           └── storkv-top.rs       # Live ratatui observability & compaction dashboard
│   └── storkv-cli/
│       └── src/
│           └── bin/
│               └── storkvctl.rs        # Healthchecks, integrity doctor, hot backup, YCSB bench
└── tests/
    └── chaos/                          # Deterministic SIGKILL & torn-write verification suite
```

---

## Operational Tooling Quickstart

```bash
# Start the standalone daemon with dedicated admin Unix socket
cargo run --release --bin storkvd -- --data-dir ./data/storkv --admin-sock /tmp/storkv-admin.sock

# Launch live terminal telemetry dashboard (amplification, L0-L6 levels, WAL fsync p99)
cargo run --release --bin storkv-top -- --sock /tmp/storkv-admin.sock

# Run one-shot health probe (for Docker / systemd healthchecks)
cargo run --release --bin storkvctl -- health --sock /tmp/storkv-admin.sock

# Verify all SSTable footers, block CRC32C checksums, and MANIFEST invariants
cargo run --release --bin storkvctl -- doctor --data-dir ./data/storkv

# Create a zero-copy hard-linked MVCC checkpoint
cargo run --release --bin storkvctl -- backup --sock /tmp/storkv-admin.sock --out ./backups/snap-01
```
