# AGENTS.md

Instructions and architectural contracts for AI coding agents working in the **StorKV** (`storkv`) repository.

## 1. Project Overview

- **Name:** StorKV (`storkv`), pronounced `/ˈstɔːrkviː/` (_stork-vee_).
- **Summary:** Embeddable and network-capable Log-Structured Merge-tree (LSM) key-value storage engine written in Rust, supporting scoped column families, MVCC snapshot isolation, optimistic concurrency control (OCC), TTL/FIFO retention, merge operators, resumable watch streams, asynchronous primary-replica WAL streaming, continuous backup/PITR, multi-protocol transports, and multi-language FFI bindings.
- **Toolchain & Licensing:**
  - Rust `1.100.0` (`rust-toolchain.toml`, components: `rustfmt`, `clippy`), Rust 2024 edition, MSRV `1.87` (`Cargo.toml`).
  - License: `AGPL-3.0`.

## 2. Repository & Crate Map

| Path                   | Responsibility                                                                                                                                                                                                                                                                       | Runtime / Async Boundary                                                                |
| :--------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :-------------------------------------------------------------------------------------- |
| `crates/storkv-core`   | Core LSM engine: WAL, MemTable, SSTable, Manifest, MVCC/OCC transactions, scopes, TTL, compaction, watch registry, replication registry, backup/restore, VFS (`PosixVfs`, `MemoryVfs`, `FaultInjectingVfs`), embedded `Tree`/`Batch` adapters, telemetry.                            | **Strictly synchronous.** Never add `tokio` or any async runtime dependency.            |
| `crates/storkv-proto`  | Wire protocol (`STORKPRT` v1), frame codec (`MAX_FRAME_SIZE = 64 MiB`), request/response codecs, `Consistency` levels, protocol errors.                                                                                                                                              | Synchronous byte codecs (`bytes::Bytes`).                                               |
| `crates/storkv-server` | Network daemon (`storkvd`), unified `Server::dispatch_request` core, transports (Native TCP/TLS/UDS, HTTP REST/JSON-RPC, WebSocket binary/JSON-RPC, gRPC), compatibility front ends (Redis RESP2/RESP3, Memcached ASCII, etcd v3 HTTP), replica runner, Prometheus/internal metrics. | Async (`tokio`, `axum`, `tonic`, `hyper`). Uses `spawn_blocking` for core engine calls. |
| `crates/storkv-client` | `AsyncClient` and `SyncClient`, URI parser (`storkv://`, `grpc://`, `http://`, `https://`, `ws://`, `wss://`, `unix://`), transport backends, client transactions.                                                                                                                   | Async (`tokio`) with a blocking wrapper in `SyncClient`.                                |
| `crates/storkv-sdk`    | Unified `StorkvPool` API abstracting embedded (`file://`, `embedded://`) and remote server URIs, typed `Collection<T>`, `RetryPolicy`, `WatchStream`, and opt-in per-scope LRU `ScopeCache`.                                                                                         | Async (`tokio`).                                                                        |
| `crates/storkv-ffi`    | C ABI (`cdylib` + `staticlib`) exposing `storkv-sdk` via `#[repr(C)]` types, opaque handles, thread-local error messages, and an internal multi-threaded Tokio runtime.                                                                                                              | Owns its own Tokio runtime; foreign callers never poll futures.                         |
| `crates/storkv-cli`    | `storkvctl` operator CLI (`health`, `doctor`, `backup`, `backup-status`, `backup-verify`, `restore`, `migrate` from Redis/JSONL, `ycsb`).                                                                                                                                            | CLI binary (`clap`).                                                                    |
| `crates/storkv-tui`    | `storkv-top` terminal dashboard (`ratatui` + `crossterm`) polling live server metrics and backup state.                                                                                                                                                                              | TUI binary.                                                                             |
| `xtask`                | Workspace build, test tier runner (`fast`, `pr`, `full`, `bindings`, `all`), `cbindgen` header verifier, local multi-transport `sandbox`, and release packager (`dist`).                                                                                                             | Build automation binary.                                                                |
| `bindings/`            | Foreign-language wrappers and test suites: `c/` (`storkv.h`), `cpp/` (`storkv.hpp`), `java/` (FFM API), `python/` (`ctypes`), `ts/` (`koffi`).                                                                                                                                       | Tested via `cargo xtask test bindings`.                                                 |
| `proto/`               | Canonical gRPC service and message definitions (`proto/storkv/v1/storkv.proto`).                                                                                                                                                                                                     | Compiled at build time via `protoc-bin-vendored`.                                       |
| `tests/`               | All Rust integration, property-based (`proptest`), golden binary format, chaos, transport, compatibility, FFI, CLI, SDK, server, and TUI test suites.                                                                                                                                | Wired via `[[test]]` targets in crate `Cargo.toml` files.                               |
| `docs/`                | User guides, concepts, operations, API/ABI references, SDK docs, and internal plans (`docs/internal/`).                                                                                                                                                                              | Markdown documentation.                                                                 |

