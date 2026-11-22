# StorKV Security Audit

## 1. Audit Metadata

| Field          | Value                                                                           |
| :------------- | :------------------------------------------------------------------------------ |
| Project        | StorKV (`storkv`)                                                               |
| Audit date     | 2026-11-20                                                                      |
| Branch         | `main`                                                                          |
| HEAD commit    | `cc14dec8221f9b9e21cda9146015a4c5391859ef` (`2026-11-20 22:04:44 +0100`)        |
| Worktree state | Clean at audit start. `git status --porcelain` returned no entries.             |
| Workspace size | 402 Rust source files (workspace, `target/` excluded)                           |
| Method         | Source-grounded review with entrypoint-to-sink tracing and adversarial disproof |
| Constraints    | Read-only inspection. No live network probe. No destructive command.            |
| Sole output    | This file.                                                                      |

This audit uses the Cloudflare `security-audit` bar. A finding needs a real trust boundary, a verified code path, a missing control, and a bounded impact. A defense-in-depth gap is a hardening note.

The previous report was written against commit `009ff6f`. This report re-reads the current tree at `cc14dec`. It does not reuse the previous findings. Section 6 records the previous findings that the current code closes.

### 1.1 Severity Counts

| Severity            | Confirmed |
| :------------------ | --------: |
| `CRITICAL`          |         0 |
| `HIGH`              |         1 |
| `MEDIUM`            |         2 |
| `LOW`               |         1 |
| **Total confirmed** |     **4** |

| Other class        | Count |
| :----------------- | ----: |
| `NEEDS_VALIDATION` |     2 |
| `HARDENING_NOTE`   |     8 |
| `REJECTED`         |    15 |

### 1.2 Executive Summary

The current code fixes the confirmed findings of the previous audit, with one exception. Watch and replication queues are bounded. Subscriber counts are capped. Redis inline commands and glob matching are bounded. Consistency is per connection. Backup paths are confined. Decompression output is capped. The native admin Unix socket uses `0700` and `0600` permissions. The TTL part of the previous SDK cache finding remains.

This audit confirms four issues. Three are new. One is the remaining TTL part of the previous SDK cache finding.

The compatibility listeners bypass the replica write rejection. The native, HTTP, WebSocket, and gRPC data paths call `dispatch_request`, which rejects writes on a replica. The Redis, Memcached, and etcd write paths call the database directly. A replica that runs an enabled compatibility listener accepts local writes. A local write also advances the local sequence counter. The replica then discards primary frames below that counter. The result is silent divergence.

Replica recovery from an expired cursor is broken. The primary sends a snapshot before the acknowledgment. The client expects the acknowledgment first. The client treats the snapshot as a protocol error and retries the same expired cursor forever.

The recovery snapshot also drops time-to-live values and does not remove keys that are absent from the snapshot. This defect is latent while the recovery loop is broken.

The SDK cache ignores server-side time-to-live expiry. A value cached through `Scope::get` can stay readable after the server hides it.

## 2. Architecture and Trust-Boundary Map

### 2.1 Untrusted Entrypoints

| Entrypoint                            | Code                                                                                                                                                                          | Privilege gate                                                                                                              |
| :------------------------------------ | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------- |
| Native TCP, TLS, and UDS (`STORKPRT`) | `crates/storkv-server/src/server/listener.rs:20-60,68-101,108-158`; `crates/storkv-proto/src/frame.rs:11-12`                                                                  | `is_admin` comes from the listener. The native protocol has no token field.                                                 |
| HTTP REST and JSON-RPC                | `crates/storkv-server/src/transports/http.rs:31-50`; `crates/storkv-server/src/transports/http/rpc.rs:34-38`; `crates/storkv-server/src/transports/http/rest_admin.rs:13-103` | `HttpState::is_admin` reads `x-storkv-admin-token` or a bearer token (`crates/storkv-server/src/transports/http.rs:32-41`). |
| WebSocket binary and JSON-RPC         | `crates/storkv-server/src/transports/ws/session/routing.rs:25-44`; `crates/storkv-server/src/transports/ws/session.rs:141,183`                                                | Token check at upgrade. Requests call `dispatch_request`.                                                                   |
| gRPC data and admin                   | `crates/storkv-server/src/transports/grpc.rs:72-96`; `crates/storkv-server/src/transports/grpc/service/data.rs:16-169`                                                        | Metadata token at `crates/storkv-server/src/transports/grpc.rs:73-86`. Data methods call `dispatch`.                        |
| Redis RESP2/RESP3                     | `crates/storkv-server/src/compat/redis.rs:30-70`                                                                                                                              | No authentication. No replica role gate.                                                                                    |
| Memcached ASCII                       | `crates/storkv-server/src/compat/memcached.rs:20-60`                                                                                                                          | No authentication. No replica role gate.                                                                                    |
| etcd v3 JSON                          | `crates/storkv-server/src/compat/etcd_http.rs:18-135`; `transports/http.rs:83-87`                                                                                             | No authentication. No replica role gate.                                                                                    |
| Replication stream                    | `crates/storkv-server/src/server/session.rs:123-206`                                                                                                                          | `strict_admin` check. Primary-only check.                                                                                   |
| C ABI                                 | `crates/storkv-ffi/src/*.rs`                                                                                                                                                  | Caller-owned raw pointers. Documented `unsafe` contract.                                                                    |
| On-disk WAL, SST, manifest, backup    | `crates/storkv-core/src/wal/*`; `sstable/*`; `backup/*`                                                                                                                       | Local filesystem trust. Restore uses an operator-selected root.                                                             |

