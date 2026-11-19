# Use StorKV from C

The C binding calls the `storkv-ffi` library through the generated header `bindings/c/storkv.h`. Every function returns a `StorkvStatus` value. You own every output buffer and must free it.

Read the [C ABI reference](../reference/ffi-c-abi.md) for every signature, struct layout, and ownership rule.

## Prerequisites

- A C99 compiler
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

The crate also builds a static library with the same base name. A static build does not need a runtime library path.

Set the loader path at run time:

| Platform | Command |
| :--- | :--- |
| Windows | Add `target\debug` to `PATH` |
| Linux | `export LD_LIBRARY_PATH="$PWD/target/debug:$LD_LIBRARY_PATH"` |
| macOS | `export DYLD_LIBRARY_PATH="$PWD/target/debug:$DYLD_LIBRARY_PATH"` |

## Compile a program

Include the header from `bindings/c` and link the library:

```bash
cc -std=c99 -I bindings/c program.c -L target/debug -lstorkv_ffi -o target/c-program
```

On Windows with MSVC, link the import library:

```bat
cl /I bindings\c program.c /link /LIBPATH:target\debug storkv_ffi.dll.lib
```

## Quickstart

This program opens an embedded pool, writes and reads a value, scans a prefix, runs a transaction, and watches a key:

```c
#include "storkv.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

static StorkvSlice slice_of(const char *text) {
    StorkvSlice slice;
    slice.data = (const uint8_t *)text;
    slice.len = strlen(text);
    return slice;
}

int main(void) {
    StorkvPoolHandle *pool = NULL;
    StorkvScopeHandle *scope = NULL;
    StorkvBytes value;
    StorkvEntries entries;
    StorkvWatchEvent event;
    StorkvWatchHandle *watch = NULL;
    StorkvTxHandle *tx = NULL;

    if (storkv_pool_open("file://./data/c-app", NULL, &pool) != Ok) {
        return 1;
    }
    if (storkv_scope_open(pool, "default", &scope) != Ok) {
        storkv_pool_close(pool);
        return 1;
    }

    if (storkv_put(scope, slice_of("greeting"), slice_of("hello")) != Ok) {
        return 1;
    }
    if (storkv_get(scope, slice_of("greeting"), &value) != Ok) {
        return 1;
    }
    printf("%.*s\n", (int)value.len, (const char *)value.data);
    storkv_bytes_free(value);

    if (storkv_put_with_ttl(scope, slice_of("session"), slice_of("live"), 900000) != Ok) {
        return 1;
    }
    if (storkv_get_and_touch(scope, slice_of("session"), 900000, &value) != Ok) {
        return 1;
    }
    storkv_bytes_free(value);

    if (storkv_put(scope, slice_of("scan:1"), slice_of("one")) != Ok) {
        return 1;
    }
    if (storkv_put(scope, slice_of("scan:2"), slice_of("two")) != Ok) {
        return 1;
    }
    if (storkv_scan_prefix(scope, slice_of("scan:"), &entries) != Ok) {
        return 1;
    }
    for (size_t index = 0; index < entries.len; index += 1) {
        StorkvEntry entry = entries.entries[index];
        printf("%.*s\n", (int)entry.key.len, (const char *)entry.key.data);
    }
    storkv_entries_free(entries);

    if (storkv_tx_begin(pool, &tx) != Ok) {
        return 1;
    }
    if (storkv_tx_put(tx, scope, slice_of("tx:key"), slice_of("value")) != Ok) {
        return 1;
    }
    if (storkv_tx_commit(tx) != Ok) {
        return 1;
    }

    if (storkv_watch_prefix(scope, slice_of("events:"), &watch) != Ok) {
        return 1;
    }
    if (storkv_put(scope, slice_of("events:1"), slice_of("payload")) != Ok) {
        return 1;
    }
    if (storkv_watch_next(watch, &event, 5000) == Ok) {
        printf("%.*s\n", (int)event.key.len, (const char *)event.key.data);
        storkv_bytes_free(event.key);
        storkv_bytes_free(event.value);
    }
    storkv_watch_close(watch);

    storkv_scope_close(scope);
    storkv_pool_close(pool);
    return 0;
}
```

