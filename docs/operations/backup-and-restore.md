# Backup and restore

StorKV has two backup paths: a point-in-time checkpoint image and an autonomous backup catalog. They serve different purposes.

| Path                  | Command or flag         | Output                                      |
| :-------------------- | :---------------------- | :------------------------------------------ |
| Checkpoint            | `storkvctl backup`      | One directory that holds a database image   |
| Autonomous backup     | `--backup-dir`          | Full generations plus optional WAL segments |
| Manual backup trigger | `POST /v1/admin/backup` | One new full generation in the catalog      |
| Restore               | `storkvctl restore`     | A new data directory from the catalog       |

Only the catalog supports retention, verification, and point-in-time restore.

## Create a checkpoint

```bash
./target/release/storkvctl backup \
  --sock 127.0.0.1:7878 \
  --out ./backups/snap-01
```

The command prints:

```text
Backup successfully created at ./backups/snap-01
```

A checkpoint does these steps:

1. It flushes every active memtable to an L0 SSTable.
2. It hard-links every live SSTable into `scopes/<scope-id>/` under the output.
3. It copies `CURRENT`, the active manifest, and every WAL segment.
4. It syncs the output directory.

The hard links share file data with the live database, so the copy uses little extra space. A later compaction removes the live file name, but the checkpoint keeps its link.

A checkpoint is a self-contained data directory. Start `storkvd` with `--data-dir` set to the checkpoint to open it.

> [!NOTE]
> A checkpoint has no backup manifest, no retention policy, and no sequence filter. Use an autonomous backup when you need those features.

## Enable autonomous backup

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --backup-dir ./backups \
  --backup-interval-secs 3600 \
  --backup-retention 7 \
  --backup-ship-wal
```

The worker takes a full backup at startup, then one every interval. With `--backup-ship-wal`, it also copies each sealed WAL segment between full backups.

A full backup runs these steps:

1. It takes a checkpoint into `<backup-dir>/.tmp/<generation-id>/`.
2. It lists every file with a size and CRC32C checksum.
3. It writes `BACKUP_MANIFEST.json` and `BACKUP_MANIFEST.json.crc32c`.
4. It publishes the generation into `<backup-dir>/generations/`.
5. It applies the retention plan.

### Directory layout

```text
backups/
├── generations/
│   ├── full-1763400000000-00000000000000001234/
│   │   ├── CURRENT
│   │   ├── MANIFEST-000001
│   │   ├── BACKUP_MANIFEST.json
│   │   ├── BACKUP_MANIFEST.json.crc32c
│   │   ├── scopes/
│   │   └── wal/
│   └── full-...
├── wal/
│   ├── 00000000000000000001.wal
│   └── ...
└── .tmp/
```

A generation ID has the form `full-<unix-ms>-<sequence>`. A WAL generation ID has the form `wal-<segment-number>`.

### WAL shipping

Sealed WAL segments go to `<backup-dir>/wal/`. The shipper starts with the current segment number and copies later sealed segments.

A segment is sealed when it reaches 32 MiB or is 300 seconds old. The current segment is never shipped.

### Retention

The default retention keeps seven full generations and WAL segments for seven days.

A WAL segment is removable only when all of these conditions hold:

- The oldest retained full generation covers its sequence range.
- It is older than `max_wal_age`.
- No retained generation lists it as a file.

The retention pass never removes a file that a restore can still need.

## Inspect backup status

```bash
./target/release/storkvctl backup-status --sock 127.0.0.1:7878
```

Example output:

```text
Backup enabled: true
Backup root: ./backups
Sink: local
Completed backups: 4
Failed backups: 0
Last backup sequence: 1288
Last backup timestamp (ms): 1763403600000
Shipped WAL segments: 9
Next full backup in (ms): 2411220
```

The command also prints `Last error: <text>` when the last run failed. The status comes from the live worker, so it is available only when `--backup-dir` is set.

The same data is in `GET /v1/admin/stats` and in the `Backup` panel of `storkv-top`.

## Verify a backup

```bash
./target/release/storkvctl backup-verify --sock 127.0.0.1:7878
```

Example output:

```text
Checked 4 generations, 212 files, 845332480 bytes, 9 WAL segments.
All backup checks PASSED.
```

Verification checks these items:

- The manifest format marker and version.
- The sidecar checksum file.
- Each file size and CRC32C value.
- Each SSTable footer, index, and block checksum.
- Each WAL header and frame.
- The `CURRENT` pointer content.

The command exits with status 1 and lists each failure when a check fails.

To verify a specific point in time:

```bash
./target/release/storkvctl backup-verify \
  --sock 127.0.0.1:7878 \
  --target-seq 1288
```

The command rejects a sequence outside the archived range with `TargetSequenceUnavailable`.

> [!IMPORTANT]
> `backup-verify` reads the backup root that the server reports. It does not verify a checkpoint directory made by `storkvctl backup`.

## Trigger a backup now

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/admin/backup \
  -H 'content-type: application/json' \
  -d '{"wait":true}'
```

A `wait` value of true returns the report after the backup finishes. A false value wakes the worker and returns an empty report.

This call needs an admin connection when `--strict-admin` is set.

## Restore

Stop the server before you restore. A restore needs exclusive access to the target directory.

```bash
./target/release/storkvctl restore \
  --backup-dir ./backups \
  --out-dir ./restore/storkv \
  --target-seq 1288
```

Example output:

```text
Restored generation full-1763400000000-00000000000000001234 to ./restore/storkv (sequence 1288, 2 WAL segments replayed, 845332480 bytes).
```

Without `--target-seq`, the command restores the newest archived sequence.

Restore runs these steps:

1. It loads the generation catalog.
2. It selects the newest full generation whose `max_seq` is at or below the target.
3. It verifies that generation.
4. It copies the generation into a staging directory next to the target.
5. It replays matching WAL segments and keeps only frames at or below the target.
6. It renames the staging directory to the target.
7. It opens the result once to validate it.

The target directory must be absent or empty. A non-empty target returns `RestoreTargetNotEmpty`.

Restore prints each non-fatal warning on stderr.

### Start the restored database

```bash
./target/release/storkvd \
  --data-dir ./restore/storkv \
  --listen 127.0.0.1:7878
```

Run an integrity check first:

```bash
./target/release/storkvctl doctor --data-dir ./restore/storkv
```

## Restore errors

| Message                       | Cause                                       |
| :---------------------------- | :------------------------------------------ |
| `Backup generation not found` | The backup root has no generations          |
| `TargetSequenceUnavailable`   | The sequence is outside the archive range   |
| `BackupCorruption`            | Verification failed for the base generation |
| `RestoreTargetNotEmpty`       | The output directory holds files            |

Read [Error codes](../reference/error-codes.md) for the full list.

## Recommended schedule

1. Run a full backup every hour.
2. Ship sealed WAL segments.
3. Keep at least seven full generations.
4. Run `backup-verify` once a day.
5. Restore the newest generation into a test directory once a week.
6. Run `doctor` on each restored directory.

Test a restore before you need it. A backup that has never been restored is not proven.

## Related guides

- [Configuration](configuration.md)
- [Observability](observability.md)
- [Compaction and tuning](compaction-and-tuning.md)
- [Troubleshooting](troubleshooting.md)
