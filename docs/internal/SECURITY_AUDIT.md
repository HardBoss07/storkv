# StorKV Security Audit

## 1. Audit Metadata

| Field | Value |
| :--- | :--- |
| Project | StorKV (`storkv`) |
| Audit date | 2026-11-18 |
| Branch | `main` |
| HEAD commit | `009ff6f` (`test: update chaos, format, and protocol integration tests`) |
| Worktree state | Dirty. The audit read the current working tree, not the HEAD snapshot. |
| Workspace size | 56,168 lines (`project-structure.exe . -m --count`) |
| Crate sizes | `storkv-core` 10,568; `storkv-server` 7,631; `storkv-sdk` 2,790; `storkv-proto` 2,312; `storkv-client` 2,079; `storkv-ffi` 1,684; `storkv-cli` 1,115; `storkv-tui` 621 |
| Method | Source-grounded review with entrypoint-to-sink tracing and adversarial disproof |
| Constraints | Read-only workspace inspection. No live network probe. No destructive command. |
| Sole output | This file. |

This audit uses the Cloudflare `security-audit` bar. A finding needs a real trust boundary, a verified code path, a missing control, and a bounded impact. A defense-in-depth gap is not a vulnerability.

### 1.1 Severity Counts

| Severity | Confirmed |
| :--- | ---: |
| `CRITICAL` | 0 |
| `HIGH` | 2 |
| `MEDIUM` | 5 |
| `LOW` | 0 |
| **Total confirmed** | **7** |

| Other class | Count |
| :--- | ---: |
| `NEEDS_VALIDATION` | 2 |
| `HARDENING_NOTE` | 8 |
| `REJECTED` | 10 |

### 1.2 Executive Summary

The storage engine core is careful with untrusted wire input. Frame length fields, request and response vectors, WAL records, SSTable blocks, and FFI output pointers have explicit bounds checks. This audit found no memory corruption, no arbitrary code execution, and no direct authentication bypass on an admin-only request.

The main risk is availability. Watch and replication streams have no backpressure and no subscription limit. A network client can subscribe to a native, WebSocket, or gRPC watch and then stop reading. The server keeps every event in an unbounded queue. Repeated subscribe requests also leak subscriber tasks. One client can therefore exhaust server memory.

The Redis compatibility layer has two independent denial-of-service paths. The inline parser has no line limit, so one connection can grow its read buffer without bound. The `KEYS` and `SCAN MATCH` glob matcher uses exponential backtracking, so one small pattern can consume a CPU core.

Two correctness defects have security impact. The server-wide consistency setting is writable by any client, so a client can deny reads on a replica or force stale reads. The SDK read cache does not honor TTL expiry, range deletes, or merge mutations, so a deleted or expired value can stay readable.

## 2. Architecture and Trust-Boundary Map

### 2.1 Untrusted Entrypoints

| Entrypoint | Code | Privilege gate |
| :--- | :--- | :--- |
| Native TCP/UDS (`STORKPRT`) | `crates/storkv-server/src/server.rs:209-255`; `crates/storkv-proto/src/frame.rs:95-160` | `is_admin` comes from the listener. The native protocol has no token field. |
| HTTP REST and JSON-RPC | `crates/storkv-server/src/transports/http.rs:53-89`; `transports/http/rest_data.rs`; `transports/http/rpc.rs:19-56` | `HttpState::is_admin` reads the admin token header (`transports/http.rs:32-41`). |
| WebSocket binary and JSON-RPC | `crates/storkv-server/src/transports/ws/session.rs:92-196` | Token check at upgrade. Watch requests bypass `dispatch_request`. |
| gRPC `storkv.v1` data and admin | `crates/storkv-server/src/transports/grpc/service.rs`; `grpc/admin.rs` | Metadata token at `grpc.rs:73-86`. Data methods call `dispatch`. Watch does not. |
| Redis RESP2/RESP3 | `crates/storkv-server/src/compat/redis.rs:47-87` | No authentication. No `strict-admin` hook. |
| Memcached ASCII | `crates/storkv-server/src/compat/memcached.rs:37-114` | No authentication. No `strict-admin` hook. |
| etcd v3 JSON | `crates/storkv-server/src/compat/etcd_http.rs:18-135` | No authentication. |
| Replication stream | `crates/storkv-server/src/server.rs:354-419` | No admin check. Replica role rejects the request. |
| C ABI | `crates/storkv-ffi/src/*.rs` | Caller-owned raw pointers. Documented `unsafe` contract. |
| On-disk WAL, SST, manifest, backup | `crates/storkv-core/src/wal/*`; `sstable/*`; `manifest/*`; `backup/*` | Local filesystem trust. Restore accepts an operator-selected backup root. |

