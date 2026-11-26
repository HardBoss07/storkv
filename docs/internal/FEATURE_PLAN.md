# StorKV Feature Plan

This document tracks upcoming engine, protocol, and operational features for StorKV. Features already implemented in the workspace (MVCC snapshot isolation with OCC transactions, scoped column families, lazy read-time TTL expiration, on-demand compaction, automated backups via `AutoBackupHandle`, WAL pin thresholds, watch registry, and baseline TCP/HTTP/gRPC/Redis/etcd endpoints) have been removed.

## Agent & Contributor Rules

- **Checkbox Discipline:** Only mark an item `[x]` after implementation is complete, unit and integration tests pass (`cargo test --workspace`), and public docs/SDK bindings are updated where applicable.
- **Architectural Boundaries:** Keep storage engine logic inside `storkv-core`, wire types inside `storkv-proto`, and network/compatibility adapters inside `storkv-server`. Avoid leaking protocol-specific details into `storkv-core`.
- **Strict Dependency Governance:** Do not add any new crate to any `Cargo.toml` unless it is explicitly listed in the **Pre-Approved Crates** table below and is required by the specific feature being implemented.

## Pre-Approved Crates

AI agents and contributors are restricted to existing workspace dependencies (`bytes`, `tokio`, `serde`, `thiserror`, `tracing`, `clap`, etc.) plus the pre-approved crates below. Adding any crate not listed here requires explicit human approval first. Crates must be added in the root `Cargo.toml` and then reimported via `workspace = true`.

| Crate    | Target Crate(s) | Cargo Configuration                                      | Authorized Feature(s)                                                                           |
| :------- | :-------------- | :------------------------------------------------------- | :---------------------------------------------------------------------------------------------- |
| `toml`   | `storkv-server` | `toml = "0.8"`                                           | Parsing `storkv.toml` configuration files into `FileConfig` via `serde`.                        |
| `lru`    | `storkv-core`   | `lru = "0.12"`                                           | Per-shard bounded LRU storage for the SSTable Data/Index Block Cache (`block_cache_bytes`).     |
| `subtle` | `storkv-server` | `subtle = { version = "2.6", default-features = false }` | Constant-time byte comparison (`ConstantTimeEq`) for Redis `AUTH` and admin token verification. |

### Features Requiring Zero New Crates

Do not introduce external crates for the following items; implement them using `std` and existing workspace dependencies:

- **Active TTL Expiration Tracker:** Use `std::collections::BinaryHeap` with `std::cmp::Reverse` and `tokio::time::interval`.
- **Compaction Rate Limiting:** Implement a token bucket using `std::time::Instant` and `std::sync::atomic::AtomicU64`.
- **Prometheus `/metrics` Endpoint:** Format Prometheus text exposition lines directly from internal atomic counters without pulling in `metrics` or `prometheus` crate trees.
- **Redis Hashes & Sets:** Encode composite subkeys using `bytes::BytesMut` and execute multi-key mutations over `storkv-core` OCC transactions and `scan_prefix`.

## Phase 1: Core Storage Engine & Native Primitives (`storkv-core` & `storkv-proto`)

- [x] **Scope-Level TTL Guardrails (`ScopeOptions`)**
  - **Target:** `crates/storkv-core/src/scope/options.rs`, `crates/storkv-proto`, `crates/storkv-sdk/src/lib.rs`
  - **Description:** Extend `ScopeOptions` with `min_ttl: Option<Duration>`, `max_ttl: Option<Duration>`, and `allow_permanent: bool` (defaulting to `true` for backward compatibility).
  - **Behavior:** Validate TTLs during `Db::put` and `Db::put_with_ttl`. When `allow_permanent` is `false`, reject writes with `expires_at_ms == 0` with a typed `Error::InvalidTtl` error. When `min_ttl` or `max_ttl` is configured, reject or clamp out-of-bounds TTLs according to scope policy so misconfigured clients cannot cause unbounded disk/memory growth. Expose these fields in `storkv-proto` scope definitions and `storkv-sdk` `ScopeDefinition`.