### 2.2 Principals and Boundaries

| Principal                      | Resource                          | Control                                                                                                                             |
| :----------------------------- | :-------------------------------- | :---------------------------------------------------------------------------------------------------------------------------------- |
| Non-admin network client       | Admin-only request set            | `dispatch_request` at `crates/storkv-server/src/dispatch.rs:26-28`; set at `crates/storkv-proto/src/request/types/methods.rs:10-23` |
| Admin listener or token holder | Global state and backup internals | `strict_admin` and `TransportOptions::grants_admin` at `crates/storkv-server/src/transports.rs:61-83`                               |
| Local user                     | Native admin Unix socket          | Parent `0700` and socket `0600` at `crates/storkv-server/src/bin/storkvd.rs:161-172`                                                |
| Any client                     | Scope data                        | No per-scope authorization. Scopes are a namespace, not a security principal (`SPEC.md:23`).                                        |
| Any client                     | Watch and replication events      | Transport connection only. `ReplicationStart` is admin-only under `strict_admin`.                                                   |
| Any client                     | Server consistency setting        | Per-connection `ConnectionConsistency` at `crates/storkv-server/src/server/consistency.rs:9-37`                                     |
| Compat client                  | Redis, Memcached, and etcd data   | No authentication. No replica role gate. See F-01.                                                                                  |
| Replica operator               | Primary frame stream              | Operator-selected URI. No transport TLS or peer authentication.                                                                     |
| Backup operator                | Backup manifest, WAL, SST         | Local trust. Path confinement is enforced.                                                                                          |
| C caller                       | FFI handles and slices            | Documented safety contract                                                                                                          |

### 2.3 Coverage Ledger

