# Operations and Backup

This guide covers running `storkvd`, observing it with `storkv-top`, protecting
data with continuous backup and point-in-time recovery, and moving data with
`storkvctl migrate`.

## Starting the daemon

```powershell
storkvd --data-dir ./data/storkv --listen 127.0.0.1:7878
```

Optional listeners:

```powershell
storkvd `
  --data-dir ./data/storkv `
  --listen 127.0.0.1:7878 `
  --admin-sock 127.0.0.1:7879 `
  --http-listen 127.0.0.1:8080 `
  --ws-listen 127.0.0.1:8081 `
  --grpc-listen 127.0.0.1:9090 `
  --redis-listen 127.0.0.1:6379 `
  --memcached-listen 127.0.0.1:11211 `
  --strict-admin
```

`--admin-sock` accepts either `host:port` or a Unix socket path. A Unix socket
path is trusted for admin operations; TCP admin listeners should be protected
by network policy, `--strict-admin`, or `--admin-token`.

See [TRANSPORTS_AND_COMPATIBILITY.md](TRANSPORTS_AND_COMPATIBILITY.md) for
transport-specific details.

## Health and inspection

```powershell
storkvctl health --sock 127.0.0.1:7878
storkvctl doctor --data-dir ./data/storkv
```

`health` accepts any supported URI, so the same command works over HTTP,
WebSocket, and gRPC:

```powershell
storkvctl health --sock http://127.0.0.1:8080
storkvctl health --sock ws://127.0.0.1:8081
```

`doctor` is offline: stop the daemon or point it at a copied data directory. It
verifies SSTables and MANIFEST checksums and reports the exact failing path.

## Continuous backup

Enable autonomous backups with a local sink:

```powershell
storkvd `
  --data-dir ./data/storkv `
  --backup-dir ./backups `
  --backup-interval-secs 3600 `
  --backup-retention 7 `
  --backup-ship-wal
```

Behavior:

- **Full generation** every `--backup-interval-secs` seconds. A full backup is a
  zero-copy MVCC checkpoint, not a row-by-row dump.
- **Sealed WAL shipping** when `--backup-ship-wal` is set. Each rotated WAL
  segment is copied incrementally to the backup tree, bounding recovery point
  objective to the WAL rotation interval.
- **Retention** keeps the newest `--backup-retention` full generations and
  prunes WAL segments older than the retention window (`max_wal_age`, default
  7 days).
- **Manifests** record generation id, sequence range, creation time, file list,
  byte sizes, and CRC32C digests.

### Inspecting backup state

```powershell
storkvctl backup-status --sock 127.0.0.1:7878
```

The report includes enabled state, backup root, sink kind, completed and failed
backup counts, last backup sequence and timestamp, shipped WAL segment count,
next scheduled full backup, and the last error if any.

### Verifying backups

```powershell
storkvctl backup-verify --sock 127.0.0.1:7878
storkvctl backup-verify --sock 127.0.0.1:7878 --target-seq 18422
```

Verification reads every reachable manifest and checks all referenced SSTable,
MANIFEST, and WAL checksums. A non-zero exit status means at least one artifact
is missing or corrupt. Corrupted generations are never selected for restore.

### Triggering an ad-hoc backup

Use the admin HTTP endpoint or a protocol client to call `TriggerBackup`. The
daemon writes a new generation immediately without waiting for the interval.
`storkvctl backup --sock ... --out ...` remains available for a one-shot
checkpoint outside the managed backup tree.

## Point-in-time recovery

Restore offline into an empty directory:

```powershell
storkvctl restore `
  --backup-dir ./backups `
  --out-dir ./restore/storkv `
  --target-seq 18422
