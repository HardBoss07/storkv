# Configuration

You set every `storkvd` option with a command-line flag, an environment variable, a TOML file, or the built-in default. The embedded engine takes a Rust value.

## Daemon flags

`storkvd` accepts these flags:

| Flag                     | Default          | Purpose                                           |
| :----------------------- | :--------------- | :------------------------------------------------ |
| `--config`               | none             | Load settings from a TOML file                    |
| `--data-dir`             | `./data/storkv`  | Directory that holds the database                 |
| `--listen`               | `127.0.0.1:7878` | Native TCP listener                               |
| `--tls-cert`             | none             | PEM certificate for the native TLS listener       |
| `--tls-key`              | none             | PEM private key for the native TLS listener       |
| `--admin-sock`           | none             | Native admin listener; accepts TCP or a Unix path |
| `--http-listen`          | none             | HTTP REST and binary RPC listener                 |
| `--ws-listen`            | none             | WebSocket listener                                |
| `--grpc-listen`          | none             | gRPC listener                                     |
| `--redis-listen`         | none             | Redis RESP listener                               |
| `--redis-password`       | none             | Password for the Redis RESP listener              |
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

## Configuration sources and precedence

`storkvd` reads settings from four sources. From highest to lowest precedence:

1. Command-line flags.
2. `STORKV_*` environment variables.
3. A `storkv.toml` file.
4. Built-in defaults.

Each flag has one environment variable. To build the name, upper-case the long flag name, replace every `-` with `_`, and add the `STORKV_` prefix. For example, `--http-listen` reads `STORKV_HTTP_LISTEN`, `--data-dir` reads `STORKV_DATA_DIR`, and `--config` reads `STORKV_CONFIG`. For the repeatable `--backup-pipe-arg` flag, set `STORKV_BACKUP_PIPE_ARGS` to a comma-separated list.

Boolean flags accept `true`, `false`, `1`, and `0` in their environment variables. A value of `false` or `0` overrides a `true` value from the file. For example, `STORKV_STRICT_ADMIN=false` disables `strict_admin = true` in the file.

`storkvd` loads a configuration file only when you pass `--config` or set `STORKV_CONFIG`. It does not search the current working directory for `storkv.toml`. When you set both, `--config` wins over `STORKV_CONFIG`.

The repository root contains `storkv.example.toml`. It is a complete commented template for every supported key. Copy it, then remove the leading `# ` from the settings that you need.

Pass a file with `--config` or `STORKV_CONFIG`:

```toml
data_dir = "/var/lib/storkv"
listen = "0.0.0.0:7878"
http_listen = "0.0.0.0:8080"
role = "primary"
strict_admin = true
```

```bash
./target/release/storkvd --config /etc/storkv/storkv.toml
```

Use snake_case keys. The file accepts the same settings as the flags. A setting that the file does not contain keeps its environment value or built-in default.

Startup stops with an error in these cases:

- The file cannot be read.
- The file has invalid TOML.
- The file has an unknown key.
- A final listen address is empty or cannot be parsed.

The server validates the final values after the merge. A lower-precedence invalid value does not stop startup when a higher-precedence source replaces it.

## Start with a minimal set

```bash
./target/release/storkvd --data-dir ./data/storkv
```

This command starts only the native listener on `127.0.0.1:7878`. No admin listener and no compatibility listener start. The server creates the `default` scope on first open.

## Provision scopes from the file

Add one `[[scopes]]` table for each scope that the server must create at startup. The server creates or verifies every declared scope before it opens the listeners. An existing scope keeps all of its keys and receives the declared TTL guardrails. Startup fails when an entry is invalid or a name is declared more than once.

On a daemon restart, the server keeps existing scope data. It also applies the declared `min_ttl_ms`, `max_ttl_ms`, `allow_permanent`, and `active_ttl_tracking` values again, so the guardrails stay enforced. The manifest does not store these guardrails. The configuration file is the source of truth for them.

```toml
[[scopes]]
name = "sessions"
durability = "strict_fsync"
retention = "ttl"
default_ttl_ms = 3600000
min_ttl_ms = 1000
max_ttl_ms = 86400000
allow_permanent = false
active_ttl_tracking = true
storage_mode = "disk"
prefix_bloom_len = 1024
```

