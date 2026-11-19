# Transports

`storkvd` can serve the same database through several network protocols at the same time. Each listener is optional. No transport has a default port.

## Start listeners

Start each listener with its own flag:

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --listen 127.0.0.1:7878 \
  --admin-sock 127.0.0.1:7879 \
  --http-listen 127.0.0.1:8080 \
  --ws-listen 127.0.0.1:8081 \
  --grpc-listen 127.0.0.1:9090 \
  --redis-listen 127.0.0.1:6379 \
  --memcached-listen 127.0.0.1:11211
```

| Flag                 | Protocol                            | Admin by default |
| :------------------- | :---------------------------------- | :--------------- |
| `--listen`           | Native `STORKPRT` over TCP          | No               |
| `--admin-sock`       | Native over TCP or a Unix socket    | Yes              |
| `--http-listen`      | HTTP REST, binary RPC, etcd gateway | No               |
| `--ws-listen`        | WebSocket                           | No               |
| `--grpc-listen`      | gRPC over h2c                       | No               |
| `--redis-listen`     | Redis RESP2 and RESP3               | Not applicable   |
| `--memcached-listen` | Memcached ASCII                     | Not applicable   |

All listeners share one database, one scope registry, one WAL, and one backup worker.

## Native protocol

The native protocol uses one `STORKPRT` version 1 frame for each request and response. A connection can pipeline frames and can start one watch stream.

Use it for these cases:

- `storkv-client` over `storkv://`, `tcp://`, a bare `host:port`, or a Unix socket.
- `storkvctl` health and backup calls.
- `storkv-top` metrics.

The frame has a 4-byte big-endian length prefix and the `STORKPRT` magic. It also holds a 2-byte big-endian version, an 8-byte little-endian request ID, a payload tag, and the payload. The maximum frame size is 64 MiB. Read `SPEC.md` for the full layout.

A second process cannot open the data directory. The native listener does not change this rule.

> The native listener is a full-data trust boundary. It has no token field. A client on this listener can read and write every scope, and can start a replication stream that contains raw WAL frames for every scope. `--strict-admin` does not gate these streams. Serve the native listener on a private network or a Unix socket that only trusted users can reach.

## HTTP

`--http-listen` serves HTTP/1.1 and HTTP/2 on one address. It mounts these route groups:

| Route                              | Purpose                               |
| :--------------------------------- | :------------------------------------ |
| `/v1/health`, `/v1/ping`           | Liveness checks                       |
| `/v1/telemetry`, `/v1/admin/stats` | Counters and statistics               |
| `/v1/scopes`                       | Scope creation                        |
| `/v1/scopes/{scope}`               | Scope drop                            |
| `/v1/scopes/{scope}/keys/{key}`    | Key read, write, and delete           |
| `/v1/scopes/{scope}/merge/{key}`   | Merge operation                       |
| `/v1/scopes/{scope}/delete-range`  | Range delete                          |
| `/v1/scopes/{scope}/scan`          | Range or prefix scan                  |
| `/v1/tx/begin`, `/v1/tx/commit`    | Transactions                          |
| `/v1/admin/*`                      | Checkpoint, compact, backup, snapshot |
| `/v1/rpc`                          | Binary `STORKPRT` frame tunnel        |
| `/v1/ws`                           | WebSocket upgrade                     |
| `/v3/kv/*`                         | etcd v3 JSON gateway                  |

Read [HTTP and RPC API](../reference/http-and-rpc-api.md) for request and response bodies.

`POST /v1/rpc` is not a JSON-RPC endpoint. It accepts one binary `STORKPRT` request frame and returns one binary response frame. The HTTP client backend uses this route.

The `format=json` query option changes a key read from raw bytes to a JSON envelope:

```bash
curl -sS "http://127.0.0.1:8080/v1/scopes/default/keys/greeting?format=json"
```

## WebSocket

`--ws-listen` serves only the `/v1/ws` upgrade route. The shared HTTP listener also accepts `/v1/ws` when `--http-listen` is set.

A WebSocket client can send two frame types:

- Binary frames that hold one or more `STORKPRT` messages.
- Text frames that hold one JSON operation.

