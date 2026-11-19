# StorKV Feature & Refactoring Architecture Plan

**Status:** Phases 0-4 implemented; see the implementation status table below
**Baseline:** `cargo test --workspace` green, `cargo clippy --workspace --all-targets -- -D warnings` clean
**Scope:** Four feature pillars across `storkv-core`, `storkv-proto`, `storkv-server`, `storkv-client`, `storkv-cli`, `storkv-tui`, and `tests/`
**Hard constraints:** no `mod.rs`, no `.unwrap()`/`.expect()` in `crates/*/src/`, no `#[cfg(test)]` in `crates/*/src/`, no on-disk header or wire-opcode regressions

## Implementation Status

| Phase | Deliverable                                                  | Status   | Primary tests                                                                 |
| :---- | :----------------------------------------------------------- | :------- | :---------------------------------------------------------------------------- |
| 0     | Architecture plan and verified baseline                      | Complete | -                                                                             |
| 1     | Autonomous backup, retention pruning, PITR restore           | Complete | `tests/backup_watch.rs`, `tests/server/backup.rs`, `tests/cli/backup_cmds.rs` |
| 2     | Native/HTTP/WebSocket/gRPC transports and unified URI client | Complete | `tests/transports.rs`, `tests/cli/multi_uri.rs`                               |
| 3     | Redis, Memcached, etcd HTTP, embedded `Tree`, and `migrate`  | Complete | `tests/compat.rs`, `tests/cli/migrate_cmds.rs`                                |
| 4     | Operator and developer documentation                         | Complete | `docs/TRANSPORTS_AND_COMPATIBILITY.md`, `docs/OPERATIONS_AND_BACKUP.md`       |

Supporting entry points:

- `storkvd` flags: `--http-listen`, `--ws-listen`, `--grpc-listen`, `--redis-listen`, `--memcached-listen`, `--strict-admin`, `--backup-dir`, `--backup-interval-secs`, `--backup-retention`, `--backup-ship-wal`, `--backup-pipe-cmd`.
- `storkvctl` subcommands: `health`, `backup`, `backup-status`, `backup-verify`, `restore`, `migrate`, `doctor`, `ycsb`.
- Unified connection URIs: `storkv://host:port`, `unix:///path`, `http://host:port`, `ws://host:port`, `grpc://host:port`.

---

## 0. Why StorKV Wins

StorKV is not another single-purpose KV store. It combines the three markets that are normally mutually exclusive:

| Capability                   | RocksDB / sled / redb | Redis / Memcached | etcd         | **StorKV**                                                           |
| :--------------------------- | :-------------------- | :---------------- | :----------- | :------------------------------------------------------------------- |
| Embedded Rust API            | Yes                   | No                | No           | **Yes** (`Db` + `Tree` drop-in)                                      |
| Scoped physical isolation    | Partial / manual      | Logical DB only   | Namespaces   | **First-class scopes with per-scope durability, TTL, compaction**    |
| Crash-hardened MVCC + OCC    | Varies                | No                | Raft-bound   | **Single-node MVCC with CRC32C WAL + atomic MANIFEST**               |
| Continuous backup + PITR     | Manual                | RDB snapshots     | Snapshot API | **Autonomous checkpoint + sealed-WAL shipping + target-seq restore** |
| Native binary protocol       | No                    | RESP              | gRPC         | **STORKPRT v1 over TCP/UDS**                                         |
| HTTP / REST                  | No                    | No                | JSON gateway | **Native `/v1/*` REST**                                              |
| WebSocket streaming          | No                    | Pub/Sub           | Watch        | **Binary + JSON multiplexed + Watch push**                           |
| gRPC                         | No                    | No                | Yes          | **`storkv.v1` CRUD + Watch + Admin**                                 |
| Redis wire compatibility     | No                    | Native            | No           | **RESP2/RESP3 listener**                                             |
| Memcached wire compatibility | No                    | Native            | No           | **ASCII listener**                                                   |
| Operational suite            | No                    | RedisInsight      | etcdctl      | **`storkvctl`, `storkv-top`, doctor, migrate**                       |

**Positioning:** one crash-hardened MVCC engine, one data directory, every access path — embedded, native, HTTP, WebSocket, gRPC, Redis, Memcached, etcd — with backup/PITR built into the storage layer rather than bolted on.

### Non-goals (unchanged)

- No SQL parser, secondary indexes, or multi-node consensus in this release.
- No dynamic native plugin loading.
- No change to `STORKWAL`, `STORKSST`, `STORKMNF`, or `STORKPRT` v1 magic bytes.
- No new `mod.rs` files and no test code inside `crates/*/src/`.

---

## 1. Verified Baseline (Phase 0 inspection)

These are the exact structures the implementation must extend. They were read from the repository, not assumed.

### 1.1 `storkv-core::Telemetry` (`crates/storkv-core/src/telemetry.rs`)

```rust
pub struct Telemetry {
    pub writes_total: AtomicU64,
    pub reads_total: AtomicU64,
    pub write_conflicts_total: AtomicU64,
    pub compactions_total: AtomicU64,
    pub fsyncs_total: AtomicU64,
}
```

Methods: `new`, `record_write`, `record_read`, `record_write_conflict`, `record_compaction`, `record_fsync`.

### 1.2 `storkv-proto` wire surface

`WireTelemetrySnapshot` currently has exactly five `u64` fields with the same names as `Telemetry`.
`WireAdminStats` currently has:

```rust
pub struct WireAdminStats {
    pub levels: Vec<WireLevelStat>,   // { level: u8, file_count: u32, size_bytes: u64 }
    pub scopes: Vec<WireScopeStat>,   // { scope_id: u32, name: String, writes_total, reads_total, sst_files }
    pub fsync_latency_p99_us: u64,
    pub fsync_latency_samples: u64,
}
```

Existing request opcodes `0..=20`: Ping, Get, Put, PutWithTtl, Delete, DeleteRange, Merge, Scan, ScanPrefix, TxCommit, CreateScope, DropScope, CreateSnapshot, UnpinSnapshot, Checkpoint, Compact, WatchSubscribe, TelemetryQuery, HealthCheck, TxBegin, AdminStats.
Existing response opcodes `0..=9` plus `255` (Error).
Frame contract: `u32 BE length` + `STORKPRT` + `u16 version=1` + `u64 LE request_id` + `u8 payload tag` + payload.

### 1.3 `storkv-server::ServerStats` (`crates/storkv-server/src/metrics.rs`)

- `scopes: Mutex<HashMap<u32, ScopeCounters { writes, reads }>>`
- `latency_us: Mutex<VecDeque<u64>>`, bounded by `MAX_LATENCY_SAMPLES = 2048`
- `snapshot(&Db) -> WireAdminStats` walks live scope SST levels (`LEVEL_COUNT = 7`).
- `Server::dispatch_request(request, _is_admin)` currently ignores the `is_admin` flag.

### 1.4 `storkv-tui` model & UI (`crates/storkv-tui/src/model.rs`, `ui.rs`)

- `TelemetryPoll { writes_total, reads_total, write_conflicts_total, compactions_total, fsyncs_total }`
- `AdminPoll { levels, scopes, fsync_latency_p99_us, fsync_latency_samples }`
- `render_dashboard_with_stats` draws four vertical panels: Amplification (5 rows), Levels (11), WAL/Fsyncs (6), Scopes (min 4).

### 1.5 Backup foundation already present

- `Db::checkpoint(&self, dest_dir)` → `DbInner::checkpoint` flushes all scopes, hard-links live SSTables, copies `CURRENT` + active `MANIFEST` + all WAL segments, `sync_dir`.
- `WalManager` rotates at `32 MiB` or `300 s`, sets the sealed flag, and exposes `current_file_num()`.
- `WalBatchFrame::new(batch_seq, ops)` and `WalRecovery::recover_segment` provide read/write primitives for re-filtering WAL frames.
- `FileSystem` already exposes `create_file`, `append_file`, `rename`, `hard_link`, `sync_dir`, `read_dir`, `file_size`, `remove_file`, `remove_dir_all` — enough for atomic local backup generations without touching the VFS trait.

### 1.6 Consequences for design

