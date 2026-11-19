# Quickstart

This page shows how to build StorKV, start the server, write and read data, and open the terminal dashboard.

## Before you start

You need these tools:

- Rust 1.87 or later.
- `curl` for the HTTP examples.
- A terminal for `storkv-top`.

StorKV uses the Rust 2024 edition.

## Build the workspace

Run this command in the repository root:

```bash
cargo build --workspace --release
```

The build writes these programs to `target/release`:

| Program      | Purpose                                      |
| :----------- | :------------------------------------------- |
| `storkvd`    | Network server and storage engine host       |
| `storkvctl`  | Health, backup, restore, migration, and test |
| `storkv-top` | Live terminal dashboard                      |

Verify the binaries:

```bash
./target/release/storkvd --help
./target/release/storkvctl --help
./target/release/storkv-top --help
```

> [!NOTE]
> On Windows, use `target\release\storkvd.exe` and the matching `.exe` files.

## Start the server

Start `storkvd` with a data directory and the HTTP listener:

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --listen 127.0.0.1:7878 \
  --http-listen 127.0.0.1:8080
```

The server prints messages like these:

```text
storkvd listening on 127.0.0.1:7878
storkvd http listening on 127.0.0.1:8080
```

The native listener uses `127.0.0.1:7878` by default. The HTTP listener is optional.

`storkvd` creates the `default` scope on first open. Scope 0 always carries the name `default`.

Press `Ctrl+C` to stop the server. The server flushes the shutdown path and closes every listener.

## Check health

Use `storkvctl health` with any supported URI:

```bash
./target/release/storkvctl health --sock 127.0.0.1:7878
```

Expected output:

```text
OK - storkvd is healthy
```

The same check works over HTTP:

```bash
./target/release/storkvctl health --sock http://127.0.0.1:8080
```

## Write and read keys

`storkvctl` does not have key CRUD subcommands in v1. Use the REST API or the Rust client for data operations.

Write a key:

```bash
curl -sS -X PUT \
  --data-binary 'hello' \
  http://127.0.0.1:8080/v1/scopes/default/keys/greeting
```

Response:

```json
{ "ok": true }
```

Read the key:

```bash
curl -sS http://127.0.0.1:8080/v1/scopes/default/keys/greeting
```

Response:

```text
hello
```

The read route returns raw bytes with `content-type: application/octet-stream` when the key exists.

Add `format=json` to receive a JSON envelope:

```bash
curl -sS "http://127.0.0.1:8080/v1/scopes/default/keys/greeting?format=json"
```

Response:

```json
{ "found": true, "value_b64": "aGVsbG8=" }
```

Delete the key:

```bash
curl -sS -X DELETE \
  http://127.0.0.1:8080/v1/scopes/default/keys/greeting
```

Response:

```json
{ "ok": true }
```

A missing key returns HTTP 404:

```json
{ "error": { "code": "not_found", "message": "key not found" } }
```

## Write keys with a TTL

Pass `ttl_ms` in the query string:

```bash
curl -sS -X PUT \
  --data-binary 'user:42' \
  "http://127.0.0.1:8080/v1/scopes/default/keys/sess:abc?ttl_ms=60000"
```

The key becomes invisible when the wall clock reaches the expiry time. The write path does not remove the key at that moment. Reads filter expired entries.

Read the remaining time with the Rust client method `key_ttl_remaining_ms`. The REST data routes do not expose TTL metadata.

## Scan a range or prefix

Use the scan route with `prefix`:

```bash
curl -sS "http://127.0.0.1:8080/v1/scopes/default/scan?prefix=user:"
```

Response shape:

```json
{ "pairs": [{ "key_b64": "dXNlcjox", "value_b64": "YWxpY2U=" }] }
```

Use `start` and `end` for a range:

```bash
curl -sS "http://127.0.0.1:8080/v1/scopes/default/scan?start=user:1&end=user:9"
```

The `start` bound is inclusive. The `end` bound is exclusive. Keys arrive in bytewise lexicographical order.

Use `start_b64` and `end_b64` when a key is not valid UTF-8. Use `prefix_b64` for binary prefixes.

## Create a scope

Create a scope with strict durability and permanent retention:

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/scopes \
  -H 'content-type: application/json' \
  -d '{"name":"sessions","durability":"strict_fsync","retention":"permanent"}'
```

Response:

```json
{ "scope_id": 2, "name": "sessions" }
```

Create a scope with buffered durability and FIFO retention:

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/scopes \
  -H 'content-type: application/json' \
  -d '{
    "name":"cache",
    "durability":{"buffered":{"sync_interval_ms":100}},
    "retention":{"ttl":{"default_ttl_ms":900000,"compaction_style":"fifo"}},
    "prefix_bloom_len":8
  }'
```

## Run a transaction

Start a transaction and record the read sequence:

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/tx/begin
```

Response:

```json
{ "read_seq": 42 }
```

Commit a write set:

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/tx/commit \
  -H 'content-type: application/json' \
  -d '{
    "read_seq":42,
    "read_set":[],
    "write_set":[
      {"scope":"default","key_b64":"Y291bnRlcg==","value_b64":"MQ=="}
    ]
  }'
```

Response:

```json
{ "ok": true }
```

The commit fails with HTTP 409 when a read key changed after `read_seq`. See [MVCC and transactions](../concepts/mvcc-and-transactions.md) for the retry pattern.

## Open the dashboard

Start `storkv-top` against the native listener:

```bash
./target/release/storkv-top --sock 127.0.0.1:7878
```

The default poll interval is 1000 ms. Change it with `--interval-ms`:

```bash
./target/release/storkv-top --sock 127.0.0.1:7878 --interval-ms 500
```

Press `q` or `Q` to quit. The dashboard has panels for amplification, L0 to L6 levels, WAL and fsyncs, backup state, and per-scope metrics.

## Check offline integrity

Stop the server before you run `doctor` on a live data directory:

```bash
./target/release/storkvctl doctor --data-dir ./data/storkv
```

Expected output for a clean directory:

```text
Doctor Report: 12 SSTables verified, 1 Manifests verified.
All integrity checks PASSED.
```

The command exits with status 1 when it finds an error.

## Next steps

- Learn when to embed the engine in [Embedded or server](embedded-vs-server.md).
- Learn every URI form in [Connection URIs](connection-uris.md).
- Learn the write and read path in [Architecture](../concepts/architecture.md).