### 2.2 Principals and Boundaries

| Principal | Resource | Control |
| :--- | :--- | :--- |
| Non-admin network client | Admin-only request set | `dispatch_request` at `crates/storkv-server/src/dispatch.rs:20-23`; set at `crates/storkv-proto/src/request/types.rs:251-267` |
| Admin listener or token holder | Global state and backup internals | `strict_admin`, `TransportOptions::grants_admin` at `transports.rs:59-83` |
| Local user | Native admin Unix socket | Filesystem permission on the socket path |
| Any client | Scope data | No per-scope authorization. Scopes are a namespace, not a security principal. |
| Any client | Watch and replication events | Only the transport connection. No admin role. |
| Any client | Server consistency setting | `Request::ConfigureConsistency`, no admin role |
| Backup operator | Backup manifest, WAL, SST | Local trust. No manifest path confinement. |
| C caller | FFI handles and slices | Documented safety contract |

### 2.3 Coverage Ledger

| Area | Files reviewed | Result |
| :--- | :--- | :--- |
| Frame and request decoding | `storkv-proto/src/frame.rs`, `request/decode.rs`, `response/decode.rs`, `response/decode_admin.rs` | Bounds checks hold. See R-01. |
| Native dispatch and admin set | `server.rs`, `dispatch.rs`, `request/types.rs` | Admin gate holds for listed requests. Watch and replication bypass it by design. |
| HTTP and REST | `transports/http.rs`, `rest_data.rs`, `rest_scopes.rs`, `rest_admin.rs`, `rpc.rs` | Token gate holds. No path traversal. |
| WebSocket | `transports/ws/session.rs`, `ws/json_rpc.rs` | Watch path has no limit. See F-01, F-02. |
| gRPC | `transports/grpc.rs`, `grpc/service.rs`, `grpc/admin.rs` | Admin methods gate. Watch has no limit. See F-01. |
| Redis parser and commands | `compat/redis.rs`, `codec_decode.rs`, `commands*.rs`, `helpers.rs`, `keyspace.rs` | Inline and glob DoS. See F-03, F-04. |
| Memcached | `compat/memcached.rs`, `parser.rs`, `commands.rs` | Length caps hold. Flush is not admin-gated. See H-01. |
| etcd gateway | `compat/etcd_http.rs` | No memory-safety defect. |
| Watch registry and replay | `storkv-core/src/watch.rs`, `db_ops/watch.rs` | Unbounded queue and full WAL replay. See F-01, F-02. |
| Replication | `db_ops/replication.rs`, `server.rs:354-419` | Full replay, no admin gate. See F-01, H-03. |
| SDK cache | `storkv-sdk/src/cache.rs`, `scope.rs`, `pool.rs` | TTL and range-delete gaps. See F-06. |
| Consistency | `server.rs:89-95`, `dispatch.rs:37-57,107-110` | Global mutation. See F-05. |
| Backup and restore | `backup/restore.rs`, `manifest.rs`, `manifest/paths.rs`, `verify.rs` | Path confinement gap. See N-01. |
| WAL and SST decoding | `wal/codec.rs`, `wal/recovery.rs`, `sstable/block.rs`, `footer.rs`, `meta.rs` | Bounds hold. Decompression size is uncapped. See N-02. |
| FFI | `storkv-ffi/src/handles.rs`, `scope.rs`, `watch.rs`, `memory.rs` | Null checks and ownership rules hold. See R-07. |
| CLI and TUI | `storkv-cli/src/*`, `storkv-tui/src/*` | No token handling. See H-05 for the server flag. |
| Supply chain | `Cargo.toml`, crate manifests | No Git dependency, no build script from an untrusted source. |

## 3. Confirmed Findings

### F-01: Watch and replication streams have no backpressure and replay the full WAL

| Field | Value |
| :--- | :--- |
| ID | F-01 |
| Severity | `HIGH` |
| Confidence | High |
| Attack class | Resource exhaustion (remote denial of service) |
| Affected component | Core watch registry, native watch and replication loops, WebSocket watcher, gRPC watcher |
| Exact locations | `crates/storkv-core/src/watch.rs:74-78`, `:84-91`, `:94-97`; `crates/storkv-core/src/replication.rs:29-45`, `:48-52`; `crates/storkv-core/src/db_ops/watch.rs:38-54`, `:56-83`; `crates/storkv-core/src/db_ops/replication.rs:20-32`, `:112-129`; `crates/storkv-server/src/server.rs:317-345`, `:354-416`; `crates/storkv-server/src/transports/ws/session.rs:253-285`; `crates/storkv-server/src/transports/grpc/service.rs:282-302` |

