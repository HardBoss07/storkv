# Redis compatibility

`--redis-listen` serves a RESP2 and RESP3 endpoint that maps Redis string commands onto StorKV scopes. Existing Redis clients can connect without a code change for the supported command set.

## Start the listener

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --redis-listen 127.0.0.1:6379
```

Connect with any Redis client:

```bash
redis-cli -p 6379 PING
redis-cli -p 6379 SET greeting hello
redis-cli -p 6379 GET greeting
```

## Authentication

Set `--redis-password` or `STORKV_REDIS_PASSWORD`. When `--redis-password` is absent, the listener uses `--admin-token`. When no password is set, connections start authenticated.

With a password, a connection must run `AUTH` before it uses other commands. `AUTH`, `HELLO`, and `PING` work before authentication. Any other command returns `NOAUTH Authentication required.`

```bash
redis-cli -p 6379 -a "$STORKV_REDIS_PASSWORD" PING
```

`AUTH` accepts `AUTH <password>` or `AUTH default <password>`. `HELLO` accepts `AUTH <username> <password>` and `SETNAME <name>`. The adapter compares the password in constant time and returns `WRONGPASS` on a mismatch. A failed `HELLO` AUTH keeps the protocol version and the authentication state.

## Scope mapping

| Redis command | StorKV scope   |
| :------------ | :------------- |
| `SELECT 0`    | `default`      |
| `SELECT n`    | `redis_db_<n>` |

`SELECT` creates the target scope on first use with strict durability, permanent retention, and an 8-byte prefix Bloom filter. The database index is a `u32`.

Data written through Redis is visible to native clients in the same scope. A native client can read `redis_db_1` after a Redis client runs `SELECT 1`.

## Supported commands

### Connection and server

| Command    | Behavior                                                    |
| :--------- | :---------------------------------------------------------- |
| `PING`     | Returns `PONG`, or echoes the argument as a bulk string     |
| `ECHO`     | Returns the argument as a bulk string                       |
| `QUIT`     | Replies `OK` and closes the connection                      |
| `HELLO`    | Accepts `2` or `3`; returns a map of server fields          |
| `AUTH`     | Authenticates the connection with the configured password   |
| `CLIENT`   | `ID`, `SETNAME <name>`, and `GETNAME`                       |
| `COMMAND`  | Returns metadata for supported commands; `COUNT` returns the count |
| `SELECT`   | Switches the database index and creates its scope           |
| `INFO`     | Returns server, stats, and persistence text                 |
| `DBSIZE`   | Returns the number of keys in the current scope             |
| `FLUSHDB`  | Deletes every key in the current scope                      |
| `FLUSHALL` | Deletes every key in `default` and every `redis_db_*` scope |

`INFO` reports `redis_version:7.2.0` and the current StorKV version. It also reports total writes, total reads, write conflicts, compactions, and fsyncs.

`FLUSHALL` does not touch scopes that Redis does not own, such as `memcached` or `etcd`.

### Strings and keys

| Command             | Notes                                                 |
| :------------------ | :---------------------------------------------------- |
| `GET`               | Returns a bulk string or a null reply                 |
| `GETDEL`            | Returns a value and deletes the key atomically        |
| `GETEX`             | Returns a value and changes its expiry atomically     |
| `SET`               | Supports `EX`, `PX`, `EXAT`, `PXAT`, `NX`, `XX`, `GET`, and `KEEPTTL` |
| `SETEX`, `PSETEX`   | Sets a value and a positive TTL                       |
| `SETNX`             | Sets only when the key is absent                      |
| `MGET`, `MSET`      | Multi-key read and write on the current scope         |
| `MSETNX`            | Writes all pairs only when every key is absent        |
| `DEL`, `UNLINK`     | Deletes one or more keys                              |
| `EXISTS`            | Counts the keys that exist                            |
| `RENAME`, `RENAMENX`| Renames a key and keeps its expiry                    |
| `TYPE`              | Returns `string` for a live key and `none` otherwise  |
| `INCR`, `DECR`      | Adds or subtracts one from an integer value           |
| `INCRBY`, `DECRBY`  | Adds or subtracts a caller-supplied delta             |
| `APPEND`            | Appends bytes and returns the new length              |
| `STRLEN`            | Returns the byte length of a value                    |
| `EXPIRE`, `PEXPIRE` | Sets a TTL in seconds or milliseconds                 |
| `EXPIREAT`, `PEXPIREAT` | Sets an absolute expiry with `NX`, `XX`, `GT`, or `LT` |
| `EXPIRETIME`, `PEXPIRETIME` | Returns the absolute expiry in seconds or milliseconds |
| `PERSIST`           | Removes the TTL from a key                            |
| `TTL`, `PTTL`       | Returns the remaining time in seconds or milliseconds |
| `KEYS`              | Returns keys that match a glob pattern                |
| `SCAN`              | Returns a cursor and a page of keys                   |

`SET` replaces the old value. Without `KEEPTTL`, the new value has no expiry. `EXAT` and `PXAT` set an absolute Unix expiry. An expiry flag cannot combine with another expiry flag or with `KEEPTTL`.

`GETDEL`, `GETEX`, `MSETNX`, `RENAME`, and `RENAMENX` run in one OCC transaction. A concurrent writer cannot observe partial state.

`GETEX` accepts one of `EX`, `PX`, `EXAT`, `PXAT`, or `PERSIST`. It returns a null reply and creates no key when the key is absent.

`INCR` and `DECR` retry up to 16 times on a write conflict. The value must parse as a signed 64-bit integer.

### Transactions

| Command   | Behavior                                                        |
| :-------- | :-------------------------------------------------------------- |
| `MULTI`   | Starts command queuing; replies `QUEUED` for each later command |
| `EXEC`    | Runs the queue as one StorKV transaction                        |
| `DISCARD` | Clears the queue and the watch set                              |
| `WATCH`   | Records a read sequence and the watched keys                    |
| `UNWATCH` | Clears the watch set                                            |

`WATCH` captures the current visible sequence. `EXEC` validates every watched key before the commit. On a conflict, `EXEC` returns a RESP null array and writes nothing.

> [!NOTE]
> A RESP null array is the Redis 7 behavior for an aborted watched transaction. Some older Redis versions return an error array instead. Check your client library.

`FLUSHDB` and `FLUSHALL` are rejected inside `MULTI`. `WATCH` inside `MULTI` returns an error. `MULTI` cannot nest.

A queued unknown command sets an error flag. `EXEC` then returns `EXECABORT` and runs nothing.

## TTL behavior

- A positive `EXPIRE` or `PEXPIRE` value sets an absolute expiry time.
- A zero or negative `EXPIRE` deletes the key and returns 1.
- `EXPIREAT` and `PEXPIREAT` set an absolute Unix expiry. A target in the past deletes the key and returns 1.
- `EXPIREAT` and `PEXPIREAT` accept `NX`, `XX`, `GT`, and `LT`. A key without an expiry counts as unlimited for `GT` and `LT`.
- `EXPIRETIME` and `PEXPIRETIME` return -2 when the key is absent, -1 when the key has no expiry, and the absolute expiry otherwise.
- `SETEX` and `PSETEX` reject a zero or negative TTL with `ERR invalid expire time`.
- `TTL` returns -2 when the key is absent and -1 when the key has no expiry.
- `PERSIST` returns 1 when it removed a TTL and 0 when the key is absent.
- `RENAME` keeps the expiry of the source key. `RENAMENX` returns 0 when the target key exists.
- `KEYS` and `SCAN` skip expired keys because the engine filters them at read time.

## Cursors and patterns

`SCAN` uses a hexadecimal encoding of the next key as its cursor. `COUNT` defaults to 10 and sets a page limit. `MATCH` supports `*`, `?`, and `[abc]` classes.

`KEYS` reads every key in the scope. It is a full scan. Use `SCAN` for large keyspaces.

## Limits and unsupported commands

The adapter has these limits:

| Item                  | Limit                           |
| :-------------------- | :------------------------------ |
| Key length            | 65,535 bytes                    |
| Value length          | 256 MiB                         |
| Inline command line   | 64 KiB                          |
| `MATCH`/`KEYS` pattern| 1,024 bytes                     |
| Command set           | Strings, keys, transactions, and connection commands |

The parser rejects an inline command line larger than 64 KiB. It also rejects a request that grows beyond the read buffer limit. The server sends a protocol error and closes the connection.

`KEYS` and `SCAN` support `*`, `?`, and `[abc]` classes. The matcher is iterative, so its cost is at most the product of the pattern length and the key length. A pattern larger than 1,024 bytes is rejected.

When `--strict-admin` is active, `FLUSHDB` and `FLUSHALL` return `NOPERM`. Use an admin path to flush data in that mode.

A flush writes one range tombstone. It does not read or delete each key in turn.

The adapter does not implement these Redis features:

- Lists, hashes, sets, and sorted sets.
- Pub/Sub.
- Lua scripts.
- Streams.
- Cluster mode.
- Replication.
- Persistence commands such as `BGSAVE` and `AOF`.

An unsupported command returns `ERR unknown command '<name>'`.

> [!WARNING]
> The adapter is not a full Redis replacement. It has no server-side expiration thread, so expired keys remain on disk until a compaction removes them. Read [Watches and TTL](../concepts/watches-and-ttl.md) for the expiry model.

## Errors

Adapter errors use Redis error replies:

| Condition            | Reply                                         |
| :------------------- | :-------------------------------------------- |
| Wrong argument count | `ERR wrong number of arguments for ...`       |
| Non-integer value    | `ERR value is not an integer or out of range` |
| Counter overflow     | `ERR increment or decrement would overflow`   |
| Key too large        | `ERR key too large`                           |
| Value too large      | `ERR value too large`                         |
| Not authenticated    | `NOAUTH Authentication required.`             |
| Bad password         | `WRONGPASS invalid username-password pair or user is disabled.` |
| Write conflict       | `ERR write conflict, retry`                   |
| Storage failure      | `ERR <storage error text>`                    |

## Related guides

- [Scopes](../concepts/scopes.md)
- [Memcached and etcd compatibility](compatibility-memcached-etcd.md)
- [Migrating data](migrating-data.md)
- [Transports](transports.md)
