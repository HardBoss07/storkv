# Compaction and tuning

StorKV writes immutable SSTables and merges them in compaction. Version 1 has no background compaction thread. You control when compaction runs.

## When compaction runs

Compaction is a manual action through one of these paths:

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

Version 1 writes only to L0 and L1. Levels L2 to L6 exist in the manifest and in the dashboard, but no pass fills them.

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