**Trust boundary violated.** An untrusted network client reaches the watch registry and the WAL replay path. The server reserves memory on behalf of that client without any limit. No admin role is required.

**Root cause and code trace.**

1. `WatchRegistry::subscribe` and `subscribe_replaying` create `crossbeam_channel::unbounded()` channels (`crates/storkv-core/src/watch.rs:74-78`, `:84-91`).
2. `broadcast` clones every event and sends it to every sender (`watch.rs:94-97`). An event carries the full value bytes for a `Put` (`watch.rs:8-36`).
3. The native watch worker reads the unbounded receiver and calls `watch_sender.blocking_send` into a bounded 128-slot Tokio channel (`crates/storkv-server/src/server.rs:335-345`). When the peer stops reading, the writer stops, the 128-slot channel fills, and `blocking_send` blocks.
4. The blocked worker stops draining its unbounded receiver. Every later write is cloned into that receiver queue without limit. The same pattern exists in the WebSocket watcher (`transports/ws/session.rs:265-284`) and the gRPC watcher (`grpc/service.rs:284-302`).
5. `watch_from` and `replication_frames_from` first read every retained WAL segment and build a full `Vec` of events or frames (`db_ops/watch.rs:56-83`; `db_ops/replication.rs:112-129`). `WatchRegistry::subscribe_replaying` (`watch.rs:84-91`) and `ReplicationRegistry::subscribe_replaying` (`replication.rs:39-46`) then copy the whole replay vector into an unbounded queue before the receiver attaches. A request with `from_seq = 0` therefore allocates memory in proportion to the full retained WAL. The WAL frames include value bytes.
6. The client controls write size and write rate. It can subscribe, stop reading, and write large values through another connection. Each write adds a full value copy to the stalled queue.

**Impact and preconditions.** A single client can exhaust server memory and terminate the process. The client must reach a native, WebSocket, or gRPC listener. A native or WebSocket listener is enough. `--strict-admin` does not stop the attack because watch and replication are not in the admin-only set (`crates/storkv-proto/src/request/types.rs:251-267`). Repeated `WatchFrom { from_seq: 0 }` calls also cause repeated full-WAL allocation.

**Smallest effective remediation.** Replace the unbounded channels with bounded channels. On a full queue, close the slow subscriber or drop events. Cap the replay set. Reject `from_seq` values that would replay more than a fixed byte or event budget. Add a per-connection subscription cap.

---

### F-02: Repeated watch and replication requests leak subscribers and tasks

| Field | Value |
| :--- | :--- |
| ID | F-02 |
| Severity | `HIGH` |
| Confidence | High |
| Attack class | Resource exhaustion (remote denial of service) |
| Affected component | Native connection session, WebSocket session, watch registry |
| Exact locations | `crates/storkv-server/src/server.rs:303-308`, `:317-348`, `:354-416`, `:421-428`; `crates/storkv-server/src/transports/ws/session.rs:75-80`, `:170-196`, `:206-228`; `crates/storkv-core/src/watch.rs:60-63` |

**Trust boundary violated.** An untrusted client can create an unlimited number of server-side watcher tasks and registry entries on one connection. No quota or admin role applies.

**Root cause and code trace.**

1. `ConnectionSession::handle_message` sends every `WatchSubscribe`, `WatchFrom`, and `ReplicationStart` request to a start function (`server.rs:303-308`).
2. `start_watch` builds a new receiver and then assigns `self.watch_task = Some(...)` (`server.rs:317-345`). Assignment drops the old `JoinHandle`. A dropped Tokio `JoinHandle` detaches the task. It does not abort the task. The old task keeps its receiver and keeps sending into the shared `watch_sender`.
3. `start_replication` does the same with `self.replication_task` (`server.rs:399-416`). Old replication tasks stay alive.
4. `stop_watch` aborts only the current handles (`server.rs:421-428`). It cannot stop the detached tasks.
5. Each repeated request also calls `WatchRegistry::subscribe` and pushes another sender into `subscribers` (`watch.rs:60-78`). The vector has no cap.
6. The WebSocket session stores watchers in `Vec<Watcher>` and pushes one for each request (`transports/ws/session.rs:75-80`, `:186`). It stops them only when the socket closes (`:226-228`). One socket can create an unlimited number of watchers.
7. gRPC creates one watcher per `Watch` stream (`grpc/service.rs:282-302`). The stream count has no explicit cap.

