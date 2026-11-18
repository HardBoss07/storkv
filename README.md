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
│   │       ├── backup/                 # Autonomous backup, sinks, retention, PITR restore
│   │       ├── compat.rs               # Embedded sled-style `Tree` / `Batch` adapter
│   │       └── telemetry/              # Lock-free atomic metrics exported to admin socket
│   ├── storkv-proto/                   # Length-prefixed binary wire protocol
│   ├── storkv-server/                  # Standalone daemon (`storkvd`)
│   │   └── src/
│   │       ├── transports/             # Native TCP/UDS, HTTP/REST, WebSocket, gRPC
│   │       └── compat/                 # Redis RESP, Memcached ASCII, etcd v3 HTTP gateway
│   ├── storkv-client/                  # Async/sync client with unified `connect_uri`
│   ├── storkv-tui/
│   │   └── src/
│   │       └── bin/
│   │           └── storkv-top.rs       # Live ratatui observability & compaction dashboard
│   └── storkv-cli/
│       └── src/
│           └── bin/
│               └── storkvctl.rs        # Health, doctor, backup/PITR, migrate, YCSB bench
├── docs/
│   ├── FEATURE_PLAN.md
│   ├── TRANSPORTS_AND_COMPATIBILITY.md
│   └── OPERATIONS_AND_BACKUP.md
└── tests/
    ├── backup_watch/                   # Continuous backup, retention, PITR, corruption
    ├── transports/                     # Native, HTTP, WebSocket, and gRPC roundtrips
    ├── compat/                         # Redis, Memcached, etcd, Tree, and migrate
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

# Run continuous backup with sealed-WAL shipping
cargo run --release --bin storkvd -- --data-dir ./data/storkv \
  --backup-dir ./backups --backup-ship-wal --backup-interval-secs 3600

# Inspect and verify autonomous backups
cargo run --release --bin storkvctl -- backup-status --sock 127.0.0.1:7878
cargo run --release --bin storkvctl -- backup-verify --sock 127.0.0.1:7878

# Restore a point in time into a fresh directory
cargo run --release --bin storkvctl -- restore \
  --backup-dir ./backups --out-dir ./restore/storkv --target-seq 18422
```

---

## Access Paths

One engine, one data directory, multiple protocols. Choose the path that fits
the client; all of them share scopes, MVCC transactions, telemetry, and backup
coverage.

| Path                          | Enable with                   | Use for                                     |
| :---------------------------- | :---------------------------- | :------------------------------------------ |
| Native binary (`STORKPRT` v1) | `--listen`, `--admin-sock`    | StorKV clients over TCP or Unix sockets     |
| HTTP REST                     | `--http-listen`               | curl, scripts, language-agnostic CRUD/admin |
| WebSocket                     | `--ws-listen` or `/v1/ws`     | Browser tooling and Watch streaming         |
| gRPC                          | `--grpc-listen`               | Generated typed clients                     |
| Redis RESP2/RESP3             | `--redis-listen`              | Existing Redis applications                 |
| Memcached ASCII               | `--memcached-listen`          | Existing Memcached applications             |
| etcd v3 JSON                  | `/v3/kv/*` on `--http-listen` | etcd HTTP tooling                           |
| Embedded Rust `Tree`          | `db.open_tree("name")`        | In-process sled-style access                |

The unified client accepts `storkv://`, `unix://`, `http://`, `ws://`, and
`grpc://` URIs through `AsyncClient::connect_uri` and
`SyncClient::connect_uri`. See
[docs/TRANSPORTS_AND_COMPATIBILITY.md](docs/TRANSPORTS_AND_COMPATIBILITY.md)
for protocol coverage and configuration, and
[docs/OPERATIONS_AND_BACKUP.md](docs/OPERATIONS_AND_BACKUP.md) for running,
backing up, restoring, and migrating a deployment.