## 3. Mandatory Code Style & Structural Rules

Follow `docs/internal/DECOMPOSITION_PLAN.md` and workspace lints strictly:

1. **Single Responsibility & File Size Cap (~250 Lines):**
   - Keep every source file focused on one clear responsibility and **under ~250 lines** (excluding generated protobuf files and `build.rs` output).
   - When adding features or fixing bugs pushes a file past ~250 lines, decompose it into focused child modules immediately.
2. **Rust 2018+ Module Layout (Zero `mod.rs`):**
   - Always use `foo.rs` alongside `foo/bar.rs`.
   - **Never create `mod.rs` files.** `[workspace.lints.clippy] mod_module_files = "warn"` is active, and CI runs Clippy with `-D warnings`.
3. **Public API & Documentation (`#![warn(missing_docs)]`):**
   - Parent modules re-export public child items (`pub use`) to maintain a clean crate-level API.
   - Document every public module, struct, enum, variant, field, trait, type alias, and function with `///` comments.
   - Document failure modes under an `# Errors` section on every fallible public function.
   - **ASD-STE100 Compliance:** All `///` and `//!` rustdoc code comments must be **100% compliant** with ASD-STE100 (Simplified Technical English: short sentences, active voice, approved vocabulary, one instruction per sentence). All Markdown documentation (`docs/*.md`, `README.md`, guides) must be at least **80% compliant** with ASD-STE100.
   - Do not use emojis in code comments, documentation, or CLI/TUI strings.
4. **No Panics in Production Code:**
   - Never use `.unwrap()`, `.expect()`, `panic!()`, `todo!()`, or `unimplemented!()` in library or binary production code (`crates/*/src/**`). Return structured errors (`StorkvError`, `SdkError`, `ClientError`, `ServerError`, `XtaskError`, or `StorkvStatus`).
5. **No Async Runtime in `storkv-core`:**
   - Keep `storkv-core` free of `tokio`, `async-std`, or `futures`. Concurrency in `storkv-core` uses `parking_lot`, `crossbeam-skiplist`, `crossbeam-channel`, and `std::sync`.
6. **Test Organization:**
   - Place Rust integration and unit-style module tests under `tests/` (using `tests/common.rs` `TempDb` or `storkv_core::vfs::MemoryVfs`) and foreign-language binding tests under `bindings/<lang>/`. Do not add large `#[cfg(test)]` blocks inside `crates/*/src/`.
7. **C ABI Layout & Ownership Rules (`crates/storkv-ffi`):**
   - Every public struct in `crates/storkv-ffi` must use `#[repr(C)]` with fields ordered from **smallest binary size/alignment to largest** to prevent implicit padding and preserve ABI stability (`StorkvBytes` = 16 bytes, align 8; `StorkvWatchEvent` = 48 bytes, align 8).
   - Every exported `extern "C"` function returns `StorkvStatus` and stores failure details in thread-local storage retrievable via `storkv_last_error_message`.
   - Input `StorkvSlice` buffers are borrowed only for the duration of the call. Output buffers (`StorkvBytes`, `StorkvEntries`, `StorkvString`) are owned by the caller and must be released via `storkv_bytes_free`, `storkv_entries_free`, and `storkv_string_free`.

## 4. Core System & Security Invariants

Consult `SPEC.md`, `FORMAT.md`, and `docs/internal/SECURITY_AUDIT.md` before modifying storage, replication, wire codecs, or network listeners:

### 4.1 On-Disk & Wire Binary Format (`FORMAT.md`)