- `storkv-core` has **no tokio dependency** and must stay runtime-agnostic. The backup scheduler is a `std::thread` with a condvar/atomic shutdown, and `BackupSink` is a synchronous trait. Async transports live only in `storkv-server` / `storkv-client`.
- `DbOptions` already has an exhaustive struct literal in `tests/scope/wal_pinning.rs`; adding a field would break callers. Auto-backup is therefore started with `Db::start_auto_backup(BackupConfig)` instead of a `DbOptions` field. `DbOptions` remains byte-for-byte API compatible.
- Protocol growth is **append-only**: new request/response opcodes, plus optional trailing extension fields. Old clients ignore extra trailing bytes because each frame body is split out and never checked for exhaustion; new clients treat absent trailing bytes as zero defaults. `PROTOCOL_VERSION` stays `1`.

---

## 2. Phase 1 — Autonomous Continuous Backup & PITR

### 2.1 Goals

1. A background engine that periodically creates zero-copy hard-linked checkpoints and ships newly sealed WAL segments, with **RPO ≤ 5 minutes** by default.
2. A pluggable sink abstraction: local atomic generation directories, or a compressed archive streamed into an external command (`aws s3 cp -`, `rclone rcat`, `ssh`).
3. Retention pruning that never removes a WAL segment required to bridge a retained full generation to the latest shipped sequence.
4. CRC32C verification of every SSTable, MANIFEST, and WAL segment before restore.
5. Point-in-time recovery to an arbitrary `target_seq`, truncating operations above the target cleanly.

### 2.2 Module decomposition (`storkv-core`)

```text
crates/storkv-core/src/
├── backup.rs                    # module root; re-exports config/status/report/types
├── backup/
│   ├── config.rs                # BackupConfig, BackupRetention, BackupCompression
│   ├── manifest.rs              # BackupManifest, BackupFileEntry, JSON (de)serialize
│   ├── sink.rs                  # BackupSink trait, SinkStatus, RetentionPlan
│   ├── sink_local.rs            # LocalFilesystemSink (atomic dirs, sidecar CRCs)
│   ├── sink_pipe.rs             # CommandPipeSink (zstd/lz4 archive over child stdin)
│   ├── scheduler.rs             # AutoBackupHandle, worker thread, WAL rotation watch
│   ├── retention.rs             # retained-generation + WAL watermarks, prune plan
│   ├── wal_index.rs             # scan sealed WAL frames → min_seq/max_seq per segment
│   ├── verify.rs                # verify_backup + per-file/per-record validation
│   └── restore.rs               # restore_from_backup + PITR WAL filtering
```

`backup.rs` is a module file, not `backup/mod.rs`. Submodules follow `module_name.rs` + `module_name/submodule.rs`.

### 2.3 Public API

```rust
pub struct BackupConfig {
    pub backup_root: PathBuf,
    pub full_backup_interval: Duration,   // default 3600 s
    pub ship_sealed_wal: bool,            // default true
    pub retention: BackupRetention,
    pub sink: Arc<dyn BackupSink>,
    pub compression: BackupCompression,   // None | Zstd | Lz4 (pipe sinks)
}

pub struct BackupRetention {
    pub max_full_backups: usize,          // default 7
    pub max_wal_age: Duration,            // default 7 days
}

pub struct BackupStatus {
    pub enabled: bool,
    pub last_backup_timestamp_ms: u64,
    pub last_backup_seq: u64,
    pub backups_completed_total: u64,
    pub backups_failed_total: u64,
    pub wal_segments_shipped_total: u64,
    pub next_full_backup_in_ms: u64,
    pub last_error: Option<String>,
}

pub struct RestoreReport {
    pub generation_id: String,
    pub restored_seq: u64,
    pub files_verified: usize,
    pub wal_segments_replayed: usize,
    pub bytes_restored: u64,
    pub warnings: Vec<String>,
}

impl Db {
    pub fn start_auto_backup(&self, config: BackupConfig) -> Result<AutoBackupHandle>;
    pub fn stop_auto_backup(&self) -> Result<()>;
    pub fn restore_from_backup(
        backup_root: &Path,
        target_data_dir: &Path,
        target_seq: Option<u64>,
    ) -> Result<RestoreReport>;
    pub fn verify_backup(backup_root: &Path, target_seq: Option<u64>) -> Result<BackupVerifyReport>;
}

impl AutoBackupHandle {
    pub fn status(&self) -> BackupStatus;
    pub fn trigger_now(&self) -> Result<()>;
    pub fn shutdown(self) -> Result<()>;
}
```

`AutoBackupHandle` owns the worker `JoinHandle` and an `Arc<AtomicBool>` shutdown flag. `Db` keeps at most one controller in a `Mutex<Option<...>>`; `Drop` sends shutdown. A second `start_auto_backup` returns `StorkvError::InvalidOperation("auto backup already running")`.

### 2.4 Scheduler data flow

```text
              +---------------------------+
              | std::thread (backup worker)|
              +-------------+-------------+
                            |
        every 1s tick / condvar wakeup
                            |
          +-----------------+------------------+
          |                                    |
  full interval elapsed?             wal_mgr.current_file_num() advanced?
          |                                    |
  DbInner::checkpoint(tmp)             sealed = nums < current
          |                                    |
  build BackupManifest                  wal_index::inspect(segment)
          |                                    |
  sink.commit_generation()              sink.ship_wal_segment()
          |                                    |
  retention::plan_and_prune()           update counters + status
          +-----------------+------------------+
                            |
                    telemetry.record_backup_*
```

- Full backups call `DbInner::checkpoint` into `<backup_root>/.tmp/full-<timestamp_ms>-<seq>/`, then atomically rename into `generations/`.
- Sealed WAL detection is poll-based against `WalManager::current_file_num()` every second. No change to `WalManager`'s public surface or WAL format is required. The first tick records the baseline segment number so pre-existing segments are not re-shipped.
- No `.unwrap()`/`.expect()`: every IO, channel, and join result maps into `StorkvError`; worker panics are reported as `backups_failed_total` + `last_error` rather than propagating.
- Counter updates use relaxed atomics on the existing `Telemetry` struct. Timestamps and last sequence use `AtomicU64`.

### 2.5 Sink trait

```rust
pub trait BackupSink: Send + Sync {
    fn begin_generation(&self, generation_id: &str) -> Result<()>;
    fn write_file(&self, relative_path: &Path, bytes: &[u8], crc32c: u32) -> Result<()>;
    fn commit_generation(&self, manifest: &BackupManifest) -> Result<()>;
    fn ship_wal_segment(&self, manifest: &BackupManifest, bytes: &[u8]) -> Result<()>;
    fn prune(&self, plan: &RetentionPlan) -> Result<PruneReport>;
    fn status(&self) -> SinkStatus;
}
```

**`LocalFilesystemSink`**

- Writes each generation under `.tmp/`; every file is fully written, `sync_all`'d, and the directory `sync_dir`'d before a single `rename` publishes the generation.
- Writes `BACKUP_MANIFEST.json` and a sidecar `BACKUP_MANIFEST.json.crc32c` (16 lowercase hex chars + `\n`) so a torn manifest is detectable.
- WAL segments go to `wal/<16hex>.wal` + `wal/<16hex>.wal.crc32c` + `wal/<16hex>.wal.manifest.json`; segments are immutable, deduplicated by file number, and published by rename.
- `prune` removes only its own backup artifacts and only after the retention plan re-verifies that each removable WAL segment is fully covered by a retained full generation.

**`CommandPipeSink`**

- Spawns a configured program + argv directly (`std::process::Command`), never through a shell. The command receives a deterministic archive on stdin.
- Archive framing: `STORKBAK` magic (8 bytes) + `u16` format version (1) + repeated entries `[u8 kind][u32 BE path_len][path][u64 LE data_len][u32 LE crc32c][data]` + `u8` end marker. Kind `1` = full generation file, `2` = WAL segment, `3` = manifest.
- Compression is applied to the whole entry stream with `zstd` or `lz4_flex` (both already workspace dependencies); the archive header records the codec so `restore` can decompress.
- Pipe sinks are append-only: `prune` returns `SinkStatus::RemoteManaged` / empty plan. Operators use object-store lifecycle rules. This is documented explicitly.
- Child exit status, broken pipe, and write timeouts map to `StorkvError::BackupSink { sink, detail }`.

### 2.6 Backup on-disk contract (new files only)

