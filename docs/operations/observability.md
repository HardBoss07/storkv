# Observability

StorKV reports live counters and storage statistics from the running process. `storkvd` keeps the values in memory. The values reset when the process restarts.

## Access surfaces

| Surface            | Request                                        | Transport                           |
| :----------------- | :--------------------------------------------- | :---------------------------------- |
| Global counters    | `TelemetryQuery`                               | Native, HTTP, WebSocket text, gRPC  |
| Rich statistics    | `AdminStats`                                   | Native, HTTP, WebSocket text, gRPC  |
| Backup state       | `BackupStatus`                                 | Native, gRPC                        |
| Terminal dashboard | `TelemetryQuery`, `AdminStats`, `BackupStatus` | `storkv-top` over any supported URI |

Use these endpoints when `--http-listen` is active:

```bash
curl -sS http://127.0.0.1:8080/v1/telemetry | jq
curl -sS http://127.0.0.1:8080/v1/admin/stats | jq
```

`AdminStats` is admin-only under `--strict-admin`. Read [Configuration](configuration.md#protect-auxiliary-transports) for the token rules.

The REST listener has no backup status route. Use `storkvctl backup-status`, a native client, or the gRPC `BackupStatus` method. The WebSocket text operations also omit backup status.

> [!NOTE]
> The counters are process counters, not lifetime totals. A restart sets every counter to zero. The backup catalog on disk keeps the durable backup history.

## Global counters

The telemetry response has these fields:

| Field                        | Meaning                                              |
| :--------------------------- | :--------------------------------------------------- |
| `writes_total`               | Committed write operations                           |
| `reads_total`                | Completed read operations                            |
| `write_conflicts_total`      | Transaction commits rejected by OCC validation       |
| `compactions_total`          | Compaction runs                                      |
| `fsyncs_total`               | WAL sync operations                                  |
| `backups_completed_total`    | Full or incremental backups that finished            |
| `backups_failed_total`       | Backup attempts that failed                          |
| `last_backup_timestamp_ms`   | Wall-clock time of the last successful backup, in ms |
| `last_backup_seq`            | Database sequence captured by the last backup        |
| `wal_segments_shipped_total` | Sealed WAL segments sent to the backup sink          |

A failed backup increments `backups_failed_total`. It does not change `last_backup_seq`.

### Interpret the counters

- `fsyncs_total` counts WAL sync calls. A `Buffered` scope can commit many writes per sync.
- `compactions_total` increases only when a caller triggers compaction. StorKV has no background compaction thread.
- `write_conflicts_total` increases only for `TxCommit` validation failures. A failed commit does not apply its writes.

## Rich statistics

`GET /v1/admin/stats` returns one document with these groups:

| Field                   | Type   | Meaning                                            |
| :---------------------- | :----- | :------------------------------------------------- |
| `levels`                | array  | One entry for each level from L0 through L6        |
| `scopes`                | array  | One entry for each live scope                      |
| `fsync_latency_p99_us`  | number | Recent write latency p99 in microseconds           |
| `fsync_latency_samples` | number | Number of latency samples in the window            |
| `transports`            | array  | Connection and request counters for each transport |
| `backups_*`             | number | The same backup counters as the telemetry response |

The `levels` entry has `level`, `file_count`, and `size_bytes`. The `scopes` entry has `scope_id`, `name`, `writes_total`, `reads_total`, and `sst_files`.

```json
{
  "levels": [
    { "level": 0, "file_count": 2, "size_bytes": 10485760 },
    { "level": 1, "file_count": 1, "size_bytes": 41943040 }
  ],
  "scopes": [
    {
      "scope_id": 0,
      "name": "default",
      "writes_total": 120,
      "reads_total": 310,
      "sst_files": 3
    }
  ],
  "fsync_latency_p99_us": 840,
  "fsync_latency_samples": 2048,
  "transports": [
    {
      "kind": 0,
      "connections_total": 4,
      "active_connections": 1,
      "requests_total": 2100
    }
  ]
}
```

### Latency sample window

The server keeps the last 2048 write request durations. It computes the p99 over that window. `fsync_latency_samples` reports the current window size.

`fsync_latency_p99_us` is zero when the window is empty. Read the value as the p99 for recent write requests, which include WAL sync time. The field name keeps its historical form.

### Transport identifiers

| `kind` | Transport                |
| :----- | :----------------------- |
| `0`    | Native `STORKPRT`        |
| `1`    | HTTP REST and binary RPC |
| `2`    | WebSocket                |
| `3`    | gRPC                     |
| `4`    | Redis RESP               |
| `5`    | Memcached ASCII          |

A transport entry appears after its first counted event: a connection or an HTTP request.

`connections_total` and `active_connections` are recorded for native, WebSocket, gRPC, Redis, and Memcached connections. The HTTP listener records requests only, so both connection fields stay zero for kind 1.

`requests_total` is recorded for the native, HTTP, gRPC, and Redis paths. WebSocket and Memcached report zero requests in v1.

## Terminal dashboard

`storkv-top` polls the daemon and draws a live dashboard.

```bash
./target/release/storkv-top --sock 127.0.0.1:7878 --interval-ms 1000
```

| Flag            | Default          | Purpose                       |
| :-------------- | :--------------- | :---------------------------- |
| `--sock`        | `127.0.0.1:7878` | Server address                |
| `--interval-ms` | `1000`           | Poll interval in milliseconds |

The dashboard has five panels:

| Panel                    | Content                                                  |
| :----------------------- | :------------------------------------------------------- |
| Write/Read Amplification | Write count, read count, and the write-to-read ratio     |
| L0..L6 SSTable Levels    | File count and size for each level                       |
| WAL / Fsyncs             | Fsync count, write p99, compactions, and write conflicts |
| Backup                   | State, last sequence, counters, sink, and last error     |
| Per-Scope Metrics        | Writes, reads, and SSTable count for each scope          |

Press `q` or `Q` to exit. The dashboard has no other keybindings.

A failed poll keeps the last good snapshot on screen. A broken connection therefore shows stale data, not an error banner. Restart `storkv-top` when the values stop changing.

> [!TIP]
> Use the dashboard for a live view and use `curl` for scripted checks. `--sock` accepts a bare `host:port` or any URI that the client supports, including `http://` and `grpc://`. The dashboard uses the same client as other tools. A native endpoint is not required.

## Health checks

Use `/v1/health` or `storkvctl health` for liveness probes.

```bash
curl -sS http://127.0.0.1:8080/v1/health
```

```json
{ "status": "ok" }
```

```bash
./target/release/storkvctl health --sock 127.0.0.1:7878
```

```text
OK - storkvd is healthy
```

A health success does not prove that the disk is healthy. Run `storkvctl doctor --data-dir <path>` while the daemon is stopped. Read [Troubleshooting](troubleshooting.md) for the diagnosis flow.

## Related guides

- [Configuration](configuration.md)
- [Backup and restore](backup-and-restore.md)
- [Compaction and tuning](compaction-and-tuning.md)
- [HTTP and RPC API](../reference/http-and-rpc-api.md)
