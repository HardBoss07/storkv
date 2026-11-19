# Error codes

StorKV reports errors through typed values in the Rust API and numeric codes on the wire. The REST and WebSocket layers add a text code to each error.

## Wire error codes

The wire protocol encodes one unsigned byte for the error class, then the fields for that class.

| Code | Variant              | Fields                    | REST status | REST code              |
| :--- | :------------------- | :------------------------ | :---------- | :--------------------- |
| 1    | `DatabaseLocked`     | message                   | 503         | `database_locked`      |
| 2    | `Corruption`         | message                   | 500         | `corruption`           |
| 3    | `WriteConflict`      | scope ID, key             | 409         | `write_conflict`       |
| 4    | `ScopeNotFound`      | scope name                | 404         | `scope_not_found`      |
| 5    | `ScopeIdNotFound`    | scope ID                  | 404         | `scope_not_found`      |
| 6    | `ScopeAlreadyExists` | scope name                | 409         | `scope_already_exists` |
| 7    | `ScopeDropped`       | scope ID                  | 400         | `scope_dropped`        |
| 8    | `SnapshotNotFound`   | snapshot name             | 404         | `snapshot_not_found`   |
| 9    | `KeyTooLarge`        | max length, actual length | 400         | `key_too_large`        |
| 10   | `ValueTooLarge`      | max length, actual length | 400         | `value_too_large`      |
| 11   | `InvalidOperation`   | message                   | 400         | `invalid_operation`    |
| 12   | `Io`                 | message                   | 500         | `io`                   |
| 13   | `Protocol`           | message                   | 400         | `protocol`             |
| 14   | `Unauthorized`       | none                      | 403         | `unauthorized`         |
| 15   | `WatchCursorExpired` | requested, oldest available | 410       | `watch_cursor_expired` |
| 16   | `NotPrimary`         | none                      | 503         | `not_primary`          |
| 17   | `StaleReplica`       | lag milliseconds, max lag | 503         | `stale_replica`        |
| 18   | `TooManySubscribers` | count, maximum            | 429         | `too_many_subscribers` |
| 19   | `ReplayBudgetExceeded` | events, bytes           | 400         | `replay_budget_exceeded` |

An unknown code on decode becomes `Protocol("Unknown error code: N")`.

### Recovery action by code

| Code | Retry   | Action                                                    |
| :--- | :------ | :-------------------------------------------------------- |
| 1    | No      | Stop the other process that holds the data directory      |
| 2    | No      | Stop the daemon, run `storkvctl doctor`, restore a backup |
| 3    | Yes     | Begin a new transaction with fresh reads                  |
| 4    | No      | Create the scope or correct the name                      |
| 5    | No      | Resolve the scope name to a live ID                       |
| 6    | No      | Use the existing scope                                    |
| 7    | No      | Reopen the scope and retry                                |
| 8    | No      | Correct the snapshot name                                 |
| 9    | No      | Shorten the key                                           |
| 10   | No      | Shorten the value                                         |
| 11   | No      | Correct the request                                       |
| 12   | Depends | Check disk health and free space, then retry              |
| 13   | No      | Correct the frame or protocol version                     |
| 14   | No      | Use an admin listener or present a valid token            |
| 15   | Depends | Resubscribe from `oldest_available` or rescan the data    |
| 16   | Yes     | Send writes to the primary, or use `Strong` on the primary |
| 17   | Yes     | Retry later, raise `max_lag_ms`, or read from the primary |
| 18   | Yes     | Close an idle stream, then retry after a delay            |
| 19   | No      | Move the cursor forward or start a new stream             |

## REST error shape

```json
{
  "error": { "code": "scope_not_found", "message": "scope not found: sessions" }
}
```

### REST-only codes

These codes come from the HTTP layer before or after dispatch:

| Code              | Status | Cause                                   |
| :---------------- | :----- | :-------------------------------------- |
| `invalid_body`    | 400    | Missing field, bad base64, invalid JSON |
| `invalid_key`     | 400    | Invalid base64 in `key_b64`             |
| `invalid_frame`   | 400    | Body is not a request frame             |
| `truncated_frame` | 400    | Frame is incomplete                     |
| `not_found`       | 404    | Key has no live value                   |

### WebSocket-only codes