```text
<backup_root>/
├── .tmp/                                  # unpublished staging; ignored/reclaimed on restart
├── generations/
│   ├── full-1760000000000-0000000000000042/
│   │   ├── BACKUP_MANIFEST.json
│   │   ├── BACKUP_MANIFEST.json.crc32c
│   │   ├── CURRENT
│   │   ├── MANIFEST-0000000000000007
│   │   ├── scopes/00000000/0000000000000011.sst
│   │   └── wal/0000000000000009.wal
│   └── full-1760003600000-0000000000000099/
└── wal/
    ├── 0000000000000010.wal
    ├── 0000000000000010.wal.crc32c
    └── 0000000000000010.wal.manifest.json
```

`BACKUP_MANIFEST.json` schema (stable, machine-readable):

```json
{
  "format": "STORKBACKUP",
  "format_version": 1,
  "generation_id": "full-1760000000000-0000000000000042",
  "kind": "full",
  "created_at_ms": 1760000000000,
  "min_seq": 1,
  "max_seq": 42,
  "base_generation": null,
  "compression": "none",
  "total_bytes": 1048576,
  "files": [
    {
      "path": "scopes/00000000/0000000000000011.sst",
      "kind": "sst",
      "size_bytes": 1044480,
      "crc32c": "0x8f2a1b3c"
    },
    {
      "path": "wal/0000000000000009.wal",
      "kind": "wal",
      "size_bytes": 4096,
      "crc32c": "0x11223344",
      "min_seq": 1,
      "max_seq": 42
    }
  ]
}
```

- `crc32c` covers the exact stored bytes.
- `min_seq`/`max_seq` for WAL entries are computed by `wal_index` from decoded frame `batch_seq` and `op_count`; the WAL file itself is not modified.
- JSON is serialized compactly with `serde_json`; key order is stable (struct order) so checksums are reproducible.
- **No existing DB file changes.** `STORKWAL`, `STORKSST`, `STORKMNF`, `CURRENT`, and all record tags remain untouched.

### 2.7 Retention algorithm

1. Enumerate `generations/full-*`, parse manifests, sort by `max_seq` then `created_at_ms`.
2. Keep the newest `max_full_backups` generations. If the count is `0`, keep at least one (never prune the only recovery base).
3. Compute `oldest_retained_base_seq = min(kept.max_seq)`.
4. A WAL segment is removable only when:
   - `segment.max_seq <= oldest_retained_base_seq`, **and**
   - `segment.created_at_ms < now - max_wal_age`, **and**
   - no retained generation manifest lists it as a base input.
5. Prune in safe order: WAL segments first, then older generations, then empty dirs. Every removal is fsync'd at the directory level.
6. `PruneReport { generations_removed, wal_segments_removed, bytes_freed, skipped }`.

### 2.8 PITR restore algorithm

```text
restore_from_backup(root, target, target_seq):
  1. Reject target if it already exists and is non-empty (no destructive overwrite).
  2. Load + schema-validate every BACKUP_MANIFEST.json; validate sidecar CRCs.
  3. Choose base = retained full generation with max_seq <= target_seq (latest if target_seq is None).
     - If none, return StorkvError::TargetSequenceUnavailable { requested, oldest, newest }.
  4. Verify the base: every file CRC32C, every SSTable footer/block via SstReader,
     MANIFEST header + VersionEdit replay, CURRENT points at an existing manifest.
  5. Materialize the base into <target>.restoring-<pid>/ (copy, not hard-link, so backup
     generations can be pruned independently). Preserve scopes/<8hex>/<16hex>.sst layout.
  6. Collect WAL segments with max_seq > base.max_seq, in ascending file-number order.
     Verify each segment CRC and header. For each frame:
       seq(op_i) = batch_seq + i
       keep only seq(op_i) <= target_seq
       re-emit contiguous kept runs as frames preserving the ORIGINAL batch_seq
       (so recovered sequence numbers stay identical to the live cluster).
     Drop frames with no kept operations. Truncation is therefore clean at op granularity.
  7. Write filtered segments into <staging>/wal/, copy CURRENT + MANIFEST from the base,
     fsync files and directories.
  8. Atomically rename <staging> to <target> (rename fails if target appeared concurrently).
  9. Reopen the restored Db once to replay WAL and validate; close it. Report restored_seq.
```

- Exactly-at-target operation ordering follows WAL order; the restored state equals the live state at `target_seq`.
- If `target_seq` is greater than the newest archived sequence, return `TargetSequenceUnavailable` rather than silently claiming a newer point.
- `restore` never mutates the backup tree.
- WAL re-emission uses `WalBatchFrame::new(original_batch_seq, kept_ops).encode(false)`; it does not introduce new opcodes or headers.

### 2.9 Verification

`verify_backup` walks a generation (or the whole catalog) and returns:

```rust
pub struct BackupVerifyReport {
    pub generations_checked: usize,
    pub files_checked: usize,
    pub bytes_checked: u64,
    pub wal_segments_checked: usize,
    pub errors: Vec<BackupVerificationError>, // path + kind + detail
}
```

Checks: manifest JSON shape/version, sidecar CRC, per-file CRC32C, SSTable footer magic `STORKSST` + data/filter/meta/index trailers via `SstReader`, MANIFEST magic `STORKMNF` + VersionEdit frame CRCs, WAL magic `STORKWAL` + batch frame CRCs, and `CURRENT` consistency.

### 2.10 Wire additions (append-only, protocol v1 stays)

| Opcode | Direction | Variant                                      | Payload               |
| :----- | :-------- | :------------------------------------------- | :-------------------- |
| `21`   | Request   | `TriggerBackup { wait: bool }`               | `u8 wait`             |
| `22`   | Request   | `BackupStatus`                               | empty                 |
| `23`   | Request   | `VerifyBackup { target_seq: Option<u64> }`   | `u8 present + u64`    |
| `24`   | Request   | `ListBackups`                                | empty                 |
| `10`   | Response  | `BackupStatus(WireBackupStatus)`             | counters + last error |
| `11`   | Response  | `BackupReport(WireBackupReport)`             | generation + report   |
| `12`   | Response  | `BackupVerifyReport(WireBackupVerifyReport)` | counts + errors       |
| `13`   | Response  | `BackupCatalog(Vec<WireBackupEntry>)`        | generations           |

Trailing extension on `Response::Telemetry` (opcode 5): after `fsyncs_total`, append five `u64 LE` fields:
`backups_completed_total`, `backups_failed_total`, `last_backup_timestamp_ms`, `last_backup_seq`, `wal_segments_shipped_total`.
Decoder reads them only when `source.len() >= 40`; otherwise defaults all five to `0`. Old decoders ignore the tail; new decoders accept old servers. This keeps `PROTOCOL_VERSION = 1` and preserves every existing opcode.

`WireAdminStats` gains a trailing optional block: `u8 present` + `u32 transport_count` + repeated `{ u8 kind, u64 connections_total, u64 active_connections, u64 requests_total }` + backup counters. Old decoders ignore it; new decoders default to an empty transport list.

Error additions to `StorkvWireError` and `StorkvError` (append-only variants):
`BackupSink { sink: String, detail: String }`, `BackupGenerationNotFound(String)`, `BackupCorruption { path: String, detail: String }`, `TargetSequenceUnavailable { requested: u64, oldest: u64, newest: u64 }`, `AutoBackupAlreadyRunning`, `AutoBackupNotRunning`, `RestoreTargetNotEmpty(PathBuf)`.

### 2.11 CLI / TUI integration

`storkvctl`:

```bash
storkvctl backup-status --sock unix:///tmp/storkv-admin.sock
storkvctl backup-verify --sock unix:///tmp/storkv-admin.sock [--target-seq 42]
storkvctl restore --backup-dir ./backups --out-dir ./restored [--target-seq 42]
storkvctl migrate --sock ... --export-jsonl ./dump.jsonl
storkvctl migrate --data-dir ... --import-jsonl ./dump.jsonl --into-scope users
storkvctl migrate --from-redis redis://127.0.0.1:6379 --sock ... --scope migrated
```

`restore` calls `Db::restore_from_backup` directly (offline, no daemon required). `backup-status` / `backup-verify` use new wire opcodes.

`storkvd` flags:

```text
--backup-dir <DIR> --backup-interval-secs <N> --backup-retention <N>
--backup-ship-wal --backup-pipe-cmd "<program>" [--backup-pipe-arg <ARG>...]
--backup-compression <none|zstd|lz4>
```

