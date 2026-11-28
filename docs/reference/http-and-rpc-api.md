# HTTP and RPC API

`storkvd` serves a JSON REST API, a binary frame tunnel, a WebSocket upgrade route, and an etcd v3 gateway on the HTTP listener. Enable the listener with `--http-listen`:

```bash
./target/release/storkvd --data-dir ./data/storkv --http-listen 127.0.0.1:8080
```

This page uses `http://127.0.0.1:8080` in every example.

## Route summary

| Method | Route                             | Purpose                   |
| :----- | :-------------------------------- | :------------------------ |
| GET    | `/v1/health`                      | Liveness                  |
| GET    | `/v1/ping`                        | Ping                      |
| GET    | `/v1/telemetry`                   | Global counters           |
| GET    | `/v1/admin/stats`                 | Rich statistics           |
| GET    | `/metrics`                        | Prometheus text metrics   |
| POST   | `/v1/scopes`                      | Create a scope            |
| DELETE | `/v1/scopes/{scope}`              | Drop a scope              |
| GET    | `/v1/scopes/{scope}/keys/{key}`   | Read a key                |
| PUT    | `/v1/scopes/{scope}/keys/{key}`   | Write a key               |
| DELETE | `/v1/scopes/{scope}/keys/{key}`   | Delete a key              |
| POST   | `/v1/scopes/{scope}/merge/{key}`  | Merge an operand          |
| POST   | `/v1/scopes/{scope}/delete-range` | Delete a key range        |
| GET    | `/v1/scopes/{scope}/scan`         | Range or prefix scan      |
| POST   | `/v1/tx/begin`                    | Start a transaction       |
| POST   | `/v1/tx/commit`                   | Validate and commit       |
| POST   | `/v1/admin/checkpoint`            | Write a checkpoint        |
| POST   | `/v1/admin/compact`               | Run compaction            |
| POST   | `/v1/admin/backup`                | Trigger a full backup     |
| POST   | `/v1/admin/snapshots`             | Pin a snapshot            |
| DELETE | `/v1/admin/snapshots/{name}`      | Unpin a snapshot          |
| POST   | `/v1/admin/verify-backup`         | Verify backup generations |
| GET    | `/v1/admin/backups`               | List backup generations   |
| POST   | `/v1/rpc`                         | Binary `STORKPRT` tunnel  |
| GET    | `/v1/ws`                          | WebSocket upgrade         |
| POST   | `/v3/kv/put`                      | etcd gateway put          |
| POST   | `/v3/kv/range`                    | etcd gateway range        |
| POST   | `/v3/kv/deleterange`              | etcd gateway range delete |
| POST   | `/v3/kv/txn`                      | etcd gateway transaction  |
| POST   | `/v3/lease/grant`                 | etcd lease grant          |
| POST   | `/v3/lease/keepalive`             | etcd lease keepalive      |
| POST   | `/v3/lease/revoke`                | etcd lease revoke         |
| POST   | `/v3/lease/timetolive`            | etcd lease time to live   |

The route paths use axum syntax. `{scope}` and `{key}` are path segments. The service mounts no route at `/`.

## Common behavior

### Success bodies

Most data routes return a JSON object. The exact body depends on the operation.

| Response          | Body                                                  |
| :---------------- | :---------------------------------------------------- |
| Empty result      | `{"ok": true}`                                        |
| Ping              | `{"pong": true}`                                      |
| Health            | `{"status": "ok"}`                                    |
| Value present     | `{"found": true, "value_b64": "<base64>"}`            |
| Scan              | `{"pairs": [{"key_b64": "...", "value_b64": "..."}]}` |
| Scope             | `{"scope_id": 1, "name": "sessions"}`                 |
| Transaction start | `{"read_seq": 18422}`                                 |

Base64 uses the standard alphabet. The decoder also accepts URL-safe base64 without padding.

### Error bodies

Every error uses this shape:

```json
{
  "error": { "code": "scope_not_found", "message": "scope not found: sessions" }
}
```

The status comes from the wire error class:

| Status | Wire error class                                  |
| :----- | :------------------------------------------------ |
| 400    | Invalid operation, length limits, protocol, other |
| 403    | `Unauthorized`                                    |
| 404    | Scope or snapshot not found                       |
| 409    | Scope already exists, write conflict              |
| 500    | Corruption, I/O                                   |
| 503    | Database locked                                   |

