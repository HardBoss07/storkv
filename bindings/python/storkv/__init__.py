"""ctypes bindings for the StorKV C ABI."""

from __future__ import annotations

import ctypes
import os
import sys
from ctypes import (
    POINTER,
    Structure,
    byref,
    c_bool,
    c_char_p,
    c_int32,
    c_size_t,
    c_uint8,
    c_uint32,
    c_uint64,
    c_void_p,
)
from dataclasses import dataclass
from enum import IntEnum
from pathlib import Path
from typing import Iterator, Optional, Union

__all__ = [
    "Event",
    "EventKind",
    "Pool",
    "Scope",
    "StorkvError",
    "Status",
    "Transaction",
    "Watch",
]

BytesLike = Union[bytes, bytearray, memoryview, str]


class Status(IntEnum):
    OK = 0
    NULL_POINTER = 1
    INVALID_ARGUMENT = 2
    POOL = 3
    SCOPE = 4
    TRANSACTION = 5
    SERIALIZATION = 6
    WATCH = 7
    TIMEOUT = 8
    INTERNAL = 9


class EventKind(IntEnum):
    PUT = 0
    DELETE = 1
    DROP_SCOPE = 2


class StorkvError(Exception):
    """Raised when the FFI layer returns a non-success status."""

    def __init__(self, status: Status, message: str) -> None:
        super().__init__(message)
        self.status = status


class StorkvString(Structure):
    _fields_ = [("data", c_char_p), ("len", c_size_t)]


class StorkvBytes(Structure):
    _fields_ = [("data", POINTER(c_uint8)), ("len", c_size_t)]


class StorkvSlice(Structure):
    _fields_ = [("data", POINTER(c_uint8)), ("len", c_size_t)]


class StorkvEntry(Structure):
    _fields_ = [("key", StorkvBytes), ("value", StorkvBytes)]


class StorkvEntries(Structure):
    _fields_ = [("entries", POINTER(StorkvEntry)), ("len", c_size_t)]


class StorkvPoolConfig(Structure):
    _fields_ = [
        ("health_check", c_bool),
        ("max_connections", c_uint32),
        ("min_connections", c_uint32),
        ("acquire_timeout_ms", c_uint64),
        ("idle_timeout_ms", c_uint64),
    ]


class StorkvRetryConfig(Structure):
    _fields_ = [
        ("jitter", c_bool),
        ("max_retries", c_uint32),
        ("base_delay_ms", c_uint64),
        ("max_delay_ms", c_uint64),
    ]


class StorkvWatchEvent(Structure):
    _fields_ = [
        ("kind", c_int32),
        ("scope_id", c_uint32),
        ("seq", c_uint64),
        ("key", StorkvBytes),
        ("value", StorkvBytes),
    ]


def _library_candidates() -> Iterator[Path]:
    names = ["storkv_ffi.dll", "libstorkv_ffi.so", "libstorkv_ffi.dylib"]
    configured = os.environ.get("STORKV_FFI_LIB")
    if configured:
        yield Path(configured)
    package_directory = Path(__file__).resolve().parent
    for name in names:
        yield package_directory / name
    package_root = Path(__file__).resolve().parents[3]
    for profile in ("debug", "release"):
        for name in names:
            yield package_root / "target" / profile / name
    for name in names:
        yield Path(name)


def _load_library() -> ctypes.CDLL:
    errors = []
    for candidate in _library_candidates():
        try:
            return ctypes.CDLL(str(candidate))
        except OSError as error:
            errors.append(f"{candidate}: {error}")
    raise StorkvError(
        Status.INTERNAL,
        "could not load storkv_ffi; set STORKV_FFI_LIB\n" + "\n".join(errors),
    )


_lib = _load_library()

_lib.storkv_last_error_message.argtypes = []
_lib.storkv_last_error_message.restype = StorkvString
_lib.storkv_string_free.argtypes = [StorkvString]
_lib.storkv_string_free.restype = None
_lib.storkv_bytes_free.argtypes = [StorkvBytes]
_lib.storkv_bytes_free.restype = None
_lib.storkv_entries_free.argtypes = [StorkvEntries]
_lib.storkv_entries_free.restype = None