The `%.*s` format prints a byte buffer that has no NUL terminator. Cast `len` to `int` for the precision field.

## Ownership rules

The library allocates each output buffer on the Rust heap. You must return it with the matching free function:

| Output | Free with |
| :--- | :--- |
| `StorkvBytes` from `storkv_get` or `storkv_get_and_touch` | `storkv_bytes_free` |
| `StorkvEntry.key` and `StorkvEntry.value` | `storkv_entries_free` for the parent array |
| `StorkvEntries` from a scan | `storkv_entries_free` |
| `StorkvWatchEvent.key` and `StorkvWatchEvent.value` | `storkv_bytes_free` for each field |
| `StorkvString` from `storkv_last_error_message` | `storkv_string_free` |

Do not call `free` on an FFI buffer. Do not free one buffer twice. Do not read a buffer after you free it.

An input `StorkvSlice` is borrowed only for the duration of one call. The library copies the bytes before the call returns. You can change or release the input buffer after the call.

A `StorkvSlice` with a null `data` pointer and a zero `len` is an empty value. In `storkv_scan_range`, a null pointer means an open bound.

## Handle rules

Follow this close order:

1. Close a watch handle with `storkv_watch_close`.
2. Commit or roll back a transaction.
3. Close a scope handle with `storkv_scope_close`.
4. Close the pool with `storkv_pool_close`.

A closed scope handle rejects later calls. The function returns `InvalidArgument` or `Scope` and records a message.

`storkv_pool_close` releases idle network clients. A scope handle that came from the pool stays valid, but you should close it first.

## Errors

Check every status value:

```c
StorkvBytes value;
StorkvStatus status = storkv_get(scope, slice_of("key"), &value);
if (status != Ok) {
    StorkvString message = storkv_last_error_message();
    fprintf(stderr, "storkv_get failed: %.*s\n", (int)message.len, message.data);
    storkv_string_free(message);
}
```

The last error message is thread-local. Read it on the thread that made the failed call. A successful data call clears the message for that thread.

## Transactions

`storkv_tx_begin` creates an explicit transaction. Buffer writes with `storkv_tx_put`, `storkv_tx_put_with_ttl`, and `storkv_tx_delete`. Call `storkv_tx_commit` or `storkv_tx_rollback` to release the handle.

The automatic helper retries write conflicts:

```c
static StorkvStatus increment(StorkvTxHandle *tx, void *user) {
    StorkvScopeHandle *scope = (StorkvScopeHandle *)user;
    StorkvBytes current;
    uint64_t next = 0;
    StorkvStatus status = storkv_tx_get(tx, scope, slice_of("hits"), &current);
    if (status != Ok) {
        return status;
    }
    if (current.len > 0) {
        next = strtoull((const char *)current.data, NULL, 10);
    }
    storkv_bytes_free(current);
    next += 1;
    char buffer[32];
    snprintf(buffer, sizeof(buffer), "%llu", (unsigned long long)next);
    return storkv_tx_put(tx, scope, slice_of("hits"), slice_of(buffer));
}

StorkvStatus status = storkv_tx_run(pool, NULL, increment, scope, NULL);
```

`storkv_tx_run` passes a borrowed transaction handle to the callback. The callback must not commit or roll back. The helper commits when the callback returns `Ok` and rolls back on any other status.

A null `config` uses the default retry policy.

## Watches

`storkv_watch_prefix` creates a stream. `storkv_watch_next` waits for one event. A `timeout_ms` value of zero waits without a limit.

| Return | Meaning |
| :--- | :--- |
| `Ok` | One event is in `out_event` |
| `Timeout` | No event arrived before the deadline |
| `Watch` | The stream ended |
| Other | A failure with a message |

Free `event.key` and `event.value` after each successful call. Close the watch handle when you stop.

## Test example

The workspace test is `bindings/c/test_c_ffi.c`. This smaller test covers CRUD, a scan, a transaction, a watch, and a null-pointer error:

```c
#include "storkv.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int failures = 0;

#define CHECK(expression)                                                       \
    do {                                                                        \
        if (!(expression)) {                                                    \
            fprintf(stderr, "FAIL %s:%d: %s\n", __FILE__, __LINE__, #expression); \
            failures += 1;                                                      \
        }                                                                       \
    } while (0)

static StorkvSlice text(const char *value) {
    StorkvSlice slice;
    slice.data = (const uint8_t *)value;
    slice.len = strlen(value);
    return slice;
}

int main(void) {
    StorkvPoolHandle *pool = NULL;
    StorkvScopeHandle *scope = NULL;
    StorkvBytes value;
    StorkvEntries entries;
    StorkvTxHandle *tx = NULL;
    StorkvWatchHandle *watch = NULL;
    StorkvWatchEvent event;
    int32_t flag = 0;

    CHECK(storkv_get(NULL, text("key"), &value) == NullPointer);

    CHECK(storkv_pool_open("file://./data/c-test", NULL, &pool) == Ok);
    CHECK(storkv_scope_open(pool, "default", &scope) == Ok);

    CHECK(storkv_put(scope, text("c:greeting"), text("hello")) == Ok);
    CHECK(storkv_get(scope, text("c:greeting"), &value) == Ok);
    CHECK(value.len == 5);
    CHECK(memcmp(value.data, "hello", 5) == 0);
    storkv_bytes_free(value);

    CHECK(storkv_exists(scope, text("c:greeting"), &flag) == Ok);
    CHECK(flag == 1);
    CHECK(storkv_delete(scope, text("c:greeting")) == Ok);
    CHECK(storkv_exists(scope, text("c:greeting"), &flag) == Ok);
    CHECK(flag == 0);

    CHECK(storkv_put_with_ttl(scope, text("c:session"), text("live"), 300) == Ok);
    CHECK(storkv_get_and_touch(scope, text("c:session"), 5000, &value) == Ok);
    storkv_bytes_free(value);

    CHECK(storkv_put(scope, text("c:scan:1"), text("one")) == Ok);
    CHECK(storkv_put(scope, text("c:scan:2"), text("two")) == Ok);
    CHECK(storkv_scan_prefix(scope, text("c:scan:"), &entries) == Ok);
    CHECK(entries.len == 2);
    storkv_entries_free(entries);

    CHECK(storkv_tx_begin(pool, &tx) == Ok);
    CHECK(storkv_tx_put(tx, scope, text("c:tx"), text("value")) == Ok);
    CHECK(storkv_tx_commit(tx) == Ok);
    CHECK(storkv_get(scope, text("c:tx"), &value) == Ok);
    storkv_bytes_free(value);

    CHECK(storkv_watch_prefix(scope, text("c:events:"), &watch) == Ok);
    CHECK(storkv_put(scope, text("c:events:1"), text("payload")) == Ok);
    memset(&event, 0, sizeof(event));
    CHECK(storkv_watch_next(watch, &event, 5000) == Ok);
    CHECK(event.kind == Put);
    storkv_bytes_free(event.key);
    storkv_bytes_free(event.value);
    storkv_watch_close(watch);

    storkv_scope_close(scope);
    storkv_pool_close(pool);

    if (failures == 0) {
        printf("C checks passed\n");
        return 0;
    }
    return 1;
}
```

Compile and run the workspace test:

```bash
cc -std=c99 -I bindings/c bindings/c/test_c_ffi.c -L target/debug -lstorkv_ffi -o target/c-ffi-test
./target/c-ffi-test
```

Run the same test against a server:

```bash
STORKV_TEST_URI=storkv://127.0.0.1:7878 ./target/c-ffi-test
```

The test skips the networked section when `STORKV_TEST_URI` is unset.

## Related guides

- [C ABI reference](../reference/ffi-c-abi.md)
- [C++ guide](cpp.md)
- [Connection URIs](../getting-started/connection-uris.md)
- [Error codes](../reference/error-codes.md)
