# Use StorKV from Rust

The `storkv-sdk` crate gives a pooled API in the style of `sqlx`. One type, `StorkvPool`, connects to an embedded directory or a remote `storkvd` server. The application code stays the same.

Use `storkv-sdk` for new Rust applications. Use `storkv-client` only when you need direct control of one transport connection. Read [Rust client](../guides/rust-client.md) for that lower-level API.

## Add the dependency

Add the crate to `Cargo.toml`:

```toml
[dependencies]
storkv-sdk = { path = "crates/storkv-sdk" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
```

`tokio` supplies the async runtime. `serde` is necessary only for typed collections.

The `storkv-core` engine keeps no mandatory dependencies. The SDK adds Tokio for async calls and file I/O bridging.

## Feature flags

| Feature    | Default | Effect                                                       |
| :--------- | :------ | :----------------------------------------------------------- |
| `json`     | Yes     | Typed collections encode values as JSON through `serde_json` |
| `postcard` | No      | Typed collections can encode values as Postcard              |
| `serde`    | No      | Enable the typed collection API without a default codec      |

`json` enables `serde`. A build with `postcard` and without `json` uses Postcard as the collection codec.

```toml
storkv-sdk = { path = "crates/storkv-sdk", default-features = false, features = ["postcard"] }
```

## Connect a pool

Open an embedded database:

```rust
use storkv_sdk::StorkvPool;

# async fn example() -> storkv_sdk::Result<()> {
let pool = StorkvPool::connect("file://./data/app").await?;
# pool.close().await;
# Ok(())
# }
```

Open a remote server with the same methods:

```rust
use storkv_sdk::StorkvPool;

# async fn example() -> storkv_sdk::Result<()> {
let pool = StorkvPool::connect("storkv://127.0.0.1:7878").await?;
# pool.close().await;
# Ok(())
# }
```

Use `StorkvPoolOptions` for explicit control:

```rust
use std::time::Duration;
use storkv_sdk::{StorkvPool, StorkvPoolOptions};

# async fn example() -> storkv_sdk::Result<()> {
let pool = StorkvPoolOptions::new()
    .max_connections(16)
    .min_connections(2)
    .acquire_timeout(Duration::from_secs(3))
    .idle_timeout(Some(Duration::from_secs(120)))
    .health_check(true)
    .connect("storkv://127.0.0.1:7878")
    .await?;
# pool.close().await;
# Ok(())
# }
```

## Pool options

| Method            | Default                   | Effect                                         |
| :---------------- | :------------------------ | :--------------------------------------------- |
| `max_connections` | 10                        | Maximum simultaneous connections               |
| `min_connections` | 1                         | Connections opened during `connect`            |
| `acquire_timeout` | 5 seconds                 | Maximum wait for a free connection             |
| `idle_timeout`    | 300 seconds               | Age limit for an idle networked connection     |
| `health_check`    | `true`                    | Probe a client before reuse                    |
| `retry_policy`    | `RetryPolicy::default`    | Default retry policy for transactions          |
| `cache`           | `CacheOptions::default()` | Optional client-side read cache                |
| `consistency`     | `Consistency::Strong`     | Consistency level that networked pools request |

An embedded pool shares one database handle. The connection limit still controls concurrent callers.

A networked pool sends a request that waits up to two seconds for a health probe. A failed probe discards the client. The pool then opens a fresh client.

## Client-side cache

The SDK cache is off by default. Turn it on with `CacheOptions`:

```rust
use storkv_sdk::{CacheOptions, StorkvPoolOptions};

# async fn example() -> storkv_sdk::Result<()> {
let pool = StorkvPoolOptions::default()
    .cache(
        CacheOptions::default()
            .enable()
            .max_entries_per_scope(4096),
    )
    .connect("storkv://127.0.0.1:7878")
    .await?;
# pool.close().await;
# Ok(())
# }
```

| Field                   | Default | Meaning                            |
| :---------------------- | :------ | :--------------------------------- |
| `enabled`               | `false` | Turn the cache on                  |
| `max_entries_per_scope` | `1024`  | Maximum cached keys for each scope |

The cache stores point reads. A `put` or `delete` through the same pool updates the cache. A change from another client arrives through a watch subscription and invalidates the matching key. A scope drop clears the whole scope cache.

