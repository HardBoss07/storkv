# StorKV Architecture & Guarantees Specification (v1.0)

## 1. System Overview

StorKV is an embeddable and network-capable Log-Structured Merge-tree (LSM) key-value storage engine written in Rust. It elevates multi-tenant isolation, expiration, atomic cross-namespace transactions, reactive change streams, and crash consistency from fragile application-layer patterns into core storage primitives.

### Design Principles

- **Explicit Configuration over Implicit Magic:** Every scope declares its durability, retention, and compaction behavior explicitly at creation time.
- **Zero-Allocation Hot Paths:** Internal keys, block slices, and wire frames rely on reference-counted byte buffers (`bytes::Bytes`) and prefix compression.
- **Crash-First Engineering:** Every persistent mutation is framed with `CRC32C` checksums and committed through atomic POSIX file replacement semantics.

---

## 2. Core Invariants

### 2.1 Scoped Column Families over a Shared Atomic WAL

- **Storage Isolation:** Each scope (e.g., `sessions`, `jobs`, `metadata`) owns an independent active `MemTable`, immutable flush queue, `SSTable` hierarchy (`L0` through `L6`), and compaction worker state.
- **Atomicity Across Scopes:** All scopes share a single segmented Write-Ahead Log (`wal/`). A `WriteBatch` or `Transaction` mutating keys across multiple scopes is serialized into a single checksummed WAL frame with one contiguous sequence range, guaranteeing all-or-nothing durability across scope boundaries.
- **O(1) Scope & Range Drops:** Dropping a scope (`db.drop_scope("sessions")`) writes a `DropScope` record to the WAL and `MANIFEST`, immediately hiding all keys at `seq <= drop_seq` without scanning individual records. Compaction reclaims orphaned `.sst` files asynchronously.

### 2.2 MVCC Snapshot Isolation, OCC & Merge Resolution

- **Monotonic Sequence Numbers:** The engine maintains a global atomic `u64` sequence counter (`last_visible_seq`). Every individual operation within a committed batch receives a strictly increasing sequence number.
- **Lock-Free Snapshot Reads & Read-Your-Own-Writes:**
  - Calling `db.begin_tx()` or `db.snapshot()` captures `read_seq = last_visible_seq.load(Acquire)`.
  - `tx.get()` and `tx.scan()` first inspect the transaction's local `WriteSet` (so transactions observe their own uncommitted mutations), then merge results from the active `MemTable`, immutable `MemTable`s, and `SSTable`s at `seq <= read_seq`, filtering out entries where `expires_at_ms != 0 && expires_at_ms <= now_ms`.
- **Optimistic Conflict Detection:**
  - Transactions buffer writes locally in a thread-local `WriteSet` and record every read key `(scope_id, user_key)` in a `ReadSet`.
  - At `tx.commit()`, the single-writer commit coordinator acquires the commit lock and verifies that for every key in `ReadSet`, the latest committed version in the engine has `latest_seq <= tx.read_seq`.
  - If any key in `ReadSet` was modified after `tx.read_seq`, `tx.commit()` aborts without writing to the WAL and returns `Err(StorKvError::WriteConflict)`.
  - Blind writes (`put`, `delete`, `merge` without prior reads) bypass conflict checks and serialize linearly in commit order.
- **Read-Modify-Write (`MergeOperator`) Resolution:**
  - `Merge` operations append a `MergeOperand` record without reading existing state first.
  - During reads, the iterator collects contiguous `MergeOperand`s from newest to oldest until hitting a base `Value`, `Tombstone`, `RangeTombstone`, or key end, then invokes `MergeOperator::full_merge` to materialize the final value.
  - During compaction, adjacent `MergeOperand`s strictly below `gc_safe_seq` are folded via `MergeOperator::partial_merge`.

### 2.3 WAL-Pinning Mitigation

Because multiple scopes share one WAL, a low-throughput scope (e.g., `config`) could retain an unflushed `MemTable` holding a low `oldest_unflushed_seq`, preventing old WAL segments from being recycled while a high-throughput scope (e.g., `sessions`) churns gigabytes of data.

StorKV enforces two bounded WAL watermarks:

1. **Byte Lag Watermark (`max_wal_pin_bytes`, default `64 MiB`):** Whenever `current_wal_offset - min_unflushed_wal_offset > max_wal_pin_bytes`, the flush scheduler identifies the scope holding the lowest `oldest_unflushed_seq` and forces an immediate minor flush of its active `MemTable`.
2. **Time Lag Watermark (`max_wal_pin_age`, default `60s`):** Any unflushed `MemTable` older than `max_wal_pin_age` with pending writes is scheduled for background flush, bounding recovery replay time and disk usage.

