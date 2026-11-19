# Troubleshooting

This page lists common StorKV failures and the checks for each one. Start with the health check and the daemon output.

## First checks

Run these commands in order:

```bash
./target/release/storkvctl health --sock 127.0.0.1:7878
curl -sS http://127.0.0.1:8080/v1/health
./target/release/storkvctl doctor --data-dir ./data/storkv
```

`storkvctl doctor` reads the data directory directly. Stop the daemon before you run it.

`storkvctl` prints each runtime error to standard error. The process exits with code 1 for an integrity failure, a backup verification failure, or a migrate flag error.

## The daemon does not start

### Database is locked

```text
Database is locked by another process: ./data/storkv/LOCK
```

One process owns the data directory. Stop the other `storkvd` or embedded process. Then start the daemon again.

Check for a stale process:

```bash
# Linux and macOS
lsof ./data/storkv/LOCK

# Windows PowerShell
Get-Process storkvd
```

The lock uses an operating system file lock. The file can remain after a crash, but the lock itself is free. A new start then succeeds.

### Address already in use

```text
Error: Address already in use (os error 98)
```

Another process holds the listen address. Select a different port or stop the other process.

```bash
./target/release/storkvd --listen 127.0.0.1:7979
```

### Invalid backup compression

```text
unsupported backup compression `gzip` (expected none, zstd, or lz4)
```

Use `none`, `zstd`, or `lz4` for `--backup-compression`. The value is case-insensitive.

### Admin socket path is in use

On Unix, `storkvd` removes a file at the admin socket path before the bind. A directory at that path stops startup. Do not place another file at that path.

## Scope errors

The error text names the missing scope.

| Message                        | Cause                            | Fix                                  |
| :----------------------------- | :------------------------------- | :----------------------------------- |
| `Scope not found: <name>`      | The name is not registered       | Create the scope or correct the name |
| `Scope id not found: <id>`     | The numeric ID is not registered | Resolve the name first               |
| `Scope already exists: <name>` | A create used a live name        | Use the existing scope               |
| `Scope has been dropped: <id>` | A drop raced with a request      | Reopen the scope and retry           |

The `default` scope is created on first open. Redis and Memcached adapters create their scope on first use. A custom scope needs an explicit create.

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/scopes \
  -H 'content-type: application/json' \
  -d '{"name":"sessions"}'
```

## HTTP request errors

Every REST error has this shape:

```json
{
  "error": { "code": "scope_not_found", "message": "scope not found: sessions" }
}
```

| Status | Code                   | Usual cause                                |
| :----- | :--------------------- | :----------------------------------------- |
| 400    | `invalid_body`         | Missing field or bad base64                |
| 400    | `invalid_key`          | Bad base64 in `key_b64`                    |
| 403    | `unauthorized`         | Strict admin mode rejected the request     |
| 404    | `scope_not_found`      | Scope name is not registered               |
| 404    | `not_found`            | The key has no live value                  |
| 409    | `write_conflict`       | Transaction read set changed before commit |
| 409    | `scope_already_exists` | Duplicate scope name                       |
| 503    | `database_locked`      | Another process holds the directory        |
| 500    | `corruption` or `io`   | Storage or operating system failure        |

### A 404 can mean two things

A missing key returns `not_found`. A missing scope returns `scope_not_found`. Read the `code` field, not only the status.

A missing key is not an error for a scan. A scan returns an empty `pairs` array.

### Binary keys and values

The key path segment is text. StorKV uses the path bytes as the key. Use `key_b64` for binary keys:

```bash
curl -sS \
  "http://127.0.0.1:8080/v1/scopes/default/keys/x?key_b64=$(printf 'user:42' | base64)"
```

Use `value_b64` for binary values, or send the raw bytes as the request body. A JSON body with `value_b64` is also accepted.

### A key read returns binary data

`GET /v1/scopes/{scope}/keys/{key}` returns the raw value bytes by default. Add `?format=json` to receive an envelope:

```bash
curl -sS "http://127.0.0.1:8080/v1/scopes/default/keys/greeting?format=json"
```

### Admin requests return 403

With `--strict-admin`, an admin request needs an admin listener, an admin flag, or the token. Present the token on HTTP:

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/admin/compact \
  -H "x-storkv-admin-token: $STORKV_ADMIN_TOKEN"
```

