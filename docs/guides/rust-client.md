# Rust client

The `storkv-client` crate provides a non-blocking client (`AsyncClient`) and a blocking wrapper (`SyncClient`). Both clients use the same transports and the same request types.

## Choose the SDK or the client

Use the high-level `storkv-sdk` crate for new applications. The SDK gives you `StorkvPool`, which handles connection pooling, embedded and networked URIs, scope provisioning, transactions with retries, and watch streams.

Use `storkv-client` only when you need direct control of one transport connection. The client gives you one connection, wire-level request types, and admin calls. It does not pool connections or retry write conflicts.

Read the [Rust SDK guide](../storkv-sdk/rust.md) for the pooled API. Continue on this page for the low-level client.

## Add the dependency

Add the client crate to `Cargo.toml`:

```toml
[dependencies]
storkv-client = { path = "crates/storkv-client" }
storkv-proto = { path = "crates/storkv-proto" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

`storkv-proto` supplies the request and option types. You need Tokio for `AsyncClient`.

## Connect

Pick the connect method that matches your address:

| Method        | Accepts                                    | Use for                      |
| :------------ | :----------------------------------------- | :--------------------------- |
| `connect`     | `host:port`                                | Native TCP only              |
| `connect_any` | Bare TCP, Unix paths, and every URI scheme | Mixed local deployments      |
| `connect_uri` | Every URI scheme                           | Explicit transport selection |

```rust
use storkv_client::AsyncClient;

# async fn example() -> storkv_client::ClientResult<()> {
let client = AsyncClient::connect_uri("storkv://127.0.0.1:7878").await?;
client.ping().await?;
client.health_check().await?;
# Ok(())
# }
```

The client starts no background tasks. Each `AsyncClient` keeps one backend connection behind a mutex. A clone shares that backend, so concurrent calls on one client run one at a time. Use one client for each worker when you need parallel connections.

Read [Connection URIs](../getting-started/connection-uris.md) for every accepted form.

## Resolve a scope

Every data call takes a `ScopeHandle`. Create a scope and keep its handle:

```rust
use storkv_proto::{WireDurability, WireRetention};

# async fn example(client: &storkv_client::AsyncClient) -> storkv_client::ClientResult<()> {
let scope = client
    .create_scope(
        "sessions",
        WireDurability::StrictFsync,
        WireRetention::Permanent,
        Some(8),
    )
    .await?;
# Ok(())
# }
```

The `default` scope has ID 0. You can build its handle directly:

```rust
use storkv_client::ScopeHandle;

let default = ScopeHandle {
    scope_id: 0,
    name: "default".into(),
};
```

`create_scope` returns `ClientError::Server` with `ScopeAlreadyExists` when the name is taken. There is no lookup-by-name call in the client. Read scope names and IDs from `admin_stats`, or keep your own map.

## Write and read keys

```rust
# async fn example(client: &storkv_client::AsyncClient, scope: &storkv_client::ScopeHandle) -> storkv_client::ClientResult<()> {
client.put(scope, b"user:42", b"active").await?;

let value = client.get(scope, b"user:42").await?;
assert_eq!(value.as_deref(), Some(&b"active"[..]));

client.delete(scope, b"user:42").await?;
assert!(client.get(scope, b"user:42").await?.is_none());
# Ok(())
# }
```

`get` returns `Ok(None)` for a missing key. It does not return an error.

## Write with a TTL

```rust
use std::time::Duration;

# async fn example(client: &storkv_client::AsyncClient, scope: &storkv_client::ScopeHandle) -> storkv_client::ClientResult<()> {
client
    .put_with_ttl(scope, b"sess:abc", b"user:42", Duration::from_secs(900))
    .await?;

let meta = client.key_meta(scope, b"sess:abc").await?;
assert!(meta.is_some());
# Ok(())
# }
```

`key_meta` returns `expires_at_ms` and `seq` for the visible version. An `expires_at_ms` value of 0 means the key never expires.

## Conditional write

`put_if` sends a condition with the write. The conditions are `WirePutCondition::Absent`, `Present`, `ExpectedSeq(u64)`, and `ExpectedValue(Bytes)`:

```rust
use storkv_proto::{WirePutCondition, WirePutIfResult};

# async fn example(client: &storkv_client::AsyncClient, scope: &storkv_client::ScopeHandle) -> storkv_client::ClientResult<()> {
let result = client
    .put_if(scope, b"job:42", &WirePutCondition::Absent, b"worker-1", None)
    .await?;
match result {
    WirePutIfResult::Applied { seq } => println!("applied at {seq}"),
    WirePutIfResult::Conflict { current_seq, .. } => println!("conflict at {current_seq:?}"),
}
# Ok(())
# }
```

A conflict is a normal result. The request does not fail.

## Delete a range and merge

```rust
use storkv_proto::WireMergeOp;