| Code           | Cause                                                                  |
| :------------- | :--------------------------------------------------------------------- |
| `invalid_json` | Text frame is not valid JSON, or the operation is unknown or malformed |

A binary frame decode failure returns wire error `Protocol` (code 13). The WebSocket error body has `"ok": false` and the same `error` object.

```json
{
  "id": 7,
  "ok": false,
  "error": { "code": "invalid_json", "message": "malformed JSON request" }
}
```

## Limits

| Limit             | Value        | Error on excess |
| :---------------- | :----------- | :-------------- |
| Key length        | 65,535 bytes | `KeyTooLarge`   |
| Value length      | 256 MiB      | `ValueTooLarge` |
| Protocol frame    | 64 MiB       | `Protocol`      |
| Memcached key     | 250 bytes    | Adapter error   |
| Memcached value   | 1 MiB        | Adapter error   |
| Memcached command | 8 KiB        | Adapter error   |
| Redis bulk value  | 256 MiB      | Protocol error  |
| Redis inline line | 64 KiB       | Protocol error  |
| Redis request buffer | about 256 MiB | Protocol error |
| Redis glob pattern | 1,024 bytes | Adapter error   |
| Watch queue       | 1024 events  | Subscriber removed |
| Watch subscribers | 1024         | `TooManySubscribers` |
| Watch replay      | 1024 events or 16 MiB | `ReplayBudgetExceeded` |
| Watchers per WebSocket | 16      | `TooManySubscribers` |
| Replication queue | 1024 frames  | Subscriber removed |
| Replication replay | 1024 frames or 16 MiB | `ReplayBudgetExceeded` |

The Memcached limits come from the protocol. A native write can exceed the Memcached key limit. A network write cannot exceed the 64 MiB frame limit, so the 256 MiB value limit applies to the embedded engine.

## Core errors that become code 11

The engine has more error types than the wire protocol. These core errors have no dedicated wire code and become `InvalidOperation` (code 11) with the original message:

| Core error                  | Message form                                                |
| :-------------------------- | :---------------------------------------------------------- |
| `BackupSink`                | `Backup sink `<name>` failed: <detail>`                     |
| `BackupGenerationNotFound`  | `Backup generation not found: <id>`                         |
| `BackupCorruption`          | `Backup corruption at `<path>`: <detail>`                   |
| `TargetSequenceUnavailable` | `Requested sequence N is outside the archived range [a, b]` |
| `AutoBackupAlreadyRunning`  | `Automatic backup is already running`                       |
| `AutoBackupNotRunning`      | `Automatic backup is not running`                           |
| `RestoreTargetNotEmpty`     | `Restore target directory is not empty: <path>`             |
| `Serialization`             | `Serialization error: <detail>`                             |

The REST status for these errors is 400, not 500. Read the message for the cause.

`storkvctl restore` and `storkvctl doctor` run in process. They print the core message and do not use the wire format.

## Rust API errors

The embedded engine returns `storkv_core::StorkvError`. The client returns `storkv_client::ClientError`, which wraps a wire error or a transport error.

```rust
match db.get(&scope, b"user:42") {
    Ok(value) => handle(value),
    Err(StorkvError::ScopeNotFound(name)) => {
        eprintln!("no such scope: {name}");
    }
    Err(StorkvError::KeyTooLarge { max, actual }) => {
        eprintln!("key is {actual} bytes, limit is {max}");
    }
    Err(error) => return Err(error.into()),
}
```

A client connection failure is a transport error, not a wire error. Inspect `ClientError` for the I/O or protocol cause.

## Conflict handling example

Code 3 is the only error that a normal retry loop should handle:

```rust
loop {
    let mut tx = db.begin_tx();
    let current = tx.get(&accounts, b"user:42:balance")?;
    tx.put(&accounts, b"user:42:balance", next_value(&current))?;
    match tx.commit() {
        Ok(()) => break,
        Err(StorkvError::WriteConflict { .. }) => continue,
        Err(error) => return Err(error.into()),
    }
}
```

Add a retry limit in production. A hot key can conflict many times in a row.

## Related guides

- [HTTP and RPC API](http-and-rpc-api.md)
- [Troubleshooting](../operations/troubleshooting.md)
- [MVCC and transactions](../concepts/mvcc-and-transactions.md)
- [Rust client](../guides/rust-client.md)
