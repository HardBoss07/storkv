# Embedded or server

StorKV has two deployment modes. An application can embed `storkv-core` as a Rust library. An operator can run `storkvd` as a network server.

Both modes use the same storage engine, the same on-disk format, and the same scope rules.

## Compare the modes

| Topic            | Embedded `storkv-core`                       | Server `storkvd`                                |
| :--------------- | :------------------------------------------- | :---------------------------------------------- |
| Access           | Direct Rust method calls                     | Native, HTTP, WebSocket, gRPC, Redis, Memcached |
| Process model    | One process owns the data directory          | One `storkvd` process owns the data directory   |
| Latency          | No network hop                               | One network hop per request                     |
| Language support | Rust, plus C, C++, Java, TypeScript, and Python through `storkv-ffi` | Any client that speaks a supported protocol |
| Admin surface    | Library types and methods                    | CLI, REST admin routes, gRPC admin service      |
| Backup           | `Db::start_auto_backup` and `Db::checkpoint` | Server flags and `storkvctl` commands           |
| Upgrade path     | Rebuild the host application                 | Restart `storkvd`                               |

## Use the embedded engine when

Choose `storkv-core` for these cases:

- Your application is written in Rust.
- You want the lowest possible read and write latency.
- One process can own the data directory.
- You do not need multi-language clients.
- You can ship a new binary for engine upgrades.

The engine uses blocking file I/O and standard synchronisation primitives. It does not require a Tokio runtime.

## Embed StorKV

Add `storkv-core` to `Cargo.toml`:

```toml
[dependencies]
storkv-core = { path = "crates/storkv-core" }
```

Open the database and create a scope:

```rust
use std::time::Duration;
use storkv_core::{Db, DbOptions, Durability, Retention, ScopeOptions};

fn main() -> storkv_core::Result<()> {
    let db = Db::open("./data/storkv", DbOptions::default())?;
    let sessions = db.create_scope(
        "sessions",
        ScopeOptions {
            durability: Durability::StrictFsync,
            retention: Retention::Permanent,
            prefix_bloom_len: Some(8),
        },
    )?;

    db.put(&sessions, b"sess:abc", b"user:42")?;
    let owner = db.get(&sessions, b"sess:abc")?;
    assert_eq!(owner.as_deref(), Some(&b"user:42"[..]));
    Ok(())
}
```

`Db::open` takes an exclusive lock on the data directory. A second open call returns `StorkvError::DatabaseLocked`.

## Use the server when

Choose `storkvd` for these cases:

- Clients use different languages or protocols.
- You need Redis, Memcached, or etcd compatibility.
- You want the `storkv-top` dashboard.
- You want remote backup and restore control.
- You want one owner process for a shared data directory.

## Share data between modes

Do not open one data directory from two processes. The lock prevents this action. Use one of these patterns instead:

1. Run `storkvd` and connect from every client.
2. Embed the engine in one process and expose your own API.
3. Copy a checkpoint and open the copy in a second process for read-only analysis.

## Move from embedded to server

Follow these steps:

1. Stop the embedded application.
2. Copy the data directory or create a checkpoint.
3. Start `storkvd` with the copied directory.
4. Change clients to use a StorKV URI.
5. Run `storkvctl doctor` on the directory after the cutover.

The on-disk format does not change between modes.

## One API for both modes

The `storkv-sdk` crate gives one pooled API for both modes. Open a `StorkvPool` with a `file://` or `embedded://` URI for in-process storage. Open the same type with a `storkv://` URI for a remote server. The application code does not change.

```rust
use storkv_sdk::StorkvPool;

# async fn example() -> storkv_sdk::Result<()> {
let embedded = StorkvPool::connect("file://./data/storkv").await?;
let remote = StorkvPool::connect("storkv://127.0.0.1:7878").await?;
# embedded.close().await;
# remote.close().await;
# Ok(())
# }
```

The pool controls connections, acquire timeouts, idle timeouts, and health checks in both modes. A scope handle, a transaction, and a watch use the same methods.

Read the [SDK overview](../storkv-sdk/index.md) for the architecture. Read the [Rust SDK guide](../storkv-sdk/rust.md) for the full API. The [C ABI reference](../reference/ffi-c-abi.md) covers the C, C++, Java, TypeScript, and Python paths.

## Related guides

- [Quickstart](quickstart.md)
- [Rust client](../guides/rust-client.md)
- [Configuration](../operations/configuration.md)
- [Backup and restore](../operations/backup-and-restore.md)
