# Watches and TTL

StorKV provides change event streams and per-key time to live (TTL). This page explains both contracts.

## Watch events

A watch stream receives one event for each committed mutation:

| Event             | Fields                         | Produced by                      |
| :---------------- | :----------------------------- | :------------------------------- |
| `Put`             | scope ID, key, value, sequence | Put, TTL put, or transaction put |
| `Delete`          | scope ID, key, sequence        | Delete or transaction delete     |
| `DropScope`       | scope ID, sequence             | Scope drop                       |
| `InvalidateScope` | scope ID, sequence             | Range delete or merge            |

A range delete and a merge can change many keys. They emit one `InvalidateScope` event. A client must discard its cached state for that scope. A point read from the engine still applies the range tombstone to each key.

### Subscribe in Rust

Use `Db::watch` to receive events from the current process:

```rust
use std::time::Duration;
use storkv_core::watch::ChangeEvent;

let receiver = db.watch()?;
db.put(&scope, b"user:42", b"active")?;
let event = receiver.recv_timeout(Duration::from_secs(1))?;
match event {
    ChangeEvent::Put { key, value, .. } => {
        assert_eq!(key.as_ref(), b"user:42");
        assert_eq!(value.as_ref(), b"active");
    }
    _ => {}
}
```

`db.watch()` returns a bounded channel. The registry holds one sender for each live receiver.

### Stream limits

The watch registry applies these limits:

| Limit | Value | Behavior when reached |
| :--- | ---: | :--- |
| Queued events per subscriber | 1024 | The registry removes the slow subscriber |
| Concurrent subscribers | 1024 | Subscribe returns `StorkvError::TooManySubscribers` |
| Replayed events per resume | 1024 | Resume returns `StorkvError::ReplayBudgetExceeded` |
| Replayed payload bytes per resume | 16 MiB | Resume returns `StorkvError::ReplayBudgetExceeded` |
| Watchers per WebSocket connection | 16 | The server returns a `TooManySubscribers` wire error |

The registry uses a non-blocking send. A subscriber with a full queue cannot stall a writer. Instead, the registry drops that subscriber, and the receiver reports a disconnect. A slow client must reconnect and resume from its last sequence number.

A new `WatchFrom` request on the same native connection replaces the previous watch task. It does not add a second task.

### Event order

Events arrive in commit order. The sequence number in each event is the operation sequence, not the batch start sequence.

- A batch with three writes emits three events.
- Each event carries its own sequence number.
- A watch event is broadcast after the memtable apply, not before the WAL append.

A watch event does not guarantee that the write is on stable storage in `Buffered` mode. The WAL append has happened, but the fsync may be pending.

### Scope filtering

The native and WebSocket watch paths send every event to every subscriber. They do not filter by scope or key.

The gRPC `Watch` call has `scope_id` and `all_scopes` fields:

- `all_scopes = true` sends every event.
- `all_scopes = false` sends only events for `scope_id`.

Filter on the client when you use the native or WebSocket path:

```rust
while let Some(event) = stream.next().await? {
    if event.scope_id == scope.scope_id {
        handle(event);
    }
}
```

V1 has no prefix filter and no key filter. Filter on the client for those cases.

### Resume a watch stream

A watch cursor lets a client continue after a disconnect without a gap. Pass the sequence number of the last event that the client processed.

In Rust, use `db.watch_scope_from`:

```rust
let last_processed = 41;
let receiver = db.watch_scope_from(&scope, last_processed)?;
```

The engine first replays retained WAL events with `seq > last_processed`, then delivers live events. The commit lock makes the switch atomic, so no event is lost or duplicated in that window.

This rule has a limit. Retained WAL has a start sequence. If `from_seq` is older than that start, the call returns `StorkvError::WatchCursorExpired { requested, oldest_available }`. The client must resubscribe from `oldest_available` or read the data again as a full rescan.