**Impact and preconditions.** One native connection can create thousands of detached watcher tasks and registry entries with a short request loop. Each watcher receives every event, so CPU and memory cost multiplies. Combined with F-01, the detached tasks also fill their own unbounded queues. A WebSocket client can do the same without request replacement. The precondition is the same as F-01: reach a native, WebSocket, or gRPC listener.

**Smallest effective remediation.** Abort the previous watch and replication tasks before replacement. Reject a second subscription on the same connection, or set a small fixed limit. Cap the number of subscribers per connection and the total subscriber count.

---

### F-03: Redis inline commands have no line or buffer limit

| Field | Value |
| :--- | :--- |
| ID | F-03 |
| Severity | `MEDIUM` |
| Confidence | High |
| Attack class | Resource exhaustion (remote denial of service) |
| Affected component | Redis RESP session and parser |
| Exact locations | `crates/storkv-server/src/compat/redis.rs:47-87`; `crates/storkv-server/src/compat/redis/codec_decode.rs:38-41`, `:149-167`; `codec_decode.rs:12` |

**Trust boundary violated.** An untrusted Redis client controls the growth of a server-side buffer. The only limit is the declared bulk length. Inline commands have no limit at all.

**Root cause and code trace.**

1. The session reads 16 KiB chunks and appends them to `buffer` without a total cap (`redis.rs:50-59`).
2. The parser calls `parse_inline` when the first byte is not a RESP prefix (`codec_decode.rs:24-28`).
3. `parse_inline` searches for `\r\n` and returns `Ok(None)` when it finds none (`codec_decode.rs:38-41`). The session then reads more bytes and calls the parser again (`redis.rs:60-84`).
4. No code checks the inline line length. A client can send an endless stream of bytes with no CRLF. The buffer grows until memory is exhausted.
5. For bulk strings, `MAX_BULK_LEN` is 512 MiB (`codec_decode.rs:12`). The parser accepts a declared length up to that value and waits for the body (`codec_decode.rs:149-167`). One connection can reserve up to 512 MiB this way. The inline path is worse because it has no bound.
6. The same growth applies before the command reaches `RedisSession::execute`. The authentication and admin layers do not run first because the Redis listener has no token gate.

**Impact and preconditions.** A client that can connect to `--redis-listen` can exhaust process memory with one connection. The attack needs no valid command and no authentication. Multiple connections multiply the effect.

**Smallest effective remediation.** Add a maximum inline line length, for example 64 KiB. Reject the connection when the buffer exceeds that length without a CRLF. Also cap the total read buffer and lower `MAX_BULK_LEN` to the documented value limit.

---

### F-04: Redis glob matching has exponential backtracking

| Field | Value |
| :--- | :--- |
| ID | F-04 |
| Severity | `MEDIUM` |
| Confidence | High |
| Attack class | Resource exhaustion (CPU denial of service) |
| Affected component | Redis `KEYS`, `SCAN MATCH`, and transaction key scans |
| Exact locations | `crates/storkv-server/src/compat/redis/helpers.rs:182-227`; `crates/storkv-server/src/compat/redis/commands_keys.rs:120-138`, `:140-173`, `:184-207`; `crates/storkv-server/src/compat/redis/commands_tx.rs:457-500` |

**Trust boundary violated.** An untrusted client supplies a pattern. The server runs a recursive matcher on that pattern for every key in the scope. The matcher has no step budget.

**Root cause and code trace.**

1. `glob_match` calls `glob_match_at` (`helpers.rs:182-184`).
2. On `*`, the function removes repeated stars and then loops over every text position. For each position it recurses on the rest of the pattern (`helpers.rs:190-205`).
3. The function has no memoization and no visited set. A pattern with several stars and a text with many matching prefixes causes exponential work. A pattern such as `*a*a*a*a*a*a*a*a*a*a` with a long `a` string and a final mismatch is enough.
4. `KEYS` scans the whole scope and calls `glob_match` for each key (`commands_keys.rs:128-135`). `SCAN` does the same for every returned key (`:156-170`). The MATCH argument has no length limit beyond the RESP limit (`:184-207`).
5. The transaction key scan uses the same matcher (`commands_tx.rs:457-500`).

**Impact and preconditions.** A client that can connect to the Redis listener can pin a CPU core with one `KEYS` or `SCAN MATCH` command. A few connections can saturate all cores and block normal service. The attack needs a scope with at least one key. The key value can be small.

**Smallest effective remediation.** Replace the recursive matcher with an iterative linear-time matcher, or use a memoized dynamic program with a step budget. Also cap the pattern length and the number of keys examined per command.

---

### F-05: A non-admin client can change server-wide consistency

