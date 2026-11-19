# Connection URIs

The StorKV clients select a transport from a connection URI. `AsyncClient::connect_uri` and `SyncClient::connect_uri` accept every URI form on this page.

## Supported schemes

| URI form               | Transport          | Default port | Notes                                        |
| :--------------------- | :----------------- | :----------- | :------------------------------------------- |
| `storkv://host:port`   | Native TCP         | None         | Port is required                             |
| `tcp://host:port`      | Native TCP         | None         | Alias for `storkv://`                        |
| `host:port`            | Native TCP         | None         | Bare address keeps the historical meaning    |
| `unix:///path/to.sock` | Native Unix socket | Not used     | The path must be absolute                    |
| `/path/to.sock`        | Native Unix socket | Not used     | A path with `/` or `\` selects a Unix socket |
| `http://host:port`     | HTTP tunnel        | None         | Uses `POST /v1/rpc`                          |
| `https://host:port`    | HTTP tunnel        | None         | The client uses the same HTTP backend        |
| `ws://host:port`       | WebSocket          | None         | The client appends `/v1/ws`                  |
| `wss://host:port`      | WebSocket          | None         | The client uses the same WebSocket backend   |
| `grpc://host:port`     | gRPC               | None         | Connects with h2c                            |

A URI without a target fails with a protocol error. An unknown scheme fails with `unsupported transport scheme`.

## Parse a URI

Use `TransportKind::parse_uri` to inspect a URI before you connect:

```rust
use storkv_client::TransportKind;

let (kind, target) = TransportKind::parse_uri("unix:///run/storkv/admin.sock")?;
assert_eq!(target, "/run/storkv/admin.sock");
```

`TransportKind` has these values:

| Value        | Meaning                       |
| :----------- | :---------------------------- |
| `NativeTcp`  | Native protocol over TCP      |
| `NativeUnix` | Native protocol over a socket |
| `Http`       | HTTP binary tunnel            |
| `WebSocket`  | WebSocket binary frames       |
| `Grpc`       | gRPC services                 |

## Choose a connect method

The client has three connect methods:

| Method        | Accepts                                    | Use for                      |
| :------------ | :----------------------------------------- | :--------------------------- |
| `connect`     | `host:port`                                | Native TCP only              |
| `connect_any` | Bare TCP, Unix paths, and every URI scheme | Mixed local deployments      |
| `connect_uri` | Every URI scheme on this page              | Explicit transport selection |

Use `connect_uri` in new code.

## Path handling

The client keeps the URI target after it strips the scheme.

- For `unix:///var/run/storkv.sock`, the target is `/var/run/storkv.sock`.
- For `ws://127.0.0.1:8081`, the WebSocket backend appends `/v1/ws`.
- For `ws://127.0.0.1:8081/v1/ws`, the backend keeps the path.
- For `http://127.0.0.1:8080`, the HTTP backend appends `/v1/rpc`.
- For `http://127.0.0.1:8080/v1/rpc`, the backend keeps the path.

A trailing slash is removed from `storkv://`, `tcp://`, and `grpc://` targets.

## Query parameters

The client does not parse query parameters in a connection URI. Do not put options in the URI.

Use the REST API query strings for REST options such as `ttl_ms`, `prefix`, `start`, and `end`. The URI examples in this page do not carry query strings.

## Credentials and TLS

The client URI parser has no user-info or TLS option fields.

- The parser accepts `https://` and `wss://` schemes, but the current HTTP and WebSocket backends do not add a TLS layer. Use a local TLS proxy in front of `storkvd` and connect to the proxy with `http://` or `ws://`.
- gRPC uses h2c over plaintext. A `grpc://host:port` target connects with `http://host:port`.
- Admin tokens are request headers, not URI fields. Use `x-storkv-admin-token` or an `Authorization: Bearer` header on HTTP requests.

## No multi-URI failover

`connect_uri` takes one URI. The client does not accept a comma-separated list and has no built-in failover.

Implement failover in application code. Try each URI in order and keep the first client that passes a health check:

```rust
use storkv_client::{AsyncClient, ClientError};

async fn connect_first(uris: &[&str]) -> Result<AsyncClient, ClientError> {
    let mut last_error = None;
    for uri in uris {
        match AsyncClient::connect_uri(uri).await {
            Ok(client) => match client.health_check().await {
                Ok(()) => return Ok(client),
                Err(error) => last_error = Some(error),
            },
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        ClientError::Protocol("no connection URI was provided".into())
    }))
}
```

This pattern gives an explicit order and an explicit health gate.

## URI examples

Connect over native TCP:

```rust
let client = storkv_client::AsyncClient::connect_uri("storkv://127.0.0.1:7878").await?;
```

Connect over a Unix socket:

```rust
let client = storkv_client::AsyncClient::connect_uri("unix:///run/storkv/admin.sock").await?;
```

Connect over HTTP:

```rust
let client = storkv_client::AsyncClient::connect_uri("http://127.0.0.1:8080").await?;
```

Connect over WebSocket:

```rust
let client = storkv_client::AsyncClient::connect_uri("ws://127.0.0.1:8081").await?;
```

Connect over gRPC:

```rust
let client = storkv_client::AsyncClient::connect_uri("grpc://127.0.0.1:9090").await?;
```

## Related guides

- [Rust client](../guides/rust-client.md)
- [Transports](../guides/transports.md)
- [Troubleshooting](../operations/troubleshooting.md)