Read [Error codes](error-codes.md) for the full code list.

### Admin policy

Under `--strict-admin`, these routes need an admin grant:

- `POST /v1/admin/checkpoint`
- `POST /v1/admin/compact`
- `POST /v1/admin/backup`
- `POST /v1/admin/snapshots`
- `DELETE /v1/admin/snapshots/{name}`
- `POST /v1/admin/verify-backup`
- `GET /v1/admin/backups`
- `DELETE /v1/scopes/{scope}`

`GET /v1/admin/stats` and `GET /v1/telemetry` are not in the admin-only set.

Present the token in one of these headers:

```bash
-H "x-storkv-admin-token: $STORKV_ADMIN_TOKEN"
-H "authorization: Bearer $STORKV_ADMIN_TOKEN"
```

### Key encoding

The key path segment is used as raw bytes from its UTF-8 text. A key with binary bytes or a slash needs `key_b64`:

```bash
curl -sS \
  "http://127.0.0.1:8080/v1/scopes/default/keys/ignored?key_b64=$(printf 'user:42' | base64)"
```

When `key_b64` is present, it replaces the path key.

## Health, ping, and metrics

```bash
curl -sS http://127.0.0.1:8080/v1/health
curl -sS http://127.0.0.1:8080/v1/ping
curl -sS http://127.0.0.1:8080/v1/telemetry
curl -sS http://127.0.0.1:8080/v1/admin/stats
curl -sS http://127.0.0.1:8080/metrics
```

The `/metrics` route returns the Prometheus text exposition format. Read [Observability](../operations/observability.md#prometheus-metrics) for the metric names.

```json
{ "status": "ok" }
```

```json
{ "pong": true }
```

Read [Observability](../operations/observability.md) for every counter field.

## Scopes

### Create a scope

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/scopes \
  -H 'content-type: application/json' \
  -d '{
    "name": "sessions",
    "durability": {"buffered": {"sync_interval_ms": 100}},
    "retention": {"ttl": {"default_ttl_ms": 900000, "compaction_style": "fifo"}},
    "prefix_bloom_len": 8
  }'
```

```json
{ "scope_id": 1, "name": "sessions" }
```

| Field                 | Required | Values                                                                               |
| :-------------------- | :------- | :----------------------------------------------------------------------------------- |
| `name`                | yes      | Scope name                                                                           |
| `durability`          | no       | `"strict_fsync"`, `"buffered"`, or `{"buffered":{"sync_interval_ms":N}}`             |
| `retention`           | no       | `"permanent"` or `{"ttl":{"default_ttl_ms":N,"compaction_style":"leveled"\|"fifo"}}` |
| `prefix_bloom_len`    | no       | Bloom prefix length in bytes                                                         |
| `min_ttl_ms`          | no       | Rejects writes with a shorter TTL; `null` removes the limit                          |
| `max_ttl_ms`          | no       | Rejects writes with a longer TTL; `null` removes the limit                           |
| `allow_permanent`     | no       | Boolean; permits writes without an expiry. Default `true`                            |
| `active_ttl_tracking` | no       | Boolean; tracks deadlines and emits expiry events. Default `false`                   |

The defaults are `strict_fsync`, `permanent`, no Bloom prefix override, no TTL bounds, `allow_permanent: true`, and `active_ttl_tracking: false`. `"buffered"` uses a 100 ms sync interval. A write that breaks a TTL guardrail fails with `invalid_ttl`.

`default_ttl_ms` is stored in the scope options. It is not applied to writes that omit a TTL. Read [Watches and TTL](../concepts/watches-and-ttl.md#ttl-edge-cases).

### Drop a scope

```bash
curl -sS -X DELETE http://127.0.0.1:8080/v1/scopes/sessions
```

```json
{ "ok": true }
```

A drop removes the scope, its memtables, and its SSTables. It emits a `DropScope` watch event.

## Keys

### Read a key

```bash
curl -sS http://127.0.0.1:8080/v1/scopes/default/keys/greeting
```

By default the route returns the raw value with `content-type: application/octet-stream`. Use `format=json` for a JSON envelope:

```bash
curl -sS "http://127.0.0.1:8080/v1/scopes/default/keys/greeting?format=json"
```

```json
{ "found": true, "value_b64": "aGVsbG8=" }
```

A missing key returns status 404 with code `not_found`. A missing scope returns status 404 with code `scope_not_found`.

### Write a key

```bash
curl -sS -X PUT \
  --data-binary 'hello' \
  http://127.0.0.1:8080/v1/scopes/default/keys/greeting