| Field | Value |
| :--- | :--- |
| ID | F-05 |
| Severity | `MEDIUM` |
| Confidence | High |
| Attack class | Authorization gap and availability (cross-client state change) |
| Affected component | Request dispatch, server consistency state, SDK pool connect path |
| Exact locations | `crates/storkv-proto/src/request/types.rs:244-248`; `crates/storkv-proto/src/request/decode.rs:63-69`; `crates/storkv-server/src/dispatch.rs:107-110`; `crates/storkv-server/src/server.rs:89-95`; `crates/storkv-server/src/dispatch.rs:37-57`; `crates/storkv-sdk/src/pool.rs:275-287` |

**Trust boundary violated.** One connection changes a server-global setting that controls read acceptance for every other client. The request is not in the admin-only set (`request/types.rs:251-267`). `dispatch_request` calls `set_consistency` for any client when `strict_admin` is off, and `strict_admin` does not help because the request is not admin-only.

**Root cause and code trace.**

1. The wire request is `ConfigureConsistency { consistency }` (`request/types.rs:244-248`). The decoder accepts it from any client (`request/decode.rs:63-69`).
2. `dispatch_request` checks the admin gate first (`dispatch.rs:20-23`). `ConfigureConsistency` is not `is_admin_only`, so the gate does not apply.
3. `route_request` calls `self.set_consistency(consistency)` and returns `Empty` (`dispatch.rs:107-110`).
4. `set_consistency` writes two process-wide atomics: `consistency` and `consistency_max_lag_ms` (`server.rs:89-95`). Every connection reads the same values through `Server::consistency` (`server.rs:97-103`).
5. On a replica, `replica_rejection` applies the global value to every data read (`dispatch.rs:37-57`). `Strong` returns `NotPrimary`. `BoundedStaleness { max_lag_ms: 0 }` returns `StaleReplica` when the lag is above zero. `Eventual` removes the staleness guard.
6. The SDK pool sends its configured consistency on each new connection (`pool.rs:275-287`). The default is `Strong`, as the SDK documentation states (`docs/storkv-sdk/rust.md:133-155`). A replica starts at `Eventual` (`server.rs:64-70`). The first default SDK client therefore changes the replica to `Strong` and makes all data reads fail.

**Impact and preconditions.** Any client can deny reads to all clients on a replica. Any client can downgrade a replica to `Eventual` and defeat an operator's strong-read policy. A default SDK client can trigger the denial by accident. The precondition is a reachable replica and at least one client that sends the request. The impact is bounded to availability and read freshness, not data modification.

**Smallest effective remediation.** Store consistency per connection, not on `Server`. If server-wide configuration is intended, add `ConfigureConsistency` to `is_admin_only` and require an admin grant.

---

### F-06: SDK cache serves expired and range-deleted values

| Field | Value |
| :--- | :--- |
| ID | F-06 |
| Severity | `MEDIUM` |
| Confidence | High |
| Attack class | Data lifecycle and stale-read exposure |
| Affected component | SDK `ScopeCache`, core watch events, transaction commit |
| Exact locations | `crates/storkv-sdk/src/cache.rs:16-19`, `:109-119`, `:209-227`; `crates/storkv-sdk/src/scope.rs:77-91`; `crates/storkv-core/src/db_ops/watch.rs:87-100`; `crates/storkv-core/src/db_ops/commit.rs:202-227` |

**Trust boundary violated.** The SDK promises that a remote change invalidates the cached key (`docs/storkv-sdk/rust.md:125`). The cache does not observe TTL expiry, `DeleteRange`, or `Merge`. A value that the backend no longer returns remains readable through `Scope::get`.

**Root cause and code trace.**

1. `ScopeCache` stores only `Bytes -> Option<Bytes>` (`cache.rs:16-19`). It stores no expiry timestamp.
2. `Scope::get` returns the cached value immediately and never checks TTL (`scope.rs:77-83`).
3. The backend enforces expiry at read time (`crates/storkv-core/src/db_ops/scan.rs:88`; `crates/storkv-core/src/memtable.rs:78`). The cache has no equivalent check and no timer.
4. `ScopeCache::apply_event` handles only `Put`, `Delete`, and `DropScope` (`cache.rs:109-119`). `apply_wire_event` does the same (`cache.rs:209-227`).
5. Core watch event conversion returns `None` for `WalOp::Merge` and `WalOp::DeleteRange` (`db_ops/watch.rs:87-100`).
6. Transaction commit broadcasts `ChangeEvent::Put` and `ChangeEvent::Delete`, but not for `TxMutation::Merge` or `TxMutation::DeleteRange` (`db_ops/commit.rs:187-227`). No event reaches the cache.
7. A `DeleteRange` or an expiry therefore leaves the old value in the cache until LRU eviction, a later put or delete of the same key, a scope drop, or a watcher restart.