| Area                          | Files reviewed                                                                                                                                                                                                                                                                 | Result                                                         |
| :---------------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :------------------------------------------------------------- |
| Frame and request decoding    | `crates/storkv-proto/src/frame.rs`, `crates/storkv-proto/src/request/decode.rs`, `crates/storkv-proto/src/response/decode.rs`                                                                                                                                                  | Bounds checks hold. See R-10.                                  |
| Native dispatch and admin set | `crates/storkv-server/src/server.rs`, `crates/storkv-server/src/server/listener.rs`, `crates/storkv-server/src/server/session.rs`, `crates/storkv-server/src/dispatch.rs`, `crates/storkv-proto/src/request/types/methods.rs`                                                  | Admin gate holds. Replica gate covers this path. See R-12.     |
| HTTP and REST                 | `crates/storkv-server/src/transports/http.rs`, `crates/storkv-server/src/transports/http/rest.rs`, `crates/storkv-server/src/transports/http/rest_admin.rs`, `crates/storkv-server/src/transports/http/rest_data.rs`, `crates/storkv-server/src/transports/http/rpc.rs`        | Token gate holds. Admin handlers call `dispatch`.              |
| WebSocket                     | `crates/storkv-server/src/transports/ws/session.rs`, `crates/storkv-server/src/transports/ws/session/watch.rs`, `crates/storkv-server/src/transports/ws/session/routing.rs`                                                                                                    | Token gate and dispatch parity hold.                           |
| gRPC                          | `crates/storkv-server/src/transports/grpc.rs`, `crates/storkv-server/src/transports/grpc/service/data.rs`, `crates/storkv-server/src/transports/grpc/service/build.rs`, `crates/storkv-server/src/transports/grpc/service/watch.rs`                                            | Token gate and dispatch parity hold.                           |
| Redis parser and commands     | `crates/storkv-server/src/compat/redis.rs`, `crates/storkv-server/src/compat/redis/codec_decode/parse.rs`, `crates/storkv-server/src/compat/redis/helpers/glob.rs`, `crates/storkv-server/src/compat/redis/commands_*.rs`, `crates/storkv-server/src/compat/redis/keyspace.rs` | Bounds hold. Replica write gate is missing. See F-01.          |
| Memcached                     | `crates/storkv-server/src/compat/memcached.rs`, `crates/storkv-server/src/compat/memcached/parser.rs`, `crates/storkv-server/src/compat/memcached/commands/store.rs`, `crates/storkv-server/src/compat/memcached/commands/modify.rs`                                           | Length caps hold. Replica write gate is missing. See F-01.     |
| etcd gateway                  | `crates/storkv-server/src/compat/etcd_http.rs`                                                                                                                                                                                                                                 | Replica write gate is missing. See F-01.                       |
| Watch registry and replay     | `crates/storkv-core/src/watch.rs`, `crates/storkv-core/src/db_ops/watch.rs`                                                                                                                                                                                                    | Bounded queue and subscriber cap hold. See R-01, R-02.         |
| Replication server            | `crates/storkv-server/src/server/session.rs:123-206`, `crates/storkv-core/src/replication.rs`, `crates/storkv-core/src/db_ops/replication.rs`                                                                                                                                  | Recovery ordering and snapshot semantics fail. See F-02, F-03. |
| Replication client            | `crates/storkv-client/src/backend.rs`, `crates/storkv-client/src/backend/native.rs`, `crates/storkv-client/src/async_client.rs`, `crates/storkv-server/src/replication.rs`                                                                                                     | Cursor error handling fails. See F-02.                         |
| SDK cache                     | `crates/storkv-sdk/src/scope/cached.rs`, `crates/storkv-sdk/src/cache/store.rs`, `crates/storkv-sdk/src/cache/events.rs`, `crates/storkv-sdk/src/watch/invalidator.rs`                                                                                                         | TTL gap. See F-04, N-02.                                       |
| Backup and restore            | `crates/storkv-core/src/backup/manifest/paths.rs`, `crates/storkv-core/src/backup/manifest.rs`, `crates/storkv-core/src/backup/restore/staging.rs`                                                                                                                             | Path confinement and symlink rejection hold. See R-08.         |
| WAL and SST decoding          | `crates/storkv-core/src/wal/codec.rs`, `crates/storkv-core/src/sstable/footer.rs`                                                                                                                                                                                              | Decompression caps hold. See R-09.                             |
| FFI                           | `crates/storkv-ffi/src/handles.rs`, `crates/storkv-ffi/src/scope.rs`, `crates/storkv-ffi/src/watch.rs`, `crates/storkv-ffi/src/memory.rs`                                                                                                                                      | Null checks and ownership rules hold. See H-08.                |
| CLI and TUI                   | `crates/storkv-cli/src/*`, `crates/storkv-tui/src/*`                                                                                                                                                                                                                           | No token handling.                                             |
| Supply chain                  | `Cargo.toml`, crate manifests                                                                                                                                                                                                                                                  | No Git dependency. No untrusted build script.                  |

## 3. Confirmed Findings

### F-01: Compatibility listeners bypass replica write rejection

| Field    | Value                                       |
| :------- | :------------------------------------------ |
| Verdict  | `CONFIRMED`                                 |
| Severity | `HIGH`                                      |
| Boundary | Network client to replica storage integrity |
| Status   | New                                         |

**Locations**

- `crates/storkv-server/src/dispatch.rs:29-31,42-66`
- `crates/storkv-server/src/compat/redis.rs:30-52`
- `crates/storkv-server/src/compat/redis/keyspace.rs:70-90`
- `crates/storkv-server/src/compat/redis/commands_strings.rs:56,81,101,139,170`
- `crates/storkv-server/src/compat/redis/commands_keys.rs:22-30,67-77,92-97`
- `crates/storkv-server/src/compat/memcached/commands/store.rs:25-71`
- `crates/storkv-server/src/compat/memcached/commands/modify.rs:14-19,32-63,76-81`
- `crates/storkv-server/src/compat/etcd_http.rs:31-35,119-128`
- `crates/storkv-server/src/bin/storkvd.rs:71-76,234-249`

**Description**

The server rejects writes on a replica inside `dispatch_request`. The function calls `replica_rejection` for every dispatched request (`crates/storkv-server/src/dispatch.rs:29-31`). That helper returns `NotPrimary` when `self.role()` is `Replica` and the request is a write (`crates/storkv-server/src/dispatch.rs:47-52`).