```

```json
{ "ok": true }
```

The request body is the value. These query options change the input:

| Query       | Meaning                                                          |
| :---------- | :--------------------------------------------------------------- |
| `key_b64`   | Base64 key that replaces the path key                            |
| `value_b64` | Base64 value that replaces the body                              |
| `ttl_ms`    | Positive TTL in milliseconds; 0 or an invalid value means no TTL |

A JSON body with a `value_b64` field is also accepted when `value_b64` is absent from the query.

```bash
curl -sS -X PUT \
  -H 'content-type: application/json' \
  -d '{"value_b64":"aGVsbG8="}' \
  http://127.0.0.1:8080/v1/scopes/default/keys/greeting
```

A write with `ttl_ms` stores an absolute expiry. A later write without `ttl_ms` removes the expiry.

### Conditional write

A conditional write stores a value only when the newest visible state matches a condition. The binary RPC carries this operation as `Request::PutIf`. The Rust client exposes it as `put_if`, the language clients expose the same operation, and the C FFI exposes `storkv_put_if`. The REST API has no put-if route.

The conditions are:

| Condition       | Match rule                                   |
| :-------------- | :------------------------------------------- |
| `Absent`        | The key is absent or expired                 |
| `Present`       | The key is present and not expired           |
| `ExpectedSeq`   | The visible version has this sequence number |
| `ExpectedValue` | The visible value equals these bytes         |

The result is `Applied { seq }` on a match. A mismatch returns `Conflict { current_seq, current_value }`, where both fields are `None` when the key is absent. A conflict is not an error. The caller can retry with the new state.

### Delete a key

```bash
curl -sS -X DELETE http://127.0.0.1:8080/v1/scopes/default/keys/greeting
```

```json
{ "ok": true }
```

A delete of an absent key succeeds.

### Merge

The default operation adds a signed 64-bit value:

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/scopes/default/merge/user:42:logins \
  -H 'content-type: application/json' \
  -d '{"delta":1}'
```

| Field         | Default | Meaning                              |
| :------------ | :------ | :----------------------------------- |
| `op`          | counter | `"custom"` selects a byte operand    |
| `delta`       | `1`     | Signed delta for the counter merge   |
| `operand_b64` | none    | Required base64 operand for `custom` |

A custom merge applies the operand bytes to the existing value. Read [Merge operators](../concepts/mvcc-and-transactions.md#merge-operators) for the merge rules.

### Delete a range

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/scopes/default/delete-range \
  -H 'content-type: application/json' \
  -d '{
    "start_b64": "dXNlcjowMDAw",
    "end_b64": "dXNlcjowMTAw"
  }'
```

```json
{ "ok": true }
```

Both fields are required. `start_b64` is inclusive and `end_b64` is exclusive.

### Scan

```bash
curl -sS "http://127.0.0.1:8080/v1/scopes/default/scan?start=user:0000&end=user:0100"
curl -sS "http://127.0.0.1:8080/v1/scopes/default/scan?prefix=user:"
curl -sS "http://127.0.0.1:8080/v1/scopes/default/scan?prefix_b64=dXNlcjo="
```

```json
{ "pairs": [{ "key_b64": "dXNlcjowMDAw", "value_b64": "YWN0aXZl" }] }
```

| Query        | Meaning                         |
| :----------- | :------------------------------ |
| `prefix_b64` | Base64 prefix; takes precedence |
| `prefix`     | UTF-8 prefix, when not empty    |
| `start_b64`  | Base64 start key, inclusive     |
| `start`      | UTF-8 start key, when not empty |
| `end_b64`    | Base64 end key, exclusive       |
| `end`        | UTF-8 end key, when not empty   |

A scan returns every matching pair. A scan with no bounds returns the whole scope. Native and Redis clients can bound a scan with a limit. The HTTP query parameters above do not take a limit.

## Transactions

Transactions use optimistic concurrency control. The server returns a read sequence, and the commit carries the read set and the write set.

### Begin

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/tx/begin
```