**Impact and preconditions.** A process that uses the SDK cache can read data after the backend deleted or expired it. This can break data-retention rules and can expose expired session tokens or credentials. The cache is off by default. The attack needs a victim process with cache enabled and a second client that deletes a range or relies on TTL. The impact is bounded to the victim process and the cached keys.

**Smallest effective remediation.** Store the absolute expiry with each cache entry and reject expired entries on lookup. Add a scope invalidation event for `DeleteRange` and `Merge`, or clear the scope cache when the watch stream cannot represent a mutation. A simple safe option is to clear the whole scope cache on any `DeleteRange` or `Merge` event.

---

### F-07: The native admin Unix socket uses default filesystem permissions

| Field | Value |
| :--- | :--- |
| ID | F-07 |
| Severity | `MEDIUM` |
| Confidence | Medium |
| Attack class | Local privilege escalation |
| Affected component | `storkvd` admin Unix socket setup |
| Exact locations | `crates/storkv-server/src/bin/storkvd.rs:125-135`; no `set_permissions` call exists in the workspace (`grep` over `crates/**/*.rs`) |

**Trust boundary violated.** The admin socket grants `is_admin = true` to every accepted connection (`storkvd.rs:134`). The process does not restrict the socket mode. A local user who can traverse the socket directory can connect and obtain admin rights.

**Root cause and code trace.**

1. `storkvd` removes the old path and calls `tokio::net::UnixListener::bind(path)` (`storkvd.rs:129-131`).
2. The admin listener starts with `is_admin = true` (`storkvd.rs:133-135`).
3. The code never calls `std::fs::set_permissions` or `PermissionsExt::set_mode`. The audit grep found no such call in any crate.
4. A newly bound Unix socket receives mode `0777` masked by the process umask. With the common umask `022`, the mode is `0755`. Any local user can connect.
5. The native protocol has no token field. The socket mode is the only local gate. The documentation presents `--admin-sock` as the protected admin channel (`docs/operations/configuration.md:107`).

**Impact and preconditions.** A local unprivileged user can run admin requests: `DropScope`, `Compact`, `Checkpoint`, snapshot operations, and backup operations. The impact is data destruction and control of backup paths. The precondition is a Unix host with an admin socket, a socket directory that other users can search, and a umask that grants group or other access. On Windows, the Unix branch does not compile.

**Smallest effective remediation.** After bind, set the socket mode to `0600` or `0660` with the intended group. Create the parent directory with mode `0700` before bind. Fail startup if the mode cannot be set.

## 4. Leads Requiring Validation

### N-01: Backup manifest paths are not confined to the backup root

| Field | Value |
| :--- | :--- |
| ID | N-01 |
| Severity | `MEDIUM` (provisional) |
| Confidence | Medium |
| Attack class | Path traversal and symlink escape |
| Exact locations | `crates/storkv-core/src/backup/manifest.rs:146-169`; `crates/storkv-core/src/backup/manifest/paths.rs:11-20`; `crates/storkv-core/src/backup/restore.rs:95-115`, `:247-264`; `crates/storkv-core/src/backup/verify.rs:96-104` |

`BackupManifest::validate` checks the format marker, the version, and the sequence range. It does not validate `generation_id` or `files[].path`. `generation_directory` joins `generation_id` to the root without filtering `..` or an absolute prefix (`paths.rs:18-20`). `manifest_relative_path` filters `/` segments equal to `..`, but it does not filter backslash segments on Windows and it does not reject an absolute drive prefix (`paths.rs:11-16`). `verify_generation` joins the manifest path and reads the result (`verify.rs:96-110`). `copy_tree` follows directory symlinks and copies the target contents (`restore.rs:247-264`).

**Missing fact.** The audit cannot establish whether a backup root can contain files written by an untrusted principal. Local backups under operator control do not cross this boundary. A restore from a shared archive, a remote synchronisation target, or an attacker-writable directory does.

**Safe verification.** Create a temporary backup tree with a manifest that has `generation_id: "../../outside"` and one symlink. Run `storkvctl restore` against a temporary target. Confirm whether the command reads outside the root. Do not run this against production data.

### N-02: Decompression output size is not capped