The Redis, Memcached, and etcd front ends do not call `dispatch_request`. They call database methods directly. The Redis session is created from the server (`crates/storkv-server/src/compat/redis.rs:47-52`). The write commands call `keyspace::atomic` and `Atomic::put` (`crates/storkv-server/src/compat/redis/commands_strings.rs:136-142`). `keyspace::atomic` commits through `db.inner().commit_tx` (`crates/storkv-server/src/compat/redis/keyspace.rs:86`). The Memcached store and modify commands use the same helper (`crates/storkv-server/src/compat/memcached/commands/store.rs:25-71`; `crates/storkv-server/src/compat/memcached/commands/modify.rs:14-19,32-63`). The etcd put and delete-range handlers call `put_absolute` and `begin_tx` directly (`crates/storkv-server/src/compat/etcd_http.rs:31-35,119-128`).

A source search under `crates/storkv-server/src/compat` finds no `role()`, `ReplicationRole`, or `NotPrimary` check. The listener startup does not reject a replica role (`crates/storkv-server/src/bin/storkvd.rs:234-249`).

**Impact**

A network client can write to a replica through any enabled compatibility listener. The write is visible to local readers and is persisted in the local WAL.

The write also advances `last_visible_seq`. `commit_tx` adds the write-set length to that counter (`storkv-core/src/db_ops/commit.rs:31-35`). The replication apply path ignores any frame whose last sequence is below the current counter (`storkv-core/src/db_ops/replication.rs:51-54`). A local compatibility write can therefore cause the replica to discard valid primary frames. The replica diverges from the primary without an error.

The Redis `HELLO` reply also reports `role=primary` on a replica (`crates/storkv-server/src/compat/redis/commands_server.rs:47`). This misleads clients and operators.

**Preconditions**

- The node runs with `--role replica`.
- The node enables `--redis-listen`, `--memcached-listen`, or the HTTP listener that mounts `/v3/kv/*` (`crates/storkv-server/src/transports/http.rs:83-87`).

The native, HTTP data, WebSocket, and gRPC data paths are not affected. They call `dispatch_request` and receive `NotPrimary`.

**Recommendation**

Add one shared replica write guard and call it from every compatibility write command. Return the Redis `READONLY` error, the Memcached `SERVER_ERROR` reply, or the etcd `NotPrimary` status. Do not rely on `dispatch_request` alone. Also report the real role in `HELLO`.

### F-02: A replica cannot recover from an expired replication cursor

| Field    | Value                                   |
| :------- | :-------------------------------------- |
| Verdict  | `CONFIRMED`                             |
| Severity | `MEDIUM`                                |
| Boundary | Replica availability and data freshness |
| Status   | New                                     |

**Locations**

- `crates/storkv-server/src/server/session.rs:141-174,204`
- `crates/storkv-client/src/backend.rs:96-107`
- `crates/storkv-client/src/backend/native.rs:44-65`
- `crates/storkv-server/src/replication.rs:37-50`

**Description**

The primary detects an expired cursor in `start_replication`. It builds a snapshot and a frame stream from `oldest_available` (`crates/storkv-server/src/server/session.rs:141-160`). It writes `Response::ReplicationSnapshot` first (`crates/storkv-server/src/server/session.rs:168-173`). It starts the frame task and then writes the `Response::Empty` acknowledgment (`crates/storkv-server/src/server/session.rs:204`).

The client starts replication through `Backend::start_replication`. That function sends one request and reads the first response. It accepts only `Response::Empty` (`crates/storkv-client/src/backend.rs:96-107`). The native backend returns the first decoded response without checking the request ID (`crates/storkv-client/src/backend/native.rs:44-65`). The snapshot is therefore consumed and converted into `ClientError::Protocol`.

`run_replica` advances `from_seq` only for `ClientError::Server(WatchCursorExpired)` (`crates/storkv-server/src/replication.rs:37-46`). A protocol error falls into the generic retry branch (`crates/storkv-server/src/replication.rs:47-50`). The loop reconnects with the same expired cursor. The primary sends the same snapshot first. The loop never progresses.

**Impact**

A replica that falls behind the retained WAL never recovers. It retries every 100 ms and remains stale or unavailable. The frame stream never starts, so the replica never applies new primary writes.

The snapshot handler in `run_replica` (`crates/storkv-server/src/replication.rs:57-59`) is unreachable for this case because `start_replication` fails first.

**Preconditions**

- The replica cursor is older than the retained WAL history.
- The primary returns `WatchCursorExpired` from `replication_frames_from` (`storkv-core/src/db_ops/replication.rs:29-34`).

**Recommendation**

Define one explicit recovery sequence. Either send `Empty` before the snapshot, or make the client accept a snapshot as the first response and queue it for the stream. Do not advance `from_seq` on a protocol error. Add a state-machine test for the expired-cursor path.

