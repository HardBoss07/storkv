# C ABI reference

The `storkv-ffi` crate exposes the StorKV SDK through a C ABI. It builds a `cdylib` and a `staticlib`. Every data function returns a `StorkvStatus` value. The caller owns every returned buffer.

The generated header is `bindings/c/storkv.h`. A build of `storkv-ffi` regenerates it from the Rust source. The C header is the contract for C, C++, Java, TypeScript, and Python.

## Build the library

```bash
cargo build -p storkv-ffi
```

| Platform | Shared library | Static library |
| :--- | :--- | :--- |
| Windows | `storkv_ffi.dll` | `storkv_ffi.lib` |
| Linux | `libstorkv_ffi.so` | `libstorkv_ffi.a` |
| macOS | `libstorkv_ffi.dylib` | `libstorkv_ffi.a` |

The files are in `target/debug` after a debug build. A release build writes them to `target/release`.

## ABI rules

The ABI obeys these rules:

1. Every struct uses `#[repr(C)]`.
2. Fields are ordered from smallest binary field to largest.
3. Enum values use a 32-bit integer representation.
4. Opaque handles are pointers to incomplete types.
5. The library never unwinds a panic across the boundary.
6. Strings are NUL-terminated only when a function documents a string type.

The field order convention is:

```text
1-byte field -> 2-byte field -> 4-byte field -> 8-byte field -> 16-byte compound -> larger compound
```

This order keeps the natural alignment simple. Padding appears only where the C compiler requires it. A new field goes at the end of its size class.

Do not reorder fields. Do not change a field type in place. A layout change requires a new function or a new struct name.

## Scalar types

| C type | Rust type | Size | Alignment |
| :--- | :--- | :--- | :--- |
| `int32_t` | `i32` | 4 | 4 |
| `uint32_t` | `u32` | 4 | 4 |
| `uint64_t` | `u64` | 8 | 8 |
| `uintptr_t` | `usize` | 8 on a 64-bit target | 8 |
| `bool` | `bool` | 1 | 1 |
| `char *` | `*mut c_char` | 8 on a 64-bit target | 8 |

All sizes in this page use a 64-bit target. A 32-bit target changes every pointer and `uintptr_t` field to 4 bytes.

## Struct layouts

### `StorkvStatus`

A status code. The type is `#[repr(i32)]`.

| Field | Offset | Size |
| :--- | :--- | :--- |
| Value | 0 | 4 |

Size: 4. Alignment: 4.

### `StorkvEventKind`

A watch event kind. The type is `#[repr(i32)]`.

| Field | Offset | Size |
| :--- | :--- | :--- |
| Value | 0 | 4 |

Size: 4. Alignment: 4.

### `StorkvBytes`

An owned byte buffer.

| Field | Offset | Size | Meaning |
| :--- | :--- | :--- | :--- |
| `data` | 0 | 8 | Pointer to the first byte, or null |
| `len` | 8 | 8 | Number of bytes |

Size: 16. Alignment: 8.

### `StorkvSlice`

A borrowed byte slice.

| Field | Offset | Size | Meaning |
| :--- | :--- | :--- | :--- |
| `data` | 0 | 8 | Pointer to the first byte, or null for an open bound |
| `len` | 8 | 8 | Number of bytes |

Size: 16. Alignment: 8.

### `StorkvString`

An owned NUL-terminated string.

| Field | Offset | Size | Meaning |
| :--- | :--- | :--- | :--- |
| `data` | 0 | 8 | Pointer to the first character |
| `len` | 8 | 8 | Characters before the NUL terminator |

Size: 16. Alignment: 8.

### `StorkvPoolConfig`

The pool configuration.

| Field | Offset | Size | Meaning |
| :--- | :--- | :--- | :--- |
| `health_check` | 0 | 1 | Probe idle connections before reuse |
| Padding | 1 | 3 | Alignment padding |
| `max_connections` | 4 | 4 | Maximum simultaneous connections |
| `min_connections` | 8 | 4 | Connections opened eagerly |
| Padding | 12 | 4 | Alignment padding |
| `acquire_timeout_ms` | 16 | 8 | Acquire timeout in milliseconds |
| `idle_timeout_ms` | 24 | 8 | Idle timeout in milliseconds |

Size: 32. Alignment: 8.

