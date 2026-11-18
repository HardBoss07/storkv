# Transports and Compatibility

StorKV exposes one storage engine through several access paths. Every path
resolves to the same scopes, MVCC transactions, WAL, telemetry, and backup
policy. There is no replication or format conversion between them.

## Choosing a transport

| Transport          | URI / flag                                  | Best for                                             |
| :----------------- | :------------------------------------------ | :--------------------------------------------------- |
| Native binary      | `storkv://host:port`, `--listen`            | StorKV clients, lowest overhead, full opcode surface |
| Native Unix socket | `unix:///path`, `--admin-sock`              | Local processes and admin-only access                |
| HTTP REST          | `http://host:port`, `--http-listen`         | Scripts, curl, language-agnostic CRUD and admin      |
| WebSocket          | `ws://host:port`, `--ws-listen`             | Browser tools and multiplexed Watch streaming        |
| gRPC               | `grpc://host:port`, `--grpc-listen`         | Generated clients and typed service contracts        |
| Redis RESP         | `--redis-listen`                            | Existing Redis client libraries and applications     |
| Memcached ASCII    | `--memcached-listen`                        | Existing Memcached client libraries                  |
| etcd v3 JSON       | mounted on `--http-listen` under `/v3/kv/*` | etcd HTTP/JSON tooling                               |
| Embedded Rust      | `db.open_tree("name")`                      | In-process use without a daemon                      |

All listeners are optional. Starting `storkvd` without extra flags serves only
the native binary protocol.

## Native binary protocol

The native protocol is `STORKPRT` v1 over TCP or a Unix-domain socket. It
remains byte-for-byte compatible with earlier releases. Use
`--strict-admin` to require admin privileges for admin opcodes; admin
listeners created with `--admin-sock` are always trusted.

## HTTP REST

The HTTP transport accepts both HTTP/1.1 and HTTP/2 and serves:

- `GET /v1/health`, `GET /v1/ping`, `GET /v1/telemetry`
- `GET /v1/admin/stats`
- `POST /v1/scopes`, `DELETE /v1/scopes/{scope}`
- `GET|PUT|DELETE /v1/scopes/{scope}/keys/{key}`
- `POST /v1/scopes/{scope}/merge/{key}`
- `POST /v1/scopes/{scope}/delete-range`
- `GET /v1/scopes/{scope}/scan`
- `POST /v1/tx/begin`, `POST /v1/tx/commit`
- `POST /v1/admin/checkpoint`, `POST /v1/admin/compact`
- `POST /v1/admin/backup`, `GET /v1/admin/backups`
- `POST /v1/admin/snapshots`, `DELETE /v1/admin/snapshots/{name}`
- `POST /v1/admin/verify-backup`
- `POST /v1/rpc` for the framed binary tunnel used by the unified client

Key writes accept either raw bytes (`application/octet-stream`) or a JSON body
such as `{"value_b64": "...", "ttl_ms": 60000}`. Query parameters `ttl_ms`,
`start`, `end`, and `prefix` are supported where relevant.

`POST /v1/rpc` carries exactly the same `STORKPRT` frames as the native
transport. This is how `AsyncClient::connect_uri("http://...")` preserves the
full request/response surface without duplicating REST semantics.

## WebSocket

The WebSocket transport lives at `/v1/ws` (the dedicated `--ws-listen` listener
serves the same path). Binary frames carry `STORKPRT` requests and responses.
`WatchSubscribe` replies are pushed as `Response::WatchEvent` frames, which the
client exposes through `WatchStream`. JSON RPC frames are also accepted for
tooling that prefers text.

The unified client appends `/v1/ws` automatically when a `ws://` URI contains
only an authority, so both `ws://host:port` and `ws://host:port/v1/ws` work.

## gRPC

The generated `storkv.v1.StorKv` service provides Ping, Health, Get, Put,
Delete, DeleteRange, Merge, Scan, BeginTx, Commit, CreateScope, DropScope,
Watch, and Telemetry. `storkv.v1.StorKvAdmin` provides AdminStats, Checkpoint,
Compact, CreateSnapshot, UnpinSnapshot, TriggerBackup, BackupStatus, and
VerifyBackup.

Descriptors are generated at build time with a vendored `protoc`, so no system
installation is required. The gRPC client maps StorKV requests onto generated
messages; operations without a gRPC equivalent (for example `ListBackups`) are
reported as `Unsupported` on the gRPC send path.