A cached value can have an expiry time. `put_with_ttl` stores the value with its expiry. `get` treats an expired entry as a cache miss and evicts it. A watch `put` keeps the expiry of the existing entry.

A range delete or a merge sends a scope-wide invalidation event. The cache clears every key in that scope. This rule prevents stale reads after `delete_range` or `merge`.

The cache uses a watch cursor. When a networked watch disconnects, the pool clears the cache and resubscribes from the next sequence. When the server reports `WatchCursorExpired`, the pool also clears the cache. The embedded watcher stops when the pool releases the scope cache.

A scan, a transaction, or `get_and_touch` does not populate the cache. They read through to the backend.

## Consistency on a replica

A pool sends its `consistency` value on each new networked connection. Set it in the options or later with `set_consistency`:

```rust
use storkv_sdk::{Consistency, StorkvPoolOptions};

# async fn example() -> storkv_sdk::Result<()> {
let pool = StorkvPoolOptions::default()
    .consistency(Consistency::BoundedStaleness { max_lag_ms: 500 })
    .connect("storkv://127.0.0.1:7979")
    .await?;
pool.set_consistency(Consistency::Eventual).await?;
# pool.close().await;
# Ok(())
# }
```

| Level                             | Effect                                                                                   |
| :-------------------------------- | :--------------------------------------------------------------------------------------- |
| `Strong`                          | Send the request to a primary. A replica rejects data reads and writes with `NotPrimary` |
| `BoundedStaleness { max_lag_ms }` | A replica serves reads while its lag is at or below the bound; otherwise `StaleReplica`  |
| `Eventual`                        | A replica serves reads at any lag                                                        |

The default is `Strong`. A replica node switches its own default to `Eventual` when it starts with `--role replica`. The embedded backend ignores the setting.

## Provision scopes

`ScopeDefinition` describes a scope at startup. The pool creates the scope when it is absent. The pool verifies the scope when it exists.

```rust
use std::time::Duration;
use storkv_sdk::{CompactionStyle, ScopeDefinition};

let scopes = [
    ScopeDefinition::new("users"),
    ScopeDefinition::new("sessions").with_ttl(
        Duration::from_secs(3600),
        CompactionStyle::Leveled,
    ),
];
```

A `ScopeDefinition` has these parts:

| Field              | Default                   | Meaning                           |
| :----------------- | :------------------------ | :-------------------------------- |
| `name`             | Required                  | Unique scope name                 |
| `durability`       | `Durability::StrictFsync` | Commit durability policy          |
| `retention`        | `Retention::Permanent`    | Key retention policy              |
| `prefix_bloom_len` | `None`                    | Prefix Bloom filter size in bytes |

Call `provision_scopes` during startup:

```rust
# use std::time::Duration;
# use storkv_sdk::{CompactionStyle, ScopeDefinition, StorkvPool};
# async fn example(pool: &StorkvPool) -> storkv_sdk::Result<()> {
pool.provision_scopes(&[
    ScopeDefinition::new("users"),
    ScopeDefinition::new("sessions").with_ttl(
        Duration::from_secs(3600),
        CompactionStyle::Leveled,
    ),
])
.await?;
# Ok(())
# }
```

The `default` scope exists in a new database. A scope handle resolves its identifier on first use and caches the result.

## Raw scope operations

Get a scope handle from the pool:

```rust
# use storkv_sdk::StorkvPool;
# async fn example(pool: &StorkvPool) -> storkv_sdk::Result<()> {
let users = pool.scope("users");
users.put("alice", b"active").await?;
let value = users.get("alice").await?;
assert_eq!(value.as_deref(), Some(&b"active"[..]));
users.delete("alice").await?;
# Ok(())
# }
```

A raw scope has these methods:

| Method          | Result                | Notes                                      |
| :-------------- | :-------------------- | :----------------------------------------- |
| `get`           | `Option<Bytes>`       | `None` means the key is absent             |
| `exists`        | `bool`                | A read that returns a flag                 |
| `put`           | `()`                  | Store one key/value pair                   |
| `put_with_ttl`  | `()`                  | Store a pair and set an expiry             |
| `put_if`        | `PutIfResult`         | Store a pair only when a condition matches |
| `get_and_touch` | `Option<Bytes>`       | Read a value and restart its expiry        |
| `delete`        | `()`                  | Remove one key                             |
| `merge`         | `()`                  | Apply a `MergeOp` to one key               |
| `scan_prefix`   | `Vec<(Bytes, Bytes)>` | Bytewise order, one prefix                 |
| `scan_range`    | `Vec<(Bytes, Bytes)>` | Half-open range, optional bounds           |
| `watch_prefix`  | `WatchStream`         | Changes for one key prefix                 |