```json
{ "read_seq": 18422 }
```

### Commit

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/tx/commit \
  -H 'content-type: application/json' \
  -d '{
    "read_seq": 18422,
    "read_set": [
      {"scope": "default", "key_b64": "dXNlcjo0Mg=="}
    ],
    "write_set": [
      {"scope": "default", "key_b64": "dXNlcjo0Mg==", "value_b64": "YWN0aXZl"},
      {"scope": "default", "key_b64": "dXNlcjo0MzpiYWxhbmNl", "kind": "delete"}
    ]
  }'
```

```json
{ "ok": true }
```

Each read-set entry needs a scope and a key. Each write-set entry needs a scope, a key, and a `kind`:

| `kind`          | Extra fields                 | Effect                             |
| :-------------- | :--------------------------- | :--------------------------------- |
| `put` (default) | `value_b64`, `expires_at_ms` | Write a value with optional expiry |
| `delete`        | none                         | Delete the key                     |
| `delete_range`  | `end_b64`                    | Delete from the key to `end_b64`   |
| `merge`         | `operand_b64`                | Apply the operand                  |

A scope can be a name or a numeric `scope_id`. A commit returns status 409 with code `write_conflict` when a read key changed after `read_seq`.

Read [MVCC and transactions](../concepts/mvcc-and-transactions.md) for validation details.

## Admin routes

### Checkpoint

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/admin/checkpoint \
  -H 'content-type: application/json' \
  -d '{"dest_dir":"./backups/snap-01"}'
```

`dest_dir` is required. The response is `{"ok":true}`.

### Compact

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/admin/compact
```

```json
{ "ok": true }
```

Add the `scope` query parameter to compact one scope. Add `start` and `end` to compact only the SSTables that overlap the half-open range `[start, end)`:

```bash
curl -sS -X POST 'http://127.0.0.1:8080/v1/admin/compact?scope=default&start=a&end=z'
```

| Parameter   | Default | Meaning                             |
| :---------- | :------ | :---------------------------------- |
| `scope`     | (all)   | Scope to compact                    |
| `start`     | (open)  | Inclusive start key, UTF-8 text     |
| `end`       | (open)  | Exclusive end key, UTF-8 text       |
| `start_b64` | (none)  | Base64 start key. Overrides `start` |
| `end_b64`   | (none)  | Base64 end key. Overrides `end`     |

An omitted bound is open. The route returns `400` with code `invalid_operation` when `start >= end`.

### Trigger a backup

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/admin/backup \
  -H 'content-type: application/json' \
  -d '{"wait":true}'
```

| Field  | Default | Meaning                       |
| :----- | :------ | :---------------------------- |
| `wait` | `false` | Wait for the backup to finish |

The daemon must have started with `--backup-dir`. Without it, the route returns code `invalid_operation` with the message `auto-backup is not configured`.

A wait request returns a backup report:

```json
{
  "generation_id": "full-1763400000000-00000000000000001234",
  "sequence": 18422,
  "files": 84,
  "bytes": 734003200,
  "generations_removed": 0,
  "wal_segments_removed": 0,
  "bytes_freed": 0
}
```

A request with `wait: false` queues the backup and returns an empty report at once. The report has zero values in every field.

The WebSocket text operation defaults `wait` to `true`. The REST route defaults to `false`.

### Create and unpin a snapshot

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/admin/snapshots \
  -H 'content-type: application/json' \
  -d '{"name":"pre-migration"}'
```

```bash
curl -sS -X DELETE http://127.0.0.1:8080/v1/admin/snapshots/pre-migration
```

A pinned snapshot prevents compaction from removing versions that the snapshot can still need.

### Verify backups

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/admin/verify-backup \
  -H 'content-type: application/json' \
  -d '{"target_seq":18422}'
```

```json
{
  "generations_checked": 3,
  "files_checked": 84,
  "bytes_checked": 734003200,
  "wal_segments_checked": 12,
  "errors": []
}
```

`target_seq` is optional. When absent, the server verifies the full reachable archive.

### List backups

```bash
curl -sS http://127.0.0.1:8080/v1/admin/backups
```

```json
{
  "generations": [
    {
      "generation_id": "full-1763400000000-00000000000000001234",
      "kind": "full",
      "created_at_ms": 1763400000000,
      "min_seq": 18000,
      "max_seq": 18422,
      "total_bytes": 734003200,
      "files": 84,
      "compression": "none"
    }
  ]
}
```