A zero `acquire_timeout_ms` selects the SDK default of 5 seconds. A zero `idle_timeout_ms` disables idle expiry.

### `StorkvRetryConfig`

The retry settings for `storkv_tx_run`.

| Field | Offset | Size | Meaning |
| :--- | :--- | :--- | :--- |
| `jitter` | 0 | 1 | Add random offset to each delay |
| Padding | 1 | 3 | Alignment padding |
| `max_retries` | 4 | 4 | Retries after the first attempt |
| `base_delay_ms` | 8 | 8 | Delay before the first retry |
| `max_delay_ms` | 16 | 8 | Maximum backoff delay |

Size: 24. Alignment: 8.

### `StorkvEntry`

One scan entry.

| Field | Offset | Size | Meaning |
| :--- | :--- | :--- | :--- |
| `key` | 0 | 16 | Owned key bytes |
| `value` | 16 | 16 | Owned value bytes |

Size: 32. Alignment: 8.

### `StorkvEntries`

An owned array of scan entries.

| Field | Offset | Size | Meaning |
| :--- | :--- | :--- | :--- |
| `entries` | 0 | 8 | Pointer to the first entry |
| `len` | 8 | 8 | Number of entries |

Size: 16. Alignment: 8.

### `StorkvWatchEvent`

One watch event.

| Field | Offset | Size | Meaning |
| :--- | :--- | :--- | :--- |
| `kind` | 0 | 4 | Kind of change |
| `scope_id` | 4 | 4 | Scope that changed |
| `seq` | 8 | 8 | Commit sequence number |
| `key` | 16 | 16 | Owned key bytes |
| `value` | 32 | 16 | Owned value bytes |

Size: 48. Alignment: 8.

The field order in the Rust source is `kind`, `scope_id`, `seq`, `key`, then `value`. The order follows the smallest-field-first rule: 4-byte fields, then an 8-byte field, then 16-byte compounds.

## Opaque handles

| Handle | Created by | Released by |
| :--- | :--- | :--- |
| `StorkvPoolHandle` | `storkv_pool_open` | `storkv_pool_close` |
| `StorkvScopeHandle` | `storkv_scope_open` | `storkv_scope_close` |
| `StorkvTxHandle` | `storkv_tx_begin` | `storkv_tx_commit` or `storkv_tx_rollback` |
| `StorkvWatchHandle` | `storkv_watch_prefix` | `storkv_watch_close` |

The header declares each handle as an incomplete struct. Do not dereference a handle. Do not allocate one in your code.

## Status codes

| Name | Value | Meaning |
| :--- | :--- | :--- |
| `Ok` | 0 | Success |
| `NullPointer` | 1 | A required pointer is null |
| `InvalidArgument` | 2 | The caller supplied a malformed URI, option, or argument |
| `Pool` | 3 | The pool cannot open or is closed |
| `Scope` | 4 | The scope does not resolve |
| `Transaction` | 5 | The transaction cannot start or commit |
| `Serialization` | 6 | A value cannot encode or decode |
| `Watch` | 7 | The watch stream ended or failed |
| `Timeout` | 8 | The operation exceeded its deadline |
| `Internal` | 9 | Another internal failure |

`storkv_watch_next` returns `Timeout` when no event arrives before the deadline. It returns `Watch` when the stream ends.

## Event kinds

| Name | Value | Meaning |
| :--- | :--- | :--- |
| `Put` | 0 | A key received a new value |
| `Delete` | 1 | A delete event for one key |
| `DropScope` | 2 | A drop event for one scope |

## Error retrieval

Call `storkv_last_error_message` after a failure:

```c
StorkvStatus status = storkv_get(scope, key, &value);
if (status != Ok) {
    StorkvString message = storkv_last_error_message();
    fprintf(stderr, "%.*s\n", (int)message.len, message.data);
    storkv_string_free(message);
}
```

The message belongs to the calling thread. A successful data call clears the message for that thread. A null `data` pointer means no message.

## Ownership summary

| Direction | Type | Owner | Release |
| :--- | :--- | :--- | :--- |
| Input | `StorkvSlice` | Caller | None, borrow only during the call |
| Output | `StorkvBytes` | Caller | `storkv_bytes_free` |
| Output | `StorkvEntries` | Caller | `storkv_entries_free` |
| Output | `StorkvWatchEvent.key` | Caller | `storkv_bytes_free` |
| Output | `StorkvWatchEvent.value` | Caller | `storkv_bytes_free` |
| Output | `StorkvString` | Caller | `storkv_string_free` |