```

- Without `--target-seq`, the newest valid full generation is restored.
- With `--target-seq`, the newest full generation whose sequence is less than or
  equal to the target is restored, then archived WAL segments are replayed up
  to the target sequence.
- The destination must be empty or absent. Existing files cause a
  `RestoreTargetNotEmpty` error instead of being overwritten.
- The command prints the restored generation, final sequence, replayed WAL
  segment count, bytes restored, and any warnings.

After restoring, validate before switching traffic:

```powershell
storkvctl doctor --data-dir ./restore/storkv
```

Start `storkvd` against the restored directory only after verification.

## Pipe sinks (S3, rclone, NAS, SSH)

A pipe sink streams a compressed archive to an external program's stdin. This
avoids embedding cloud SDKs in the daemon:

```powershell
storkvd `
  --data-dir ./data/storkv `
  --backup-dir s3://my-bucket/storkv `
  --backup-pipe-cmd rclone `
  --backup-pipe-arg copy `
  --backup-pipe-arg - `
  --backup-pipe-arg s3:my-bucket/storkv `
  --backup-compression zstd `
  --backup-ship-wal
```

The command is started once per backup artifact and receives the archive on
stdin. `--backup-compression` accepts `none`, `zstd`, or `lz4`. Because the
remote object store owns lifecycle and retention, pipe-sink pruning reports
`RemoteManaged` and leaves deletion to bucket lifecycle rules. Keep local
retention configured for any filesystem `--backup-dir` you also maintain.

## Migration

`storkvctl migrate` exports and imports JSONL and copies Redis databases.

### Export and import

```powershell
# Export every key in "default" to JSONL
storkvctl migrate --sock 127.0.0.1:7878 --scope default --export-jsonl ./default.jsonl

# Import into another scope on a running daemon
storkvctl migrate --sock 127.0.0.1:7878 --import-jsonl ./default.jsonl --into-scope imported

# Import into an offline data directory
storkvctl migrate --data-dir ./data/storkv --import-jsonl ./default.jsonl --into-scope imported
```

Each JSONL line is:

```json
{ "key_b64": "...", "value_b64": "...", "expires_at_ms": 0 }
```

`expires_at_ms` is an absolute Unix millisecond timestamp; `0` means permanent.
Exports read exact TTL metadata, and online imports convert the absolute expiry
back into a remaining TTL at write time.

Downtime migration path:

1. Stop writers.
2. Export with `--sock` (or read from an existing data directory).
3. Import with `--data-dir` into the new directory.
4. Run `storkvctl doctor` and start the new daemon.

### From Redis

```powershell
storkvctl migrate `
  --from-redis redis://127.0.0.1:6379 `
  --sock 127.0.0.1:7878 `
  --into-scope redis_import
```

The migrator scans Redis with `SCAN`, reads values with `GET`, reads remaining
TTLs with `PTTL`, and writes into the destination scope. Expiring Redis keys
keep an equivalent remaining TTL. Stop writes to Redis during the copy for a
consistent snapshot.

## Observability with `storkv-top`

`storkv-top` polls a running daemon and renders live scope, WAL, compaction,
latency, connection, and backup panels. Point it at any supported URI:

```powershell
storkv-top --sock 127.0.0.1:7878
storkv-top --sock http://127.0.0.1:8080
```

The backup panel shows whether autonomous backup is enabled, the sink and root,
last backup sequence and age, completed and failed counts, and shipped WAL
segments. Use it alongside `storkvctl backup-status` when diagnosing a stalled
or failing backup schedule.

## Routine checklist

| Cadence         | Action                                                                  |
| :-------------- | :---------------------------------------------------------------------- |
| Continuous      | Run `storkvd` with `--backup-dir` and `--backup-ship-wal`               |
| Hourly / daily  | Check `storkvctl backup-status` and `storkv-top` backup panel           |
| Daily           | Run `storkvctl backup-verify` and alert on non-zero exit                |
| Weekly          | Restore the latest generation into a scratch directory and run `doctor` |
| Before upgrades | Trigger a full backup and record the sequence number                    |
| After restore   | Run `doctor`, confirm sequence, then start the daemon                   |