# async fn example(client: &storkv_client::AsyncClient, scope: &storkv_client::ScopeHandle) -> storkv_client::ClientResult<()> {
client.delete_range(scope, b"tmp:0", b"tmp:9").await?;
client.merge(scope, b"page_views", WireMergeOp::CounterAdd(1)).await?;
# Ok(())
# }
```

The range is half-open. The start key is included and the end key is excluded.

`WireMergeOp` supports `CounterAdd(i64)`, `AppendBytes { data, max_len }`, `MaxI64(i64)`, `MinI64(i64)`, `BitOr(u64)`, `BitAnd(u64)`, and `Custom(Bytes)`. Read [Merge operators](../concepts/mvcc-and-transactions.md#merge-operators).

## Scan

Use `scan` with optional bounds:

```rust
# async fn example(client: &storkv_client::AsyncClient, scope: &storkv_client::ScopeHandle) -> storkv_client::ClientResult<()> {
let all = client.scan(scope, None::<&[u8]>, None::<&[u8]>).await?;
let page = client.scan(scope, Some(&b"user:100"[..]), Some(&b"user:200"[..])).await?;
# Ok(())
# }
```

Use `scan_prefix` for a prefix:

```rust
# async fn example(client: &storkv_client::AsyncClient, scope: &storkv_client::ScopeHandle) -> storkv_client::ClientResult<()> {
let users = client.scan_prefix(scope, b"user:").await?;
for (key, value) in users {
    println!("{} = {} bytes", String::from_utf8_lossy(&key), value.len());
}
# Ok(())
# }
```

A scan returns `Vec<(Bytes, Bytes)>` in bytewise key order. The client does not page the result. These client helpers send `limit: None`, `reverse: false`, and `keys_only: false`. The wire request supports these flags, so a custom `Request::Scan` can bound or reverse a scan. Read [HTTP and RPC API](../reference/http-and-rpc-api.md).

## Run a transaction

`begin_tx` returns an `AsyncTransaction`:

```rust
# async fn example(client: &storkv_client::AsyncClient, scope: &storkv_client::ScopeHandle) -> storkv_client::ClientResult<()> {
let mut tx = client.begin_tx().await?;
let current = tx.get(scope, b"counter").await?;
tx.put(scope, b"counter", b"1");
tx.commit().await?;
# Ok(())
# }
```

The transaction records every key that `tx.get` reads. A later write to a read key by another client makes `commit` fail with `ClientError::Server(WriteConflict { .. })`.

Buffer mutations with these methods:

| Method         | Effect                           |
| :------------- | :------------------------------- |
| `put`          | Buffer a value write             |
| `put_with_ttl` | Buffer a value write with a TTL  |
| `delete`       | Buffer a key delete              |
| `delete_range` | Buffer a half-open range delete  |
| `merge`        | Buffer a counter or custom merge |

`commit` consumes the transaction. Read your own buffered writes with `tx.get` before the commit.

## Watch changes

```rust
# async fn example(client: &storkv_client::AsyncClient) -> storkv_client::ClientResult<()> {
let mut stream = client.subscribe_watch().await?;
while let Some(event) = stream.next().await? {
    println!("{event:?}");
}
# Ok(())
# }
```

The native and WebSocket paths send every event. Filter by `scope_id` in your code. Read [Watches and TTL](../concepts/watches-and-ttl.md) for the event contract.

## Admin calls

```rust
# async fn example(client: &storkv_client::AsyncClient) -> storkv_client::ClientResult<()> {
let telemetry = client.telemetry().await?;
let stats = client.admin_stats().await?;
let backups = client.list_backups().await?;
client.compact().await?;
# Ok(())
# }
```

These calls need an admin connection when `--strict-admin` is set. Use `--admin-sock`, a listener marked as admin, or an admin token.

## Use the blocking client

`SyncClient` runs the async client on a private current-thread Tokio runtime:

```rust
use storkv_client::SyncClient;
use storkv_proto::{WireDurability, WireRetention};

fn main() -> storkv_client::ClientResult<()> {
    let client = SyncClient::connect_uri("storkv://127.0.0.1:7878")?;
    let scope = client.create_scope(
        "cache",
        WireDurability::StrictFsync,
        WireRetention::Permanent,
        None,
    )?;
    client.put(&scope, b"key", b"value")?;
    let value = client.get(&scope, b"key")?;
    assert_eq!(value.as_deref(), Some(&b"value"[..]));
    Ok(())
}
```

`SyncClient` covers ping, health, key data, scans, `key_meta`, telemetry, admin statistics, backups, scopes, and `checkpoint`. It does not expose transactions, watch streams, snapshots, or `compact`. Use `AsyncClient` for those calls.

## Handle errors

`ClientError` has these variants:

| Variant       | Cause                                       |
| :------------ | :------------------------------------------ |
| `Server`      | A wire error from the server                |
| `Io`          | A socket or file error                      |
| `Protocol`    | An unexpected response or malformed message |
| `Unsupported` | The transport cannot carry the operation    |
| `Http`        | An HTTP transport failure                   |
| `Grpc`        | A gRPC transport failure                    |
| `GrpcConnect` | A gRPC connection failure                   |
| `GrpcStatus`  | A non-OK gRPC status                        |

Match `ClientError::Server` to inspect the wire error. Read [Error codes](../reference/error-codes.md) for every wire code.

## Related guides

- [Rust SDK](../storkv-sdk/rust.md)
- [SDK overview](../storkv-sdk/index.md)
- [Connection URIs](../getting-started/connection-uris.md)
- [Transports](transports.md)
- [MVCC and transactions](../concepts/mvcc-and-transactions.md)
- [Observability](../operations/observability.md)
