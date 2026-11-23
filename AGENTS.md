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
   - Write `///` rustdoc comments and `# Errors` sections **while writing the code**, not as a cleanup pass after `cargo doc` fails.
   - Document every public module, struct, enum, variant, field, trait, type alias, and function with `///` comments, and document failure modes under `# Errors` on every fallible public function.
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

## 4. Subsystem Specifications & Audit Invariants

Before modifying binary codecs, storage/compaction, replication, transport listeners, or SDK caching, read the relevant canonical file directly rather than guessing invariants:

- **On-Disk & Wire Binary Format:** Read `FORMAT.md` (LE integers, `inverted_tag` BE sorting, CRC32C checksums, `MAX_KEY_LEN`/`MAX_VAL_LEN`/`MAX_FRAME_SIZE`/`MAX_SCOPES` bounds, decompression bomb checks, and `tests/format/fixtures/v0_1_0.*` backward compatibility).
- **Storage Engine, MVCC, Compaction & Transport Parity:** Read `SPEC.md` (shared WAL across scopes, OCC `ReadSet`/`WriteSet` validation, explicit compaction `gc_safe_seq`, WAL-pinning watermarks, `Server::dispatch_request` unified dispatch, and bounded stream/replay caps).
- **Open Security Audit Findings:** Read `docs/internal/SECURITY_AUDIT.md` before touching compatibility protocols (`F-01` replica write bypass in Redis/Memcached/etcd listeners), replication cursor recovery/snapshots (`F-02`/`F-03`), or SDK `ScopeCache` TTL tracking (`F-04`).

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

### Fast Iteration Loop (Use During Coding)

- **Never run `cargo xtask` during edit-compile-fix cycles.** Scope checks to the touched crate and use short diagnostics to keep output compact:

```bash
cargo check -p <crate> --tests --message-format=short
cargo clippy -p <crate> --tests --message-format=short -- -D warnings
cargo test -p <crate> --test <target_name> <test_filter>
```

### `cargo xtask` Final Verification Tiers

Run workspace-wide `cargo xtask` **only once** after the crate-scoped loop passes:

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
