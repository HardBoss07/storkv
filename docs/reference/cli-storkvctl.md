# storkvctl reference

`storkvctl` is the operational command line tool for StorKV. It runs health checks, offline integrity checks, backups, restore operations, data migration, and a YCSB-style benchmark.

```bash
storkvctl <subcommand> [flags]
```

The tool reads no environment variables. It takes every value from a flag.

## Subcommands

| Subcommand      | Connects to         | Purpose                                        |
| :-------------- | :------------------ | :--------------------------------------------- |
| `health`        | Running daemon      | Check daemon liveness                          |
| `doctor`        | Data directory      | Verify the manifest and every SSTable offline  |
| `backup`        | Running daemon      | Write a checkpoint image                       |
| `backup-status` | Running daemon      | Show autonomous backup state                   |
| `backup-verify` | Running daemon      | Verify archived backup generations             |
| `restore`       | Backup directory    | Restore a generation into a new data directory |
| `migrate`       | Daemon or directory | Export, import, or copy keys                   |
| `ycsb`          | Running daemon      | Run a benchmark workload                       |

`storkvctl` has no key read, key write, scan, or TTL subcommand. Use the [HTTP API](http-and-rpc-api.md), the Rust client, or a compatibility front end for data operations.

## Exit codes and output

| Condition                              | Stream | Exit code |
| :------------------------------------- | :----- | :-------- |
| Success                                | stdout | 0         |
| Connection or protocol error           | stderr | 1         |
| `doctor` integrity error               | stderr | 1         |
| `backup-verify` verification error     | stderr | 1         |
| Missing or conflicting `migrate` flags | stderr | 1         |
| Invalid subcommand or flag             | stderr | 2         |

A connection error shows the client error text. The tool does not retry.

## health

Check a native listener:

```bash
storkvctl health --sock 127.0.0.1:7878
```

```text
OK - storkvd is healthy
```

| Flag     | Default          | Meaning                         |
| :------- | :--------------- | :------------------------------ |
| `--sock` | `127.0.0.1:7878` | Any URI that the client accepts |

The command sends a health request over the client. A successful command prints the line above. Use this command for a container or service health probe.

## doctor

Verify stored files without a running daemon:

```bash
storkvctl doctor --data-dir ./data/storkv
```

```text
Doctor Report: 12 SSTables verified, 1 Manifests verified.
All integrity checks PASSED.
```

| Flag         | Default         | Meaning                       |
| :----------- | :-------------- | :---------------------------- |
| `--data-dir` | `./data/storkv` | Database directory to inspect |

The check does these steps:

1. Read `CURRENT`.
2. Open the active manifest.
3. Check every `.sst` file under `scopes/<scope-id>/`.
4. Check the SSTable footer, index blocks, Bloom filter, and block checksums.

On a failure, the tool prints each error and exits with code 1:

```text
Doctor Report: 12 SSTables verified, 1 Manifests verified.
Integrity errors detected (1):
 - SSTable `scopes/0/000042.sst` failed verification: block checksum mismatch
```

`Manifests verified` counts the active manifest only. Historical manifest files are not opened.

> [!WARNING]
> Stop the daemon before you run `doctor`. The check reads live files directly and does not take the directory lock.

## backup

Write a checkpoint image through a running daemon:

```bash
storkvctl backup --sock 127.0.0.1:7878 --out ./backups/snap-01
```

```text
Backup successfully created at ./backups/snap-01
```

| Flag     | Default          | Meaning                        |
| :------- | :--------------- | :----------------------------- |
| `--sock` | `127.0.0.1:7878` | Daemon address                 |
| `--out`  | required         | Output directory for the image |

The output is a complete data directory with hard-linked SSTables. It has no catalog, retention policy, or sequence filter. Under `--strict-admin`, the request needs admin access.

Read [Backup and restore](../operations/backup-and-restore.md) for the file layout and guarantees.

## backup-status

Show the autonomous backup worker state:

```bash
storkvctl backup-status --sock 127.0.0.1:7878
```

```text
Backup enabled: true
Backup root: ./backups
Sink: local
Completed backups: 4
Failed backups: 0
Last backup sequence: 18422
Last backup timestamp (ms): 1763403600000
Shipped WAL segments: 12
Next full backup in (ms): 2400000
```

| Flag     | Default          | Meaning        |
| :------- | :--------------- | :------------- |
| `--sock` | `127.0.0.1:7878` | Daemon address |

The command prints `Last error: <text>` when the worker has a recorded failure. `Backup enabled: false` means that the daemon started without `--backup-dir`.

## backup-verify

Verify archived generations:

```bash
storkvctl backup-verify --sock 127.0.0.1:7878
storkvctl backup-verify --sock 127.0.0.1:7878 --target-seq 18422
```

```text
Checked 3 generations, 84 files, 734003200 bytes, 12 WAL segments.
All backup checks PASSED.
```

| Flag           | Default          | Meaning                                       |
| :------------- | :--------------- | :-------------------------------------------- |
| `--sock`       | `127.0.0.1:7878` | Daemon address                                |
| `--target-seq` | none             | Verify only the generations for this sequence |