_lib.storkv_pool_open.argtypes = [c_char_p, POINTER(StorkvPoolConfig), POINTER(c_void_p)]
_lib.storkv_pool_open.restype = c_int32
_lib.storkv_pool_close.argtypes = [c_void_p]
_lib.storkv_pool_close.restype = None
_lib.storkv_pool_is_closed.argtypes = [c_void_p, POINTER(c_int32)]
_lib.storkv_pool_is_closed.restype = c_int32

_lib.storkv_scope_open.argtypes = [c_void_p, c_char_p, POINTER(c_void_p)]
_lib.storkv_scope_open.restype = c_int32
_lib.storkv_scope_close.argtypes = [c_void_p]
_lib.storkv_scope_close.restype = None

_lib.storkv_get.argtypes = [c_void_p, StorkvSlice, POINTER(StorkvBytes)]
_lib.storkv_get.restype = c_int32
_lib.storkv_put.argtypes = [c_void_p, StorkvSlice, StorkvSlice]
_lib.storkv_put.restype = c_int32
_lib.storkv_put_with_ttl.argtypes = [c_void_p, StorkvSlice, StorkvSlice, c_uint64]
_lib.storkv_put_with_ttl.restype = c_int32
_lib.storkv_get_and_touch.argtypes = [c_void_p, StorkvSlice, c_uint64, POINTER(StorkvBytes)]
_lib.storkv_get_and_touch.restype = c_int32
_lib.storkv_delete.argtypes = [c_void_p, StorkvSlice]
_lib.storkv_delete.restype = c_int32
_lib.storkv_exists.argtypes = [c_void_p, StorkvSlice, POINTER(c_int32)]
_lib.storkv_exists.restype = c_int32
_lib.storkv_scan_prefix.argtypes = [c_void_p, StorkvSlice, POINTER(StorkvEntries)]
_lib.storkv_scan_prefix.restype = c_int32
_lib.storkv_scan_range.argtypes = [
    c_void_p,
    StorkvSlice,
    StorkvSlice,
    POINTER(StorkvEntries),
]
_lib.storkv_scan_range.restype = c_int32

_lib.storkv_tx_begin.argtypes = [c_void_p, POINTER(c_void_p)]
_lib.storkv_tx_begin.restype = c_int32
_lib.storkv_tx_get.argtypes = [c_void_p, c_void_p, StorkvSlice, POINTER(StorkvBytes)]
_lib.storkv_tx_get.restype = c_int32
_lib.storkv_tx_put.argtypes = [c_void_p, c_void_p, StorkvSlice, StorkvSlice]
_lib.storkv_tx_put.restype = c_int32
_lib.storkv_tx_put_with_ttl.argtypes = [
    c_void_p,
    c_void_p,
    StorkvSlice,
    StorkvSlice,
    c_uint64,
]
_lib.storkv_tx_put_with_ttl.restype = c_int32
_lib.storkv_tx_delete.argtypes = [c_void_p, c_void_p, StorkvSlice]
_lib.storkv_tx_delete.restype = c_int32
_lib.storkv_tx_commit.argtypes = [c_void_p]
_lib.storkv_tx_commit.restype = c_int32
_lib.storkv_tx_rollback.argtypes = [c_void_p]
_lib.storkv_tx_rollback.restype = None

_lib.storkv_watch_prefix.argtypes = [c_void_p, StorkvSlice, POINTER(c_void_p)]
_lib.storkv_watch_prefix.restype = c_int32
_lib.storkv_watch_next.argtypes = [c_void_p, POINTER(StorkvWatchEvent), c_uint64]
_lib.storkv_watch_next.restype = c_int32
_lib.storkv_watch_close.argtypes = [c_void_p]
_lib.storkv_watch_close.restype = None


def _error_message() -> str:
    raw = _lib.storkv_last_error_message()
    try:
        if not raw.data:
            return "StorKV operation failed"
        return ctypes.string_at(raw.data, raw.len).decode("utf-8", "replace")
    finally:
        _lib.storkv_string_free(raw)


def _check(status: int) -> None:
    if status == Status.OK:
        return
    try:
        mapped = Status(status)
    except ValueError:
        mapped = Status.INTERNAL
    raise StorkvError(mapped, _error_message())