`storkv-top`: add a Backup panel (or extend the WAL panel without breaking existing titles) showing last backup age, last sequence, completed/failed counts, shipped WAL segments, and next full backup countdown. `TelemetryPoll` gains the five backup fields; `fetch_dashboard` also calls `BackupStatus` when the server supports it. Existing tests are updated for the new fields with `..Default::default()`; panel titles `Amplification`, `Level`, `WAL`, `Fsync` remain.

---

## 3. Phase 2 — Multi-Transport Server & Unified Client

### 3.1 Transport-agnostic execution core

`Server` remains `Arc<Server>` and keeps `dispatch_request(request, is_admin)`. Changes are internal:

- `ServerConfig` gains `strict_admin: bool`, transport listen addresses, and optional admin token.
- `Request::is_admin_only()` marks `Checkpoint`, `Compact`, `CreateSnapshot`, `UnpinSnapshot`, `TriggerBackup`, `VerifyBackup`, `DropScope`, and `ListBackups`.
- When `strict_admin == false` (default), behavior is exactly today's: every connection may issue every request.
- When `strict_admin == true`, `dispatch_request` returns `Response::Error(StorkvWireError::Unauthorized)` for admin-only requests on a connection whose `is_admin` is false. Native admin TCP/UDS listeners pass `is_admin = true`; native client listeners pass `false`.
- HTTP/WS/gRPC connections default to `is_admin = false`, or `true` when `--http-admin` / `--ws-admin` / `--grpc-admin` is set or a valid `--admin-token` is presented (`x-storkv-admin-token` header / `authorization: Bearer ...` metadata). This preserves existing native behavior while making remote admin opt-in.

### 3.2 Server module map

```text
crates/storkv-server/src/
├── transports.rs                 # start_* functions, TransportKind, shared state
├── transports/
│   ├── http.rs                   # axum Router for /v1/*
│   ├── ws.rs                     # WebSocket upgrade + standalone listener
│   └── grpc.rs                   # tonic service impl + Watch stream
├── compat.rs                     # RESP / Memcached / etcd dispatch entry points
├── compat/
│   ├── redis.rs                  # listener + session loop
│   ├── redis/codec.rs            # RESP2/RESP3 parser/encoder (zero-copy)
│   ├── redis/commands.rs         # command table + argument parsing
│   ├── redis/keyspace.rs         # DB→scope mapping, TTL, INCR, MULTI
│   ├── memcached.rs              # listener + session loop
│   ├── memcached/parser.rs       # ASCII line/bulk parser
│   ├── memcached/commands.rs     # get/set/add/... handlers
│   └── etcd_http.rs              # /v3/kv/* handlers
├── dispatch/backup.rs            # TriggerBackup/BackupStatus/Verify/Catalog
├── build.rs                      # tonic-build + protoc-bin-vendored
└── bin/storkvd.rs                # all listen flags
```

`proto/storkv/v1/storkv.proto` at the workspace root is compiled by both `storkv-server/build.rs` and `storkv-client/build.rs` into each crate's `OUT_DIR`; generated code is `include!`d into `transports/grpc/generated.rs` and `client/src/grpc/generated.rs`. No system `protoc` is required: `protoc-bin-vendored` supplies the binary and `build.rs` sets `PROTOC`.

### 3.3 HTTP/1.1 + HTTP/2 REST API

Served by `--http-listen 127.0.0.1:8080` (axum; HTTP/2 prior-knowledge/h2c and HTTP/1.1).

| Method | Path                                          | Semantics                                                                |
| :----- | :-------------------------------------------- | :----------------------------------------------------------------------- |
| GET    | `/v1/health`                                  | `{"status":"ok"}`                                                        |
| GET    | `/v1/ping`                                    | `{"pong":true}`                                                          |
| GET    | `/v1/telemetry`                               | JSON telemetry + backup counters                                         |
| GET    | `/v1/admin/stats`                             | levels, scopes, p99, transports                                          |
| POST   | `/v1/scopes`                                  | `{"name":"users","durability":...,"retention":...,"prefix_bloom_len":8}` |
| DELETE | `/v1/scopes/{name}`                           | drop                                                                     |
| GET    | `/v1/scopes/{scope}/keys/{key}`               | raw `application/octet-stream` or `{"value_b64":...,"seq":...}`          |
| PUT    | `/v1/scopes/{scope}/keys/{key}?ttl_ms=`       | body raw bytes or `{"value_b64":"...","ttl_ms":...}`                     |
| DELETE | `/v1/scopes/{scope}/keys/{key}`               | delete                                                                   |
| POST   | `/v1/scopes/{scope}/merge/{key}`              | `{"op":"counter_add","delta":1}`                                         |
| POST   | `/v1/scopes/{scope}/delete-range`             | `{"start_b64":"...","end_b64":"..."}`                                    |
| GET    | `/v1/scopes/{scope}/scan?start=&end=&prefix=` | `{"pairs":[{"key_b64":"...","value_b64":"..."}]}`                        |
| POST   | `/v1/tx/begin`                                | `{"read_seq":42}`                                                        |
| POST   | `/v1/tx/commit`                               | read_set + write_set, returns committed                                  |
| POST   | `/v1/admin/checkpoint`                        | `{"dest_dir":"..."}`                                                     |
| POST   | `/v1/admin/compact`                           | empty                                                                    |
| POST   | `/v1/admin/backup`                            | trigger                                                                  |
| POST   | `/v1/admin/snapshots`                         | create named                                                             |
| DELETE | `/v1/admin/snapshots/{name}`                  | unpin                                                                    |
| POST   | `/v1/admin/verify-backup`                     | verify catalog                                                           |
| GET    | `/v1/admin/backups`                           | catalog                                                                  |

Behavior notes:

- Keys are URL-decoded raw bytes; ambiguous raw keys must be base64url-encoded in `{key_b64}` query/body variants. Binary values are raw by default; JSON uses `value_b64`.
- Errors use `{"error":{"code":"write_conflict","message":"..."}}` with HTTP 400/404/409/500 mapping.
- Request bodies are capped at `MAX_FRAME_SIZE` (64 MiB) and values at `MAX_VAL_LEN` (256 MiB).
- Admin routes obey `strict_admin` / token policy.
- No TLS in this release; deployments terminate TLS at a proxy. This is documented.

### 3.4 WebSocket transport

- `--ws-listen 127.0.0.1:8081` and `GET /v1/ws` on the HTTP listener (axum `WebSocketUpgrade`).
- Binary frames: one or more `storkv_proto::Message` frames, byte-for-byte identical to native TCP. Request IDs multiplex concurrent requests on one socket.
- Text frames: JSON RPC `{"id":1,"op":"get","scope":"users","key_b64":"...","ttl_ms":null}`; response `{"id":1,"ok":true,"result":{...}}` or `{"id":1,"ok":false,"error":{"code":"...","message":"..."}}`. Ops mirror the HTTP surface: `ping`, `health`, `get`, `put`, `delete`, `delete_range`, `merge`, `scan`, `scan_prefix`, `tx_begin`, `tx_commit`, `create_scope`, `drop_scope`, `telemetry`, `admin_stats`, `checkpoint`, `compact`, `backup`, `verify_backup`.
- `{"id":N,"op":"watch","scope":"users"}` starts a push stream. Events are `{"type":"event","event":{"kind":"put","scope":...,"key_b64":"...","value_b64":"...","seq":42}}` on text sockets, or `Message::new_response(id, Response::WatchEvent(...))` on binary sockets.
- Backpressure: bounded channel of 128 messages per connection; slow consumers receive a close frame with code 1013 (Try Again Later) instead of unbounded memory growth.
- Both protocols can coexist per connection: the frame type selects the codec.

### 3.5 gRPC

`proto/storkv/v1/storkv.proto`:

```protobuf
syntax = "proto3";
package storkv.v1;

message Empty {}
message PingReply { bool ok = 1; }
message ScopeRef { uint32 scope_id = 1; string name = 2; }
message GetRequest { uint32 scope_id = 1; bytes key = 2; }
message GetReply { bool found = 1; bytes value = 2; }
message PutRequest { uint32 scope_id = 1; bytes key = 2; bytes value = 3; uint64 ttl_ms = 4; }
message DeleteRequest { uint32 scope_id = 1; bytes key = 2; }
message DeleteRangeRequest { uint32 scope_id = 1; bytes start_key = 2; bytes end_key = 3; }
message MergeRequest { uint32 scope_id = 1; bytes key = 2; int64 counter_delta = 3; bytes custom = 4; }
message Pair { bytes key = 1; bytes value = 2; }
message ScanRequest { uint32 scope_id = 1; bytes start_key = 2; bytes end_key = 3; bytes prefix = 4; }
message ScanReply { repeated Pair pairs = 1; }
message BeginTxReply { uint64 read_seq = 1; }
message Mutation { uint32 scope_id = 1; bytes key = 2; bytes value = 3; uint64 expires_at_ms = 4; uint32 kind = 5; bytes end_key = 6; }
message CommitRequest { uint64 read_seq = 1; repeated Pair read_set = 2; repeated Mutation write_set = 3; }
message CommitReply { bool committed = 1; }
message CreateScopeRequest { string name = 1; ScopeOptions options = 2; }
message ScopeOptions { uint32 durability = 1; uint64 sync_interval_ms = 2; uint32 retention = 3; uint64 default_ttl_ms = 4; uint32 compaction_style = 5; uint32 prefix_bloom_len = 6; bool has_prefix_bloom = 7; }
message WatchRequest { uint32 scope_id = 1; bool all_scopes = 2; }
message ChangeEvent { uint32 kind = 1; uint32 scope_id = 2; bytes key = 3; bytes value = 4; uint64 seq = 5; }
message TelemetryReply { uint64 writes_total = 1; uint64 reads_total = 2; uint64 write_conflicts_total = 3; uint64 compactions_total = 4; uint64 fsyncs_total = 5; uint64 backups_completed_total = 6; uint64 backups_failed_total = 7; uint64 last_backup_timestamp_ms = 8; uint64 last_backup_seq = 9; uint64 wal_segments_shipped_total = 10; }
message CheckpointRequest { string dest_dir = 1; }
message BackupRequest { bool wait = 1; }
message BackupReply { string generation_id = 1; uint64 seq = 2; }
message BackupStatusReply { bool enabled = 1; uint64 last_backup_timestamp_ms = 2; uint64 last_backup_seq = 3; uint64 backups_completed_total = 4; uint64 backups_failed_total = 5; uint64 wal_segments_shipped_total = 6; string last_error = 7; }
message VerifyBackupRequest { bool has_target_seq = 1; uint64 target_seq = 2; }
message VerifyBackupReply { uint32 generations_checked = 1; uint64 files_checked = 2; uint64 bytes_checked = 3; repeated string errors = 4; }

service Storkv {
  rpc Ping(Empty) returns (PingReply);
  rpc Health(Empty) returns (PingReply);
  rpc Get(GetRequest) returns (GetReply);
  rpc Put(PutRequest) returns (Empty);
  rpc Delete(DeleteRequest) returns (Empty);
  rpc DeleteRange(DeleteRangeRequest) returns (Empty);
  rpc Merge(MergeRequest) returns (Empty);
  rpc Scan(ScanRequest) returns (ScanReply);
  rpc BeginTx(Empty) returns (BeginTxReply);
  rpc Commit(CommitRequest) returns (CommitReply);
  rpc CreateScope(CreateScopeRequest) returns (ScopeRef);
  rpc DropScope(ScopeRef) returns (Empty);
  rpc Watch(WatchRequest) returns (stream ChangeEvent);
  rpc Telemetry(Empty) returns (TelemetryReply);
}

service StorkvAdmin {
  rpc AdminStats(Empty) returns (TelemetryReply);
  rpc Checkpoint(CheckpointRequest) returns (Empty);
  rpc Compact(Empty) returns (Empty);
  rpc CreateSnapshot(ScopeRef) returns (Empty);
  rpc UnpinSnapshot(ScopeRef) returns (Empty);
  rpc TriggerBackup(BackupRequest) returns (BackupReply);
  rpc BackupStatus(Empty) returns (BackupStatusReply);
  rpc VerifyBackup(VerifyBackupRequest) returns (VerifyBackupReply);
}
```

All handlers convert protobuf → existing `Request`/`Response` and call `Server::dispatch_request`; Watch subscribes to `db.watch()` and maps `ChangeEvent` → `ChangeEvent` proto, filtering by scope when requested. No duplicate storage logic.

### 3.6 Unified client

```rust
pub enum TransportKind { NativeTcp, NativeUnix, Http, WebSocket, Grpc }

impl TransportKind {
    pub fn parse_uri(uri: &str) -> ClientResult<(TransportKind, String)>;
}

impl AsyncClient {
    pub async fn connect(address: &str) -> ClientResult<Self>;       // unchanged, TCP
    pub async fn connect_any(address: &str) -> ClientResult<Self>;   // unchanged, TCP/UDS
    pub async fn connect_uri(uri: &str) -> ClientResult<Self>;
}

impl SyncClient {
    pub fn connect_uri(uri: &str) -> ClientResult<Self>;
}
```

URI matrix:

| URI                                 | Transport                          |
| :---------------------------------- | :--------------------------------- |
| `storkv://host:port`                | Native TCP                         |
| `unix:///path/to.sock` or bare path | Native UDS                         |
| `http://host:port`                  | HTTP/1.1 + HTTP/2 REST             |
| `ws://host:port/v1/ws`              | WebSocket (binary STORKPRT frames) |
| `grpc://host:port`                  | gRPC (h2c)                         |
| `host:port` (no scheme)             | Native TCP, backwards compatible   |

Internal refactor: `AsyncClient { backend: Arc<Mutex<Backend>>, request_counter }` where `Backend` is an enum of `Native`, `Http`, `Ws`, `Grpc`. Every existing public method keeps its signature and routes through `send_request`. The existing `ClientStream` remains the native implementation. `subscribe_watch()` is new and returns a `WatchStream`; it is supported by Native, WS, and gRPC, and returns `ClientError::Unsupported("watch")` for plain HTTP.

`storkvctl` and `storkv-top` resolve every `--sock` / `--endpoint` through `connect_uri`, so `--sock http://127.0.0.1:8080` just works. Bare addresses keep their current meaning.

### 3.7 Lifecycle & concurrency

- `storkvd` spawns one accept loop per configured transport and holds `JoinHandle`s; Ctrl-C broadcasts `shutdown_sender`, then awaits all listeners with a bounded timeout.
- A single `Arc<Server>` and one `Arc<Db>` are shared by all transports; there is no per-transport DB handle.
- Per-transport connection counters feed the appended `WireAdminStats` transport extension.
- Blocking core work inside async handlers (checkpoint, restore, backup verification) runs on `tokio::task::spawn_blocking` so the reactor is never stalled.

---

## 4. Phase 3 — Zero-Cost Drop-In Compatibility

### 4.1 Redis RESP2/RESP3 (`--redis-listen`)

Scope mapping: `SELECT 0` → `default` (scope 0); `SELECT n>0` → `redis_db_<n>` (auto-created `Durability::StrictFsync`, `Retention::Permanent`, `prefix_bloom_len: Some(8)`).

| Group        | Commands                                                                                                                                                                | Mapping notes                                                                                                                          |
| :----------- | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------- |
| Connection   | `PING`, `ECHO`, `QUIT`, `HELLO`, `SELECT`, `INFO`, `DBSIZE`, `FLUSHDB`, `FLUSHALL`                                                                                      | `HELLO 2/3` switches response codec; `DBSIZE` counts a scan; flush clears the mapped scope                                             |
| Strings      | `GET`, `SET` (`EX`,`PX`,`NX`,`XX`,`GET`), `SETEX`, `PSETEX`, `SETNX`, `MGET`, `MSET`, `DEL`, `UNLINK`, `EXISTS`, `INCR`, `DECR`, `INCRBY`, `DECRBY`, `APPEND`, `STRLEN` | `SET ... GET` does read+write in one OCC tx; counters use OCC read-modify-write with bounded retries (Return the new value atomically) |
| Expiration   | `EXPIRE`, `PEXPIRE`, `PERSIST`, `TTL`, `PTTL`                                                                                                                           | Exact via new `DbInner::key_ttl_remaining_ms`; `PERSIST` rewrites the value with `expires_at_ms = 0` in an OCC tx                      |
| Keyspace     | `KEYS`, `SCAN`                                                                                                                                                          | trailing-`*` glob → `scan_prefix`; general glob → bounded scan + matcher; `SCAN` cursor is an opaque hex next-key                      |
| Transactions | `MULTI`, `EXEC`, `DISCARD`                                                                                                                                              | Queue commands per connection; `EXEC` evaluates reads and writes inside one SOTKV OCC transaction; `DISCARD` clears the queue          |

RESP codec:

