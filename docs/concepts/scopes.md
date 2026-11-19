# Scopes

A scope is an isolated keyspace inside one StorKV database. Every read and write targets one scope.

Scopes give you these properties:

- Separate memtables and SSTables.
- Separate durability and retention options.
- Atomic writes across several scopes.
- One name for each keyspace across every transport.

Use one consistent word in your design: scope. The Redis adapter maps logical databases to scopes. The Memcached and etcd adapters also use scopes.

## Scope names and IDs

Every scope has a name and a numeric ID. The engine assigns the ID at creation.

- Scope 0 is always `default`.
- `storkvd` creates `default` on first open.
- Names are unique in one database.
- A name is a UTF-8 string.
- The engine stores SSTables under `scopes/<8-hex-id>/`.

The default scope uses strict durability and permanent retention.

## Create a scope

Use the Rust API:

```rust
use std::time::Duration;
use storkv_core::{
    CompactionStyle, Db, DbOptions, Durability, Retention, ScopeOptions, StorageMode,
};

let db = Db::open("./data/storkv", DbOptions::default())?;
let cache = db.create_scope(
    "cache",
    ScopeOptions {
        durability: Durability::Buffered {
            sync_interval: Duration::from_millis(100),
        },
        retention: Retention::Ttl {
            default_ttl: Duration::from_secs(900),
            compaction_style: CompactionStyle::Fifo,
        },
        prefix_bloom_len: Some(8),
        storage_mode: StorageMode::InMemoryPersistent,
    },
)?;
```

The call returns `StorkvError::ScopeAlreadyExists` when the name is taken. It returns `StorkvError::InvalidOperation` when the database already holds 1024 scopes. The constant `storkv_core::MAX_SCOPES` holds this limit.

Use the REST API:

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/scopes \
  -H 'content-type: application/json' \
  -d '{"name":"cache","durability":"buffered","retention":{"ttl":{"default_ttl_ms":900000,"compaction_style":"fifo"}}}'