class _Borrowed:
    """Owns a temporary buffer for the duration of one FFI call."""

    def __init__(self, value: BytesLike) -> None:
        if isinstance(value, str):
            value = value.encode("utf-8")
        data = bytes(value)
        self._buffer = (c_uint8 * len(data)).from_buffer_copy(data) if data else None
        pointer = ctypes.cast(self._buffer, POINTER(c_uint8)) if self._buffer is not None else None
        self.slice = StorkvSlice(pointer, len(data))


def _slice(value: Optional[BytesLike]) -> StorkvSlice:
    if value is None:
        return StorkvSlice(None, 0)
    return _Borrowed(value).slice


def _take_bytes(value: StorkvBytes) -> Optional[bytes]:
    if not value.data or value.len == 0:
        _lib.storkv_bytes_free(value)
        return None
    data = ctypes.string_at(value.data, value.len)
    _lib.storkv_bytes_free(value)
    return data


@dataclass(frozen=True)
class Event:
    kind: EventKind
    scope_id: int
    seq: int
    key: bytes
    value: Optional[bytes]


class Watch:
    def __init__(self, handle: c_void_p) -> None:
        self._handle = handle

    def __enter__(self) -> "Watch":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def next(self, timeout_ms: int = 5000) -> Optional[Event]:
        if not self._handle:
            return None
        raw = StorkvWatchEvent()
        status = _lib.storkv_watch_next(self._handle, byref(raw), timeout_ms)
        if status == Status.TIMEOUT:
            return None
        _check(status)
        key = _take_bytes(raw.key) or b""
        value = _take_bytes(raw.value)
        return Event(EventKind(raw.kind), raw.scope_id, raw.seq, key, value)

    def close(self) -> None:
        if self._handle:
            _lib.storkv_watch_close(self._handle)
            self._handle = None


class Transaction:
    def __init__(self, handle: c_void_p) -> None:
        self._handle = handle

    def __enter__(self) -> "Transaction":
        return self

    def __exit__(self, *_: object) -> None:
        self.rollback()

    def _require(self) -> c_void_p:
        if not self._handle:
            raise StorkvError(Status.TRANSACTION, "transaction is closed")
        return self._handle

    def get(self, scope: "Scope", key: BytesLike) -> Optional[bytes]:
        key_slice = _Borrowed(key)
        value = StorkvBytes()
        _check(_lib.storkv_tx_get(self._require(), scope._require(), key_slice.slice, byref(value)))
        return _take_bytes(value)

    def put(self, scope: "Scope", key: BytesLike, value: BytesLike) -> None:
        key_slice = _Borrowed(key)
        value_slice = _Borrowed(value)
        _check(
            _lib.storkv_tx_put(
                self._require(), scope._require(), key_slice.slice, value_slice.slice
            )
        )

    def put_with_ttl(
        self, scope: "Scope", key: BytesLike, value: BytesLike, ttl_ms: int
    ) -> None:
        key_slice = _Borrowed(key)
        value_slice = _Borrowed(value)
        _check(
            _lib.storkv_tx_put_with_ttl(
                self._require(),
                scope._require(),
                key_slice.slice,
                value_slice.slice,
                ttl_ms,
            )
        )

    def delete(self, scope: "Scope", key: BytesLike) -> None:
        key_slice = _Borrowed(key)
        _check(_lib.storkv_tx_delete(self._require(), scope._require(), key_slice.slice))

    def commit(self) -> None:
        handle = self._handle
        self._handle = None
        _check(_lib.storkv_tx_commit(handle))

    def rollback(self) -> None:
        if self._handle:
            handle = self._handle
            self._handle = None
            _lib.storkv_tx_rollback(handle)