- Parser reads `+`, `-`, `:`, `$`, `*`, `_`, `#`, `,`, `%`, `>`, `~` prefixes; inline commands supported for `telnet`/`nc`.
- Bulk strings borrow from the connection `BytesMut` and are converted to `Bytes` only when a command needs ownership (zero-copy for `GET`/`SET`).
- Encoder supports RESP2 and RESP3 (map, set, double, boolean, push).
- Protocol limits: bulk ≤ 512 MiB, array ≤ 1M elements, nesting ≤ 32; violations return `-ERR Protocol error`.
- Unknown commands: `-ERR unknown command '<name>'`.
- All errors are structured `StorkvError` → RESP `-ERR`/`-WRONGTYPE`/`-EXECABORT`; no panics.

Exact TTL support (no new on-disk fields):

- Add `DbInner::key_meta(scope_id, key) -> Result<Option<KeyMeta { expires_at_ms: u64, seq: u64 }>>` in `db_ops/meta.rs`, reading the same version chain used by `get_versions` and returning the internal key's `expires_at_ms`/`seq`.
- Add `Db::key_ttl_remaining_ms(&ScopeHandle, key)` and `Db::key_version(&ScopeHandle, key)` public wrappers.
- `TTL` returns `-2` for missing, `-1` for permanent, and ceil(remaining_ms/1000) otherwise; `PTTL` returns milliseconds.

### 4.2 Memcached ASCII (`--memcached-listen`)

Dedicated scope `"memcached"` auto-created with `Retention::Ttl { default_ttl: 0, compaction_style: Fifo }`; `exptime` maps to per-key TTL. Values are stored as `[flags: u32 LE][user bytes]` so `get`/`gets` return the original flags; the storage scope is isolated from native traffic.

| Command                 | Semantics                                                                                                          |
| :---------------------- | :----------------------------------------------------------------------------------------------------------------- |
| `get`, `gets`           | Return `VALUE <key> <flags> <bytes> [<cas>]` + data; miss is `END`                                                 |
| `set`, `add`, `replace` | OCC transaction guards `add` (must be absent) and `replace` (must exist)                                           |
| `delete`                | `DELETED` / `NOT_FOUND`                                                                                            |
| `incr`, `decr`          | Parse `u64` payload; OCC retry; return new value or `CLIENT_ERROR cannot increment or decrement non-numeric value` |
| `touch`                 | Rewrite TTL in an OCC tx                                                                                           |
| `cas`                   | Compare current `seq` token from `gets`; `EXISTS`/`NOT_FOUND` on mismatch                                          |
| `flush_all`             | Clear the memcached scope                                                                                          |
| `stats`, `version`      | Minimal `STAT` set + `VERSION storkv-0.1.0`                                                                        |

Parser: strict `\r\n` framing, max key 250 bytes, max value 1 MiB, exptime > 30 days treated as absolute Unix seconds per memcached convention. Malformed input returns `CLIENT_ERROR bad command line format`.

### 4.3 etcd v3 HTTP/JSON gateway (on `--http-listen`)

Dedicated scope `"etcd"` (`Permanent`, StrictFsync). Base64 JSON, matching etcd's gateway:

| Endpoint                  | Request                                                                            | Response                                                                                                        |
| :------------------------ | :--------------------------------------------------------------------------------- | :-------------------------------------------------------------------------------------------------------------- |
| `POST /v3/kv/put`         | `{"key":"<b64>","value":"<b64>","lease":0}`                                        | `{"header":{...}}`                                                                                              |
| `POST /v3/kv/range`       | `{"key":"<b64>","range_end":"<b64>","limit":N,"count_only":bool,"keys_only":bool}` | `{"header":{...},"kvs":[{"key":"<b64>","value":"<b64>","version":"<seq>","mod_revision":"<seq>"}],"count":"N"}` |
| `POST /v3/kv/deleterange` | same range fields                                                                  | `{"header":{...},"deleted":"N"}`                                                                                |

- `range_end` empty → single key; `AAAAAA==` (a single `\0`) → all keys ≥ key; otherwise half-open `[key, range_end)`.
- `header.revision` is the engine's `last_visible_seq`; `cluster_id`/`member_id` are stable hex constants; `raft_term` is `1`.
- `lease` must be `0`; non-zero returns `{"error":"leases are not supported","code":3}`. `sort_order`/`sort_target` are accepted but only key-ascending is honored.
- Mounted without conflicting with `/v1/*`; admin auth policy applies only to `/v1/admin/*`.

### 4.4 Embedded drop-in adapters (`storkv-core/src/compat.rs`)

```rust
pub trait KvEngine {
    type Tree;
    fn open_tree(&self, name: &str) -> Result<Self::Tree>;
    fn flush(&self) -> Result<()>;
}

pub struct Tree { db: Db, scope: ScopeHandle }

impl Tree {
    pub fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Bytes>>;
    pub fn insert<K: AsRef<[u8]>, V: AsRef<[u8]>>(&self, key: K, value: V) -> Result<Option<Bytes>>;
    pub fn remove<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Bytes>>;
    pub fn compare_and_swap<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self, key: K, expected: Option<&[u8]>, new_value: V,
    ) -> Result<bool>;
    pub fn scan_prefix<K: AsRef<[u8]>>(&self, prefix: K) -> Result<Vec<(Bytes, Bytes)>>;
    pub fn range<K: AsRef<[u8]>>(&self, start: K, end: K) -> Result<Vec<(Bytes, Bytes)>>;
    pub fn batch(&self) -> Batch;
    pub fn apply_batch(&self, batch: Batch) -> Result<()>;
    pub fn flush(&self) -> Result<()>;
}
```

- `Db::open_tree(name)` maps to a scope with the same name (created if absent) and returns an owned `Tree`. To make `Tree` owned, `Db` becomes `Clone` (manual impl cloning the inner `Arc` and `try_clone`-ing the lock file; both clones keep the exclusive lock). This is additive and does not change `Db::open`.
- `compare_and_swap` uses an OCC transaction: read current value + `read_seq`, write only if the expectation matches; returns `false` on mismatch and retries internally only on `WriteConflict`.
- `insert` returns the previous value (`None` for a new key) via a read-before-write OCC transaction.
- `Batch` buffers puts/deletes and applies them in one `Transaction` (single WAL frame, atomic across keys).
- `flush` calls a new `Db::flush()` → `DbInner::flush_all_scopes`.
- No new storage format: trees are ordinary scopes, visible to native clients and backup.

---

## 5. Phase 4 — Documentation Set