A JSON request has an `id` and an `op`:

```json
{ "id": 1, "op": "get", "scope": "default", "key": "greeting" }
```

The reply is a text frame:

```json
{ "id": 1, "ok": true, "result": { "found": true, "value_b64": "aGVsbG8=" } }
```

The text surface supports health, data, scan, transaction, admin, and watch operations. Read [HTTP and RPC API](../reference/http-and-rpc-api.md#supported-text-operations) for the exact list.

> [!WARNING]
> A watch subscription sends later events as binary `STORKPRT` frames, even when the subscription used a text frame. The initial acknowledgement is a text frame. Decode binary frames after you subscribe.

Use `wss://` only through a TLS proxy. The `storkvd` WebSocket listener has no TLS layer.

## gRPC

`--grpc-listen` serves two plaintext h2c services from `proto/storkv/v1/storkv.proto`:

| Service       | Methods                                                                                                                                         |
| :------------ | :---------------------------------------------------------------------------------------------------------------------------------------------- |
| `Storkv`      | `Ping`, `Health`, `Get`, `Put`, `Delete`, `DeleteRange`, `Merge`, `Scan`, `BeginTx`, `Commit`, `CreateScope`, `DropScope`, `Watch`, `Telemetry` |
| `StorkvAdmin` | `AdminStats`, `Checkpoint`, `Compact`, `CreateSnapshot`, `UnpinSnapshot`, `TriggerBackup`, `BackupStatus`, `VerifyBackup`, `ListBackups`        |

`Watch` is the only stream method. It accepts `scope_id` and `all_scopes` and filters on the server.

The client connects with `grpc://host:port`. The backend uses `http://host:port` for the tonic channel. A TLS terminator in front of the port needs a separate client setup.

## Redis, Memcached, and etcd

These compatibility front ends map their data onto scopes:

| Front end | Listener             | Scope rule                                                |
| :-------- | :------------------- | :-------------------------------------------------------- |
| Redis     | `--redis-listen`     | `SELECT 0` uses `default`; `SELECT n` uses `redis_db_<n>` |
| Memcached | `--memcached-listen` | Every item uses the `memcached` scope                     |
| etcd      | `/v3/kv/*` on HTTP   | Every key uses the `etcd` scope                           |

The adapters create their scope on first use. Read [Redis compatibility](compatibility-redis.md) and [Memcached and etcd compatibility](compatibility-memcached-etcd.md) for command coverage.

## Admin policy

Admin-only requests control global state or expose backup internals. The set is:

- `Checkpoint`
- `Compact`
- `CreateSnapshot`
- `UnpinSnapshot`
- `DropScope`
- `TriggerBackup`
- `VerifyBackup`
- `ListBackups`
- `ReplicationStart`

These requests are always allowed when `--strict-admin` is off.

With `--strict-admin`, a request needs one of these grants:

1. A native connection accepted on `--admin-sock`.
2. A listener started with `--http-admin`, `--ws-admin`, or `--grpc-admin`.
3. A token that matches `--admin-token`, `--admin-token-file`, or `STORKV_ADMIN_TOKEN`.

Use `--admin-token-file` or `STORKV_ADMIN_TOKEN` to keep the token out of the process arguments. The token file must hold one line.

HTTP and WebSocket clients present the token in `x-storkv-admin-token` or in `Authorization: Bearer <token>`. A denied request returns wire error `Unauthorized` (code 14), which maps to HTTP 403.

On Unix, the `--admin-sock` directory uses mode `0700` and the socket file uses mode `0600`. Startup fails when the server cannot apply these modes.

## TLS

StorKV has no built-in TLS. The `https://` and `wss://` URI schemes use the plaintext backends. Put a TLS proxy in front of the listener when you need encryption:

```text
client -> TLS proxy (443) -> storkvd HTTP (8080)
```

Connect the client to the proxy address with `http://` or `ws://` when the proxy forwards plaintext to `storkvd`.

## Related guides

- [Connection URIs](../getting-started/connection-uris.md)
- [HTTP and RPC API](../reference/http-and-rpc-api.md)
- [Configuration](../operations/configuration.md)
- [Troubleshooting](../operations/troubleshooting.md)