### 2.4 Compaction Garbage Collection Floor

Background compaction preserves historical versions only down to `gc_safe_seq`, defined as:

```text
gc_safe_seq = min(
    oldest_active_tx_read_seq,
    oldest_pinned_named_snapshot_seq,
    oldest_active_watch_cursor_seq
)
```

Any overwritten or deleted version with `seq < gc_safe_seq` is dropped during compaction. Expired TTL entries (`expires_at_ms != 0 && expires_at_ms <= compaction_start_ms`) are purged unconditionally during compaction without emitting tombstones.

---

## 3. Scope Presets & Configuration Matrix

Every scope is configured via a `ScopeOptions` struct or built-in preset at creation time.

### 3.1 Durability Modes (`Durability`)

| Mode                                     | Commit Behavior                                                                                                                                                       | Crash Guarantee                                                                                            |
| :--------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :--------------------------------------------------------------------------------------------------------- |
| `Durability::StrictFsync`                | Leader thread in the Group Commit queue writes the batch to the active `.wal` file and calls `fdatasync()` before unblocking waiting writers.                         | **Zero acknowledged data loss** on `SIGKILL`, kernel panic, or sudden power cut.                           |
| `Durability::Buffered { sync_interval }` | Writes batch bytes to the OS page cache (`write_all`) and acknowledges immediately. A background flusher calls `fdatasync()` every `sync_interval` (default `100ms`). | **Zero loss on process crash (`SIGKILL`)**; bounded loss up to `sync_interval` only on full OS/power loss. |

- **Cross-Scope Durability Precedence:** When a single `WriteBatch` or `Transaction` mutates multiple scopes with differing durability modes, the batch executes with the strictest durability of all touched scopes (`Durability::StrictFsync` overrides `Durability::Buffered`).

### 3.2 Retention Modes (`Retention`)

| Mode                                               | Compaction & Expiration Behavior                                                                                                                                                                | Target Workload                                              |
| :------------------------------------------------- | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :----------------------------------------------------------- |
| `Retention::Permanent`                             | Keys never expire unless explicitly deleted or given a per-key TTL (`put_with_ttl`). Uses **Leveled Compaction** (`L1..L6`, 10x size ratio per level) to minimize read and space amplification. | User accounts, configuration, metadata, ledger state.        |
| `Retention::Ttl { default_ttl, compaction_style }` | Assigns `default_ttl` to any `put` without an explicit TTL. Supports **FIFO / Time-Window Compaction** to drop entire expired SSTables in `O(1)` without rewriting keys.                        | Sessions, rate-limit counters, ephemeral caches, job queues. |

- **TTL Clock Semantics:** Expiration timestamps (`expires_at_ms`) are stored as wall-clock Unix epoch milliseconds so TTLs remain deterministic across process restarts and checkpoint restores. Expired keys become invisible to reads immediately once `now_ms >= expires_at_ms`, independent of background compaction timing.

---

## 4. Failure & Backup Model

### 4.1 Crash & Torn-Write Recovery

1. **Single-Process Lock:** `Db::open()` acquires an exclusive `flock()` on `<db_root>/LOCK`. Opening an already-locked directory fails immediately with `StorKvError::DatabaseLocked`.
2. **Atomic `MANIFEST` Activation:**
   - All structural state transitions (flushed SSTables, compacted SSTables, created/dropped scopes, pinned snapshots) are appended as `CRC32C`-framed `VersionEdit` records to `MANIFEST-<num>`.
   - To roll a new manifest, the engine writes `CURRENT.tmp`, calls `fdatasync()`, executes atomic POSIX `rename("CURRENT.tmp", "CURRENT")`, and calls `fsync()` on `<db_root>`.
3. **Torn WAL Tail Handling:**
   - During startup recovery, WAL segments are replayed sequentially from `min_unflushed_wal_number`.
   - Each frame's `CRC32C` is verified before applying its operations.
   - If the final frame in the last WAL segment fails `CRC32C` verification or is truncated (due to a mid-write `SIGKILL` or power loss), the engine truncates the file back to the end of the last valid frame, logs a structured `RecoveryWarning::TornWalTailTruncated` metric, and completes startup cleanly.
   - Any corrupted frame in the _middle_ of a WAL segment (indicating storage media bit-rot) halts startup with `StorKvError::Corruption` unless `RecoveryMode::SkipCorruptedFrames` is explicitly enabled.

### 4.2 Hot Checkpoints & <= 5-Minute RPO Backup Model