A second limit bounds memory use. A resume request that needs more than 1024 events or 16 MiB of payload fails with `StorkvError::ReplayBudgetExceeded`. Move the cursor forward or start a new stream.

Over the native protocol, send `Request::WatchFrom { from_seq }` (opcode `26`). The `storkv-client` helper is `AsyncClient::subscribe_watch_from(Some(from_seq))`. The gRPC `Watch` call carries the same cursor in `WatchRequest.from_seq`.

Each `WatchEvent` has a `seq` field. Store that value after you process the event. Use it as the next `from_seq`.

### Subscribe over WebSocket

Send a `watch` operation after you connect:

```json
{ "id": 1, "op": "watch" }
```

The server replies:

```json
{ "id": 1, "ok": true, "watching": true }
```

The acknowledgement is a text frame. Later events are binary `STORKPRT` frames. Each event frame has the subscription `id` as its request ID and a `WatchEvent` response payload. Decode the frame with `storkv_proto::Message::decode`.

This rule applies to a text subscription and to a binary subscription. The WebSocket watch path has no text event frames.

Use `ws://` or a raw WebSocket client. The HTTP REST routes do not stream watch events.

## TTL basics

A TTL write stores an absolute expiry time in Unix epoch milliseconds.

- `expires_at_ms = 0` means no expiry.
- `expires_at_ms > 0` means the key expires at that wall-clock time.
- The clock does not depend on process start time.

Set a TTL with the Rust API:

```rust
use std::time::Duration;
db.put_with_ttl(&scope, b"sess:abc", b"user:42", Duration::from_secs(900))?;
```

Set a TTL with REST:

```bash
curl -sS -X PUT \
  --data-binary 'user:42' \
  "http://127.0.0.1:8080/v1/scopes/default/keys/sess:abc?ttl_ms=900000"
```

Read the remaining time in Rust:

```rust
let remaining = db.key_ttl_remaining_ms(&scope, b"sess:abc")?;
assert!(remaining.is_some());
```

## Lazy expiry

Reads apply expiry at read time. A key is invisible when `now_ms >= expires_at_ms`.

This rule applies to these operations:

- Point reads.
- Range scans.
- Prefix scans.
- Transaction reads.
- Key metadata reads.

The engine does not delete a key at the exact expiry moment. It filters the key from results.

A key can remain in the memtable or an SSTable after it expires. The storage cost remains until compaction or flush removes it.

## Compaction-driven expiry

Compaction removes expired data in two ways.

### FIFO compaction

For a scope with `Retention::Ttl` and `CompactionStyle::Fifo`, compaction checks each SSTable. It drops a whole file when the newest expiry in that file is at or before the current time.

Use FIFO for sessions, caches, and rate-limit data. Expiry removes files without rewriting live keys.

### Leveled compaction

For leveled compaction, the merge drops entries whose expiry is at or before the compaction start time. It does not write a tombstone for an expired entry.

### Manual trigger

Run compaction through any of these paths:

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

StorKV does not run background compaction in v1. Expired data stays on disk until you invoke compaction or a memtable flush rewrites it.

## TTL edge cases

| Case                                       | Behavior                                             |
| :----------------------------------------- | :--------------------------------------------------- |
| TTL of zero duration                       | Expiry equals the write time; the key is not visible |
| Negative TTL from Redis                    | `EXPIRE` deletes the key; `SETEX` rejects the value  |
| TTL overwrite without a new TTL            | The new value has no expiry                          |
| Redis `KEEPTTL`                            | The new value keeps the old expiry                   |
| Memcached `exptime` above 30 days          | The adapter treats it as an absolute Unix time       |
| Scope `default_ttl_ms` without a TTL write | The key has no expiry                                |

Use an explicit TTL on each write when you need a lifetime.

## Related guides

- [Scopes](scopes.md)
- [MVCC and transactions](mvcc-and-transactions.md)
- [Compaction and tuning](../operations/compaction-and-tuning.md)
- [Redis compatibility](../guides/compatibility-redis.md)