| File                                   | Contents                                                                                                                                                                                                                    |
| :------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/FEATURE_PLAN.md`                 | This document: architecture, contracts, refactor map, test matrix                                                                                                                                                           |
| `docs/TRANSPORTS_AND_COMPATIBILITY.md` | Choosing a transport; URI matrix; `storkvd` flags; curl/gRPC/WebSocket/Redis/Memcached/etcd examples; embedded `Tree` migration guide; auth/`--strict-admin`; limits; troubleshooting                                       |
| `docs/OPERATIONS_AND_BACKUP.md`        | `storkvd` deployment, systemd/Docker healthchecks, backup policy design, local + pipe sinks (S3/rclone/NAS), retention math, PITR runbooks, `storkvctl restore`/`migrate`, `storkv-top` panels, disaster recovery checklist |
| `README.md`                            | Update architecture diagram, feature matrix, quickstart with all transports, repository layout; keep existing sections intact                                                                                               |
| `SPEC.md`                              | Add sections for continuous backup/PITR, transport guarantees, compatibility semantics, and the new admin-authorization model; keep all existing invariants                                                                 |

Documentation rules: every command is copy-pasteable, every flag is shown with its default, every wire example includes the exact magic/version where relevant, and no existing section is removed.

---

## 6. Refactoring & Module Decomposition Map

All new modules follow `clippy::mod_module_files`: a module is either a single `name.rs` or `name.rs` + `name/child.rs`. **No `mod.rs` anywhere.**

### 6.1 `storkv-core`

| Change                       | Files                                                                                                                                                          |
| :--------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Backup pillar                | `src/backup.rs`, `src/backup/{config,manifest,sink,sink_local,sink_pipe,scheduler,retention,wal_index,verify,restore}.rs`                                      |
| Embedded compat              | `src/compat.rs`, `src/compat/{tree,batch,engine}.rs`                                                                                                           |
| TTL/version metadata + flush | `src/db_ops/meta.rs` (new), `src/db_ops/maintenance.rs` (extend `flush_all_scopes` visibility)                                                                 |
| Telemetry counters           | `src/telemetry.rs` (append fields + `record_backup_*`)                                                                                                         |
| Public exports               | `src/lib.rs` adds `pub mod backup; pub mod compat;` + re-exports                                                                                               |
| Errors                       | `src/error.rs` append variants; no existing variant changes                                                                                                    |
| `Db` handle                  | `src/db/handle.rs` adds `start/stop_auto_backup`, `restore_from_backup`, `verify_backup`, `open_tree`, `key_ttl_remaining_ms`, `key_version`, `flush`, `Clone` |

No existing file is deleted. `checkpoint.rs` gains a thin `checkpoint_with_manifest` helper but keeps `checkpoint` semantics.

### 6.2 `storkv-proto`

| Change                              | Files                                                                                                                      |
| :---------------------------------- | :------------------------------------------------------------------------------------------------------------------------- |
| Request variants `21..=24`          | `src/request/types.rs`, `src/request/encode.rs`, `src/request/decode.rs`                                                   |
| Backup wire structs                 | `src/response/types.rs`, `src/response/encode.rs`, `src/response/decode_backup.rs` (new)                                   |
| Telemetry/admin trailing extensions | `src/response/decode.rs`, `src/response/decode_admin.rs`, `src/response/encode.rs`                                         |
| Public exports                      | `src/lib.rs` adds `WireBackupStatus`, `WireBackupReport`, `WireBackupVerifyReport`, `WireBackupEntry`, `WireTransportStat` |

All existing opcode numbers and the `STORKPRT` v1 header remain unchanged.

### 6.3 `storkv-server`

| Change             | Files                                                                                   |
| :----------------- | :-------------------------------------------------------------------------------------- |
| Transport registry | `src/transports.rs`                                                                     |
| HTTP REST          | `src/transports/http.rs`                                                                |
| WebSocket          | `src/transports/ws.rs`                                                                  |
| gRPC               | `src/transports/grpc.rs`, `src/transports/grpc/generated.rs`, `build.rs`                |
| Redis              | `src/compat.rs`, `src/compat/redis.rs`, `src/compat/redis/{codec,commands,keyspace}.rs` |
| Memcached          | `src/compat/memcached.rs`, `src/compat/memcached/{parser,commands}.rs`                  |
| etcd gateway       | `src/compat/etcd_http.rs`                                                               |
| Backup dispatch    | `src/dispatch/backup.rs`                                                                |
| Admin policy       | `src/config.rs`, `src/dispatch.rs`, `src/server.rs`                                     |
| Daemon flags       | `src/bin/storkvd.rs`                                                                    |
| Metrics            | `src/metrics.rs` adds transport counters; keeps `WireAdminStats` shape + extension      |

### 6.4 `storkv-client`

| Change              | Files                                                                                                             |
| :------------------ | :---------------------------------------------------------------------------------------------------------------- |
| URI parsing         | `src/transport/uri.rs` (new)                                                                                      |
| Transport backends  | `src/backend.rs`, `src/backend/{native,http,ws,grpc}.rs` (new)                                                    |
| gRPC generated      | `src/grpc.rs`, `src/grpc/generated.rs`, `build.rs`                                                                |
| Unified facade      | `src/async_client.rs`, `src/sync_client.rs` add `connect_uri`, keep all existing methods                          |
| Watch stream        | `src/watch_stream.rs` (new)                                                                                       |
| Native stream/error | `src/transport.rs` keeps `ClientStream`/`ClientError` and adds `Unsupported`, `HttpStatus`, `GrpcStatus` variants |

### 6.5 `storkv-cli` / `storkv-tui`

| Crate | Files                                                                                                                        |
| :---- | :--------------------------------------------------------------------------------------------------------------------------- |
| CLI   | `src/backup.rs` (status/verify), `src/restore.rs`, `src/migrate.rs`, `src/bin/storkvctl.rs` new subcommands                  |
| TUI   | `src/model.rs` backup fields, `src/poller.rs` `fetch_backup_status`, `src/ui.rs` Backup panel, `src/bin/storkv-top.rs` flags |

### 6.6 `tests/` (no deletions)

- `tests/backup_watch.rs` becomes a thin harness that still contains its four existing tests plus `#[path = "backup_watch/<module>.rs"]` includes. Existing test bodies move into `tests/backup_watch/{checkpoint,watch,telemetry}.rs` unchanged.
- New `tests/transports.rs` + `tests/transports/{native,http,ws,grpc,uri,watch}.rs`.
- New `tests/compat.rs` + `tests/compat/{redis,memcached,etcd,tree,migrate}.rs`.
- `tests/cli.rs` and `tests/tui.rs` gain modules under `tests/cli/` and `tests/tui/` for the new commands/panels; no existing test is removed.
- `tests/common.rs` gains `TempDb::open_with_backup` and a `spawn_server_with_config` helper; existing helpers stay.

---

## 7. Test Matrix

### 7.1 `tests/backup_watch.rs` (+ `tests/backup_watch/`)

| Area               | Test module                  | Cases                                                                                                                                                                                                  |
| :----------------- | :--------------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Periodic scheduler | `backup_watch/scheduler.rs`  | first full backup created; interval triggers second generation; `stop_auto_backup` joins cleanly; double start rejected; restart does not re-ship baseline WAL                                         |
| WAL shipping       | `backup_watch/wal_ship.rs`   | sealed segment detected after rotation; shipped bytes match source CRC; duplicate segment not shipped twice; shipping disabled leaves no `wal/` entries                                                |
| Retention          | `backup_watch/retention.rs`  | `max_full_backups=2` prunes oldest; `max_wal_age` prunes only covered segments; sole base never pruned; retained WAL needed for bridge is kept                                                         |
| Local sink         | `backup_watch/local_sink.rs` | generation published atomically; `.tmp` ignored; manifest sidecar CRC; file CRC matches bytes                                                                                                          |
| Pipe sink          | `backup_watch/pipe_sink.rs`  | zstd/lz4 archive round-trips; child failure increments `backups_failed_total`; args with spaces are not shell-split                                                                                    |
| PITR               | `backup_watch/restore.rs`    | restore latest; restore exact mid-batch `target_seq` truncates cleanly; target below oldest errors; target above newest errors; restored DB opens and reads expected values; non-empty target rejected |
| Verification       | `backup_watch/verify.rs`     | clean backup passes; one flipped SSTable byte rejected; truncated MANIFEST rejected; torn WAL tail rejected; manifest JSON tamper rejected                                                             |
| Chaos              | `backup_watch/chaos.rs`      | `FaultInjectingFs` fails mid-generation → no published generation; crash before rename → previous generation still restorable; torn manifest sidecar rejected                                          |
| Telemetry          | `backup_watch/telemetry.rs`  | completed/failed/last seq/shipped-counters update; existing five counters unchanged                                                                                                                    |

### 7.2 `tests/transports.rs` (+ `tests/transports/`)

| Area           | Test module            | Cases                                                                                                                                                                 |
| :------------- | :--------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| URI parsing    | `transports/uri.rs`    | all schemes; bare host:port; UDS path; unsupported scheme; malformed port                                                                                             |
| Native         | `transports/native.rs` | CRUD, scan, prefix, OCC conflict, watch events, error propagation over TCP and UDS; existing `storkv://`/`unix://`                                                    |
| HTTP           | `transports/http.rs`   | health/ping/telemetry; raw + JSON put/get; TTL; delete/range; scan; scopes; tx begin/commit; admin endpoints; 404/409/400 mapping; strict-admin denial                |
| WebSocket      | `transports/ws.rs`     | binary STORKPRT request/response; JSON RPC get/put; multiplexed IDs; watch push; oversized frame closes; slow-consumer handling                                       |
| gRPC           | `transports/grpc.rs`   | Ping/Health; CRUD; Scan; transaction commit; Watch stream receives events; admin backup/verify; invalid argument mapping                                              |
| Unified client | `transports/client.rs` | `AsyncClient::connect_uri` and `SyncClient::connect_uri` across all five transports; identical CRUD result; watch support/unsupported matrix                          |
| Auth           | `transports/auth.rs`   | default permissive behavior preserved; `strict_admin` denies non-admin checkpoint/compact/backup on native client socket; admin socket still allowed; HTTP token path |

### 7.3 `tests/compat.rs` (+ `tests/compat/`)