`storkv_entries_free` frees each key and each value in the array. Do not call `storkv_bytes_free` on an entry field after `storkv_entries_free`.

## Function reference

### Pool functions

| Function | Signature | Returns | Ownership |
| :--- | :--- | :--- | :--- |
| `storkv_pool_open` | `(const char *uri, const StorkvPoolConfig *config, StorkvPoolHandle **out)` | Status | Writes one handle |
| `storkv_pool_close` | `(StorkvPoolHandle *pool)` | None | Releases the handle |
| `storkv_pool_is_closed` | `(StorkvPoolHandle *pool, int32_t *out_closed)` | Status | Writes one flag |

`config` may be null. A null config uses the default options.

### Scope functions

| Function | Signature | Returns | Ownership |
| :--- | :--- | :--- | :--- |
| `storkv_scope_open` | `(StorkvPoolHandle *pool, const char *name, StorkvScopeHandle **out)` | Status | Writes one handle |
| `storkv_scope_close` | `(StorkvScopeHandle *scope)` | None | Releases the handle |

A scope handle keeps a clone of the pool. The handle stays valid after `storkv_pool_close`, but close it first.

### Data functions

| Function | Signature | Returns | Ownership |
| :--- | :--- | :--- | :--- |
| `storkv_get` | `(StorkvScopeHandle *scope, StorkvSlice key, StorkvBytes *out_value)` | Status | `out_value` needs `storkv_bytes_free` |
| `storkv_put` | `(StorkvScopeHandle *scope, StorkvSlice key, StorkvSlice value)` | Status | No output |
| `storkv_put_with_ttl` | `(StorkvScopeHandle *scope, StorkvSlice key, StorkvSlice value, uint64_t ttl_ms)` | Status | No output |
| `storkv_get_and_touch` | `(StorkvScopeHandle *scope, StorkvSlice key, uint64_t ttl_ms, StorkvBytes *out_value)` | Status | `out_value` needs `storkv_bytes_free` |
| `storkv_delete` | `(StorkvScopeHandle *scope, StorkvSlice key)` | Status | No output |
| `storkv_exists` | `(StorkvScopeHandle *scope, StorkvSlice key, int32_t *out_exists)` | Status | Writes one flag |
| `storkv_scan_prefix` | `(StorkvScopeHandle *scope, StorkvSlice prefix, StorkvEntries *out_entries)` | Status | `out_entries` needs `storkv_entries_free` |
| `storkv_scan_range` | `(StorkvScopeHandle *scope, StorkvSlice start, StorkvSlice end, StorkvEntries *out_entries)` | Status | `out_entries` needs `storkv_entries_free` |

A missing key returns `Ok` with an empty `StorkvBytes`. Check `len` before you read `data`.

A null slice pointer with a zero length means an empty value. In `storkv_scan_range`, a null pointer means an open bound.

### Transaction functions

| Function | Signature | Returns | Ownership |
| :--- | :--- | :--- | :--- |
| `storkv_tx_begin` | `(StorkvPoolHandle *pool, StorkvTxHandle **out)` | Status | Writes one handle |
| `storkv_tx_get` | `(StorkvTxHandle *tx, StorkvScopeHandle *scope, StorkvSlice key, StorkvBytes *out_value)` | Status | `out_value` needs `storkv_bytes_free` |
| `storkv_tx_put` | `(StorkvTxHandle *tx, StorkvScopeHandle *scope, StorkvSlice key, StorkvSlice value)` | Status | No output |
| `storkv_tx_put_with_ttl` | `(StorkvTxHandle *tx, StorkvScopeHandle *scope, StorkvSlice key, StorkvSlice value, uint64_t ttl_ms)` | Status | No output |
| `storkv_tx_delete` | `(StorkvTxHandle *tx, StorkvScopeHandle *scope, StorkvSlice key)` | Status | No output |
| `storkv_tx_commit` | `(StorkvTxHandle *tx)` | Status | Releases the handle |
| `storkv_tx_rollback` | `(StorkvTxHandle *tx)` | None | Releases the handle |
| `storkv_tx_run` | `(StorkvPoolHandle *pool, const StorkvRetryConfig *config, StorkvTxCallback callback, void *user, int32_t *out_committed)` | Status | Writes one flag |