The check reads the backup catalog through the daemon and validates file sizes and CRC32C checksums. On an error, it prints each path and detail, then exits with code 1.

## restore

Restore a catalog backup without a daemon:

```bash
storkvctl restore \
  --backup-dir ./backups \
  --out-dir ./restore/storkv \
  --target-seq 18422
```

```text
Restored generation full-1763400000000-00000000000000001234 to ./restore/storkv (sequence 18422, 12 WAL segments replayed, 734003200 bytes).
```

| Flag           | Default     | Meaning                            |
| :------------- | :---------- | :--------------------------------- |
| `--backup-dir` | `./backups` | Root of the autonomous backup tree |
| `--out-dir`    | required    | New data directory                 |
| `--target-seq` | newest      | Point-in-time sequence to restore  |

The output directory must be empty or absent. The command replays the WAL segments that are needed to reach `--target-seq`. It prints each non-fatal warning to standard error.

Start the daemon with `--data-dir` set to `--out-dir` after the restore.

## migrate

Move keys between StorKV, a JSONL file, or a Redis server.

```bash
storkvctl migrate --sock 127.0.0.1:7878 --export-jsonl ./export.jsonl --scope default
storkvctl migrate --data-dir ./data/storkv --import-jsonl ./export.jsonl --into-scope restored
storkvctl migrate --sock 127.0.0.1:7878 --from-redis redis://127.0.0.1:6379 --into-scope cache
```

| Flag             | Default   | Meaning                                         |
| :--------------- | :-------- | :---------------------------------------------- |
| `--sock`         | none      | Daemon URI for an online operation              |
| `--data-dir`     | none      | Offline data directory for an import            |
| `--export-jsonl` | none      | Write every key in `--scope` to this file       |
| `--import-jsonl` | none      | Import records from this file                   |
| `--from-redis`   | none      | Copy string keys from this Redis server         |
| `--scope`        | `default` | Source scope for export, destination for import |
| `--into-scope`   | `--scope` | Destination scope for an import                 |

Use one action per command: `--export-jsonl`, `--import-jsonl`, or `--from-redis`. The tool applies `--from-redis` first when more than one action is present.

### Online and offline targets

| Action           | `--sock` | `--data-dir` |
| :--------------- | :------- | :----------- |
| `--export-jsonl` | Required | Not used     |
| `--import-jsonl` | Optional | Optional     |
| `--from-redis`   | Optional | Optional     |

An import needs one target. An offline import opens the data directory directly, so the daemon must be stopped. An online import sends one write for each record. When both flags are present, `--data-dir` wins.

### JSONL record shape

Each line has this form:

```json
{ "key_b64": "dXNlcjo0Mg==", "value_b64": "YWN0aXZl", "expires_at_ms": 0 }
```

`key_b64` and `value_b64` use standard base64. `expires_at_ms` is optional and defaults to 0, which means no expiry. A malformed line stops the import with the file name and line number.

An export calls `key_meta` for each key, so it preserves absolute expiry times.

### Redis copy rules

The Redis copy connects to database 0 and runs `SCAN` with `COUNT 1000`. It reads each key with `GET` and each remaining TTL with `PTTL`.

- Only string values are copied. A key of another type causes a Redis error and stops the copy.
- A key with no expiry receives no TTL in StorKV.
- A positive TTL becomes an absolute `expires_at_ms` value.
- A missing destination scope is created with strict durability and permanent retention.

The tool prints the number of imported keys:

```text
Imported 842 keys from redis://127.0.0.1:6379 into scope cache
```

## ycsb

Run a YCSB-style workload against the `default` scope:

```bash
storkvctl ycsb --sock 127.0.0.1:7878 --workload a --records 1000 --ops 5000
```

```text
YCSB Workload `a` finished: 6000 ops in 812 ms (7389.16 ops/sec)
```

| Flag         | Default          | Meaning                          |
| :----------- | :--------------- | :------------------------------- |
| `--sock`     | `127.0.0.1:7878` | Daemon address                   |
| `--workload` | `a`              | Workload name                    |
| `--records`  | `100`            | Records to load before the run   |
| `--ops`      | `100`            | Operations in the measured phase |

The load phase writes keys `user00000000` through `user<records-1>`. The measured phase starts after the load.

| Workload | Operation mix                        |
| :------- | :----------------------------------- |
| `a`      | 50 percent reads, 50 percent updates |
| `b`      | 95 percent reads, 5 percent updates  |
| other    | 100 percent reads                    |

The reported operation count is `--records` plus `--ops`. The rate covers the load and measured phases together.

The benchmark always uses scope ID 0. Do not run it against a production `default` scope.

## Related guides

- [HTTP and RPC API](http-and-rpc-api.md)
- [Error codes](error-codes.md)
- [Backup and restore](../operations/backup-and-restore.md)
- [Migrating data](../guides/migrating-data.md)
- [Troubleshooting](../operations/troubleshooting.md)