- [x] **Single-Key Atomic Conditional Writes (`Request::PutIf`)**
  - **Target:** `crates/storkv-core/src/db/handle_kv.rs`, `crates/storkv-core/src/compat.rs`, `crates/storkv-proto/src/request/types/enum_def.rs`, `crates/storkv-sdk`, `crates/storkv-ffi`
  - **Description:** Add a single-round-trip `PutIf` operation supporting `Condition::Absent`, `Condition::Present`, `Condition::ExpectedSeq(u64)`, and `Condition::ExpectedValue(Bytes)` with an optional TTL.
  - **Behavior:** Evaluate the condition directly inside the single-writer commit path (`db_ops/commit`) without allocating a full OCC read-set or triggering retry loops. Return the resulting `seq` on success or the current metadata/mismatch reason on failure. Refactor `Tree::compare_and_swap` in `storkv-core/src/compat.rs` to delegate to `PutIf`.

- [x] **Extended Lock-Free Merge Operators (`MergeOp`)**
  - **Target:** `crates/storkv-core/src/mvcc.rs`, `crates/storkv-core/src/compaction.rs`, `crates/storkv-proto`, `crates/storkv-sdk`
  - **Description:** Expand `MergeOp` beyond `CounterAdd(i64)` and `Custom(Bytes)` to support common backend read-free mutations:
    - `AppendBytes { data: Bytes, max_len: Option<usize> }` for bounded event/log appending.
    - `MaxI64(i64)` and `MinI64(i64)` for monotonic high-water marks and telemetry bounds.
    - `BitOr(u64)` and `BitAnd(u64)` for lock-free bitmask and feature-flag updates.
  - **Behavior:** Ensure associative folding runs identically during point-read merge resolution and SSTable compaction, and expose typed helpers in `storkv-sdk`.

- [x] **Bounded Cursor-Based `Scan` and `ScanPrefix`**
  - **Target:** `crates/storkv-core/src/db/handle_kv.rs`, `crates/storkv-proto`, `crates/storkv-server/src/compat/`
  - **Description:** Extend `Db::scan`, `Db::scan_prefix`, `Request::Scan`, and `Request::ScanPrefix` with `limit: Option<usize>`, `reverse: bool`, and `keys_only: bool`.
  - **Behavior:** Stop iterator traversal immediately once `limit` live, non-expired entries are collected so paginated reads execute in `O(limit)` time and memory rather than materializing the entire scope. Skip value decompression/cloning when `keys_only` is `true`. Update Redis `SCAN` and etcd `/v3/kv/range` handlers to pass `limit` and `keys_only` down to `storkv-core`.

- [x] **Active TTL Expiration Tracker & `ChangeEvent::Expired`**
  - **Target:** `crates/storkv-core/src/db/`, `crates/storkv-core/src/scope/options.rs`
  - **Description:** Add an opt-in per-scope background expiration tracker (`active_ttl_tracking: bool` in `ScopeOptions`) using a min-heap (`BinaryHeap<Reverse<(u64, Bytes)>>`) fed whenever a key is written with a non-zero `expires_at_ms`.
  - **Behavior:** Periodically pop due deadlines, verify that the key's current MVCC entry still matches the expired deadline (ignoring keys that were deleted or overwritten with a newer TTL), emit `ChangeEvent::Expired` to `watch_registry`, and append an expiration tombstone so watch subscribers receive timely expiry events without polling.

## Phase 2: LSM Engine Scalability & Resource Controls (`storkv-core`)

- [x] **Autonomous Background Compaction Scheduler**
  - **Target:** `crates/storkv-core/src/compaction.rs`, `crates/storkv-core/src/options.rs`, `crates/storkv-core/src/db/`
  - **Description:** Add an autonomous background compaction worker governed by new `DbOptions` fields: `enable_background_compaction: bool`, `l0_compaction_trigger: usize`, `max_background_compactions: usize`, and `compaction_rate_limit_bytes_per_sec: Option<u64>`.
  - **Behavior:** Wake the scheduler on MemTable flushes whenever a scope's L0 SSTable count reaches `l0_compaction_trigger`. Bound concurrent scope compactions to `max_background_compactions`, pace SSTable writes via a token-bucket rate limiter when configured, and shut down cleanly on `Db::close` or `Drop`.

- [x] **Shared Sharded SSTable Block Cache**
  - **Target:** `crates/storkv-core/src/options.rs`, `crates/storkv-core/src/sstable/block_cache.rs`
  - **Description:** Add `block_cache_bytes: usize` to `DbOptions` and implement a sharded LRU cache keyed by `(scope_id, sst_id, block_offset)` for decompressed 4 KiB SSTable Data Blocks and Index Blocks.
  - **Behavior:** Check the block cache before reading/decompressing blocks from disk. Support a `fill_cache: bool` read option (default `true`, set to `false` for background compaction and full-scope scans) to avoid cache pollution. Track hit and miss counts in engine statistics.