`storkv_tx_get` adds the key to the read set. `storkv_tx_commit` validates that set and returns `Transaction` on a conflict. `storkv_tx_run` retries a conflict according to the policy.

`storkv_tx_run` passes a borrowed handle to the callback. The callback must not commit or roll back. The helper commits when the callback returns `Ok` and rolls back for any other status. `out_committed` may be null.

### Watch functions

| Function | Signature | Returns | Ownership |
| :--- | :--- | :--- | :--- |
| `storkv_watch_prefix` | `(StorkvScopeHandle *scope, StorkvSlice prefix, StorkvWatchHandle **out_watch)` | Status | Writes one handle |
| `storkv_watch_next` | `(StorkvWatchHandle *watch, StorkvWatchEvent *out_event, uint64_t timeout_ms)` | Status | Event fields need `storkv_bytes_free` |
| `storkv_watch_close` | `(StorkvWatchHandle *watch)` | None | Releases the handle |

A `timeout_ms` value of zero waits without a limit. The function returns `Timeout` on a deadline.

### Memory functions

| Function | Signature | Returns | Ownership |
| :--- | :--- | :--- | :--- |
| `storkv_bytes_free` | `(StorkvBytes value)` | None | Releases one buffer |
| `storkv_entries_free` | `(StorkvEntries entries)` | None | Releases one array and its fields |
| `storkv_last_error_message` | `(void)` | `StorkvString` | Needs `storkv_string_free` |
| `storkv_string_free` | `(StorkvString value)` | None | Releases one string |

A free function ignores a null pointer. Do not free one buffer twice.

## Thread safety

Each pool owns a multithreaded Tokio runtime. The runtime lives until the pool handle closes.

A pool handle can serve more than one thread. The pool uses a semaphore to limit concurrent work.

Do not use one transaction handle or one watch handle from two threads at the same time. Wrap the handle in a mutex when you need shared access.

The last error message is thread-local. Read it on the thread that made the failed call.

## Header

The following block contains the generated header `bindings/c/storkv.h`.

<details>
<summary>Show the generated C header</summary>