## Unified client URIs

`AsyncClient::connect_uri` and `SyncClient::connect_uri` accept:

```text
storkv://127.0.0.1:7878
unix:///run/storkv/admin.sock
http://127.0.0.1:8080
ws://127.0.0.1:8081
grpc://127.0.0.1:9090
```

Bare `host:port` and filesystem-looking paths keep their historical meaning:
native TCP and native Unix sockets.

## Redis compatibility

Start the listener with `--redis-listen 127.0.0.1:6379`. Logical databases map
to scopes:

| Redis SELECT | StorKV scope   |
| :----------- | :------------- |
| `0`          | `default`      |
| `n > 0`      | `redis_db_<n>` |

Supported commands: `PING`, `ECHO`, `QUIT`, `HELLO`, `SELECT`, `INFO`,
`DBSIZE`, `FLUSHDB`, `FLUSHALL`, `GET`, `SET` (`EX`, `PX`, `NX`, `XX`, `GET`,
`KEEPTTL`), `SETEX`, `PSETEX`, `SETNX`, `MGET`, `MSET`, `DEL`, `UNLINK`,
`EXISTS`, `INCR`, `DECR`, `INCRBY`, `DECRBY`, `APPEND`, `STRLEN`, `EXPIRE`,
`PEXPIRE`, `PERSIST`, `TTL`, `PTTL`, `KEYS`, `SCAN` (`MATCH`, `COUNT`),
`MULTI`, `EXEC`, and `DISCARD`.

RESP2 and RESP3 are both supported. `HELLO 3` switches null, map, set, and
boolean replies to RESP3 encodings. `MULTI`/`EXEC` executes queued commands in
order inside an MVCC transaction context; runtime errors are returned per
command, matching Redis semantics.

```
redis-cli -p 6379 SET greeting hello
redis-cli -p 6379 GET greeting
redis-cli -p 6379 -n 2 SET tenant-key value
```

## Memcached compatibility

Start the listener with `--memcached-listen 127.0.0.1:11211`. Data lives in the
`memcached` scope with FIFO compaction. Supported commands: `get`, `gets`,
`set`, `add`, `replace`, `append`, `prepend`, `cas`, `delete`, `incr`, `decr`,
`touch`, `flush_all`, `stats`, `version`, and `quit`. `noreply` is honored where
the Memcached protocol defines it. Item flags are preserved and returned on
reads.

```
printf 'set greeting 0 0 5\r\nhello\r\n' | nc 127.0.0.1 11211
printf 'get greeting\r\n' | nc 127.0.0.1 11211
```

## etcd v3 HTTP gateway

When the HTTP listener is enabled, the etcd JSON gateway is mounted at:

- `POST /v3/kv/put`
- `POST /v3/kv/range`
- `POST /v3/kv/deleterange`

Keys and values are base64 encoded. Responses include an etcd-style `header`
whose `revision` is the engine's last visible sequence number. `range_end` with
a single zero byte means "all keys from `key`", following the etcd convention.
`count_only` and `keys_only` are supported. Leases are rejected explicitly.

All etcd gateway keys live in the `etcd` scope.

## Embedded Rust adapter

`Db::open_tree` creates or opens an ordinary scope and returns a `Tree` with a
`sled`-style API:

```rust
use storkv_core::compat::KvEngine;
use storkv_core::{Db, DbOptions};

let db = Db::open("./data", DbOptions::default())?;
let users = db.open_tree("users")?;
users.insert("alice", "admin")?;
users.compare_and_swap("alice", Some(b"admin"), b"owner")?;
for (key, value) in users.scan_prefix("a")? {
    println!("{key:?} = {value:?}");
}
```

`Tree` also exposes `get`, `remove`, `range`, `merge`, `batch`, `apply_batch`,
and `flush`. Data written through a tree is visible to every transport.

## Admin policy

Auxiliary transports are non-admin by default. Grant admin in one of three
ways:

1. Use a dedicated listener flag such as `--http-admin`, `--ws-admin`, or
   `--grpc-admin`.
2. Present `--admin-token` as `x-storkv-admin-token`, a bearer token, or a gRPC
   metadata value.
3. Connect through `--admin-sock`, which is always trusted.

With `--strict-admin`, admin opcodes on a non-admin connection fail with a
permission error instead of silently succeeding.
