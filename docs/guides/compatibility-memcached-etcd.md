# Memcached and etcd compatibility

StorKV has a Memcached ASCII listener and an etcd v3 JSON gateway. Both use ordinary scopes, so their data is visible to native clients and is included in backups.

## Memcached

### Start the listener

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --memcached-listen 127.0.0.1:11211
```

Connect with a Memcached client or with a raw TCP tool:

```bash
printf 'set greeting 0 0 5\r\nhello\r\n' | nc 127.0.0.1 11211
printf 'get greeting\r\n' | nc 127.0.0.1 11211
```

### Scope and storage format

Every Memcached item lives in the `memcached` scope. The adapter creates the scope on first use with strict durability, FIFO compaction, and an 8-byte prefix Bloom filter.

Each stored value has a 4-byte little-endian flags field in front of the value bytes. The adapter removes that prefix on read. Native clients that read the raw scope see the prefix.

### Supported commands

| Command                                       | Behavior                                   |
| :-------------------------------------------- | :----------------------------------------- |
| `get <key>...`                                | Returns each found value, then `END`       |
| `gets <key>...`                               | Returns each value with its CAS token      |
| `set <key> <flags> <exptime> <bytes>`         | Stores a value unconditionally             |
| `add`                                         | Stores only when the key is absent         |
| `replace`                                     | Stores only when the key exists            |
| `append`, `prepend`                           | Extends an existing value                  |
| `cas <key> <flags> <exptime> <bytes> <token>` | Stores only when the token matches         |
| `delete <key>`                                | Deletes a key                              |
| `incr`, `decr`                                | Adds or subtracts an unsigned delta        |
| `touch <key> <exptime>`                       | Changes the expiry of a key                |
| `flush_all`                                   | Deletes every key in the `memcached` scope |
| `stats`                                       | Returns a fixed set of `STAT` lines        |
| `version`                                     | Returns `VERSION storkv-0.1.0`             |
| `quit`                                        | Closes the connection                      |

Add `noreply` to a storage, delete, increment, touch, or flush command to suppress the reply.

`flush_all` accepts a delay value, but the adapter ignores the delay and flushes at once.

When `--strict-admin` is active, `flush_all` returns `CLIENT_ERROR`. The adapter reads the strict-admin mode when the connection opens, so set the mode before you connect.

A flush writes one range tombstone. It does not read or delete each key in turn.

`delete` does not support the optional time value.

### Limits

| Item         | Limit                   |
| :----------- | :---------------------- |
| Key length   | 250 bytes               |
| Value length | 1 MiB                   |
| Command line | 8 KiB                   |
| Counter type | Unsigned 64-bit integer |

The parser returns `CLIENT_ERROR` for a bad command line, an oversized value, or a bad data chunk. Unknown commands return `ERROR`.

### Expiry

An `exptime` of 0 means no expiry. A value up to 30 days is a relative time in seconds. A value above 30 days is an absolute Unix time in seconds, as in Memcached.

`incr` and `decr` keep the current expiry. A decrement stops at zero. A non-numeric value returns `CLIENT_ERROR cannot increment or decrement non-numeric value`.

### CAS tokens

The CAS token is the visible sequence number of the key. `gets` returns it as the fourth header field. A `cas` command compares the token to the current sequence:

| Result      | Meaning                                  |
| :---------- | :--------------------------------------- |
| `STORED`    | The token matched and the write happened |
| `EXISTS`    | The key changed after the token was read |
| `NOT_FOUND` | The key is absent                        |

### Stats

`stats` returns these fields:

```text
STAT pid 1
STAT uptime 0
STAT time 0
STAT version storkv-0.1.0
STAT curr_items <count>
STAT total_items <count>
END
```

The adapter computes the item count with a full scan. It does not track hits, misses, or evictions.

## etcd

### Route set

The etcd gateway uses the HTTP listener. It has three routes:

| Route                     | etcd method   |
| :------------------------ | :------------ |
| `POST /v3/kv/put`         | `Put`         |
| `POST /v3/kv/range`       | `Range`       |
| `POST /v3/kv/deleterange` | `DeleteRange` |

Every key lives in the `etcd` scope. The scope is created on first use with strict durability and permanent retention.

Keys and values in requests and responses are standard base64 strings.

### Put

```bash
curl -sS -X POST http://127.0.0.1:8080/v3/kv/put \
  -H 'content-type: application/json' \
  -d '{"key":"L2FwcC9uYW1l","value":"c3Rvcmt2"}'
```

Response:

```json
{
  "header": {
    "cluster_id": "53544f524b56",
    "member_id": "73746f726b76",
    "revision": "42",
    "raft_term": "1"
  }
}
```

A `lease` field other than 0 returns HTTP 400 with code 3. Leases are not supported.

### Range

```bash
curl -sS -X POST http://127.0.0.1:8080/v3/kv/range \
  -H 'content-type: application/json' \
  -d '{"key":"L2FwcC8=","range_end":"L2FwcC8w"}'
```

The response has this shape:

```json
{
  "header": {
    "cluster_id": "53544f524b56",
    "member_id": "73746f726b76",
    "revision": "42",
    "raft_term": "1"
  },
  "kvs": [
    {
      "key": "L2FwcC9uYW1l",
      "value": "c3Rvcmt2",
      "version": "42",
      "create_revision": "42",
      "mod_revision": "42",
      "lease": "0"
    }
  ],
  "count": "1",
  "more": false
}
```

Range options:

| Field        | Effect                                                               |
| :----------- | :------------------------------------------------------------------- |
| `range_end`  | Empty reads one key; one zero byte reads every key at or after `key` |
| `limit`      | Caps the returned entries; 0 means no limit                          |
| `count_only` | Returns only the count                                               |
| `keys_only`  | Omits values from the entries                                        |

`version`, `create_revision`, and `mod_revision` all carry the current sequence number of the key. The gateway does not store per-key revision history.

### DeleteRange

```bash
curl -sS -X POST http://127.0.0.1:8080/v3/kv/deleterange \
  -H 'content-type: application/json' \
  -d '{"key":"L2FwcC8=","range_end":"L2FwcC8w"}'
```

Response:

```json
{
  "header": {
    "cluster_id": "53544f524b56",
    "member_id": "73746f726b76",
    "revision": "43",
    "raft_term": "1"
  },
  "deleted": "1"
}
```

The gateway deletes the matched keys in one transaction.

### Revisions and errors

The `revision` field is the last visible sequence number of the whole database. All scopes share it.

An error response carries three fields:

```json
{ "error": "key is required", "message": "key is required", "code": 3 }
```

| HTTP status | Code | Cause                            |
| :---------- | :--- | :------------------------------- |
| 400         | 3    | Missing key or unsupported lease |
| 500         | 13   | Storage failure                  |

### Unsupported etcd features

The gateway does not support:

- Leases and keepalives.
- Transactions.
- Watches.
- Compaction.
- Authentication.
- Cluster and member APIs.
- gRPC etcd clients.

Use the native watch stream, the WebSocket watch operation, or gRPC `Watch` when you need change events.

## Related guides

- [Redis compatibility](compatibility-redis.md)
- [Scopes](../concepts/scopes.md)
- [Transports](transports.md)
- [HTTP and RPC API](../reference/http-and-rpc-api.md)
