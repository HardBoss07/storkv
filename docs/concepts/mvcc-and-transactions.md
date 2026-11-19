# MVCC and transactions

StorKV uses multi-version concurrency control (MVCC) with snapshot isolation. It uses optimistic concurrency control (OCC) for write conflicts.

Reads see a stable snapshot. Writes buffer in memory. A commit validates the read set before it appends to the WAL.

## Sequence numbers

Every database has one atomic sequence counter. Each operation in a committed batch receives a unique sequence number.

A transaction captures a read sequence when it starts. Reads use the newest version at or below that sequence.

- `db.begin_tx()` captures the current visible sequence.
- `db.begin_tx_at(seq)` uses a caller-provided sequence.
- A direct `db.get()` uses the current visible sequence.

Sequence numbers increase in commit order.

## Snapshot isolation

A transaction sees these writes:

- Every committed write at or below its read sequence.
- Its own buffered writes.

A transaction does not see a write that commits after it starts.

Outside readers do not see a transaction write before commit.

This example shows read-your-own-writes:

```rust
let mut tx = db.begin_tx();
tx.put(&scope, b"key", b"new")?;
assert_eq!(tx.get(&scope, b"key")?.as_deref(), Some(&b"new"[..]));
assert_eq!(db.get(&scope, b"key")?.as_deref(), Some(&b"old"[..]));
tx.commit()?;
```

## OCC read sets

A transaction records every key that it reads.

- `tx.get` adds the key to the read set.
- `tx.record_read` adds a key without a value read.
- `tx.commit` checks every read key.

The commit acquires the single-writer commit lock. It then checks whether a read key has a newer committed version than the transaction read sequence.

If a key changed, the commit returns `StorkvError::WriteConflict` with the scope ID and key. The transaction writes nothing to the WAL.

Blind writes do not add keys to the read set. A put, delete, or merge without a prior read needs no conflict check. Blind writes serialize in commit order.

## When a conflict occurs

Two transactions read the same key:

```text
Time  Transaction A                  Transaction B
1     begin at seq 10               begin at seq 10
2     read balance = 100            read balance = 100
3     write balance = 150
4     commit at seq 11              ...
5                                  write balance = 200
6                                  commit: WriteConflict
```

Transaction B read sequence 10. Transaction A committed a newer version at sequence 11. The B commit fails.

The database still holds the value from A. Transaction B must start again and read the new value.

## Retry a conflicted transaction

The engine does not retry a conflict for you. Run the whole read-modify-write unit again.

This loop is the canonical pattern:

```rust
use storkv_core::{Db, ScopeHandle, StorkvError};

fn increment(db: &Db, scope: &ScopeHandle, key: &[u8]) -> storkv_core::Result<u64> {
    loop {
        let mut tx = db.begin_tx();
        let current = tx
            .get(scope, key)?
            .and_then(|value| std::str::from_utf8(&value).ok()?.parse::<u64>().ok())
            .unwrap_or(0);
        let next = current + 1;
        tx.put(scope, key, next.to_string().as_bytes())?;
        match tx.commit() {
            Ok(()) => return Ok(next),
            Err(StorkvError::WriteConflict { .. }) => continue,
            Err(error) => return Err(error),
        }
    }
}
```

Keep the retry unit small. Do not perform external side effects before the commit succeeds.

Add a retry limit in production code. A limit prevents a live-lock under very high contention.

## Read-only transactions

A transaction with no writes returns from `commit` without a WAL append. The engine still releases the read snapshot.

Use a read-only transaction when you need several keys from one point in time:

```rust
let mut tx = db.begin_tx();
let account = tx.get(&accounts, b"user:42")?;
let session = tx.get(&sessions, b"sess:abc")?;
assert!(account.is_some() || session.is_some());
```

The read sequence holds the compaction floor until the transaction drops or commits.

## Range deletes in transactions

`tx.delete_range(start, end)` records a range tombstone.

- `start` is inclusive.
- `end` is exclusive.
- A later put inside the range with a higher sequence is visible.
- A read at the transaction snapshot hides keys in the range.

## Merge operators

A merge appends an operand without a prior read. Reads resolve the chain of operands against the newest base value.

```rust
use storkv_core::MergeOp;

db.put(&scope, b"page_views", b"10")?;
db.merge(&scope, b"page_views", MergeOp::CounterAdd(5))?;
assert_eq!(db.get(&scope, b"page_views")?.as_deref(), Some(&b"15"[..]));
```

`MergeOp::CounterAdd` stores the decimal delta as text. The default merge operator parses the base value as an integer.

A custom merge receives the operand bytes. The default operator returns the operand when it cannot apply an integer add.

## Isolation limits

StorKV provides snapshot isolation, not serializable isolation.

- Write skew can occur when two transactions read different keys and write disjoint keys.
- A read set protects only keys that the transaction reads.
- A range scan does not add every key in the range to the read set.

Add explicit `record_read` calls for keys that guard a range invariant.

## Related guides

- [Scopes](scopes.md)
- [Watches and TTL](watches-and-ttl.md)
- [Rust client](../guides/rust-client.md)
- [Error codes](../reference/error-codes.md)
