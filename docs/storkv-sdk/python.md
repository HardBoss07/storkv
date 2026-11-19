# Use StorKV from Python

The Python binding uses the standard `ctypes` module. It loads the `storkv-ffi` library and maps each C function to a Python class. Context managers release native handles.

The binding lives in `bindings/python/storkv/__init__.py`. Read the [C ABI reference](../reference/ffi-c-abi.md) for the functions behind the class.

## Prerequisites

- Python 3.9 or later
- Rust 1.87 or later, to build the FFI library

Build the shared library:

```bash
cargo build -p storkv-ffi
```

The binding searches `target/debug` and `target/release` from the repository root. It also checks the current directory. Set `STORKV_FFI_LIB` to an absolute path when the library is in another location:

```bash
export STORKV_FFI_LIB="$PWD/target/debug/libstorkv_ffi.so"
```

On Windows PowerShell:

```powershell
$env:STORKV_FFI_LIB = "$PWD\target\debug\storkv_ffi.dll"
```

No package install is necessary. Add the parent directory to `PYTHONPATH`:

```bash
export PYTHONPATH="$PWD/bindings/python:$PYTHONPATH"
```

## Quickstart

```python
import storkv

with storkv.Pool("file://./data/python-app") as pool:
    with pool.scope("default") as scope:
        scope.put("greeting", "hello")
        print(scope.get_str("greeting"))

        scope.put_with_ttl("session", "live", 900000)
        print(scope.get_and_touch("session", 900000))

        scope.put("scan:1", "one")
        scope.put("scan:2", "two")
        print(len(scope.scan_prefix("scan:")))

        with pool.transaction() as tx:
            tx.put(scope, "tx:key", "value")
            tx.commit()

        with scope.watch_prefix("events:") as watch:
            scope.put("events:1", "payload")
            event = watch.next(5000)
            if event is not None:
                print(event.kind, event.key)
```

The `with` statement calls `close` at the end of the block. Each class also has a `close()` method.

## API types

| Type | Purpose |
| :--- | :--- |
| `storkv.Pool` | Pool handle |
| `storkv.Scope` | Scope handle |
| `storkv.Transaction` | Explicit OCC transaction |
| `storkv.Watch` | Stream of change events |
| `storkv.Event` | Frozen dataclass with event fields |
| `storkv.Status` | `IntEnum` of status values |
| `storkv.EventKind` | `IntEnum` with `PUT`, `DELETE`, and `DROP_SCOPE` |
| `storkv.StorkvError` | Exception with a `status` attribute |

The `Event` dataclass has these fields:

| Field | Type | Meaning |
| :--- | :--- | :--- |
| `kind` | `EventKind` | Kind of change |
| `scope_id` | `int` | Scope that changed |
| `seq` | `int` | Commit sequence number |
| `key` | `bytes` | Key that changed |
| `value` | `bytes` or `None` | New value for a put only |

## Pool options

Pass a `StorkvPoolConfig` structure as the second argument:

```python
import storkv

config = storkv.StorkvPoolConfig()
config.health_check = True
config.max_connections = 16
config.min_connections = 1
config.acquire_timeout_ms = 3000
config.idle_timeout_ms = 300000

with storkv.Pool("storkv://127.0.0.1:7878", config) as pool:
    with pool.scope("default") as scope:
        scope.put("key", "value")
```

A zero value for `acquire_timeout_ms` selects the SDK default. A zero value for `idle_timeout_ms` disables idle expiry.

## Scope methods

| Method | Result | Notes |
| :--- | :--- | :--- |
| `get` | `bytes` or `None` | Missing key returns `None` |
| `get_str` | `str` or `None` | UTF-8 decode |
| `put` | `None` | Accepts `bytes` or `str` |
| `put_with_ttl` | `None` | TTL in milliseconds |
| `get_and_touch` | `bytes` or `None` | Read and restart the expiry |
| `delete` | `None` | Remove one key |
| `exists` | `bool` | Presence check |
| `scan_prefix` | `list[tuple[bytes, bytes]]` | One prefix |
| `scan_range` | `list[tuple[bytes, bytes]]` | Half-open range, optional bounds |
| `watch_prefix` | `Watch` | Change stream |

Keys and values accept `bytes`, `bytearray`, `memoryview`, or `str`. A string encodes as UTF-8. An output value is always `bytes`.

## Memory rules

The binding wraps each input in a temporary `ctypes` buffer. The buffer stays alive during the call. The C layer copies the bytes before the call returns.

The binding copies each output buffer into Python `bytes`. It then calls `storkv_bytes_free` or `storkv_entries_free`. Python code therefore never calls a free function.

## Transactions