StorKV guarantees a **Recovery Point Objective (RPO) <= 5 minutes** without halting reads or writes:

1. **WAL Segment Rotation:** The active `.wal` file rotates automatically when it reaches `max_wal_segment_bytes` (`32 MiB`) or when `wal_seal_interval` (`300s` / 5 minutes) elapses since segment creation. Sealed WAL segments (`0000000000000001.wal`) are immutable and immediately eligible for incremental shipping to off-box object storage or backup targets.
2. **Zero-Copy MVCC Checkpoints (`db.checkpoint(dest_dir)`):**
   - Acquires a temporary snapshot pin at `last_visible_seq` so compaction cannot delete live SSTables.
   - Flushes active `MemTable`s or records the exact active WAL offset.
   - Creates hard links (`std::fs::hard_link`) for every live `.sst` file into `dest_dir/scopes/<id>/` in `O(1)` time with zero block copying.
   - Copies `CURRENT`, the active `MANIFEST`, and unmerged WAL segments into `dest_dir/`, calls `fsync()` on all copied files and directories, and releases the snapshot pin.

### 4.3 Autonomous Backup & Point-in-Time Recovery

1. **Scheduler:** `Db::start_auto_backup(BackupConfig)` runs a dedicated std thread that triggers full checkpoint generations on `full_backup_interval`, optionally ships each newly sealed WAL segment, and prunes expired generations. `Db::stop_auto_backup()` stops it without affecting foreground reads or writes.
2. **Sinks:** `LocalFilesystemSink` writes atomic `full-<timestamp_ms>-<seq>/` generation directories plus `wal/<segment>.wal` files and a checksummed `BACKUP_MANIFEST.json` (sequence range, creation time, file list, sizes, CRC32C digests). `CommandPipeSink` streams optionally compressed (`none`, `zstd`, `lz4`) archives to an external command's stdin so object storage is handled by tools such as `rclone` or `aws s3 cp -`.
3. **Retention:** The scheduler keeps the newest `max_full_backups` generations and removes WAL segments older than `max_wal_age`. Pipe sinks report remote-managed pruning and leave object lifecycle to the operator.
4. **Restore:** `Db::restore_from_backup(backup_root, target_dir, target_seq)` verifies every reachable manifest and artifact checksum, selects the newest full generation with `checkpoint_seq <= target_seq` (or the newest generation when no target is given), replays archived WAL segments up to the target sequence, and refuses to write into a non-empty destination.
5. **Verification:** `Db::verify_backup(target_seq)` reports generations, files, bytes, WAL segments, and structured errors without mutating the backup tree. Corrupt or incomplete generations are never selected for restore.
6. **Wire and tooling:** `TriggerBackup`, `BackupStatus`, and `VerifyBackup` are append-only protocol extensions. `storkvd` exposes backup flags, `storkvctl` exposes `backup`, `backup-status`, `backup-verify`, and `restore`, and `storkv-top` renders live backup state.

### 4.4 Multi-Transport and Compatibility Front Ends

All transports execute through the same `Server::dispatch_request` core, so admin policy, telemetry, OCC conflict detection, and Watch events are identical regardless of protocol. HTTP, WebSocket, and gRPC are additive listeners; the native `STORKPRT` v1 frame format is unchanged. Redis, Memcached, and etcd adapters map onto ordinary scopes (`default` / `redis_db_<n>`, `memcached`, and `etcd`) with no separate storage format or recovery path. The embedded `Tree` adapter is a thin wrapper over `Db` scopes.

---

## 5. Out of Scope (v1 Non-Goals)

To keep the core storage engine rock-solid, fast, and maintainable, the following are explicitly excluded from v1:

1. **No SQL Query Parser or Relational Planner:** Access is strictly programmatic via key-value, prefix-scan, range-scan, `Merge`, and `Watch` APIs.
2. **No Secondary Indexes:** Applications requiring secondary lookups maintain index keys inside a dedicated scope within the same atomic `Transaction`.
3. **No Multi-Node Distributed Consensus (Raft):** v1 focuses on single-node vertical performance, crash-proof local I/O, and asynchronous WAL/checkpoint replication. Distributed consensus is deferred to v2.
4. **No Multi-Process Shared-Memory Access:** Multiple processes must communicate with `storkvd` over TCP or Unix Domain Sockets rather than opening the same data directory directly.
5. **No Dynamic User-Loaded Native `.so`/Wasm Plugins in `storkv-core`:** Custom merge operators in v1 are registered statically via Rust traits (`MergeOperator`) to preserve zero-overhead inlining and type safety.
