# Configuration

StorKV has no configuration file. You set every option with a command line flag or with a Rust value.

## Daemon flags

`storkvd` accepts these flags:

| Flag                     | Default          | Purpose                                           |
| :----------------------- | :--------------- | :------------------------------------------------ |
| `--data-dir`             | `./data/storkv`  | Directory that holds the database                 |
| `--listen`               | `127.0.0.1:7878` | Native TCP listener                               |
| `--tls-cert`             | none             | PEM certificate for the native TLS listener       |
| `--tls-key`              | none             | PEM private key for the native TLS listener       |
| `--admin-sock`           | none             | Native admin listener; accepts TCP or a Unix path |
| `--http-listen`          | none             | HTTP REST and binary RPC listener                 |
| `--ws-listen`            | none             | WebSocket listener                                |
| `--grpc-listen`          | none             | gRPC listener                                     |
| `--redis-listen`         | none             | Redis RESP listener                               |
| `--memcached-listen`     | none             | Memcached ASCII listener                          |
| `--strict-admin`         | off              | Restrict admin-only requests                      |
| `--admin-token`          | none             | Shared admin secret for auxiliary transports      |
| `--admin-token-file`     | none             | Read the admin secret from this file              |
| `--http-admin`           | off              | Treat every HTTP connection as admin              |
| `--ws-admin`             | off              | Treat every WebSocket connection as admin         |
| `--grpc-admin`           | off              | Treat every gRPC connection as admin              |
| `--backup-dir`           | none             | Enable autonomous backup into this directory      |
| `--backup-interval-secs` | 3600             | Seconds between full backups                      |
| `--backup-retention`     | 7                | Number of full generations to keep                |
| `--backup-ship-wal`      | off              | Ship sealed WAL segments between full backups     |
| `--backup-pipe-cmd`      | none             | Send backup archives to a child process           |
| `--backup-pipe-arg`      | none             | Repeatable argument for the pipe command          |
| `--backup-compression`   | `none`           | Archive compression: `none`, `zstd`, or `lz4`     |
| `--role`                 | `standalone`     | Replication role: `standalone`, `primary`, or `replica` |
| `--primary-uri`          | none             | Primary URI that a replica streams from           |

Run `storkvd --help` to see the same list from the binary.

## Start with a minimal set

```bash
./target/release/storkvd --data-dir ./data/storkv
```

This command starts only the native listener on `127.0.0.1:7878`. No admin listener and no compatibility listener start. The server creates the `default` scope on first open.

## Add a dedicated admin listener

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --listen 127.0.0.1:7878 \
  --admin-sock 127.0.0.1:7879 \
  --strict-admin
```

A native connection on `127.0.0.1:7879` can run admin-only requests. A connection on `127.0.0.1:7878` cannot, even if it knows the admin token, because the native listener has no token header.

On Unix, use a socket path:

```bash
./target/release/storkvd \
  --data-dir /var/lib/storkv \
  --admin-sock unix:///run/storkv-admin.sock \
  --strict-admin
```

An existing socket file at the path is removed before the bind.

On Unix, `storkvd` sets the socket directory mode to `0700` and the socket file mode to `0600`. Only the server user can connect. Startup fails when the server cannot apply these modes.

## Enable TLS on the native listener

The native TCP listener supports built-in TLS. Pass both the certificate and the key:

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --listen 127.0.0.1:7878 \
  --tls-cert /etc/storkv/server.crt \
  --tls-key /etc/storkv/server.key
```

Both files use PEM format. You must pass both flags together. Startup fails when only one flag is set or when the server cannot read a file. The server also checks that the certificate and key form a valid server identity.

TLS applies to the native TCP listener only. The HTTP, WebSocket, gRPC, Redis, and Memcached listeners stay plaintext. Put a TLS proxy in front of those listeners when you need encrypted access.

## Protect auxiliary transports

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --http-listen 127.0.0.1:8080 \
  --grpc-listen 127.0.0.1:9090 \
  --strict-admin \
  --admin-token-file /etc/storkv/admin.token
```

The token file must hold one line. Use `--admin-token-file` or set `STORKV_ADMIN_TOKEN`. Do not pass the token on the command line. Process arguments are visible to other users.

A client presents the token in a request header:

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/admin/compact \
  -H "x-storkv-admin-token: $STORKV_ADMIN_TOKEN"
```

Or as a bearer token:

```bash
curl -sS -X POST http://127.0.0.1:8080/v1/admin/compact \
  -H "authorization: Bearer $STORKV_ADMIN_TOKEN"
```

A gRPC client sends the token as metadata. The exact metadata key depends on your generated client.

Use `--http-admin` when the HTTP listener sits on a private network and every caller is trusted:

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --http-listen 127.0.0.1:8080 \
  --http-admin \
  --strict-admin