class Scope:
    def __init__(self, handle: c_void_p) -> None:
        self._handle = handle

    def __enter__(self) -> "Scope":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def _require(self) -> c_void_p:
        if not self._handle:
            raise StorkvError(Status.SCOPE, "scope is closed")
        return self._handle

    def get(self, key: BytesLike) -> Optional[bytes]:
        key_slice = _Borrowed(key)
        value = StorkvBytes()
        _check(_lib.storkv_get(self._require(), key_slice.slice, byref(value)))
        return _take_bytes(value)

    def get_str(self, key: BytesLike) -> Optional[str]:
        value = self.get(key)
        if value is None:
            return None
        return value.decode("utf-8")

    def put(self, key: BytesLike, value: BytesLike) -> None:
        key_slice = _Borrowed(key)
        value_slice = _Borrowed(value)
        _check(_lib.storkv_put(self._require(), key_slice.slice, value_slice.slice))

    def put_with_ttl(self, key: BytesLike, value: BytesLike, ttl_ms: int) -> None:
        key_slice = _Borrowed(key)
        value_slice = _Borrowed(value)
        _check(
            _lib.storkv_put_with_ttl(
                self._require(), key_slice.slice, value_slice.slice, ttl_ms
            )
        )

    def get_and_touch(self, key: BytesLike, ttl_ms: int) -> Optional[bytes]:
        key_slice = _Borrowed(key)
        value = StorkvBytes()
        _check(
            _lib.storkv_get_and_touch(
                self._require(), key_slice.slice, ttl_ms, byref(value)
            )
        )
        return _take_bytes(value)

    def delete(self, key: BytesLike) -> None:
        key_slice = _Borrowed(key)
        _check(_lib.storkv_delete(self._require(), key_slice.slice))

    def exists(self, key: BytesLike) -> bool:
        key_slice = _Borrowed(key)
        flag = c_int32(0)
        _check(_lib.storkv_exists(self._require(), key_slice.slice, byref(flag)))
        return flag.value != 0

    def _take_entries(self, entries: StorkvEntries) -> list[tuple[bytes, bytes]]:
        owned: list[tuple[bytes, bytes]] = []
        for index in range(entries.len):
            entry = entries.entries[index]
            key = ctypes.string_at(entry.key.data, entry.key.len) if entry.key.len else b""
            value = (
                ctypes.string_at(entry.value.data, entry.value.len)
                if entry.value.len
                else b""
            )
            owned.append((key, value))
        _lib.storkv_entries_free(entries)
        return owned

    def scan_prefix(self, prefix: BytesLike) -> list[tuple[bytes, bytes]]:
        prefix_slice = _Borrowed(prefix)
        entries = StorkvEntries()
        _check(_lib.storkv_scan_prefix(self._require(), prefix_slice.slice, byref(entries)))
        return self._take_entries(entries)

    def scan_range(
        self,
        start: Optional[BytesLike] = None,
        end: Optional[BytesLike] = None,
    ) -> list[tuple[bytes, bytes]]:
        start_slice = _Borrowed(start) if start is not None else None
        end_slice = _Borrowed(end) if end is not None else None
        entries = StorkvEntries()
        _check(
            _lib.storkv_scan_range(
                self._require(),
                start_slice.slice if start_slice else _slice(None),
                end_slice.slice if end_slice else _slice(None),
                byref(entries),
            )
        )
        return self._take_entries(entries)

    def watch_prefix(self, prefix: BytesLike) -> Watch:
        prefix_slice = _Borrowed(prefix)
        handle = c_void_p()
        _check(
            _lib.storkv_watch_prefix(self._require(), prefix_slice.slice, byref(handle))
        )
        return Watch(handle)

    def close(self) -> None:
        if self._handle:
            _lib.storkv_scope_close(self._handle)
            self._handle = None


class Pool:
    def __init__(self, uri: str, options: Optional[StorkvPoolConfig] = None) -> None:
        handle = c_void_p()
        encoded = uri.encode("utf-8")
        _check(_lib.storkv_pool_open(encoded, options, byref(handle)))
        self._handle = handle

    def __enter__(self) -> "Pool":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def _require(self) -> c_void_p:
        if not self._handle:
            raise StorkvError(Status.POOL, "pool is closed")
        return self._handle

    def scope(self, name: str) -> Scope:
        handle = c_void_p()
        _check(_lib.storkv_scope_open(self._require(), name.encode("utf-8"), byref(handle)))
        return Scope(handle)

    def transaction(self) -> Transaction:
        handle = c_void_p()
        _check(_lib.storkv_tx_begin(self._require(), byref(handle)))
        return Transaction(handle)

    def is_closed(self) -> bool:
        flag = c_int32(0)
        _check(_lib.storkv_pool_is_closed(self._require(), byref(flag)))
        return flag.value != 0

    def close(self) -> None:
        if self._handle:
            _lib.storkv_pool_close(self._handle)
            self._handle = None


if __name__ == "__main__":
    print("storkv python bindings loaded from", _lib._name, file=sys.stderr)