`get_and_touch` reads and rewrites the key in one transaction. The returned value and the new expiry always agree. The default policy retries a write conflict.

`put_if` takes a `PutCondition` and an optional TTL. The conditions are `Absent`, `Present`, `ExpectedSeq`, and `ExpectedValue`. The result is `PutIfResult::Applied { seq }` or `PutIfResult::Conflict { .. }`. A conflict is a normal result, not an error.

```rust
use storkv_core::PutCondition;
# use storkv_sdk::StorkvPool;
# async fn example(pool: &StorkvPool) -> storkv_sdk::Result<()> {
let locks = pool.scope("locks");
let result = locks
    .put_if("job:42", &PutCondition::Absent, b"worker-1", None)
    .await?;
# Ok(())
# }
```

`merge` applies a `MergeOp`. The helper methods `counter_add`, `append_bytes`, `max_i64`, `min_i64`, `bit_or`, and `bit_and` build the common operators. The supported operators are `CounterAdd`, `AppendBytes`, `MaxI64`, `MinI64`, `BitOr`, `BitAnd`, and `Custom`. Read [Merge operators](../concepts/mvcc-and-transactions.md#merge-operators).

A TTL write uses `Duration`:

```rust
use std::time::Duration;
# use storkv_sdk::StorkvPool;
# async fn example(pool: &StorkvPool) -> storkv_sdk::Result<()> {
let sessions = pool.scope("sessions");
sessions
    .put_with_ttl("sess:abc", b"user:42", Duration::from_secs(900))
    .await?;
# Ok(())
# }
```

A scan returns each key in bytewise order:

```rust
# use storkv_sdk::StorkvPool;
# async fn example(pool: &StorkvPool) -> storkv_sdk::Result<()> {
let users = pool.scope("users");
let page = users.scan_prefix("user:").await?;
for (key, value) in &page {
    println!("{} = {} bytes", String::from_utf8_lossy(key), value.len());
}
let window = users
    .scan_range(Some(&b"user:100"[..]), Some(&b"user:200"[..]))
    .await?;
# Ok(())
# }
```

The range includes the start key and excludes the end key. A `None` bound is open. These convenience methods return every match. The wire protocol also carries `limit`, `reverse`, and `keys_only` flags for a bounded scan. Use `storkv-core` `ScanOptions` for embedded scans with these flags.

## Typed collections

A collection maps one prefix to a Rust type. The default `json` feature encodes each value as JSON.

```rust
use serde::{Deserialize, Serialize};
use storkv_sdk::{ScopeDefinition, StorkvPool};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct User {
    name: String,
    age: u32,
}

# async fn example(pool: &StorkvPool) -> storkv_sdk::Result<()> {
pool.provision_scopes(&[ScopeDefinition::new("app")]).await?;
let users = pool.scope("app").collection::<User>("user:");

users.put("1", &User { name: "alice".into(), age: 30 }).await?;
let alice = users.get("1").await?;
assert_eq!(alice.map(|user| user.age), Some(30));

let all = users.scan().await?;
let window = users.scan_range(Some("1"), Some("9")).await?;
users.delete("1").await?;
# Ok(())
# }
```

The collection methods are:

| Method          | Result                |
| :-------------- | :-------------------- |
| `get`           | `Option<T>`           |
| `exists`        | `bool`                |
| `put`           | `()`                  |
| `put_with_ttl`  | `()`                  |
| `get_and_touch` | `Option<T>`           |
| `delete`        | `()`                  |
| `scan_prefix`   | `Vec<(Bytes, T)>`     |
| `scan`          | `Vec<(Bytes, T)>`     |
| `scan_range`    | `Vec<(Bytes, T)>`     |
| `watch_prefix`  | `TypedWatchStream<T>` |

The returned key is relative to the collection prefix. A scan removes the prefix before it returns the key.

## Transactions

Use `pool.transaction` for a closure-based OCC transaction. The pool retries write conflicts with exponential backoff and optional jitter.

```rust
use storkv_sdk::StorkvPool;

# async fn example(pool: &StorkvPool) -> storkv_sdk::Result<()> {
let counters = pool.scope("counters");
counters.put("hits", b"0").await?;

pool.transaction(|transaction| {
    let counters = counters.clone();
    Box::pin(async move {
        let current = transaction.get(&counters, "hits").await?;
        let next = current
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .and_then(|text| text.parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        transaction.put(&counters, "hits", next.to_string()).await?;
        Ok(())
    })
})
.await?;
# Ok(())
# }
```

The closure must return a boxed future. A borrowed `&mut Transaction` cannot cross an `async` block boundary. The `Box::pin` call gives the future a stable address.

`RetryPolicy` controls the loop:

| Field         | Default          | Meaning                          |
| :------------ | :--------------- | :------------------------------- |
| `jitter`      | `true`           | Add random offset to each delay  |
| `max_retries` | 5                | Attempts after the first failure |
| `base_delay`  | 2 milliseconds   | Delay before the first retry     |
| `max_delay`   | 250 milliseconds | Upper bound for one delay        |

Use `RetryPolicy::none()` to disable retries. Use `RetryPolicy::with_max_retries`, `with_base_delay`, `with_max_delay`, and `with_jitter` to change one field.

```rust
use std::time::Duration;
use storkv_sdk::RetryPolicy;

let policy = RetryPolicy::default()
    .with_max_retries(16)
    .with_base_delay(Duration::from_millis(1))
    .with_max_delay(Duration::from_millis(50))
    .with_jitter(true);
```

Pass the policy to `transaction_with`:

```rust
# use storkv_sdk::{RetryPolicy, StorkvPool};
# async fn example(pool: &StorkvPool) -> storkv_sdk::Result<()> {
# let counters = pool.scope("counters");
# let policy = RetryPolicy::none();
pool.transaction_with(policy, |transaction| {
    let counters = counters.clone();
    Box::pin(async move {
        transaction.put(&counters, "key", b"value").await?;
        Ok(())
    })
})
.await?;
# Ok(())
# }
```

The retry loop retries only errors for which `is_retryable` returns true. A write conflict is retryable. `SdkError::Unsupported` and other domain errors are not retryable. The loop returns the first non-retryable error.

An explicit transaction uses a checked-out connection:

```rust
# use storkv_sdk::StorkvPool;
# async fn example(pool: &StorkvPool) -> storkv_sdk::Result<()> {
let scope = pool.scope("app");
let connection = pool.acquire().await?;
let mut transaction = connection.begin().await?;
transaction.put(&scope, "key", b"value").await?;
transaction.commit().await?;
# Ok(())
# }
```

`Transaction` buffers writes until `commit`. Dropping a transaction rolls it back. The explicit form does not retry. You must run the loop yourself.

## Watch streams

`watch_prefix` returns a stream of change events. The stream filters events by scope and key prefix.

```rust
use std::time::Duration;
use storkv_sdk::{StorkvPool, WatchEventKind};

# async fn example(pool: &StorkvPool) -> storkv_sdk::Result<()> {
let scope = pool.scope("events");
let mut stream = scope.watch_prefix("hot:").await?;

scope.put("hot:key", b"value").await?;
let event = tokio::time::timeout(Duration::from_secs(5), stream.next())
    .await
    .expect("watch timed out")
    .expect("stream ended")
    .expect("watch failed");

assert_eq!(event.kind, WatchEventKind::Put);
assert_eq!(event.key, bytes::Bytes::from_static(b"hot:key"));
# Ok(())
# }
```

A `WatchEvent` has these fields:

| Field      | Type             | Meaning                         |
| :--------- | :--------------- | :------------------------------ |
| `kind`     | `WatchEventKind` | `Put`, `Delete`, or `DropScope` |
| `scope_id` | `u32`            | Scope that changed              |
| `seq`      | `u64`            | Commit sequence number          |
| `key`      | `Bytes`          | Key that changed                |
| `value`    | `Option<Bytes>`  | New value for a put only        |

`WatchEventKind::DropScope` appears only when the watched prefix is empty. The event carries no key.

A networked stream reconnects with exponential backoff. It observes changes that are written after the subscription is ready. `watch_prefix` waits for that readiness before it returns. A write after the call therefore produces an event.

## Close the pool

`close` stops new acquires and releases idle clients:

```rust
# use storkv_sdk::StorkvPool;
# async fn example(pool: StorkvPool) {
pool.close().await;
# }
```

A later `acquire` returns `SdkError::PoolClosed`. The embedded database handle closes when the last clone of the pool is dropped.

## Testing with storkv-sdk

The workspace test target is `sdk`. Its helpers build an isolated `file://` directory and a live `storkv://` server for each test.

These tests are complete and runnable:

```rust
use std::time::Duration;
use storkv_sdk::{ScopeDefinition, StorkvPool, StorkvPoolOptions};

async fn embedded_pool(name: &str) -> (StorkvPool, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "storkv-rust-test-{name}-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&path).ok();
    std::fs::create_dir_all(&path).expect("create directory");
    let uri = format!("file://{}", path.display());
    let pool = StorkvPoolOptions::new()
        .connect(&uri)
        .await
        .expect("open pool");
    (pool, path)
}

#[tokio::test]
async fn crud_and_ttl() {
    let (pool, path) = embedded_pool("crud").await;
    pool.provision_scopes(&[ScopeDefinition::new("app")])
        .await
        .expect("provision");
    let scope = pool.scope("app");

    scope.put("greeting", b"hello").await.expect("put");
    assert_eq!(
        scope.get("greeting").await.expect("get").as_deref(),
        Some(&b"hello"[..])
    );

    scope
        .put_with_ttl("session", b"live", Duration::from_millis(100))
        .await
        .expect("put with ttl");
    assert_eq!(
        scope.get_and_touch("session", Duration::from_secs(5))
            .await
            .expect("touch")
            .as_deref(),
        Some(&b"live"[..])
    );

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(scope.get("session").await.expect("get").is_none());

    pool.close().await;
    std::fs::remove_dir_all(path).ok();
}

#[tokio::test]
async fn occ_retry_increments_counter() {
    let (pool, path) = embedded_pool("occ").await;
    pool.provision_scopes(&[ScopeDefinition::new("counters")])
        .await
        .expect("provision");
    let counters = pool.scope("counters");
    counters.put("hits", b"0").await.expect("seed");

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        let counters = counters.clone();
        tasks.push(tokio::spawn(async move {
            pool.transaction(move |transaction| {
                let counters = counters.clone();
                Box::pin(async move {
                    let current = transaction.get(&counters, "hits").await?;
                    let value = current
                        .as_deref()
                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                        .and_then(|text| text.parse::<u64>().ok())
                        .unwrap_or(0);
                    transaction
                        .put(&counters, "hits", (value + 1).to_string())
                        .await?;
                    Ok(())
                })
            })
            .await
            .expect("transaction");
        }));
    }
    for task in tasks {
        task.await.expect("task");
    }

    assert_eq!(
        counters.get("hits").await.expect("get").as_deref(),
        Some(&b"8"[..])
    );

    pool.close().await;
    std::fs::remove_dir_all(path).ok();
}

#[tokio::test]
async fn watch_delivers_put() {
    let (pool, path) = embedded_pool("watch").await;
    pool.provision_scopes(&[ScopeDefinition::new("events")])
        .await
        .expect("provision");
    let scope = pool.scope("events");
    let mut stream = scope.watch_prefix("hot:").await.expect("watch");

    scope.put("cold:key", b"ignored").await.expect("put");
    scope.put("hot:key", b"value").await.expect("put");

    let event = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("timed out")
        .expect("stream ended")
        .expect("watch failed");
    assert_eq!(event.key.as_ref(), b"hot:key");
    assert_eq!(event.value.as_deref(), Some(&b"value"[..]));

    pool.close().await;
    std::fs::remove_dir_all(path).ok();
}
```

Run the workspace tests:

```bash
cargo test --test sdk
```

Run one test by name:

```bash
cargo test --test sdk occ_contention_is_retried
```

Run the rustdoc examples for the SDK:

```bash
cargo test --doc -p storkv-sdk
```

The `tests/sdk/support.rs` helper also starts a server on an ephemeral port. That helper connects with a `storkv://` URI. The same test body then runs against both modes.

## Related guides

- [C ABI reference](../reference/ffi-c-abi.md)
- [Rust client](../guides/rust-client.md)
- [Embedded or server](../getting-started/embedded-vs-server.md)
- [Scopes](../concepts/scopes.md)
- [MVCC and transactions](../concepts/mvcc-and-transactions.md)
- [Watches and TTL](../concepts/watches-and-ttl.md)
