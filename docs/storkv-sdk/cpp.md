# Use StorKV from C++

The C++ binding is a header-only RAII layer over the C ABI. Include `bindings/cpp/storkv.hpp` and link the `storkv-ffi` library. Each handle is a move-only object. The destructor releases the handle.

Read the [C ABI reference](../reference/ffi-c-abi.md) for the functions behind the wrapper.

## Prerequisites

- A C++20 compiler with `std::span`, `std::optional`, and `std::exchange`
- Rust 1.87 or later, to build the FFI library

Build the shared library:

```bash
cargo build -p storkv-ffi
```

The build writes one file to `target/debug`:

| Platform | File |
| :--- | :--- |
| Windows | `storkv_ffi.dll` |
| Linux | `libstorkv_ffi.so` |
| macOS | `libstorkv_ffi.dylib` |

Set the loader path at run time:

| Platform | Command |
| :--- | :--- |
| Windows | Add `target\debug` to `PATH` |
| Linux | `export LD_LIBRARY_PATH="$PWD/target/debug:$LD_LIBRARY_PATH"` |
| macOS | `export DYLD_LIBRARY_PATH="$PWD/target/debug:$DYLD_LIBRARY_PATH"` |

## Compile a program

```bash
c++ -std=c++20 -I bindings/cpp program.cpp -L target/debug -lstorkv_ffi -o target/cpp-program
```

The header includes the C declarations. You do not need to include `bindings/c/storkv.h` yourself.

## Quickstart

```cpp
#include "storkv.hpp"

#include <cstdio>
#include <string>

int main() {
    storkv::Pool pool = storkv::Pool::connect("file://./data/cpp-app");
    storkv::Scope scope = pool.scope("default");

    scope.put("greeting", "hello");
    std::optional<std::vector<std::uint8_t>> value = scope.get("greeting");
    if (value.has_value()) {
        std::printf("%.*s\n",
                    static_cast<int>(value->size()),
                    reinterpret_cast<const char *>(value->data()));
    }

    scope.put_with_ttl("session", "live", 900000);
    std::optional<std::vector<std::uint8_t>> touched =
        scope.get_and_touch("session", 900000);
    (void)touched;

    scope.put("scan:1", "one");
    scope.put("scan:2", "two");
    std::vector<storkv::detail::Entry> entries = scope.scan_prefix("scan:");
    std::printf("entries: %zu\n", entries.size());

    {
        storkv::Transaction tx = pool.begin();
        tx.put(scope, "tx:key", "value");
        tx.commit();
    }

    {
        storkv::Watch watch = scope.watch_prefix("events:");
        scope.put("events:1", "payload");
        std::optional<storkv::Event> event = watch.next(5000);
        if (event.has_value()) {
            std::printf("kind: %d\n", static_cast<int>(event->kind));
        }
    }

    scope.close();
    pool.close();
    return 0;
}
```

The `Pool`, `Scope`, `Transaction`, and `Watch` objects use the same names as the Rust SDK. The C++ layer adds C++ containers and error exceptions.

## Types

| C++ type | Purpose |
| :--- | :--- |
| `storkv::Pool` | Pooled connection to an embedded or networked backend |
| `storkv::Scope` | Handle to one scope |
| `storkv::Transaction` | Explicit OCC transaction |
| `storkv::Watch` | Stream of change events |
| `storkv::Options` | Pool configuration |
| `storkv::Event` | One decoded watch event |
| `storkv::EventKind` | `Put`, `Delete`, or `DropScope` |
| `storkv::Error` | Exception with a `status()` method |
| `storkv::detail::Entry` | Pair of key bytes and value bytes |

`Options` has these fields:

| Field | Default | Effect |
| :--- | :--- | :--- |
| `health_check` | `true` | Probe idle clients before reuse |
| `max_connections` | 10 | Maximum simultaneous connections |
| `min_connections` | 1 | Connections opened during connect |
| `acquire_timeout_ms` | 0 | Zero selects the SDK default |
| `idle_timeout_ms` | 0 | Zero disables idle expiry |

## RAII and move rules

Every handle class has these properties:

- The copy constructor is deleted.
- The move constructor transfers the native pointer.
- The destructor calls the matching close function.
- `close()` releases the handle early.
- `operator bool` reports whether the handle is open.

Use `std::move` when you pass a handle to another owner:

```cpp
storkv::Pool pool = storkv::Pool::connect("file://./data/cpp-app");
storkv::Pool moved = std::move(pool);
if (!pool) {
    std::puts("the source handle is empty");
}
```

A moved-from handle does nothing in its destructor. This rule prevents a double close.

Do not call `close()` and then use the handle. The wrapper throws `storkv::Error` with status `InvalidArgument` when a method sees a null handle.

## Values and ownership

The wrapper copies each output buffer into `std::vector<std::uint8_t>` and then frees the native buffer. You do not call `storkv_bytes_free` or `storkv_entries_free` in C++ code.

Input values accept two forms:

- `std::string_view` for text
- `std::span<const std::uint8_t>` for bytes

A string view borrows the bytes. Keep the string alive during the call. The wrapper copies the bytes into the C ABI slice.

A missing key returns `std::optional` with no value. It does not throw an exception.

