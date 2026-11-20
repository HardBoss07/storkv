//! Scope read operations for the C ABI.

use crate::error::{clear_error, fail_with, failure, guard_status};
use crate::handles::{exec, scope_from_raw};
use crate::memory::{bytes_to_ffi, entries_to_ffi};
use crate::types::{StorkvBytes, StorkvEntries, StorkvScopeHandle, StorkvSlice, StorkvStatus};

pub(super) fn slice_bytes(slice: StorkvSlice) -> Option<&'static [u8]> {
    if slice.data.is_null() {
        if slice.len == 0 {
            return Some(&[]);
        }
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(slice.data, slice.len) })
}

/// Read one key from a scope.
///
/// # Safety
///
/// `scope` must be a live scope handle, `key` must reference readable bytes,
/// and `out_value` must point to writable storage for one byte buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_get(
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
    out_value: *mut StorkvBytes,
) -> StorkvStatus {
    guard_status(|| {
        if scope.is_null() {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        }
        if out_value.is_null() {
            return failure(StorkvStatus::NullPointer, "out_value is null");
        }
        unsafe { *out_value = StorkvBytes::EMPTY };
        let Some(key) = slice_bytes(key) else {
            return failure(StorkvStatus::InvalidArgument, "key pointer is null");
        };
        let Some(handle) = scope_from_raw(scope) else {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        };
        match exec(&handle.runtime, handle.scope.get(key)) {
            Ok(value) => {
                clear_error();
                unsafe { *out_value = bytes_to_ffi(value) };
                StorkvStatus::Ok
            }
            Err(error) => fail_with(error),
        }
    })
}

/// Report whether a key exists in a scope.
///
/// # Safety
///
/// `scope` must be a live scope handle, `key` must reference readable bytes,
/// and `out_exists` must point to writable storage for one integer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_exists(
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
    out_exists: *mut i32,
) -> StorkvStatus {
    guard_status(|| {
        if scope.is_null() {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        }
        if out_exists.is_null() {
            return failure(StorkvStatus::NullPointer, "out_exists is null");
        }
        let Some(key) = slice_bytes(key) else {
            return failure(StorkvStatus::InvalidArgument, "key pointer is null");
        };
        let Some(handle) = scope_from_raw(scope) else {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        };
        match exec(&handle.runtime, handle.scope.exists(key)) {
            Ok(exists) => {
                clear_error();
                unsafe { *out_exists = if exists { 1 } else { 0 } };
                StorkvStatus::Ok
            }
            Err(error) => fail_with(error),
        }
    })
}

/// Scan every key with a prefix.
///
/// # Safety
///
/// `scope` must be a live scope handle, `prefix` must reference readable
/// bytes, and `out_entries` must point to writable storage for one entry array.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_scan_prefix(
    scope: *mut StorkvScopeHandle,
    prefix: StorkvSlice,
    out_entries: *mut StorkvEntries,
) -> StorkvStatus {
    guard_status(|| {
        if scope.is_null() {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        }
        if out_entries.is_null() {
            return failure(StorkvStatus::NullPointer, "out_entries is null");
        }
        unsafe { *out_entries = StorkvEntries::EMPTY };
        let Some(prefix) = slice_bytes(prefix) else {
            return failure(StorkvStatus::InvalidArgument, "prefix pointer is null");
        };
        let Some(handle) = scope_from_raw(scope) else {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        };
        match exec(&handle.runtime, handle.scope.scan_prefix(prefix)) {
            Ok(entries) => {
                clear_error();
                unsafe { *out_entries = entries_to_ffi(entries) };
                StorkvStatus::Ok
            }
            Err(error) => fail_with(error),
        }
    })
}

/// Scan a half-open key range. A null bound pointer leaves that side open.
///
/// # Safety
///
/// `scope` must be a live scope handle, both slices must either be null or
/// reference readable bytes, and `out_entries` must point to writable storage
/// for one entry array.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_scan_range(
    scope: *mut StorkvScopeHandle,
    start: StorkvSlice,
    end: StorkvSlice,
    out_entries: *mut StorkvEntries,
) -> StorkvStatus {
    guard_status(|| {
        if scope.is_null() {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        }
        if out_entries.is_null() {
            return failure(StorkvStatus::NullPointer, "out_entries is null");
        }
        unsafe { *out_entries = StorkvEntries::EMPTY };
        let start = if start.is_null() {
            None
        } else {
            let Some(bytes) = slice_bytes(start) else {
                return failure(StorkvStatus::InvalidArgument, "range start is invalid");
            };
            Some(bytes)
        };
        let end = if end.is_null() {
            None
        } else {
            let Some(bytes) = slice_bytes(end) else {
                return failure(StorkvStatus::InvalidArgument, "range end is invalid");
            };
            Some(bytes)
        };
        let Some(handle) = scope_from_raw(scope) else {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        };
        match exec(&handle.runtime, handle.scope.scan_range(start, end)) {
            Ok(entries) => {
                clear_error();
                unsafe { *out_entries = entries_to_ffi(entries) };
                StorkvStatus::Ok
            }
            Err(error) => fail_with(error),
        }
    })
}