## Binary RPC tunnel

`POST /v1/rpc` accepts one complete `STORKPRT` version 1 request frame. The body is binary. The response is one `STORKPRT` version 1 response frame with `content-type: application/octet-stream`.

```bash
curl -sS -X POST \
  -H 'content-type: application/octet-stream' \
  --data-binary @request.bin \
  http://127.0.0.1:8080/v1/rpc \
  --output response.bin
```

This route is not JSON-RPC. It carries the same frames as the native TCP listener. The frame has these parts:

| Part          | Size     | Encoding            |
| :------------ | :------- | :------------------ |
| Length prefix | 4 bytes  | Big-endian          |
| Magic         | 8 bytes  | `STORKPRT`          |
| Version       | 2 bytes  | Big-endian, `1`     |
| Request ID    | 8 bytes  | Little-endian       |
| Payload tag   | 1 byte   | Request or response |
| Payload       | variable | Tagged fields       |

The maximum frame size is 64 MiB. Read `SPEC.md` for the field layout.

Invalid input returns status 400 with code `invalid_frame` or `truncated_frame`.

## WebSocket

`GET /v1/ws` upgrades to a WebSocket session. Send either binary `STORKPRT` frames or one JSON operation in a text frame.

A JSON request has an `id` and an `op`:

```json
{ "id": 1, "op": "get", "scope": "default", "key": "greeting" }
```

```json
{ "id": 1, "ok": true, "result": { "found": true, "value_b64": "aGVsbG8=" } }
```

An error response has `"ok": false`:

```json
{
  "id": 1,
  "ok": false,
  "error": { "code": "scope_not_found", "message": "scope not found: nope" }
}
```

### Supported text operations

`ping`, `health`, `telemetry`, `admin_stats`, `tx_begin`, `tx_commit`, `compact`, `list_backups`, `create_scope`, `drop_scope`, `checkpoint`, `backup`, `verify_backup`, `watch`, `get`, `put`, `delete`, `delete_range`, `merge`, `scan`, and `scan_prefix`.

The text surface is smaller than the REST surface. It has no `backup_status`, `create_snapshot`, or `unpin_snapshot` operation.

### Text operation limits

- `create_scope` always uses strict durability and permanent retention. It accepts only `name` and `prefix_bloom_len`.
- `put` has no TTL field. Use REST, the Rust client, or a binary frame for a TTL write.
- Key and value fields accept plain text (`key`, `value`) or base64 (`key_b64`, `value_b64`).
- A scope is resolved from `scope` or from `scope_id`.

### Watch

```json
{ "id": 1, "op": "watch" }
```

```json
{ "id": 1, "ok": true, "watching": true }
```

Later events arrive as binary `STORKPRT` frames. Read [Watches and TTL](../concepts/watches-and-ttl.md#subscribe-over-websocket).

## etcd gateway

The HTTP listener also mounts eight etcd v3 JSON routes:

| Route                  | etcd operation     |
| :--------------------- | :----------------- |
| `/v3/kv/put`           | Put                |
| `/v3/kv/range`         | Range read         |
| `/v3/kv/deleterange`   | Delete range       |
| `/v3/kv/txn`           | Transaction        |
| `/v3/lease/grant`      | Lease grant        |
| `/v3/lease/keepalive`  | Lease keepalive    |
| `/v3/lease/revoke`     | Lease revoke       |
| `/v3/lease/timetolive` | Lease time to live |

Every key maps to the `etcd` scope. Request and response fields use base64 for keys and values. Lease identifiers are decimal strings. A transaction runs one compare list over the StorKV OCC read set and retries on a write conflict.

```bash
curl -sS -X POST http://127.0.0.1:8080/v3/kv/put \
  -H 'content-type: application/json' \
  -d '{"key":"L2NvbmZpZy9h","value":"b24="}'
```

Read [Memcached and etcd compatibility](../guides/compatibility-memcached-etcd.md) for field coverage and limits.

## Related guides

- [Transports](../guides/transports.md)
- [Error codes](error-codes.md)
- [storkvctl reference](cli-storkvctl.md)
- [Observability](../operations/observability.md)
- On-disk format: `FORMAT.md`