## Transactions

`Pool::begin()` starts an explicit transaction. Buffer writes with `put`, `put_with_ttl`, and `erase`. Call `commit` or `rollback`.

```cpp
storkv::Transaction tx = pool.begin();
tx.put(scope, "counter", "1");
tx.commit();
```

`commit` consumes the handle. A destructor after `commit` does nothing. A destructor before `commit` rolls back the transaction.

C++ callers must write their own retry loop. The C ABI has `storkv_tx_run` for automatic retries, but the header does not wrap it.

## Watches

`Scope::watch_prefix` returns a `Watch`. `Watch::next` takes a timeout in milliseconds and returns `std::optional<Event>`.

A timeout returns an empty optional. Other failures throw `storkv::Error`.

An `Event` holds these fields:

| Field | Type | Meaning |
| :--- | :--- | :--- |
| `kind` | `EventKind` | `Put`, `Delete`, or `DropScope` |
| `scope_id` | `std::uint32_t` | Scope that changed |
| `seq` | `std::uint64_t` | Commit sequence number |
| `key` | `std::vector<std::uint8_t>` | Key that changed |
| `value` | `std::vector<std::uint8_t>` | New value, empty for a delete |

Close the watch before the scope. The destructor of `Watch` calls `storkv_watch_close`.

## Errors

A non-zero status becomes a `storkv::Error` exception. The exception keeps the status code and the message text.

```cpp
try {
    scope.get("missing");
} catch (const storkv::Error &error) {
    std::fprintf(stderr, "status %d: %s\n", static_cast<int>(error.status()), error.what());
}
```

Read [Error codes](../reference/error-codes.md) for the server-side wire codes.

## Test example

The workspace test is `bindings/cpp/test_cpp_ffi.cpp`. This smaller test covers CRUD, a transaction, a watch, and one error:

```cpp
#include "storkv.hpp"

#include <cstdio>
#include <string>
#include <vector>

namespace {

int failures = 0;

void check(bool condition, const char *expression, int line) {
    if (!condition) {
        std::fprintf(stderr, "FAIL line %d: %s\n", line, expression);
        failures += 1;
    }
}

#define CHECK(expression) check((expression), #expression, __LINE__)

std::string as_text(const std::vector<std::uint8_t> &bytes) {
    return std::string(reinterpret_cast<const char *>(bytes.data()), bytes.size());
}

}  // namespace

int main() {
    storkv::Pool pool = storkv::Pool::connect("file://./data/cpp-test");
    storkv::Scope scope = pool.scope("default");

    scope.put("cpp:greeting", "hello");
    auto greeting = scope.get("cpp:greeting");
    CHECK(greeting.has_value());
    CHECK(greeting.has_value() && as_text(*greeting) == "hello");
    CHECK(scope.exists("cpp:greeting"));
    scope.erase("cpp:greeting");
    CHECK(!scope.exists("cpp:greeting"));

    scope.put_with_ttl("cpp:session", "live", 300);
    auto touched = scope.get_and_touch("cpp:session", 5000);
    CHECK(touched.has_value() && as_text(*touched) == "live");

    scope.put("cpp:scan:1", "one");
    scope.put("cpp:scan:2", "two");
    auto entries = scope.scan_prefix("cpp:scan:");
    CHECK(entries.size() == 2);

    {
        storkv::Transaction tx = pool.begin();
        tx.put(scope, "cpp:tx", "value");
        auto inside = tx.get(scope, "cpp:tx");
        CHECK(inside.has_value() && as_text(*inside) == "value");
        tx.commit();
    }
    auto committed = scope.get("cpp:tx");
    CHECK(committed.has_value() && as_text(*committed) == "value");

    {
        storkv::Watch watch = scope.watch_prefix("cpp:events:");
        scope.put("cpp:events:1", "payload");
        auto event = watch.next(5000);
        CHECK(event.has_value());
        if (event.has_value()) {
            CHECK(event->kind == storkv::EventKind::Put);
            CHECK(as_text(event->key) == "cpp:events:1");
            CHECK(as_text(event->value) == "payload");
        }
    }

    try {
        storkv::Scope closed = pool.scope("default");
        closed.close();
        closed.get("key");
        CHECK(false);
    } catch (const storkv::Error &error) {
        CHECK(error.status() == InvalidArgument);
    }

    CHECK(!pool.is_closed());
    scope.close();
    pool.close();

    if (failures == 0) {
        std::puts("C++ checks passed");
        return 0;
    }
    return 1;
}
```

Compile and run:

```bash
c++ -std=c++20 -I bindings/cpp bindings/cpp/test_cpp_ffi.cpp -L target/debug -lstorkv_ffi -o target/cpp-ffi-test
./target/cpp-ffi-test
```

Run against a server:

```bash
STORKV_TEST_URI=storkv://127.0.0.1:7878 ./target/cpp-ffi-test
```

The workspace test skips the networked section when `STORKV_TEST_URI` is unset.

## Related guides

- [C ABI reference](../reference/ffi-c-abi.md)
- [C guide](c.md)
- [Connection URIs](../getting-started/connection-uris.md)
- [Error codes](../reference/error-codes.md)