- **Endianness & Checksums:** Multi-byte integers are Little-Endian (`LE`), except `inverted_tag` (`u64 BE = !((seq << 8) | value_type)`) in Internal Keys so raw byte comparison sorts by `user_key` ascending and `sequence_number` descending. All checksums use **CRC32C** (Castagnoli).
- **Size Limits:** `MAX_KEY_LEN = 65,535` bytes (`u16::MAX`), `MAX_VAL_LEN = 268,435,456` bytes (`256 MiB`), `MAX_FRAME_SIZE = 64 MiB`, `MAX_SCOPES = 1024`.
- **Golden Fixtures:** `tests/format/fixtures/v0_1_0.{mnf,prt,sst,wal}` verify backward-compatible binary decoding. Never alter v1 binary layouts in a breaking way.
- **Decompression Caps:** WAL and SSTable decoders must validate `uncompressed_len <= MAX_VAL_LEN` before/during LZ4 and Zstd decompression to prevent decompression bombs.

### 4.2 Storage Engine, MVCC & Compaction (`SPEC.md`)

- **Shared WAL, Isolated Scopes:** All scopes share one segmented WAL (`wal/%016x.wal`) for atomic cross-scope commits, while each scope maintains independent `MemTable`s and `SSTable` levels (`scopes/%08x/`). Cross-scope batches execute with the strictest `Durability` of all touched scopes (`StrictFsync` overrides `Buffered`).
- **OCC Transactions:** `tx.get()` and `tx.scan()` read the transaction's local `WriteSet` first, then merge storage layers at `seq <= tx.read_seq`, filtering expired keys (`expires_at_ms != 0 && expires_at_ms <= now_ms`). At `tx.commit()`, every key in `ReadSet` must satisfy `latest_seq <= tx.read_seq` or return `StorkvError::WriteConflict`. Blind writes do not check `ReadSet`.
- **Compaction & GC Floor:** In v1, compaction is explicitly triggered (`Db::compact`, `POST /v1/admin/compact`, or gRPC `Compact`), not run by an autonomous background compaction thread. Compaction must preserve versions down to `gc_safe_seq = min(oldest_active_tx_read_seq, oldest_pinned_named_snapshot_seq, oldest_active_watch_cursor_seq)`.
- **WAL-Pinning Watermarks:** Idle scopes holding old unflushed WAL segments are flushed automatically when `max_wal_pin_bytes` (`64 MiB`) or `max_wal_pin_age` (`60s`) is exceeded.

### 4.3 Transport Parity, Bounds & Security (`SECURITY_AUDIT.md`)

- **Unified Dispatch:** Native, HTTP REST/JSON-RPC, WebSocket, and gRPC paths must execute through `Server::dispatch_request` so admin gating (`--strict-admin`), replica write rejection (`NotPrimary`), telemetry, OCC validation, and watch broadcasts stay identical across transports.
- **Scopes Are Not Tenants:** Scopes isolate storage layout, not access control. Never introduce partial/inconsistent per-scope auth assumptions.
- **Bounded Streams & Replay:** Watch and replication registries cap subscriber queues at `1024` items and total subscribers at `1024` (`TooManySubscribers`), evicting slow subscribers non-blockingly. Cursor replays are capped at `1024` items and `16 MiB` (`ReplayBudgetExceeded`).
- **Path Confinement & Secrets:** Backup/restore must reject path traversal (`..`, absolute paths, drive prefixes) and symlinks (`crates/storkv-core/src/backup/manifest/paths.rs`). Admin token checks must use constant-time comparison (`tokens_match`).
- **Known Open Audit Items (`docs/internal/SECURITY_AUDIT.md`):**
  - `F-01`: Compatibility listeners (Redis, Memcached, etcd) currently bypass replica write rejection (`NotPrimary`) by calling `Db` directly instead of enforcing the replica role guard.
  - `F-02` / `F-03`: Expired replication cursor recovery sends `ReplicationSnapshot` before `Empty` (causing a client protocol error retry loop), and `replication_snapshot` / `apply_replication_snapshot` omit `expires_at_ms` and stale key/scope deletion.
  - `F-04`: `storkv-sdk` `ScopeCache` does not track server-side `expires_at_ms` deadlines.
  - When modifying replication, compatibility protocols, or SDK caching, ensure fixes address or respect these invariants.