```

The same pattern applies to `--ws-admin` and `--grpc-admin`.

> [!WARNING]
> `--admin-token`, `--admin-token-file`, and `STORKV_ADMIN_TOKEN` do not protect the native listener. The native protocol has no token field. Use `--admin-sock` or a private network for native admin access.

## Enable autonomous backup

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --backup-dir ./backups \
  --backup-interval-secs 3600 \
  --backup-retention 7 \
  --backup-ship-wal
```

This command writes a full generation every hour, keeps seven full generations, and ships sealed WAL segments between full backups.

The server prints this line at startup:

```text
storkvd backups enabled at ./backups
```

`--backup-ship-wal` has no effect when `--backup-dir` is absent.

## Send backups to a child process

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --backup-dir ./backups \
  --backup-pipe-cmd /usr/local/bin/upload-backup \
  --backup-pipe-arg --bucket \
  --backup-pipe-arg prod-storkv \
  --backup-compression zstd
```

The command receives the archive on standard input. Repeat `--backup-pipe-arg` for each argument.

`--backup-compression` accepts `none`, `zstd`, or `lz4`. The name is case-insensitive. An unknown value stops startup with an error:

```text
unsupported backup compression `gzip` (expected none, zstd, or lz4)
```

`--backup-compression` applies to the pipe sink. A local backup directory stores files without extra compression.

## Run a primary and a replica

A primary serves reads and writes and streams committed WAL frames. A replica applies those frames and rejects writes.

Start the primary:

```bash
./target/release/storkvd \
  --data-dir ./data/primary \
  --listen 127.0.0.1:7878 \
  --role primary
```

Start the replica against the primary native address:

```bash
./target/release/storkvd \
  --data-dir ./data/replica \
  --listen 127.0.0.1:7979 \
  --role replica \
  --primary-uri storkv://127.0.0.1:7878
```

The replica needs the same scopes as the primary. Create them on both nodes, or apply the primary snapshot path. A replica reconnects with backoff and resumes from its last applied sequence.

A replica serves reads at its configured consistency level. The default for a replica is `Eventual`. A client can change the level with `AsyncClient::set_consistency` or `StorkvPoolOptions::consistency`:

| Level                                     | Replica behavior                                              |
| :---------------------------------------- | :----------------------------------------------------------- |
| `Consistency::Strong`                     | Reject data reads and writes with `NotPrimary`                |
| `Consistency::BoundedStaleness { max_lag_ms }` | Serve reads while lag stays at or below the bound; else `StaleReplica` |
| `Consistency::Eventual`                   | Serve reads at any lag                                        |

The consistency level is per connection. Each connection starts from the server default. A `ConfigureConsistency` request changes only the connection that sent it. One client cannot change the read policy of another client.

The server default is set at startup with `Server::with_consistency`. `StorkvPoolOptions::consistency` applies to each new networked connection that the pool opens.

Replication is asynchronous. It has no quorum, leader election, or automatic failover.

## Embedded options

The embedded engine reads `DbOptions`:

```rust
use std::time::Duration;
use storkv_core::{Db, DbOptions};

let options = DbOptions {
    max_wal_pin_bytes: 128 * 1024 * 1024,
    max_wal_pin_age: Duration::from_secs(120),
};
let db = Db::open("./data/storkv", options)?;
```

| Field               | Default | Effect                              |
| :------------------ | :------ | :---------------------------------- |
| `max_wal_pin_bytes` | 64 MiB  | Memtable size that triggers a flush |
| `max_wal_pin_age`   | 60 s    | Memtable age that triggers a flush  |

The daemon always uses the default `DbOptions`.

## Tuning flags that do not exist

StorKV has no runtime flags for these values:

- WAL segment size. The fixed value is 32 MiB.
- WAL seal interval. The fixed value is 300 seconds.
- Leveled compaction trigger. The fixed value is four L0 files.
- Sync interval for the default scope. It uses strict durability.
- Background compaction. The engine has no background compaction thread.

To change the first four values, change the source and rebuild. To remove expired data, run compaction as a manual step.

## Dashboard flags

`storkv-top` accepts two flags:

| Flag            | Default          | Purpose                       |
| :-------------- | :--------------- | :---------------------------- |
| `--sock`        | `127.0.0.1:7878` | Server address                |
| `--interval-ms` | 1000             | Poll interval in milliseconds |

## CLI environment

`storkvctl` reads no environment variables. It takes every value from a flag.

## Related guides

- [Transports](../guides/transports.md)
- [Backup and restore](backup-and-restore.md)
- [Compaction and tuning](compaction-and-tuning.md)
- [Observability](observability.md)
