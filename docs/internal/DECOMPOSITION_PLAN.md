# File & Module Decomposition Plan

Baseline: `cargo test --workspace` passes 107 integration tests, `cargo clippy --workspace --all-targets -- -D warnings` is clean.

Every split preserves the existing public API. New submodules are private or `pub(crate)` unless they already exported public
items; the parent `lib.rs`/`mod.rs` retains the identical `pub use` surface so no caller or test changes.

## storkv-core

| Current file         | Lines | New modules                                                                                                     | Preserved root exports                                                                            |
| -------------------- | ----- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `manifest/record.rs` | 352   | `record/types.rs`, `record/edit.rs`, `record/encode.rs`, `record/decode.rs`                                     | `EditRecord`, `SstFileMeta`, `ScopeMeta`, `SnapshotMeta`, enum modes, magic/version               |
| `db.rs`              | 274   | `db/handle.rs`, `db/inner.rs`, `db/open.rs`                                                                     | `Db`, `DbInner`, `ScopeHandle`                                                                    |
| `db_ops.rs`          | 253   | `db_ops/scopes.rs`, `db_ops/read.rs`, `db_ops/maintenance.rs`, plus existing `commit.rs`, `flush.rs`, `scan.rs` | all `DbInner` methods and free helpers                                                            |
| `sstable/mod.rs`     | 851   | `block.rs`, `bloom.rs`, `meta.rs`, `footer.rs`, `builder.rs`, `reader.rs`                                       | `SstWriter`, `SstReader`, `CompressionType`, `BlockHandle`, plus existing `pub` items re-exported |
| `manifest/mod.rs`    | 641   | `record.rs`, `codec.rs`, `version_set.rs`, `manager.rs`                                                         | `ManifestManager`, `VersionEdit`, `EditRecord`, `SstFileMeta`, `CurrentManifest`                  |
| `wal/mod.rs`         | 549   | `record.rs`, `codec.rs`, `writer.rs`, `recovery.rs`                                                             | `WalManager`, `WalRecovery`, `WalOp`, `WalBatchFrame`, `Durability`, segment helpers              |
| `vfs/mod.rs`         | 350   | `traits.rs`, `posix.rs`, `fault_injecting.rs`                                                                   | `FileSystem`, `File`, `PosixFs`, `FaultInjectingFs`, fault policies                               |
| `scope/mod.rs`       | 261   | `registry.rs`, `options.rs` (only if helper volume warrants)                                                    | `Scope`, `ScopeOptions`, `Retention`, `CompactionStyle`, `ScopeRegistry`                          |
| `compaction/mod.rs`  | 200   | `leveled.rs`, `fifo.rs` (only if helper volume warrants)                                                        | `Compactor`, `CompactionStyle`, `CompactionResult`                                                |

## storkv-proto

| Current file | Lines | New modules                                                                                                                                                             | Preserved root exports                                                                             |
| ------------ | ----- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `src/lib.rs` | 1135  | `error.rs`, `frame.rs`, `endpoint.rs`, `request/` (`types`, `encode`, `decode`, `decode_tx`, `decode_scope`), `response/` (`types`, `encode`, `decode`, `decode_admin`) | All `Request`/`Response` variants, `Wire*` structs, frame helpers, error mapping, endpoint helpers |

## storkv-server

| Current file | Lines | New modules                                                                                                                       | Preserved root exports                                                                                |
| ------------ | ----- | --------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `src/lib.rs` | 678   | `config.rs`, `listener.rs`, `metrics.rs`, `server.rs`, `dispatch/` (`data`, `scopes`, `transactions`, `telemetry`, `conversions`) | `ServerConfig`, `ServerListener`, `AcceptedStream`, `ScopeMetrics`, `Server`, existing free functions |

## storkv-client

| Current file | Lines | New modules                                                                       | Preserved root exports                                                                                   |
| ------------ | ----- | --------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `src/lib.rs` | 743   | `transport.rs`, `async_client/` (`scope_ops`), `sync_client.rs`, `transaction.rs` | `AsyncClient`, `SyncClient`, `ClientStream`, `Transaction`, `WriteSet`, `ReadSet`, all method signatures |

## storkv-cli / storkv-tui

| Current file            | Lines | New modules                                      | Preserved root exports                                                          |
| ----------------------- | ----- | ------------------------------------------------ | ------------------------------------------------------------------------------- |
| `storkv-cli/src/lib.rs` | 232   | `health.rs`, `doctor.rs`, `backup.rs`, `ycsb.rs` | `run_health`, `run_doctor`, `run_backup`, `run_ycsb` and their argument structs |
| `storkv-tui/src/lib.rs` | 323   | `model.rs`, `poller.rs`, `ui.rs`                 | `App`, `run`, model/poller types used by binaries                               |

## tests/

- Delete empty `tests/chaos/mod.rs` (`tests/chaos.rs` remains the target).
- Split `tests/format.rs`, `server.rs`, `proto.rs`, `client.rs`, `mvcc.rs`, `tui.rs`, `chaos.rs`,
  `compaction.rs`, `wal.rs`, and `scope.rs` into `tests/<suite>/` domain submodules, each included by
  the thin `tests/<suite>.rs` harness via `#[path = "<suite>/<module>.rs"]`.
- Move shared helpers into `tests/common/mod.rs`; keep suite-local helpers in `tests/<suite>/support.rs`.
- Keep every test name, assertion, and test count identical.

## Function-level cleanup

- Split functions over ~45 lines into single-responsibility helpers.
- Rename terse identifiers (`buf`, `hdr`, `off`, `len`, `req`, `resp`, `tx`, `seq`, `k`, `v`, `f`, `s`, `res`, `tmp`)
  to domain names (`frame_buffer`, `header_bytes`, `byte_offset`, `payload_length`, `client_request`,
  `server_response`, `transaction`, `sequence_number`, `user_key`, `entry_value`, `file_system`, `scope`, etc.).
- Keep borrowed slices, zero `#[cfg(test)]` in `crates/*/src/`, zero `.unwrap()`/`.expect()` in production code.

## Verification

After every crate: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
Final: `cargo fmt --all -- --check`, example/binary smoke checks, `project-structure.exe . --count -m`,
delete `HARDENING_AUDIT.md` and `baseline_test.txt`.