```

The REST body accepts these durability values:

| Value                                 | Result                            |
| :------------------------------------ | :-------------------------------- |
| Missing or `null`                     | `StrictFsync`                     |
| `"strict_fsync"`                      | `StrictFsync`                     |
| `"buffered"`                          | `Buffered` with a 100 ms interval |
| `{"buffered":{"sync_interval_ms":N}}` | `Buffered` with `N` milliseconds  |

The REST body accepts these retention values:

| Value                                    | Result                        |
| :--------------------------------------- | :---------------------------- |
| Missing or `null`                        | `Permanent`                   |
| `"permanent"`                            | `Permanent`                   |
| `{"ttl":{"compaction_style":"fifo"}}`    | `Ttl` with FIFO compaction    |
| `{"ttl":{"compaction_style":"leveled"}}` | `Ttl` with leveled compaction |

The `default_ttl_ms` value is recorded in the scope options and the manifest. A key expires only when its write sets an explicit TTL or expiry time.

The REST body does not carry a storage mode. A scope created over REST uses `StorageMode::Disk`.

## Scope options

| Option             | Default       | Values                                   | Effect                                 |
| :----------------- | :------------ | :--------------------------------------- | :------------------------------------- |
| `durability`       | `StrictFsync` | `StrictFsync`, `Buffered { interval }`   | Commit sync rule for the scope         |
| `retention`        | `Permanent`   | `Permanent`, `Ttl { ttl, style }`        | Key lifetime rule and compaction style |
| `prefix_bloom_len` | `None`        | Bytes per prefix key, or `None`          | Prefix Bloom filter size in SSTables   |
| `storage_mode`     | `Disk`        | `Disk`, `InMemory`, `InMemoryPersistent` | Where the scope keeps its data         |

A scope cannot change its options after creation in v1. Create a new scope and copy the data when you need different options.

## Storage modes

`StorageMode` sets where one scope keeps its data:

| Mode                 | WAL            | Memtable and SSTables | Survives restart        |
| :------------------- | :------------- | :-------------------- | :---------------------- |
| `Disk`               | On disk        | On disk               | Yes                     |
| `InMemory`           | Not written    | RAM only              | No                      |
| `InMemoryPersistent` | On disk        | RAM only              | Yes, through WAL replay |

Use `InMemory` for a pure cache. Use `InMemoryPersistent` when you want RAM speed for reads and a durable copy in the WAL.

An in-memory scope keeps its memtable and SSTables in RAM. It has no SSTable files under `scopes/<id>/`.

`Db::flush()` writes a recovery snapshot for each `InMemoryPersistent` scope at `<db_root>/snapshots/<scope_id>.snapshot`. The snapshot has magic `STORKSNAP` and version `1`. On open, the engine loads the snapshot and then replays only newer WAL operations.

The last `Db` handle flushes on drop. Call `db.flush()` before a planned shutdown when you need a known snapshot time.

`Db::open_in_memory(DbOptions::default())` opens a database that uses no disk. No lock file, WAL, or manifest is created. Every scope in that database loses its data when the process exits.

## Retention and compaction

`Retention::Permanent` keeps keys until a delete arrives. It uses leveled compaction.

`Retention::Ttl` records a default TTL and selects a compaction style:

- `CompactionStyle::Leveled` merges L0 files into L1.
- `CompactionStyle::Fifo` drops SSTables whose newest expiry time is in the past.

FIFO compaction removes a whole file. It does not rewrite live keys.

## Isolation and the security boundary

Each scope has its own active memtable, flush queue, and SSTable set. A key in one scope never hides a key with the same bytes in another scope.

Scopes are logical namespaces. They are not security tenants. The network API does not check a scope owner. Any client on a listener can read or write any scope ID on that listener. A client can also list scope IDs through watch events and admin statistics.

Put separate tenants on separate servers, listeners, or reverse-proxy rules. Do not use a scope as an authorization boundary.

This example uses the same key in two scopes:

```rust
let accounts = db.create_scope("accounts", ScopeOptions::default())?;
let sessions = db.create_scope("sessions", ScopeOptions::default())?;
db.put(&accounts, b"user:42", b"active")?;
db.put(&sessions, b"user:42", b"expires-soon")?;
assert_eq!(db.get(&accounts, b"user:42")?.as_deref(), Some(&b"active"[..]));
assert_eq!(db.get(&sessions, b"user:42")?.as_deref(), Some(&b"expires-soon"[..]));
```

## Atomic writes across scopes

A transaction can write to several scopes. The commit writes one WAL frame with one contiguous sequence range.

If the transaction commits, every write in every scope becomes visible. If the process stops before the commit, no write becomes visible.

```rust
let mut tx = db.begin_tx();
tx.put(&accounts, b"user:42:balance", b"5000")?;
tx.put(&sessions, b"sess:abc", b"user:42")?;
tx.commit()?;
```

The batch uses the strictest durability of all touched scopes.

## Drop a scope

Drop a scope by name:

```rust
db.drop_scope("sessions")?;
```

Use REST:

```bash
curl -sS -X DELETE http://127.0.0.1:8080/v1/scopes/sessions
```

The drop writes a `DropScope` record to the WAL and the manifest. The scope hides all keys at or below the drop sequence. The engine reclaims the SSTable files during a later compaction.

A transaction that read a key in a dropped scope fails its commit with a write conflict.

## WAL pinning

All scopes share one WAL. A low-write scope can hold an old unflushed memtable. That memtable can stop WAL segment removal while a high-write scope creates new segments.

StorKV limits this effect with two watermarks:

| Limit               | Default | Action when exceeded                               |
| :------------------ | :------ | :------------------------------------------------- |
| `max_wal_pin_bytes` | 64 MiB  | Flush the scope with the oldest unflushed sequence |
| `max_wal_pin_age`   | 60 s    | Flush an old memtable with pending writes          |

Set these limits in `DbOptions`:

```rust
use std::time::Duration;
use storkv_core::DbOptions;

let options = DbOptions {
    max_wal_pin_bytes: 128 * 1024 * 1024,
    max_wal_pin_age: Duration::from_secs(120),
};
```

The flush scheduler uses these values during the write path. A flush turns the active memtable into an L0 SSTable.

## Scope mapping in compatibility modes

| Front end | Scope rule                                                |
| :-------- | :-------------------------------------------------------- |
| Redis     | `SELECT 0` uses `default`; `SELECT n` uses `redis_db_<n>` |
| Memcached | Every item lives in `memcached`                           |
| etcd      | Every key lives in `etcd`                                 |
| Embedded  | `db.open_tree("name")` uses scope `name`                  |

## Related guides

- [MVCC and transactions](mvcc-and-transactions.md)
- [Watches and TTL](watches-and-ttl.md)
- [Redis compatibility](../guides/compatibility-redis.md)
- [Memcached and etcd compatibility](../guides/compatibility-memcached-etcd.md)