| Field | Value |
| :--- | :--- |
| ID | N-02 |
| Severity | `MEDIUM` (provisional) |
| Confidence | Medium |
| Attack class | Decompression bomb |
| Exact locations | `crates/storkv-core/src/wal/codec.rs:129-140`; `crates/storkv-core/src/sstable/footer.rs:146-158` |

`decode_compressed_payload` reads a stored uncompressed length but ignores it (`wal/codec.rs:135`). It then calls `lz4_flex::decompress_size_prepended`, which allocates according to the prepended size (`wal/codec.rs:138`). The SSTable path calls the same function for LZ4 and `zstd::stream::decode_all` for Zstd (`footer.rs:146-158`). Neither path applies an output limit before allocation.

**Missing fact.** The audit cannot establish whether a WAL or SST file can come from an untrusted source. A remote client cannot write these files through the network API. A restore of an untrusted backup, or a data directory writable by another local user, can.

**Safe verification.** In a temporary directory, build a small LZ4 frame with a prepended size of 4 GiB. Point a test restore at it with a low memory limit. Confirm the allocation attempt. Do not run this on a shared host.

## 5. Hardening Notes

These items do not cross a verified boundary by themselves. They reduce blast radius and remove ambiguity.

### H-01: Redis and Memcached flush commands bypass `--strict-admin`

`FLUSHDB` and `FLUSHALL` run through `RedisSession::execute_now` without an admin check (`compat/redis/commands.rs:97-98`). `flush_db` calls `compat::flush_scope` (`commands_server.rs:98-107`). `flush_all` calls `compat::flush_redis_scopes` (`commands_server.rs:109-114`). Memcached `flush_all` calls `flush_scope` (`memcached/commands.rs:56`, `:269-278`). `strict_admin` is checked only in `dispatch_request` (`dispatch.rs:20-23`), and the compatibility adapters do not call it.

This is not a confirmed privilege escalation. A Redis client can already delete any key in `default` and every `redis_db_*` scope through `DEL`. A Memcached client can already delete any item in the `memcached` scope. The flush commands add no data access. They do add a global destructive shortcut that ignores the documented admin policy.

**Recommendation.** Add an explicit admin check to `FLUSHALL`, or serve the Redis and Memcached listeners only on a trusted interface. Document the decision.

### H-02: Scope creation has no quota

`Request::CreateScope` is not admin-only (`request/types.rs:251-267`). `DbInner::create_scope` allocates a new scope, a directory, and a manifest edit with no count limit (`db_ops/scopes.rs:18-51`). Redis `SELECT n` creates a scope for any `u32` index (`compat.rs:26-32`, `:65-77`). An unauthenticated client can create scopes until memory or disk runs out. Scope creation is a documented client feature, so this is a quota gap, not an authorization failure. Add a configurable maximum scope count and a per-connection creation rate limit.

### H-03: Watch, `WatchFrom`, and `ReplicationStart` are outside the admin set

`Request::is_admin_only` lists checkpoint, compaction, snapshot, scope drop, and backup requests (`request/types.rs:251-267`). It does not list `WatchSubscribe`, `WatchFrom`, or `ReplicationStart`. The native handler routes these requests before `dispatch_request` (`server.rs:303-308`), so the gate cannot apply. The replication stream contains raw WAL frames for every scope, including overwritten and deleted values (`db_ops/replication.rs:20-32`, `:112-129`).

StorKV has no per-scope authorization. Any client can already scan every scope. The extra exposure is historical data and a full database copy. Gate replication behind an admin grant if the native listener can receive untrusted traffic. Document that the native listener is a full-data trust boundary.

### H-04: `SCAN`, `DBSIZE`, `KEYS`, and `stats` materialize the whole scope

`Db::scan` returns a `Vec` of all pairs. `SCAN` loads the remaining keyspace before it applies `COUNT` (`commands_keys.rs:156-170`). `DBSIZE` scans and counts all pairs (`commands_server.rs:87-96`). Memcached `stats` does the same (`memcached/commands.rs:280-286`). `flush_scope` loads every key and value into memory before it deletes (`compat.rs:108-120`). A large scope causes a large temporary allocation for each call. Stream the cursor and count keys without values. Use a range delete for flush.

### H-05: The admin token is visible in process arguments

`storkvd` takes the token as `--admin-token` (`bin/storkvd.rs:54-56`). The documentation shows the token on the command line (`docs/operations/configuration.md:74-75`). On a multi-user host, another user can read the process arguments. Prefer an environment variable or a mode-`0600` token file. Keep the command-line form for tests only.

### H-06: Scopes are not a security boundary