### F-03: The replication snapshot drops TTL and stale-key state

| Field    | Value                    |
| :------- | :----------------------- |
| Verdict  | `CONFIRMED`              |
| Severity | `MEDIUM`                 |
| Boundary | Replica data integrity   |
| Status   | New. Latent behind F-02. |

**Locations**

- `crates/storkv-core/src/db_ops/replication.rs:86-105`
- `crates/storkv-core/src/db_ops/replication.rs:115-131`

**Description**

`replication_snapshot` returns only `(scope_id, key, value)` tuples (`crates/storkv-core/src/db_ops/replication.rs:86-105`). It does not return the expiry of each key. It also does not return the set of scopes or the set of keys that the replica must remove.

`apply_replication_snapshot` writes each tuple as a `WalOp::Put` with `expires_at_ms: 0` (`crates/storkv-core/src/db_ops/replication.rs:122-129`). It never deletes a key that is absent from the snapshot. It never drops a scope that is absent from the snapshot.

**Impact**

After a snapshot recovery, keys that had a finite TTL become permanent. Keys that the primary deleted before the snapshot remain readable on the replica. The replica reports a complete recovery but holds data that the primary no longer exposes.

The impact is currently latent because F-02 prevents the snapshot from reaching the client. The defect becomes active when F-02 is fixed or when another client applies a snapshot.

**Recommendation**

Include `expires_at_ms` in the snapshot tuple. Add a clear phase before the snapshot write, or stream explicit tombstones for keys and scopes that are absent. Tie the snapshot to a read sequence and reject frames below that sequence.

### F-04: The SDK cache ignores server-side TTL expiry

| Field    | Value                                     |
| :------- | :---------------------------------------- |
| Verdict  | `CONFIRMED`                               |
| Severity | `LOW`                                     |
| Boundary | Client data freshness and TTL enforcement |
| Status   | Reconfirmed. TTL part of previous F-06.   |

**Locations**

- `crates/storkv-sdk/src/scope/cached.rs:17-31`
- `crates/storkv-sdk/src/cache/store.rs:91-94`
- `crates/storkv-sdk/src/cache/events.rs:28-43`
- `crates/storkv-core/src/db_ops/scan.rs:88`
- `crates/storkv-core/src/db_ops/commit/apply.rs:148-188`
- `crates/storkv-core/src/db_ops/scopes.rs:79-84`

**Description**

`Scope::get` stores a fetched value with `self.cache.store(key, value.clone())` (`crates/storkv-sdk/src/scope/cached.rs:27-29`). `ScopeCache::store` inserts the entry with `expires_at: None` (`crates/storkv-sdk/src/cache/store.rs:91-94`). The cache therefore has no local deadline for that value.

The watch invalidation path does not add the server expiry. A `Put` event calls `store_event`, which calls `preserve_expiry` (`crates/storkv-sdk/src/cache/events.rs:33,40-43`). That function copies only an expiry that the local cache already holds (`crates/storkv-sdk/src/cache/store.rs:59-62`). A remote `PutWithTtl` event therefore stores the new value without the remote TTL.

The server expires keys lazily on read (`crates/storkv-core/src/db_ops/scan.rs:88`). It does not broadcast a watch event when a TTL passes. The only watch broadcasts are commit, delete, merge, range delete, and scope drop (`crates/storkv-core/src/db_ops/commit/apply.rs:148-188`; `crates/storkv-core/src/db_ops/scopes.rs:79-84`).

**Impact**

If a key is written by another client with a TTL, or by a scope default TTL, the SDK can serve the cached value after the server hides the key. If the TTL is used to expire credentials, sessions, or other sensitive values, the SDK extends their lifetime without bound.

The cache is opt-in. The defect requires that the value was read through the cache before expiry.

**Recommendation**

Return the effective expiry from the server read path and store it with the cached value. Alternatively, do not cache keys that can expire. Add a periodic sweep for cached entries whose server TTL has passed.

## 4. Leads Requiring Validation

### N-01: Stalled watch streams can exhaust the blocking pool

| Field    | Value               |
| :------- | :------------------ |
| Verdict  | `NEEDS_VALIDATION`  |
| Severity | `MEDIUM` candidate  |
| Boundary | Server availability |

**Locations**

- `crates/storkv-server/src/server/session.rs:94-114,179-203`
- `crates/storkv-server/src/server.rs:20`
- `crates/storkv-server/src/server/listener.rs:118-119`
- `crates/storkv-server/src/transports/grpc/service/watch.rs:32-52`
- `crates/storkv-server/src/transports/grpc.rs:32-33`
- `crates/storkv-server/src/transports/ws/session/watch.rs:48-67`
- `crates/storkv-server/src/transports/ws/session.rs:21-22`
- `crates/storkv-core/src/watch.rs:7-13`

