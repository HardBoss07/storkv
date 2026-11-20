# Testing

StorKV uses `cargo xtask test` for verification. The command has several tiers. Use the smallest tier that covers your change, and use `cargo xtask test all` for complete verification.

## Test tiers

| Tier | What it runs |
| --- | --- |
| `fast` | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`. |
| `pr` | The fast checks, the generated C header check, and a rustdoc build. |
| `full` | The `pr` checks and a release build of the workspace. |
| `bindings` | The FFI library build and the C, C++, Python, Java, and TypeScript binding suites. |
| `all` | The complete verification suite in one pass. |

Run a tier from the workspace root:

```bash
cargo xtask test fast
cargo xtask test pr
cargo xtask test full
cargo xtask test bindings
cargo xtask test all
```

## The all tier

`cargo xtask test all` is the single command for full verification. It runs each step once, in this order:

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. The generated C header check
4. `cargo doc --workspace --no-deps`
5. `cargo test --workspace` with `PROPTEST_CASES` set
6. `cargo build -p storkv-ffi`
7. The C, C++, Python, Java, and TypeScript binding suites

The Rust workspace test pass includes unit tests, integration test targets such as `tests/chaos.rs` and `tests/mvcc.rs`, and doctests. The command does not repeat the same test pass.

The default `PROPTEST_CASES` value is `2000`. Override the case count with:

```bash
cargo xtask test all --proptest-cases 256
```

When `--live-uri` or `--with-server` is set, the command also runs the ignored workspace tests. The binding suites use the same live server settings:

```bash
cargo xtask test all --live-uri storkv://127.0.0.1:7878
cargo xtask test all --with-server
```

The `bindings` and `all` tiers skip a language when its toolchain is not on the `PATH`.

## Binding toolchains

The binding suites need these tools when the matching language is installed:

- C: `gcc` or `clang` on Unix-like systems, `gcc` or the MSVC compiler on Windows.
- C++: `g++` or `clang++` on Unix-like systems, `g++` or the MSVC compiler on Windows.
- Java: a JDK with `javac` and `java`.
- Python: Python 3 with the standard `unittest` module.
- TypeScript: Node.js and `npm`.

On Windows with MinGW, the C and C++ tests link directly against `target/debug/storkv_ffi.dll`. The C++ test also uses `-static-libstdc++` and `-static-libgcc` so the test binary can start without the MinGW runtime DLLs on `PATH`.

The binding tests receive `PATH`, `STORKV_FFI_LIB_DIR`, and `STORKV_LIB_DIR` pointing at `target/debug`. They also receive `STORKV_FFI_LIB` pointing at the built shared library.

## Continuous integration

The `CI` workflow runs the `fast` tier on a push to `main` and the `pr` tier on a pull request. The `Full verification` workflow checks the C header, runs the `full` tier, builds the release bundle, and runs the binding tier.