The project describes scopes as multi-tenant isolation (`SPEC.md:5`). The network API has no per-scope authorization. Any client can send any `scope_id` to `Get`, `Put`, `Scan`, or `Delete` (`dispatch.rs:72-92`). A client can also enumerate scope identifiers through watch events and `AdminStats`. Operators must not treat two scopes on one listener as two security tenants. Put separate tenants on separate servers, listeners, or reverse-proxy rules.

### H-07: FFI callers must honor the ownership contract

The FFI layer checks null handles and null output pointers (`storkv-ffi/src/scope.rs:30-41`, `:196-207`; `watch.rs:47-58`, `:90-99`). It reconstructs slices from raw pointer and length (`scope.rs:8-16`). It frees only pointers that it allocated (`memory.rs:35-72`). A caller that passes a dangling pointer, a bad length, or a double free breaks the contract and can corrupt memory. This is the normal C ABI model. Keep the contract in the header and add a debug handle registry only if untrusted native callers are in scope.

### H-08: Restore staging uses a predictable name

`staging_target` uses the target name and the process id (`restore.rs:239-245`). The directory sits next to the target. A local user with write access to the parent can pre-create the path. The code removes an existing staging directory before use (`restore.rs:89-93`), so the practical risk is low. Use a random suffix and create the directory with `create_new` semantics.

## 6. Rejected Candidates

These leads were investigated and disproved. Keep them here so later audits do not repeat the work.

| ID | Candidate | Reason for rejection |
| :--- | :--- | :--- |
| R-01 | Frame length overflow or large pre-allocation in `storkv-proto` | `read_frame_length` rejects a length above 64 MiB before allocation (`frame.rs:95-107`). `get_bytes` checks the remaining length before `split_to` (`frame.rs:218-226`). Response vectors check the count against the payload size before `with_capacity` (`response/decode.rs:47-58`, `:70-78`). |
| R-02 | Non-constant-time admin token comparison | `tokens_match` compares equal-length byte slices with an XOR accumulator (`transports.rs:72-83`). Length differences return early, which is not secret-dependent in a useful way. |
| R-03 | gRPC admin method bypass | Every admin method calls `dispatch` or `ok`, and both call `dispatch_request` with `is_admin` (`grpc.rs:88-97`; `grpc/admin.rs:15-148`). `Checkpoint`, `Compact`, backup, and snapshot requests are in the admin set. |
| R-04 | Path traversal through a scope name | Scope directories use the numeric scope id, not the name (`scope/state.rs:47-55`). A name with `../` cannot select a directory. |
| R-05 | Replication injection into a replica | A replica rejects `ReplicationStart` with `NotPrimary` (`server.rs:354-364`). A replica also rejects writes (`dispatch.rs:37-43`). The apply function is not reachable from the native request path. |
| R-06 | Memcached key, value, or command overflow | The parser caps the key at 250 bytes, the value at 1 MiB, and the line at 8 KiB (`memcached/parser.rs:5-6`, `:112-114`, `:207-211`; `memcached.rs:18`, `:76-83`). |
| R-07 | FFI null pointer or double free | Every entry point checks null handles and null output pointers before use (`scope.rs:30-41`, `:128-137`, `:196-207`, `:230-240`, `:267-272`; `watch.rs:47-58`, `:90-99`). Free functions require the documented owner pointer. |
| R-08 | Transaction `read_seq` spoofing as an authorization bypass | `TxCommit` trusts the client `read_seq` (`dispatch/transactions.rs:15-47`). A client can already write any key with `Put`, so this changes conflict detection, not access. It is a consistency concern outside this threat model. |
| R-09 | Cross-scope watch events as an authorization bypass | Watch events carry `scope_id`, and the matcher filters only when the caller asks for one scope (`db_ops/watch.rs:76-78`; `grpc/service.rs:291-293`). StorKV has no per-scope authorization, so the event does not grant data that a scan would deny. See H-06. |
| R-10 | Missing TLS as a vulnerability | The project documents plaintext listeners and a reverse-proxy TLS model (`docs/guides/transports.md:153-161`). This is a documented deployment choice, not a defect. |

## 7. Final Verification

The author re-read this report against the working tree on 2026-11-18.

- Every confirmed finding cites a function that exists at the stated path and line range.
- Severity counts match the finding sections: 0 `CRITICAL`, 2 `HIGH`, 5 `MEDIUM`, 0 `LOW`.
- The audit made no source change. The only file added by this audit is `docs/internal/SECURITY_AUDIT.md`.
- The worktree already contained many modified and untracked files from earlier feature work before this audit started. `git status` therefore does not show a clean tree. This report does not claim to have created or changed those files.
- `docs/internal/SECURITY_AUDIT.md` is the sole output artifact of this audit.