The native listener has no token field. Use `--admin-sock` for native admin access. Read [Configuration](configuration.md#protect-auxiliary-transports) for the full rule set.

## WebSocket problems

### The server closes the connection

A single binary frame must hold complete `STORKPRT` messages. A partial message or an unknown version produces a protocol error and can close the connection.

### No watch events arrive

The watch acknowledgement is a text frame when you subscribe with a text frame. Later events arrive as binary `STORKPRT` frames, not text. Decode binary frames after the acknowledgement. Read [Watches and TTL](../concepts/watches-and-ttl.md#subscribe-over-websocket).

### `wss://` does not connect

The WebSocket listener has no TLS layer. Put a TLS proxy in front of `--ws-listen` and connect through the proxy.

## Backup and restore problems

### Backup subcommands fail with "not running"

`storkvctl backup`, `backup-status`, and `backup-verify` need a running daemon. `restore` and `doctor` work offline.

### Backup status shows `enabled: false`

The daemon started without `--backup-dir`. Restart it with the flag:

```bash
./target/release/storkvd --data-dir ./data/storkv --backup-dir ./backups
```

### A backup attempt failed

Read the last error:

```bash
./target/release/storkvctl backup-status --sock 127.0.0.1:7878
```

```text
Last error: Backup sink `command` failed: exit status 1
```

Check free space, write permission on the backup root, and the pipe command exit status. A failed attempt increments `backups_failed_total` and keeps the last good generation.

### Verify reports missing files

```text
Backup verification failed (2 errors):
 - full-.../BACKUP_MANIFEST.json: checksum mismatch
```

Run the verify command again after you repair the backup root. A damaged generation cannot serve a restore. Keep an intact generation and remove the damaged one only after you take a new backup.

### The requested sequence is outside the archived range

```text
Requested sequence 18422 is outside the archived range [19000, 21000]
```

The target is older than the oldest retained generation. Select a sequence inside the range, or keep more generations with `--backup-retention` and `--backup-ship-wal`.

### Restore target directory is not empty

```text
Restore target directory is not empty: ./restore/storkv
```

`storkvctl restore` needs an empty output directory. Select a new path:

```bash
./target/release/storkvctl restore \
  --backup-dir ./backups \
  --out-dir ./restore/storkv-2 \
  --target-seq 18422
```

## Integrity failures

`storkvctl doctor` prints a report and exits with code 1 on an error:

```text
Doctor Report: 12 SSTables verified, 3 Manifests verified.
Integrity errors detected (1):
 - SSTable `scopes/0/000042.sst` failed verification: block checksum mismatch
```

A checksum failure means that stored bytes changed after the write. Do not start the daemon on that directory. Restore a backup generation to a new directory.

A corrupted WAL frame in the middle of a segment also stops startup. StorKV does not skip the frame. A torn final frame is safe; startup truncates it.

## Write conflicts

```text
Write conflict detected during OCC commit validation: key `[117, 115, 101, 114]` in scope `0`
```

A transaction read key changed before the commit. Retry the transaction from `begin_tx` with fresh reads. A conflict is normal under contention, not a storage failure.

Read [MVCC and transactions](../concepts/mvcc-and-transactions.md) for the validation rules.

## Slow writes and large storage use

| Symptom                  | Check                                       | Action                                      |
| :----------------------- | :------------------------------------------ | :------------------------------------------ |
| High write latency p99   | `fsync_latency_p99_us` in `/v1/admin/stats` | Use `Buffered` durability for tolerant data |
| Many L0 files            | `levels` in `/v1/admin/stats`               | Run compaction manually                     |
| Large disk use after TTL | `sst_files` per scope                       | Use FIFO retention and run compaction       |
| Frequent conflicts       | `write_conflicts_total`                     | Reduce transaction contention or retry      |
| Slow scans               | `sst_files` and level sizes                 | Compact and narrow the scan range           |

StorKV has no background compaction. Expired keys and old versions stay on disk until a compact call or a memtable flush rewrites them.

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/admin/compact
```

Read [Compaction and tuning](compaction-and-tuning.md) for the thresholds.

## Stale dashboard values

`storkv-top` keeps the last good snapshot when a poll fails. If the values freeze:

1. Confirm that the daemon is running.
2. Confirm that the `--sock` address matches an active listener.
3. Restart `storkv-top`.

A value of `n/a` for the fsync p99 means that the latency window is empty. Send some writes, then wait for the next poll.

## Collect a diagnosis bundle

Run these commands and keep the output:

```bash
./target/release/storkvctl health --sock 127.0.0.1:7878
./target/release/storkvctl backup-status --sock 127.0.0.1:7878
curl -sS http://127.0.0.1:8080/v1/telemetry
curl -sS http://127.0.0.1:8080/v1/admin/stats
./target/release/storkvctl doctor --data-dir ./data/storkv
```

Do not copy live SSTables or the WAL while the daemon writes. Use a checkpoint for support files:

```bash
./target/release/storkvctl backup --sock 127.0.0.1:7878 --out ./support/cp-01
```

## Related guides

- [Observability](observability.md)
- [Configuration](configuration.md)
- [Backup and restore](backup-and-restore.md)
- [Compaction and tuning](compaction-and-tuning.md)
- [Error codes](../reference/error-codes.md)