- [x] **Write Stalls & WAL/L0 Backpressure Watermarks**
  - **Target:** `crates/storkv-core/src/options.rs`, `crates/storkv-core/src/db/`
  - **Description:** Extend `DbOptions` with `max_total_wal_bytes: u64`, `l0_slowdown_writes_trigger: usize`, and `l0_stop_writes_trigger: usize`.
  - **Behavior:** Inject progressive write delays when L0 SSTable counts reach `l0_slowdown_writes_trigger`. When L0 files reach `l0_stop_writes_trigger` or total un-flushed WAL bytes exceed `max_total_wal_bytes`, stall foreground writers (or return `Error::WriteStalled` when non-blocking mode is requested) until background flushes and compactions drain the backlog.

- [x] **Targeted Key-Range Compaction (`Db::compact_range`)**
  - **Target:** `crates/storkv-core/src/compaction.rs`, `crates/storkv-core/src/db/`, `crates/storkv-server`, `crates/storkvctl`
  - **Description:** Implement `Db::compact_range(scope: &str, start: Option<&[u8]>, end: Option<&[u8]>)` to compact only SSTables whose key ranges overlap `[start, end)`.
  - **Behavior:** Reclaim disk space immediately after bulk `DeleteRange` operations or Redis `FLUSHDB` without rewriting unrelated key ranges. Expose `--start` and `--end` flags in `storkvctl compact` and corresponding parameters on HTTP/gRPC admin endpoints.

## Phase 3: Drop-In Protocol Compatibility Upgrades (`storkv-server`)

- [x] **Redis Connection Handshake, `AUTH`, and `CLIENT` Commands**
  - **Target:** `crates/storkv-server/src/compat/redis/commands.rs`, `crates/storkv-server/src/compat/redis/mod.rs`
  - **Description:** Add per-connection authentication state and implement `AUTH [username] <password>`, `HELLO 3 AUTH <username> <password>`, `CLIENT SETNAME <name>`, `CLIENT GETNAME`, `CLIENT ID`, and `COMMAND` / `COMMAND COUNT`.
  - **Behavior:** Validate credentials in constant time using `subtle::ConstantTimeEq` against `--redis-password` (`STORKV_REDIS_PASSWORD`), falling back to `--admin-token` when configured. Reject unauthenticated commands with `-NOAUTH Authentication required.` when a password is active so standard Redis clients (`ioredis`, `redis-py`, `fred`, `lettuce`) and GUI tools connect without handshake errors.

- [x] **Redis Extended String & Expiry Commands**
  - **Target:** `crates/storkv-server/src/compat/redis/commands_keys.rs`, `crates/storkv-server/src/compat/redis/commands_strings.rs`
  - **Description:** Implement `GETDEL`, `GETEX` (`EX`, `PX`, `EXAT`, `PXAT`, `PERSIST`), `MSETNX`, `RENAME`, `RENAMENX`, `TYPE`, `EXPIREAT`, `PEXPIREAT`, `EXPIRETIME`, and `PEXPIRETIME`, plus `EXAT` and `PXAT` flags in `parse_set_options`.
  - **Behavior:** Execute `GETDEL`, `GETEX`, `MSETNX`, `RENAME`, and `RENAMENX` atomically via `storkv-core` OCC transactions so concurrent writers cannot observe partial state.

- [x] **Redis Composite Data Structures (Hashes & Sets)**
  - **Target:** `crates/storkv-server/src/compat/redis/`
  - **Description:** Implement Redis Hashes (`HSET`, `HGET`, `HMGET`, `HGETALL`, `HDEL`, `HINCRBY`, `HEXISTS`, `HLEN`, `HKEYS`, `HVALS`) and Sets (`SADD`, `SREM`, `SMEMBERS`, `SISMEMBER`, `SCARD`) layered cleanly over `storkv-core` primitives.
  - **Behavior:** Encode hash fields and set members under reserved internal key prefixes (e.g., `\x01h:<key>\x00<field>` and `\x01s:<key>\x00<member>`) alongside a metadata entry tracking type and cardinality. Execute multi-field updates atomically in OCC transactions and resolve `HGETALL`/`SMEMBERS` via `scan_prefix` without changing the SSTable/WAL on-disk format.

