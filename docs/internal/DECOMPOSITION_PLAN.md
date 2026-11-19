# File and Module Decomposition Plan

## Purpose

This document records the current module layout. Every source file has one
responsibility. Every file stays under about 250 lines, except generated
protobuf files and `build.rs` output.

## Rules

- Use the Rust 2018 layout: `name.rs` plus `name/child.rs`. Never use `mod.rs`.
- Keep one public API. Parent modules re-export child items.
- Put a `///` comment on every public item. Fallible functions document
  `# Errors`.
- Do not panic in production code. Return a structured error.
- Keep `storkv-core` free of async runtimes.
- Keep tests in `tests/` and `bindings/`.

## storkv-sdk

| Parent       | Child modules                                      |
| ------------ | -------------------------------------------------- |
| `pool`       | `builder`, `routing`, `ops`                        |
| `backend`    | `embedded`, `remote`                               |
| `collection` | `codec`, `ops`                                     |
| `options`    | `pool`, `policy`                                   |
| `watch`      | `stream`, `invalidator`                            |
| `scope`      | `cached`, `ops`                                    |
| `cache`      | `store`, `events`                                  |
| `transaction`| `read`, `write`                                    |

The root `lib.rs` keeps these exports: `Collection`, `TypedWatchEvent`,
`TypedWatchStream`, `Result`, `SdkError`, `CacheOptions`, `RetryPolicy`,
`ScopeDefinition`, `StorkvPoolOptions`, `PoolConnection`, `StorkvPool`,
`TxFuture`, `Scope`, `Transaction`, `WatchEvent`, `WatchEventKind`,
`WatchStream`, `CompactionStyle`, `Durability`, `Retention`, `Consistency`.

## storkv-core

| Parent            | Child modules                                           |
| ----------------- | ------------------------------------------------------- |
| `db/handle`       | `handle_kv`, `handle_scopes`, `handle_backup`, `handle_streams` |
| `backup/restore`  | `staging`, `replay`                                     |
| `backup/verify`   | `files`, `records`                                      |
| `backup/sink_pipe`| `archive`, `process`                                    |
| `transaction`     | `read`, `write`                                         |
| `db_ops/commit`   | `validate`, `apply`                                     |
| `vfs/memory`      | `fs`, `file`                                            |

## storkv-client

| Parent              | Child modules         |
| ------------------- | --------------------- |
| `backend/grpc`      | `admin`, `data`       |
| `async_client`      | `scope_ops`, `admin_ops` |

## storkv-ffi

| Parent         | Child modules          |
| -------------- | ---------------------- |
| `transaction`  | `lifecycle`, `ops`     |
| `scope`        | `point`, `scan`        |
| `types`        | `options`, `results`   |

## storkv-proto

| Parent             | Child modules           |
| ------------------ | ----------------------- |
| `response/types`   | `core`, `admin`         |
| `request/types`    | `enum_def`, `methods`   |

## storkv-server

| Parent                             | Child modules            |
| ---------------------------------- | ------------------------ |
| `compat/redis/commands_tx`         | `queue`, `exec`          |
| `server`                           | `state`, `session`       |
| `transports/ws/session`            | `loop_io`, `watchers`    |
| `transports/http/rest_data`        | `keys`, `queries`        |
| `compat/memcached/commands`        | `storage`, `ops`         |
| `transports/grpc/service`          | `data`, `streams`        |
| `compat/redis/helpers`             | `args`, `matchers`       |
| `compat/redis/codec_decode`        | `scalars`, `aggregates`  |
| `transports/ws/json_rpc`           | `dispatch`, `codec`      |
| `compat/redis/commands_strings`    | `basic`, `mutate`        |
| `bin/storkvd`                      | `args`, `bootstrap`      |

## storkv-cli and storkv-tui

The CLI and TUI crates already use focused modules. No further split is
required.

## Verification

Run these commands after each change:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
project-structure.exe . --count --minimize ascii
```