## 5. Code Generation & Synchronized Artifacts

1. **C Header (`bindings/c/storkv.h`):**
   - Generated from `crates/storkv-ffi` using `crates/storkv-ffi/cbindgen.toml`.
   - Whenever you modify any `pub` type, constant, or `extern "C"` function in `crates/storkv-ffi`, regenerate the header and verify it:
     ```bash
     cargo run -p storkv-ffi --example generate_header
     cargo xtask verify-headers
     ```
2. **Protobuf / gRPC (`proto/storkv/v1/storkv.proto`):**
   - Compiled automatically by `crates/storkv-client/build.rs` and `crates/storkv-server/build.rs` using `protoc-bin-vendored` (no system `protoc` binary required).
   - Any RPC or message change in `storkv.proto` must be mirrored across `storkv-proto`, `crates/storkv-server/src/transports/grpc/`, `crates/storkv-client/src/backend/grpc/`, and `tests/transports/grpc.rs`.

## 6. Inspection, Testing & Verification Commands

### Repository Inspection

- **Structure & Line Counts** (use to audit file sizes against the ~250-line rule):

```bash
project-structure . --minimize ascii --count
```

- **Structured Diff (including untracked files):**

```bash
better-git-diff --ai -u
```

### `cargo xtask` Test Tiers

- **`cargo xtask test fast`**: Runs `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` (matches GitHub `push` to `main` CI).
- **`cargo xtask test pr`**: Runs `fast` checks, `cargo test --workspace -- --skip chaos`, `cargo xtask verify-headers`, and `cargo doc --workspace --no-deps` (matches GitHub PR CI).
- **`cargo xtask test full`**: Runs `pr` plus `cargo build --workspace --release`.
- **`cargo xtask test bindings [--with-server | --live-uri <URI>]`**: Builds `storkv-ffi` and runs C, C++, Python, Java, and TypeScript binding test suites.
- **`cargo xtask test all [--with-server | --live-uri <URI>] [--proptest-cases <N>]`**: Runs formatting, clippy, header verification, rustdoc, all Rust workspace tests (including chaos and `--ignored` live tests), and all five language binding suites in one pass.
- **`cargo xtask verify-headers`**: Verifies `bindings/c/storkv.h` on disk matches freshly generated `cbindgen` output.
- **`cargo xtask sandbox [--tls] [--admin-token <TOKEN>] [--strict-admin]`**: Starts a local server on loopback ports `7878` (native), `7879` (HTTP), `7880` (WS), `7881` (gRPC), `7882` (Redis), and `7883` (Memcached).
- **`cargo xtask dist [--target <TRIPLE>] [--strict-sdks] [--skip-archive]`**: Builds release binaries, regenerates `storkv.h`, packages SDK artifacts, and writes release archives and SHA-256 checksums to `dist/`.

### Cross-Platform & WSL2 Verification

- **Linux / macOS:** Run `cargo xtask` commands directly in your shell.
- **Windows Host -> WSL2:** When developing on Windows, POSIX-specific behavior (Unix domain sockets `unix://`, `0700`/`0600` socket permissions, `fs2` file locks, POSIX `rename`/`fsync`, `scripts/*.sh`) must also be verified inside WSL2 from the repository root. `wsl.exe` inherits the current working directory across drives automatically:

```bash
# Default (works from Git Bash, PowerShell, and CMD on any drive/path)
wsl -e bash -lc "cargo xtask test pr"

# Explicit --cd override in Git Bash if ~/.bashrc changes the startup directory
wsl --cd "$(pwd -W)" -e bash -lc "cargo xtask test pr"

# Explicit --cd override in PowerShell
wsl --cd "$PWD" -e bash -lc "cargo xtask test pr"
```

### Pre-Completion Checklist

Before finishing any task:

1. Run `project-structure . --minimize ascii --count` to confirm no source file violates the ~250-line cap and no `mod.rs` files exist.
2. Verify documentation compliance: 100% ASD-STE100 compliance for rustdoc code comments and at least 80% ASD-STE100 compliance for Markdown documentation.
3. Run `cargo xtask test pr` (or `cargo xtask test all` when touching `storkv-ffi`, `bindings/`, `storkv-proto`, or chaos/recovery paths).
4. Run `better-git-diff --ai -u` to review all modified and untracked files.