`pool.transaction()` starts an explicit transaction. Buffer writes with `put`, `put_with_ttl`, and `delete`. Call `commit` or `rollback`.

A `with` block rolls back when the block exits without a commit:

```python
with pool.transaction() as tx:
    tx.put(scope, "counter", "1")
    tx.commit()
```

`commit` and `rollback` release the native handle. A later `close` does nothing.

The binding does not wrap `storkv_tx_run`. A Python caller that needs automatic retries must write the retry loop.

## Watches

`watch.next(timeout_ms)` returns the next event or `None` on timeout. The default timeout is 5000 milliseconds. A timeout value of zero waits without a limit.

Close the watch before the scope. A nested `with` block enforces that order.

## Errors

A non-zero status raises `StorkvError`. The `status` attribute holds a `Status` value.

```python
import storkv

try:
    scope.get("key")
except storkv.StorkvError as error:
    print(error.status, error)
```

A closed scope raises `StorkvError` with status `Status.SCOPE`. A closed pool raises with status `Status.POOL`.

Read [Error codes](../reference/error-codes.md) for the server-side wire codes.

## Test example

The workspace test is `bindings/python/tests/test_storkv.py`. This smaller test uses `unittest`:

```python
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import storkv


class StorkvTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.directory = tempfile.TemporaryDirectory(prefix="storkv-python-test-")
        cls.uri = "file://" + cls.directory.name

    @classmethod
    def tearDownClass(cls) -> None:
        cls.directory.cleanup()

    def test_crud_and_memory(self) -> None:
        with storkv.Pool(self.uri) as pool:
            self.assertFalse(pool.is_closed())
            with pool.scope("default") as scope:
                scope.put("py:greeting", "hello")
                self.assertEqual(scope.get(b"py:greeting"), b"hello")
                self.assertEqual(scope.get_str("py:greeting"), "hello")
                self.assertTrue(scope.exists("py:greeting"))
                scope.delete("py:greeting")
                self.assertFalse(scope.exists("py:greeting"))
                self.assertIsNone(scope.get("py:greeting"))

    def test_ttl_and_touch(self) -> None:
        with storkv.Pool(self.uri) as pool, pool.scope("default") as scope:
            scope.put_with_ttl("py:session", "live", 300)
            self.assertEqual(scope.get_and_touch("py:session", 5000), b"live")
            scope.put_with_ttl("py:short", "lived", 100)
            time.sleep(0.3)
            self.assertIsNone(scope.get("py:short"))

    def test_scan_and_transaction(self) -> None:
        with storkv.Pool(self.uri) as pool, pool.scope("default") as scope:
            scope.put("py:scan:1", "one")
            scope.put("py:scan:2", "two")
            self.assertEqual(len(scope.scan_prefix("py:scan:")), 2)
            self.assertEqual(len(scope.scan_range("py:scan:1", "py:scan:3")), 2)

            with pool.transaction() as tx:
                tx.put(scope, "py:tx", "value")
                self.assertEqual(tx.get(scope, "py:tx"), b"value")
                tx.commit()
            self.assertEqual(scope.get("py:tx"), b"value")

            tx = pool.transaction()
            tx.put(scope, "py:rolled", "back")
            tx.rollback()
            self.assertIsNone(scope.get("py:rolled"))

    def test_watch(self) -> None:
        with storkv.Pool(self.uri) as pool, pool.scope("default") as scope:
            with scope.watch_prefix("py:events:") as watch:
                scope.put("py:events:1", "payload")
                event = watch.next(5000)
                self.assertIsNotNone(event)
                assert event is not None
                self.assertEqual(event.kind, storkv.EventKind.PUT)
                self.assertEqual(event.key, b"py:events:1")
                self.assertEqual(event.value, b"payload")

    def test_error_mapping(self) -> None:
        with storkv.Pool(self.uri) as pool:
            with pool.scope("default") as scope:
                scope.close()
                with self.assertRaises(storkv.StorkvError) as raised:
                    scope.get("key")
                self.assertEqual(raised.exception.status, storkv.Status.SCOPE)


if __name__ == "__main__":
    unittest.main(verbosity=2)
```

Run the workspace test:

```bash
python -m unittest discover -s bindings/python/tests -v
```

Run one test class:

```bash
python -m unittest bindings.python.tests.test_storkv.StorkvTests -v
```

Run against a server:

```bash
STORKV_TEST_URI=storkv://127.0.0.1:7878 python -m unittest discover -s bindings/python/tests -v
```

The workspace test skips the networked section when `STORKV_TEST_URI` is unset.

## Related guides

- [C ABI reference](../reference/ffi-c-abi.md)
- [C guide](c.md)
- [Connection URIs](../getting-started/connection-uris.md)
- [Error codes](../reference/error-codes.md)