**Description**

Native, gRPC, and WebSocket watch workers run on `spawn_blocking`. Each worker forwards events with `blocking_send` into a bounded `tokio::sync::mpsc` channel of capacity 128. The worker stops draining the core watch channel while `blocking_send` waits.

The core watch registry caps one subscriber at 1024 queued events (`crates/storkv-core/src/watch.rs:7`). It caps the total subscriber count at 1024 (`crates/storkv-core/src/watch.rs:9`). These caps do not release a blocking thread that is parked in `blocking_send`.

Tokio uses a default maximum of 512 blocking threads. A client that opens a watch and stops reading can park one blocking thread after the 128-slot channel fills. Many such clients can exhaust the pool. Other `spawn_blocking` work then starves.

**Why validation is needed**

The impact depends on the Tokio blocking-pool size, the number of stalled subscribers, and the event rate. This audit did not run a load test. A test should open many watch streams, stop reading, and measure blocking-thread saturation and request latency.

**Recommendation**

Use a bounded async channel with `try_send` and drop slow subscribers instead of `blocking_send`. Add a per-connection watch queue limit and a global blocking-task budget.

### N-02: The networked SDK cache watcher cannot recover from an expired cursor

| Field    | Value                                |
| :------- | :----------------------------------- |
| Verdict  | `NEEDS_VALIDATION`                   |
| Severity | `LOW` candidate                      |
| Boundary | SDK cache availability and freshness |

**Locations**

- `crates/storkv-sdk/src/watch/invalidator.rs:61-101`

**Description**

The networked watcher starts at `next_seq = 0` (`crates/storkv-sdk/src/watch/invalidator.rs:67`). On every outer-loop pass it clears the cache (`crates/storkv-sdk/src/watch/invalidator.rs:72`). It then connects and subscribes from `next_seq`. If the subscribe fails, it sleeps and retries with the same cursor (`crates/storkv-sdk/src/watch/invalidator.rs:81-87`).

When the server reports an expired cursor, the watcher never resets or advances the cursor. It also never applies a snapshot. The cache is cleared every 250 ms while the failure continues. Reads can repopulate the cache between clears, and no invalidation arrives during the failure.

**Why validation is needed**

This audit did not prove a permanent stale read. The cache is cleared at the start of each retry. The defect is a cache-thrash and availability problem, with a possible short stale window. A test should expire the server cursor and observe cache hit behavior and retry load.

**Recommendation**

Handle `WatchCursorExpired` explicitly. Reset `next_seq` to zero and perform a full cache clear, or restart from `oldest_available`. Do not clear the cache on every transient connection failure.

## 5. Hardening Notes

### H-01: The replication channel has no TLS or peer authentication

The native listener can use TLS (`crates/storkv-server/src/bin/storkvd.rs:77-82`; `crates/storkv-server/src/server/listener.rs:34-60`). The replication client uses `ClientStream`, which supports only plain TCP and Unix sockets (`crates/storkv-client/src/transport.rs:10-16`). `AsyncClient::connect_uri` maps `storkv://` to plain native TCP (`crates/storkv-client/src/uri.rs:29-31`). No native TLS variant exists (`crates/storkv-client/src/uri.rs:24-58`).

A replica that crosses a host boundary sends its cursor and receives full WAL frames in plaintext. It also trusts any endpoint at `primary_uri` with no peer authentication. A replica therefore cannot use the server TLS listener.

**Recommendation:** support TLS in the native client and in `primary_uri`. Pin or verify the primary certificate.

### H-02: Redis `HELLO` reports `role=primary` on a replica

`crates/storkv-server/src/compat/redis/commands_server.rs:47` hard-codes `Resp::bulk("primary")`. A replica therefore advertises itself as a primary. Some clients and operators use this field for routing.

**Recommendation:** report the configured role.

### H-03: Compatibility listeners have no authentication

The Redis, Memcached, and etcd front ends accept unauthenticated connections (`crates/storkv-server/src/compat/redis.rs:30-52`; `crates/storkv-server/src/compat/memcached.rs:20-42`; `crates/storkv-server/src/compat/etcd_http.rs:18-35`). Any client that reaches the port can read and write the mapped scopes.

This is expected for drop-in compatibility. It still means that the compatibility ports need network-level isolation.

**Recommendation:** document the trust model. Add an optional bind restriction or a proxy requirement.

### H-04: Any data client can create scopes without a rate limit