```c
/* Generated by cbindgen from crates/storkv-ffi. Do not edit manually. */

#ifndef STORKV_H
#define STORKV_H

#pragma once

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
typedef struct StorkvPoolHandle StorkvPoolHandle;
typedef struct StorkvScopeHandle StorkvScopeHandle;
typedef struct StorkvTxHandle StorkvTxHandle;
typedef struct StorkvWatchHandle StorkvWatchHandle;


/**
 * Status code returned by every FFI function.
 */
enum StorkvStatus
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : int32_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
    /**
     * The operation succeeded.
     */
    Ok = 0,
    /**
     * A required pointer argument was null.
     */
    NullPointer = 1,
    /**
     * The caller supplied a malformed argument.
     */
    InvalidArgument = 2,
    /**
     * The pool cannot open or is closed.
     */
    Pool = 3,
    /**
     * The scope does not resolve.
     */
    Scope = 4,
    /**
     * The transaction cannot start or commit.
     */
    Transaction = 5,
    /**
     * Encoding or decoding a value failed.
     */
    Serialization = 6,
    /**
     * The watch stream failed.
     */
    Watch = 7,
    /**
     * The operation timed out.
     */
    Timeout = 8,
    /**
     * Any other internal failure.
     */
    Internal = 9,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum StorkvStatus StorkvStatus;
#else
typedef int32_t StorkvStatus;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Kind of change carried by [`StorkvWatchEvent`].
 */
enum StorkvEventKind
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : int32_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
    /**
     * A key received a new value.
     */
    Put = 0,
    /**
     * A delete event for one key.
     */
    Delete = 1,
    /**
     * A drop event for one scope.
     */
    DropScope = 2,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum StorkvEventKind StorkvEventKind;
#else
typedef int32_t StorkvEventKind;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Owned, NUL-terminated string allocated by the FFI layer.
 */
typedef struct StorkvString {
    /**
     * Pointer to the first character.
     */
    char *data;
    /**
     * Number of characters before the terminating NUL.
     */
    uintptr_t len;
} StorkvString;

/**
 * Owned byte buffer allocated by the FFI layer.
 */
typedef struct StorkvBytes {
    /**
     * Pointer to the first byte. Null when `len` is zero.
     */
    uint8_t *data;
    /**
     * Number of bytes.
     */
    uintptr_t len;
} StorkvBytes;

/**
 * One key/value pair returned by a scan.
 */
typedef struct StorkvEntry {
    /**
     * Owned key bytes.
     */
    struct StorkvBytes key;
    /**
     * Owned value bytes.
     */
    struct StorkvBytes value;
} StorkvEntry;

/**
 * Owned array of scan entries.
 */
typedef struct StorkvEntries {
    /**
     * Pointer to the first entry.
     */
    struct StorkvEntry *entries;
    /**
     * Number of entries.
     */
    uintptr_t len;
} StorkvEntries;

/**
 * Configuration for a pooled FFI connection.
 */
typedef struct StorkvPoolConfig {
    /**
     * Probe idle connections before reuse.
     */
    bool health_check;
    /**
     * Maximum number of simultaneous connections.
     */
    uint32_t max_connections;
    /**
     * Number of connections opened eagerly.
     */
    uint32_t min_connections;
    /**
     * Acquire timeout in milliseconds. Zero selects the SDK default.
     */
    uint64_t acquire_timeout_ms;
    /**
     * Idle timeout in milliseconds. Zero disables idle expiry.
     */
    uint64_t idle_timeout_ms;
} StorkvPoolConfig;

/**
 * Borrowed byte slice passed into the FFI layer.
 */
typedef struct StorkvSlice {
    /**
     * Pointer to the first byte. Null means an open range bound.
     */
    const uint8_t *data;
    /**
     * Number of bytes.
     */
    uintptr_t len;
} StorkvSlice;

/**
 * Retry configuration for the automatic transaction helper.
 */
typedef struct StorkvRetryConfig {
    /**
     * Add random jitter to each backoff delay.
     */
    bool jitter;
    /**
     * Maximum number of retries after the first attempt.
     */
    uint32_t max_retries;
    /**
     * Delay before the first retry, in milliseconds.
     */
    uint64_t base_delay_ms;
    /**
     * Maximum backoff delay, in milliseconds.
     */
    uint64_t max_delay_ms;
} StorkvRetryConfig;

/**
 * Callback invoked by the automatic transaction retry helper.
 */
typedef StorkvStatus (*StorkvTxCallback)(struct StorkvTxHandle *tx, void *user);

/**
 * One change event returned by a watch stream.
 */
typedef struct StorkvWatchEvent {
    /**
     * Kind of change.
     */
    StorkvEventKind kind;
    /**
     * Identifier of the scope that changed.
     */
    uint32_t scope_id;
    /**
     * Commit sequence number of the change.
     */
    uint64_t seq;
    /**
     * Owned key bytes.
     */
    struct StorkvBytes key;
    /**
     * Owned value bytes. Empty for deletes and scope drops.
     */
    struct StorkvBytes value;
} StorkvWatchEvent;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/**
 * Return the last error message recorded on the calling thread.
 *
 * Release the returned string with [`storkv_string_free`].
 */
struct StorkvString storkv_last_error_message(void);

/**
 * Release a string returned by [`storkv_last_error_message`].
 *
 * # Safety
 *
 * `value` must come from [`storkv_last_error_message`]. Do not use it after
 * this call.
 */
void storkv_string_free(struct StorkvString value);

/**
 * Release a byte buffer allocated by the FFI layer.
 *
 * # Safety
 *
 * `value` must come from this library. Do not use it after this call.
 */
void storkv_bytes_free(struct StorkvBytes value);

/**
 * Release an entry array allocated by the FFI layer.
 *
 * # Safety
 *
 * `entries` must come from this library. Do not use it after this call.
 */
void storkv_entries_free(struct StorkvEntries entries);

/**
 * Open a pooled connection to an embedded or networked StorKV backend.
 *
 * # Safety
 *
 * `uri` must be a valid NUL-terminated string. `config` may be null. `out`
 * must point to writable storage for one handle pointer.
 */
StorkvStatus storkv_pool_open(const char *uri, const struct StorkvPoolConfig *config, struct StorkvPoolHandle **out);

/**
 * Close a pool handle.
 *
 * # Safety
 *
 * `pool` must come from [`storkv_pool_open`]. Do not use it after this
 * call. Scope handles opened from the pool remain valid.
 */
void storkv_pool_close(struct StorkvPoolHandle *pool);

/**
 * Report whether a pool is closed.
 *
 * # Safety
 *
 * `pool` must be a live pool handle and `out_closed` must point to writable
 * storage for one integer.
 */
StorkvStatus storkv_pool_is_closed(struct StorkvPoolHandle *pool, int32_t *out_closed);

/**
 * Open a lightweight handle for one scope.
 *
 * # Safety
 *
 * `pool` must be a live pool handle, `name` must be a valid NUL-terminated
 * string, and `out` must point to writable storage for one handle pointer.
 */
StorkvStatus storkv_scope_open(struct StorkvPoolHandle *pool, const char *name, struct StorkvScopeHandle **out);

/**
 * Close a scope handle.
 *
 * # Safety
 *
 * `scope` must come from [`storkv_scope_open`]. Do not use it after this
 * call.
 */
void storkv_scope_close(struct StorkvScopeHandle *scope);

/**
 * Read one key from a scope.
 *
 * # Safety
 *
 * `scope` must be a live scope handle, `key` must reference readable bytes,
 * and `out_value` must point to writable storage for one byte buffer.
 */
StorkvStatus storkv_get(struct StorkvScopeHandle *scope, struct StorkvSlice key, struct StorkvBytes *out_value);

/**
 * Store one key/value pair in a scope.
 *
 * # Safety
 *
 * `scope` must be a live scope handle and both slices must reference readable
 * bytes.
 */
StorkvStatus storkv_put(struct StorkvScopeHandle *scope, struct StorkvSlice key, struct StorkvSlice value);

/**
 * Store one key/value pair with a time to live in milliseconds.
 *
 * # Safety
 *
 * `scope` must be a live scope handle and both slices must reference readable
 * bytes.
 */
StorkvStatus storkv_put_with_ttl(struct StorkvScopeHandle *scope, struct StorkvSlice key, struct StorkvSlice value, uint64_t ttl_ms);

/**
 * Read one key and restart its time to live.
 *
 * # Safety
 *
 * `scope` must be a live scope handle, `key` must reference readable bytes,
 * and `out_value` must point to writable storage for one byte buffer.
 */
StorkvStatus storkv_get_and_touch(struct StorkvScopeHandle *scope, struct StorkvSlice key, uint64_t ttl_ms, struct StorkvBytes *out_value);

/**
 * Delete one key from a scope.
 *
 * # Safety
 *
 * `scope` must be a live scope handle and `key` must reference readable bytes.
 */
StorkvStatus storkv_delete(struct StorkvScopeHandle *scope, struct StorkvSlice key);

/**
 * Report whether a key exists in a scope.
 *
 * # Safety
 *
 * `scope` must be a live scope handle, `key` must reference readable bytes,
 * and `out_exists` must point to writable storage for one integer.
 */
StorkvStatus storkv_exists(struct StorkvScopeHandle *scope, struct StorkvSlice key, int32_t *out_exists);

/**
 * Scan every key with a prefix.
 *
 * # Safety
 *
 * `scope` must be a live scope handle, `prefix` must reference readable
 * bytes, and `out_entries` must point to writable storage for one entry array.
 */
StorkvStatus storkv_scan_prefix(struct StorkvScopeHandle *scope, struct StorkvSlice prefix, struct StorkvEntries *out_entries);

/**
 * Scan a half-open key range. A null bound pointer leaves that side open.
 *
 * # Safety
 *
 * `scope` must be a live scope handle, both slices must either be null or
 * reference readable bytes, and `out_entries` must point to writable storage
 * for one entry array.
 */
StorkvStatus storkv_scan_range(struct StorkvScopeHandle *scope, struct StorkvSlice start, struct StorkvSlice end, struct StorkvEntries *out_entries);

/**
 * Begin an explicit transaction on a pooled connection.
 *
 * # Safety
 *
 * `pool` must be a live pool handle and `out` must point to writable storage
 * for one transaction handle pointer.
 */
StorkvStatus storkv_tx_begin(struct StorkvPoolHandle *pool, struct StorkvTxHandle **out);

/**
 * Read one key inside a transaction and add it to the read set.
 *
 * # Safety
 *
 * `tx` must be a live transaction handle, `scope` must be a live scope handle,
 * and `out_value` must point to writable storage for one byte buffer.
 */
StorkvStatus storkv_tx_get(struct StorkvTxHandle *tx, struct StorkvScopeHandle *scope, struct StorkvSlice key, struct StorkvBytes *out_value);

/**
 * Buffer a key/value write inside a transaction.
 *
 * # Safety
 *
 * Both handles must be live and both slices must reference readable bytes.
 */
StorkvStatus storkv_tx_put(struct StorkvTxHandle *tx, struct StorkvScopeHandle *scope, struct StorkvSlice key, struct StorkvSlice value);

/**
 * Buffer a key/value write with a time to live in milliseconds.
 *
 * # Safety
 *
 * Both handles must be live and both slices must reference readable bytes.
 */
StorkvStatus storkv_tx_put_with_ttl(struct StorkvTxHandle *tx, struct StorkvScopeHandle *scope, struct StorkvSlice key, struct StorkvSlice value, uint64_t ttl_ms);

/**
 * Buffer a delete inside a transaction.
 *
 * # Safety
 *
 * Both handles must be live and `key` must reference readable bytes.
 */
StorkvStatus storkv_tx_delete(struct StorkvTxHandle *tx, struct StorkvScopeHandle *scope, struct StorkvSlice key);

/**
 * Commit a transaction and release its handle.
 *
 * # Safety
 *
 * `tx` must come from [`storkv_tx_begin`]. Do not use it after this
 * call.
 */
StorkvStatus storkv_tx_commit(struct StorkvTxHandle *tx);

/**
 * Roll back a transaction and release its handle.
 *
 * # Safety
 *
 * `tx` must come from [`storkv_tx_begin`]. Do not use it after this
 * call.
 */
void storkv_tx_rollback(struct StorkvTxHandle *tx);

/**
 * Run a callback inside a transaction with automatic OCC retries.
 *
 * The callback must call [`storkv_tx_get`], [`storkv_tx_put`], and
 * [`storkv_tx_delete`] on the handle it receives. It must not commit or roll
 * back. The helper commits when the callback returns [`StorkvStatus::Ok`] and
 * rolls back otherwise.
 *
 * # Safety
 *
 * `pool` must be a live pool handle, `callback` must be a valid function
 * pointer, and the helper passes `user` through unchanged.
 */
StorkvStatus storkv_tx_run(struct StorkvPoolHandle *pool, const struct StorkvRetryConfig *config, StorkvTxCallback callback, void *user, int32_t *out_committed);

/**
 * Open a watch stream for every key with a prefix.
 *
 * # Safety
 *
 * `scope` must be a live scope handle, `prefix` must reference readable bytes,
 * and `out_watch` must point to writable storage for one watch handle pointer.
 */
StorkvStatus storkv_watch_prefix(struct StorkvScopeHandle *scope, struct StorkvSlice prefix, struct StorkvWatchHandle **out_watch);

/**
 * Receive the next watch event.
 *
 * A `timeout_ms` of zero waits indefinitely. On success the caller owns
 * `out_event` and must release `key` and `value` with `storkv_bytes_free`.
 *
 * # Safety
 *
 * `watch` must be a live watch handle and `out_event` must point to writable
 * storage for one event.
 */
StorkvStatus storkv_watch_next(struct StorkvWatchHandle *watch, struct StorkvWatchEvent *out_event, uint64_t timeout_ms);

/**
 * Close a watch handle.
 *
 * # Safety
 *
 * `watch` must come from [`storkv_watch_prefix`]. Do not use it after
 * this call.
 */
void storkv_watch_close(struct StorkvWatchHandle *watch);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* STORKV_H */
```

</details>

## Related guides

- [SDK overview](../storkv-sdk/index.md)
- [C guide](../storkv-sdk/c.md)
- [C++ guide](../storkv-sdk/cpp.md)
- [Java guide](../storkv-sdk/java.md)
- [TypeScript guide](../storkv-sdk/typescript.md)
- [Python guide](../storkv-sdk/python.md)
- [Error codes](error-codes.md)