- [x] **etcd v3 Leases & Compare-and-Swap Transactions**
  - **Target:** `crates/storkv-server/src/compat/etcd_http.rs`
  - **Description:** Implement `/v3/lease/grant`, `/v3/lease/keepalive`, `/v3/lease/revoke`, `/v3/lease/timetolive`, and `/v3/kv/txn`.
  - **Behavior:** Maintain active lease deadlines in memory and propagate each lease's expiration timestamp to attached keys' `expires_at_ms` on `/v3/kv/put` and `/v3/lease/keepalive`. Translate `/v3/kv/txn` `compare` predicates (`VERSION`, `CREATE`, `MOD`, `VALUE`) and `success`/`failure` operation lists into a single `storkv-core` OCC `Transaction` for distributed locking and leader election.

## Phase 4: Configuration, Deployment & Observability (`storkv-server` & `storkvctl`)

- [ ] **4-Layer Configuration Loader (`storkv.toml` + `STORKV_*` Env + CLI)**
  - **Target:** `crates/storkv-server/src/bin/storkvd.rs`, `crates/storkv-server/src/config.rs`
  - **Description:** Add `--config <path>` (`STORKV_CONFIG`) to `storkvd` and attach `env = "STORKV_*"` to all server CLI flags (`STORKV_DATA_DIR`, `STORKV_LISTEN`, `STORKV_HTTP_LISTEN`, `STORKV_GRPC_LISTEN`, `STORKV_REDIS_LISTEN`, `STORKV_ETCD_LISTEN`, `STORKV_ROLE`, `STORKV_PRIMARY_URI`, `STORKV_ADMIN_TOKEN`, `STORKV_REDIS_PASSWORD`).
  - **Behavior:** Deserialize `storkv.toml` using the pre-approved `toml` crate into a `FileConfig` struct (`#[serde(default)]`) and merge settings with deterministic precedence: CLI flags > environment variables > `storkv.toml` > built-in defaults. Fail fast at startup on unknown keys (`#[serde(deny_unknown_fields)]`) or invalid listen addresses.

- [ ] **Declarative `[[scopes]]` Provisioning in `storkv.toml`**
  - **Target:** `crates/storkv-server/src/config.rs`, `crates/storkv-server/src/bin/storkvd.rs`
  - **Description:** Support a `[[scopes]]` array in `storkv.toml` mirroring `ScopeDefinition` (`name`, `durability`, `retention`, `default_ttl_ms`, `min_ttl_ms`, `max_ttl_ms`, `allow_permanent`, `active_ttl_tracking`, `storage_mode`, `prefix_bloom_len`).
  - **Behavior:** Provision or verify all declared scopes on `storkvd` startup before accepting client traffic, ensuring required scopes and TTL guardrails exist immediately in containerized deployments.

- [ ] **Prometheus `/metrics` Endpoint & Engine Telemetry**
  - **Target:** `crates/storkv-server/src/http/`, `crates/storkv-core/src/db/`
  - **Description:** Expose a `/metrics` endpoint on the HTTP listener emitting Prometheus text-format metrics without external telemetry crates.
  - **Behavior:** Report per-operation request counts and latency buckets (`get`, `put`, `put_if`, `delete`, `scan`, `txn`), active watch subscribers, per-level SSTable counts and byte sizes (`L0`..`L6`), WAL pinned and total bytes, compaction bytes written, and block cache hit/miss totals.

- [ ] **Multi-Stage `Dockerfile`, `docker-compose.yml`, and CI Image Workflow**
  - **Target:** `Dockerfile`, `docker-compose.yml`, `storkv.example.toml`, `.github/workflows/docker.yml`
  - **Description:** Provide production container packaging and automated multi-arch image publishing.
  - **Behavior:**
    - Build `storkvd` and `storkvctl` in `--release` inside a multi-stage `Dockerfile` running as a non-root user with a `HEALTHCHECK` invoking `storkvctl health --sock 127.0.0.1:7878`.
    - Include a reference `docker-compose.yml` and `storkv.example.toml` demonstrating volume persistence, bind-mounted `storkv.toml`, and `STORKV_*` environment overrides for primary/replica nodes.
    - Add a `.github/workflows/docker.yml` workflow that builds and pushes `linux/amd64` and `linux/arm64` images to `ghcr.io` on `v*` git tags.