| Key                    | Default        | Purpose                                      |
| :--------------------- | :------------- | :------------------------------------------- |
| `name`                 | none           | Unique scope name. Required.                 |
| `durability`           | `strict_fsync` | `strict_fsync` or `buffered`.                |
| `retention`            | `permanent`    | `permanent` or `ttl`.                        |
| `default_ttl_ms`       | none           | Default lifetime for a `ttl` scope. Required for `ttl`. |
| `min_ttl_ms`           | none           | Minimum allowed lifetime.                    |
| `max_ttl_ms`           | none           | Maximum allowed lifetime.                    |
| `allow_permanent`      | `true`         | Permit writes without an expiry.             |
| `active_ttl_tracking`  | `false`        | Emit expiry events from a background tracker. |
| `storage_mode`         | `disk`         | `disk`, `in_memory`, or `in_memory_persistent`. |
| `prefix_bloom_len`     | none           | Prefix Bloom filter size in bytes.           |

A `ttl` scope requires a positive `default_ttl_ms`. The default lifetime must stay between `min_ttl_ms` and `max_ttl_ms` when those limits are set. A `permanent` scope must not set `default_ttl_ms`.

The `min_ttl_ms`, `max_ttl_ms`, and `prefix_bloom_len` values must be greater than zero.

The default `retention` is `permanent`. If you combine `retention = "permanent"` with `allow_permanent = false`, then every write in that scope must set an explicit per-key TTL. The TTL must fall between `min_ttl_ms` and `max_ttl_ms` when those limits are set. A write without a TTL fails with a client error.

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

## Protect the Redis listener

Pass `--redis-password` or set `STORKV_REDIS_PASSWORD`. When `--redis-password` is absent, the Redis listener uses `--admin-token`. When neither is set, Redis connections start authenticated.

```bash
./target/release/storkvd \
  --data-dir ./data/storkv \
  --redis-listen 127.0.0.1:6379 \
  --redis-password "$STORKV_REDIS_PASSWORD"
```

A Redis client must run `AUTH` before it uses other commands. `AUTH`, `HELLO`, and `PING` work before authentication. Read [Redis compatibility](../guides/compatibility-redis.md) for the handshake details.

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

## Run in a container

The repository root contains a multi-stage `Dockerfile`, a `docker-compose.yml`, and the `storkv.example.toml` reference file. The image builds `storkvd` and `storkvctl` in release mode on `rust:1.100-slim-trixie` and runs on `gcr.io/distroless/cc-debian13:nonroot`.

The runtime image has no shell. It runs as UID and GID `65532`, which the distroless `nonroot` user owns. The builder pre-creates `/var/lib/storkv`, `/var/lib/storkv/backups`, and `/etc/storkv` with that owner. The `HEALTHCHECK` runs `storkvctl health --sock 127.0.0.1:7878` in exec form, so it needs no shell.

Build and run one container:

```bash
docker build -t storkv .
docker run --rm -p 7878:7878 -v storkv-data:/var/lib/storkv storkv
```

The same image is published to the GitHub Container Registry for `linux/amd64` and `linux/arm64` when a `v*` tag is pushed:

```bash
docker pull ghcr.io/hardboss07/storkv:latest
```

Use `docker-compose.yml` for a persistent deployment. Copy the example file first, then set `STORKV_CONFIG` to the copied path:

```bash
cp storkv.example.toml storkv.toml
STORKV_CONFIG=./storkv.toml \
STORKV_ADMIN_TOKEN="$(openssl rand -hex 32)" \
STORKV_REDIS_PASSWORD="$(openssl rand -hex 32)" \
docker compose up -d
```

The compose file mounts the configuration read-only at `/etc/storkv/storkv.toml`, keeps `/var/lib/storkv` in the `storkv-data` named volume, and maps the native, HTTP, WebSocket, gRPC, Redis, and Memcached ports. It also demonstrates the `STORKV_*` overrides for the listen addresses, the replication role, the admin token, the Redis password, and backup settings. Every variable in the compose file is declared by `storkvd`. The daemon has no etcd listener flag, so the compose file sets no etcd variable. When `STORKV_CONFIG` is unset, the compose file mounts `storkv.example.toml`, so the environment values alone define the runtime.

Start the primary and the replica together with the replica profile:

```bash
docker compose --profile replica up -d
```

The replica reads from `storkv://storkv:7878`, stores its data in `storkv-replica-data`, and publishes the native listener on host port `7978`.

> [!WARNING]
> The compose defaults `STORKV_ADMIN_TOKEN` and `STORKV_REDIS_PASSWORD` to `change-me-in-production`. Replace both values before you expose any listener.

To refresh an existing deployment:

```bash
docker compose pull
docker compose up -d
```

The `.github/workflows/docker.yml` workflow checks that the image builds on pull requests and pushes the multi-arch image to GHCR on `v*` tags.

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
