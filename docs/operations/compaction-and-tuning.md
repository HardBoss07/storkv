# Compaction and tuning

StorKV writes immutable SSTables and merges them in compaction. Compaction can run in the foreground or on an optional background thread. You control both paths.

## When compaction runs

Background compaction is off by default. Set `DbOptions::enable_background_compaction` to `true` to start the scheduler. The scheduler wakes after a MemTable flush and compacts at most `max_background_compactions` scopes for each wake. A scope is due when it has at least `l0_compaction_trigger` L0 files. The default trigger is 4 and the default limit is 2. Set `compaction_rate_limit_bytes_per_sec` to pace the new SSTable writes. The scheduler stops on `Db::close` or drop.

You can also run compaction in the foreground through one of these paths:

```rust
db.compact()?;
```

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/admin/compact
```

```bash
grpcurl -plaintext -import-path proto -proto storkv/v1/storkv.proto \
  -d '{}' 127.0.0.1:9090 storkv.v1.StorkvAdmin/Compact
```

One call compacts every scope in the database. It runs in the calling thread and returns after all scopes finish.

Use `Db::compact_range` to compact one scope and one key range:

```rust
db.compact_range("events", Some(b"2026-01"), Some(b"2026-02"))?;
```

The method merges only SSTables that overlap `[start, end)`. Files outside the range stay unchanged. An empty bound means the start or the end of the key space. The method returns `InvalidOperation` when `start >= end` and `ScopeNotFound` when the scope is unknown.

## Flush before compaction

A write lands in a scope memtable. The engine flushes the memtable to an L0 SSTable when either limit is reached:

| Limit               | Default | Meaning                            |
| :------------------ | :------ | :--------------------------------- |
| `max_wal_pin_bytes` | 64 MiB  | Memtable data size                 |
| `max_wal_pin_age`   | 60 s    | Age of the oldest pending memtable |

A memtable that stays small and young stays in memory. Compaction does not see it. A checkpoint or a `compact` call does not flush memtables first. Call `db.flush()` or take a checkpoint when you need every write on disk.

The engine writes each SSTable with LZ4 block compression. A scope can add an 8-byte prefix Bloom filter with `prefix_bloom_len`.

## Leveled compaction

Leveled compaction is the default for `Retention::Permanent` scopes and for TTL scopes with `compaction_style: "leveled"`.

The trigger is four L0 files. One pass does this work:

1. Read every L0 file.
2. Merge entries in key order.
3. Drop expired entries.
4. Drop older versions below the GC-safe sequence.
5. Write one L1 file.
6. Delete the L0 files.

A regular leveled pass writes only to L0 and L1. Levels L2 to L6 exist in the manifest and in the dashboard. Only `compact_range` writes to L2 to L6. The range pass merges the selected files into the highest level it selected.

A scope with one to three L0 files stays unchanged. Run `compact` again after more flushes to reach the trigger.

### Version removal

The merge keeps old versions that a reader can still need. The safe sequence is the minimum of:

- The oldest active transaction read sequence.
- The oldest pinned snapshot sequence.
- The last visible sequence.

A version below that floor can be removed. A long-lived snapshot or transaction holds old versions on disk. Unpin snapshots and commit transactions to let compaction reclaim them.

## FIFO compaction

FIFO compaction is available for TTL scopes with `compaction_style: "fifo"`.

A pass checks every SSTable in the scope. It drops a whole file when the newest expiry in that file is at or before the current time. It does not read or rewrite the file.

FIFO is the right choice for sessions, caches, rate limits, and the Memcached scope. It gives cheap space reclamation and no rewrite cost.

> [!IMPORTANT]
> FIFO drops a file only when every key in it has expired. One live key keeps the whole file. Mix keys with similar lifetimes in one FIFO scope.

## TTL and compaction

Reads filter expired keys at read time. Compaction removes the bytes.

| Compaction style | Expired data removal                |
| :--------------- | :---------------------------------- |
| Leveled          | The merge drops each expired entry  |
| FIFO             | The pass drops a fully expired file |

A key that is only overwritten can stay on disk until a leveled merge removes the old version.

The scope `default_ttl_ms` value does not set an expiry on new writes. Use an explicit TTL on each write.

## Write stalls and backpressure

A large L0 backlog or a large un-flushed WAL slows or stops new writes. These limits control the backpressure:

| Option                       | Default | Effect                                             |
| :--------------------------- | :------ | :------------------------------------------------- |
| `l0_slowdown_writes_trigger` | 8       | Adds a growing delay to each write                 |
| `l0_stop_writes_trigger`     | 12      | Blocks foreground writes until the backlog shrinks |
| `max_total_wal_bytes`        | 256 MiB | Blocks writes until the WAL flush catches up       |

A value of `0` disables that limit. A blocked write sleeps and retries. Set `WriteOptions::non_blocking` to `true` to receive `StorkvError::WriteStalled` instead of blocking. A non-blocking write skips the slowdown delay and returns at once when a stop watermark is exceeded. Use this mode when a caller must not block, such as a network request with a deadline.

## Block cache

The engine keeps one shared block cache for SSTable reads. `DbOptions::block_cache_bytes` sets the total budget. The default is 64 MiB (`DEFAULT_BLOCK_CACHE_BYTES`). A larger cache reduces disk reads for point lookups and scans. The cache holds decoded data blocks and uses the scope ID and file number in its key.

## Storage tuning choices

Most tuning happens when you create a scope. These options are fixed for the life of the scope:

| Option             | Choose it for                                       |
| :----------------- | :-------------------------------------------------- |
| `durability`       | Strict sync for durable data, buffered for caches   |
| `retention`        | Permanent keys or TTL keys                          |
| `compaction_style` | Leveled for general data, FIFO for time-window data |
| `prefix_bloom_len` | Workloads with many prefix scans                    |

To change an option, create a new scope and migrate the data.

## Write amplification

Write amplification is total bytes written to storage divided by bytes written by clients.

It grows with these factors:

- More L0 files merged in one pass.
- More repeated flushes of the same keys.
- Long snapshots that prevent version removal.
- Frequent full checkpoints.

A small number of large batches has lower amplification than many small writes. For bulk load, write large transactions and compact after the load.

## Read amplification

A point read checks the active memtable, the flush queue, then each SSTable from L0 to L6.

It grows with these factors:

- Many L0 files that wait for a merge.
- Many old versions of one key.
- A missing prefix Bloom filter on a prefix-scan workload.

Run compaction after a large delete or a large load. Enable `prefix_bloom_len` when the scope uses prefix scans.

## WAL growth

All scopes share one WAL. An idle scope with an old memtable can pin WAL segments and delay removal. The db-wide limits address this case:

```rust
let options = DbOptions {
    max_wal_pin_bytes: 128 * 1024 * 1024,
    max_wal_pin_age: Duration::from_secs(120),
};
```

The flush scheduler uses these values in the write path.

## Suggested maintenance

| Task                | Suggested timing                   |
| :------------------ | :--------------------------------- |
| `compact`           | After bulk load or bulk delete     |
| `backup-verify`     | Once a day                         |
| `doctor`            | After a restore or an unclean stop |
| Restore drill       | Once a week                        |
| Unpin old snapshots | When a migration is complete       |

## Limits that need a rebuild

These values are compile-time constants:

| Value                 | Constant                |
| :-------------------- | :---------------------- |
| WAL segment size      | `MAX_WAL_SEGMENT_BYTES` |
| WAL seal interval     | `WAL_SEAL_INTERVAL`     |
| L0 compaction trigger | `LEVEL_ZERO_TRIGGER`    |
| Level count           | `LEVEL_COUNT`           |

Version 1 exposes no flag for these values.

## Related guides

- [Architecture](../concepts/architecture.md)
- [Scopes](../concepts/scopes.md)
- [Watches and TTL](../concepts/watches-and-ttl.md)
- [Observability](observability.md)