| Area           | Test module                | Cases                                                                                                                                                 |
| :------------- | :------------------------- | :---------------------------------------------------------------------------------------------------------------------------------------------------- |
| RESP codec     | `compat/redis_codec.rs`    | RESP2/RESP3 arrays, bulk, inline; malformed framing; nesting/size limits                                                                              |
| Redis strings  | `compat/redis_strings.rs`  | GET/SET, EX/PX, NX/XX, GET option, SETEX/PSETEX/SETNX, MGET/MSET, DEL/UNLINK/EXISTS, APPEND/STRLEN                                                    |
| Redis counters | `compat/redis_counters.rs` | INCR/DECR/INCRBY/DECRBY atomic under concurrency; non-numeric error; overflow error                                                                   |
| Redis TTL      | `compat/redis_ttl.rs`      | TTL/PTTL exact values; missing = -2; permanent = -1; EXPIRE/PEXPIRE; PERSIST; expiry invisible before compaction                                      |
| Redis keyspace | `compat/redis_keys.rs`     | SELECT 0 vs 1 isolation; DBSIZE; FLUSHDB/FLUSHALL; KEYS prefix/exact/glob; SCAN cursor round-trip                                                     |
| Redis tx       | `compat/redis_tx.rs`       | MULTI/EXEC applies atomically; DISCARD; nested MULTI error; EXEC after failed queue                                                                   |
| Memcached      | `compat/memcached.rs`      | set/get/gets flags + CAS; add/replace; delete; incr/decr; touch; flush_all; stats/version; malformed input                                            |
| etcd           | `compat/etcd.rs`           | put/range single, prefix, all-key; count/keys-only/limit; deleterange; base64 round-trip; lease rejection; revision header                            |
| Embedded Tree  | `compat/tree.rs`           | open_tree creates scope; get/insert/remove; compare_and_swap success/failure; scan_prefix; range; batch atomicity; flush; visibility from native `Db` |
| Migration      | `compat/migrate.rs`        | JSONL export/import round-trip; TTL preserved; import into existing scope; malformed line rejection; Redis source import                              |

### 7.4 `tests/cli.rs` and `tests/tui.rs`

| Area           | Module                   | Cases                                                                                                          |
| :------------- | :----------------------- | :------------------------------------------------------------------------------------------------------------- |
| CLI backup     | `cli/backup_cmds.rs`     | `backup-status` output; `backup-verify` clean/corrupt; `restore` full + target-seq; non-empty target exit code |
| CLI migrate    | `cli/migrate_cmds.rs`    | export JSONL; import JSONL; `--from-redis`; round-trip equality                                                |
| CLI URI        | `cli/uri.rs`             | `--sock storkv://`, `unix://`, `http://`, `ws://`, `grpc://` for health/backup                                 |
| TUI backup     | `tui/backup_panel.rs`    | backup counters render; zero-value no panic; long error string truncated                                       |
| TUI regression | existing `tui/render.rs` | all existing panel titles/values still rendered with added fields                                              |

### 7.5 Existing suites

`tests/format.rs`, `wal.rs`, `mvcc.rs`, `scope.rs`, `compaction.rs`, `proto.rs`, `client.rs`, `server.rs`, `chaos.rs` must remain green with zero assertion changes except struct-literal updates required by additive fields (`..Default::default()`).

---

## 8. Compatibility & Safety Invariants

1. **On-disk:** no change to `STORKWAL`, `STORKSST`, `STORKMNF`, `CURRENT`, block layouts, tags, or file naming. Backup artifacts are new paths outside the DB root.
2. **Wire:** `STORKPRT` + version `1` unchanged. New request opcodes `21..=24`, new response opcodes `10..=13`. Existing payloads are never renumbered or resized; telemetry/admin extensions are strictly trailing and optional.
3. **Public API:** all existing `Db`, `Server`, `AsyncClient`, `SyncClient`, `CliRunner`, and TUI function signatures preserved. Additive methods only.
4. **Runtime:** `storkv-core` stays tokio-free. Backup is synchronous; async transports live in `storkv-server`/`storkv-client`.
5. **Code hygiene:** zero `.unwrap()`/`.expect()` in production, zero `#[cfg(test)]` in `crates/*/src/`, no `mod.rs`, functions under ~45 lines, explicit domain names.
6. **Failure handling:** every new IO/child-process/JSON/protobuf path returns a structured error; no panics, no silent truncation except the explicitly documented torn WAL tail policy.

---

## 9. Delivery Gates

After each phase:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
project-structure.exe . --count
```

Phase 1 additionally runs the chaos suite against `FaultInjectingFs`. Phase 2 runs all five transports end-to-end. Phase 3 runs real Redis/Memcached protocol vectors. Phase 4 verifies every documented command against a live `storkvd`.

No phase is considered complete until all four commands exit `0` and the existing 107+ tests remain green.

---

## 10. Dependency Approval Request

Per the mandatory stop condition, the following **new direct workspace dependencies** are required. No cloud SDK is used. Versions are the current stable lines; exact patch versions will be pinned by `Cargo.lock`.

### 10.1 Shared serialization / encoding

| Crate        | Version | Features | Used by                                                   |
| :----------- | :------ | :------- | :-------------------------------------------------------- |
| `serde`      | `1`     | `derive` | core backup manifest, CLI migrate, server JSON            |
| `serde_json` | `1`     | default  | core manifest, HTTP/REST, WS JSON, etcd JSON, CLI migrate |
| `base64`     | `0.22`  | default  | HTTP binary values, etcd gateway, WS JSON                 |

### 10.2 Server transports

| Crate                             | Version | Features                                                          | Used by                             |
| :-------------------------------- | :------ | :---------------------------------------------------------------- | :---------------------------------- |
| `axum`                            | `0.8`   | `default-features=false`, `http1`, `http2`, `json`, `tokio`, `ws` | HTTP + WS + etcd gateway            |
| `http`                            | `1`     | default                                                           | shared HTTP types                   |
| `tower`                           | `0.5`   | `util`                                                            | axum middleware/tests               |
| `tonic`                           | `0.14`  | `transport`, `codegen`                                            | gRPC server                         |
| `prost`                           | `0.14`  | default                                                           | gRPC messages                       |
| `tokio-stream`                    | `0.1`   | `sync`                                                            | Watch broadcast → gRPC stream       |
| `futures-util`                    | `0.3`   | default                                                           | stream plumbing                     |
| `tonic-build` (build-dep)         | `0.14`  | `transport`                                                       | `build.rs` codegen                  |
| `protoc-bin-vendored` (build-dep) | `3`     | default                                                           | bundled `protoc`, no system install |

### 10.3 Client transports

| Crate                             | Version | Features                                         | Used by               |
| :-------------------------------- | :------ | :----------------------------------------------- | :-------------------- |
| `hyper`                           | `1`     | `client`, `http1`, `http2`                       | HTTP client           |
| `hyper-util`                      | `0.1`   | `client-legacy`, `http1`, `http2`, `tokio`       | HTTP client connector |
| `http-body-util`                  | `0.1`   | default                                          | HTTP body collection  |
| `tokio-tungstenite`               | `0.28`  | `default-features=false`, `connect`, `handshake` | WS client             |
| `futures-util`                    | `0.3`   | default                                          | WS/gRPC streams       |
| `tonic`                           | `0.14`  | `transport`, `codegen`                           | gRPC client           |
| `prost`                           | `0.14`  | default                                          | gRPC messages         |
| `tokio-stream`                    | `0.1`   | `sync`                                           | client watch stream   |
| `tonic-build` (build-dep)         | `0.14`  | `transport`                                      | client `build.rs`     |
| `protoc-bin-vendored` (build-dep) | `3`     | default                                          | bundled `protoc`      |

### 10.4 Explicitly not added

No AWS/GCP/Azure SDKs, no `reqwest`, no `rand`, no `uuid`, no `async-trait`, no `tempfile`, no TLS stack. Compression uses the already-present `zstd` and `lz4_flex`. Checksums use the already-present `crc32c`.

### 10.5 Tokio feature additions

`storkv-server` and `storkv-client` add `tokio` features `fs` and `rt` only if required by the transport adapters; `net`, `io-util`, `sync`, `time`, `macros`, `rt-multi-thread` are already enabled. `storkv-core` does **not** gain tokio.

**Awaiting explicit approval for the crates above before writing any `Cargo.toml` change or Phase 1 code.**