`Request::CreateScope` is a data write, not an admin-only request (`crates/storkv-proto/src/request/types/methods.rs:10-23`). The compatibility `ensure_scope` also creates a scope on first use (`crates/storkv-server/src/compat.rs:35-63`). The engine caps the total at 1024 scopes (`SPEC.md:22`), but there is no per-client rate limit.

**Recommendation:** add an operator policy for scope creation. Add a rate limit on the compatibility paths.

### H-05: `KEYS`, `SCAN`, `DBSIZE`, and Memcached `stats` materialize a whole scope

`KEYS` calls `db.scan` over the full scope and filters in memory (`crates/storkv-server/src/compat/redis/commands_keys.rs:120-141`). `DBSIZE` calls the same scan and counts the result (`crates/storkv-server/src/compat/redis/commands_server.rs:92-94`). Memcached `stats` does the same (`crates/storkv-server/src/compat/memcached/commands/modify.rs:102-108`).

**Recommendation:** add a scan budget or a streaming iterator. Reject unbounded admin-less scans on large scopes.

### H-06: The admin token can appear in process arguments

`--admin-token` is a command-line argument (`crates/storkv-server/src/bin/storkvd.rs:56-58`). A local user can read it from the process table on many systems. The `STORKV_ADMIN_TOKEN` environment variable is also readable by the same user.

**Recommendation:** prefer `--admin-token-file` and require restrictive file permissions. Document the exposure.

### H-07: Scopes are logical namespaces, not security tenants

`SPEC.md:23` states that a scope does not isolate access. Any client on a listener can address any scope ID on that listener. This audit did not report cross-scope reads as a vulnerability.

**Recommendation:** keep the documented model. Put tenants on separate servers, listeners, or proxy rules.

### H-08: The FFI ownership contract is caller-enforced

The C ABI uses caller-owned raw pointers and opaque handles. The Rust side checks null pointers and handle validity, but it cannot prevent a caller from violating the documented lifetime and aliasing rules.

**Recommendation:** keep the safety documentation next to each exported symbol. Add Miri or sanitizer coverage for the examples.

## 6. Rejected Candidates

### R-01: Watch and replication queues were unbounded

**Rejected.** `WATCH_CHANNEL_CAPACITY`, `MAX_WATCH_SUBSCRIBERS`, `MAX_REPLAY_EVENTS`, and `MAX_REPLAY_BYTES` are defined at `crates/storkv-core/src/watch.rs:6-13`. `subscribe_replaying` uses a bounded channel at `crates/storkv-core/src/watch.rs:138` and enforces the subscriber cap at `crates/storkv-core/src/watch.rs:144-150`. The replication registry has the same bounds at `crates/storkv-core/src/replication.rs:6-13,66,72-78`.

### R-02: Repeated watch and replication requests leaked subscribers and tasks

**Rejected.** Each new watch or replication request calls `stop_task` first (`crates/storkv-server/src/server/session.rs:76,140`). `stop_task` sets the stop flag and aborts a task that has not started (`crates/storkv-server/src/server/session.rs:214-224`). The worker also polls the stop flag while it waits (`crates/storkv-server/src/server/session.rs:94-114,179-203`). The registry removes a full or disconnected subscriber (`crates/storkv-core/src/watch.rs:159-165`; `crates/storkv-core/src/replication.rs:87-93`).

### R-03: Redis inline commands had no line or buffer limit

**Rejected.** `MAX_INLINE_LINE_LEN` is 64 KiB and `MAX_READ_BUFFER` is derived from the bulk limit plus that line limit (`crates/storkv-server/src/compat/redis/codec_decode/parse.rs:8-12`). The parser rejects an oversized inline line.

### R-04: Redis glob matching had exponential backtracking

**Rejected.** The matcher is iterative, uses one backtrack point, and has no recursion (`crates/storkv-server/src/compat/redis/helpers/glob.rs:8-77`). `MAX_GLOB_PATTERN_LEN` caps the pattern at 1024 bytes (`crates/storkv-server/src/compat/redis/helpers/glob.rs:5-6`). The cost is `O(pattern_len * text_len)`.

### R-05: A non-admin client could change server-wide consistency

**Rejected.** Consistency is stored per connection. `ConnectionConsistency` holds the mode and lag for one connection (`crates/storkv-server/src/server/consistency.rs:5-37`). Each transport creates one for the request or session (`crates/storkv-server/src/transports/http.rs:47`; `crates/storkv-server/src/transports/grpc.rs:93`; `crates/storkv-server/src/server/listener.rs:120-129`). `ConfigureConsistency` writes only that object (`crates/storkv-server/src/dispatch.rs:116-119`).

### R-06: The SDK cache served range-deleted and merge-mutated values

**Rejected in part.** The server broadcasts `ChangeEvent::InvalidateScope` for `Merge` and `DeleteRange` (`crates/storkv-core/src/db_ops/commit/apply.rs:167-189`). The SDK clears the whole cache on that event (`crates/storkv-sdk/src/cache/events.rs:32-36`). The TTL part of the old finding remains and is tracked as F-04.

### R-07: The native admin Unix socket used default filesystem permissions

**Rejected.** The parent directory is set to `0700` and the socket to `0600` (`crates/storkv-server/src/bin/storkvd.rs:161-172`). The code removes a stale socket before bind (`crates/storkv-server/src/bin/storkvd.rs:170`).

### R-08: Backup manifest paths could escape the backup root

**Rejected.** `manifest_relative_path` rejects absolute paths, drive prefixes, and `..` segments (`crates/storkv-core/src/backup/manifest/paths.rs:22-41`). `validate_generation_id` rejects separators, drive prefixes, and parent segments (`crates/storkv-core/src/backup/manifest/paths.rs:69-81`). The manifest validation checks each path. Restore rejects a symlink in the backup tree (`crates/storkv-core/src/backup/restore/staging.rs:67-78`).

### R-09: Decompression output had no size cap

**Rejected.** The WAL decoder rejects a declared length above `MAX_VAL_LEN` before it decompresses (`crates/storkv-core/src/wal/codec.rs:135-147`). The SSTable decoder applies the same check (`crates/storkv-core/src/sstable/footer.rs:156-171`). The Zstd path also reads at most `MAX_VAL_LEN + 1` bytes and rejects a length mismatch (`crates/storkv-core/src/sstable/footer.rs:179-199`).

### R-10: Frame and request decoding could overflow memory

**Rejected.** `MAX_FRAME_SIZE` is 64 MiB (`crates/storkv-proto/src/frame.rs:11-12`). The decoder checks the length prefix before it reserves or copies. The request and response decoders use bounded vectors. This audit found no unchecked length arithmetic.

### R-11: The admin token comparison leaked timing

**Rejected.** `tokens_match` compares equal-length byte slices and accumulates differences with XOR (`crates/storkv-server/src/transports.rs:72-83`). HTTP and gRPC use the same helper (`crates/storkv-server/src/transports/http.rs:40`; `crates/storkv-server/src/transports/grpc.rs:85`). Length inequality returns early, which leaks only the length.

### R-12: An HTTP, WebSocket, or gRPC admin route bypassed the admin gate

**Rejected.** The REST admin handlers call `state.dispatch` (`crates/storkv-server/src/transports/http/rest_admin.rs:13-103`). `HttpState::dispatch` calls `dispatch_request` with `is_admin` (`crates/storkv-server/src/transports/http.rs:43-50`). WebSocket requests call `dispatch_request` with the upgrade-time `is_admin` (`crates/storkv-server/src/transports/ws/session.rs:141,183`). gRPC data methods call `self.dispatch` (`crates/storkv-server/src/transports/grpc/service/data.rs:16-169`), which calls `dispatch_request` (`crates/storkv-server/src/transports/grpc.rs:88-96`).

### R-13: Cross-scope access was an authorization bypass

**Rejected.** `SPEC.md:23` documents scopes as logical namespaces with no per-scope authorization. The network API has no per-scope principal. This is a documented design choice, not a defect.

### R-14: Restore staging used a predictable name

**Rejected.** The staging name includes the process ID, a nanosecond timestamp, and a per-process counter (`crates/storkv-core/src/backup/restore/staging.rs:60-64`). The old predictable-name concern no longer applies.

### R-15: Redis and Memcached flush commands bypassed `--strict-admin`

**Rejected.** Redis `FLUSHDB` and `FLUSHALL` return `NOPERM` when `strict_admin` is set (`crates/storkv-server/src/compat/redis/commands_server.rs:98-120`). Memcached `flush_all` returns `CLIENT_ERROR` in the same mode (`crates/storkv-server/src/compat/memcached/commands/modify.rs:88-100`). This fixes the previous H-01.

## 7. Final Verification

This audit read the working tree at commit `cc14dec8221f9b9e21cda9146015a4c5391859ef`. It ran no live network request and no destructive command.

Every confirmed finding cites a current repository path and line range. Every rejected candidate cites the control that closes it. The two validation leads state the missing runtime evidence.

The only file changed by this audit is `docs/internal/SECURITY_AUDIT.md`. The final `git status --short` shows only that file.

```text
$ git status --short
 M docs/internal/SECURITY_AUDIT.md
```
