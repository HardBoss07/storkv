//! Scope write operations for the C ABI.

use std::time::Duration;

use crate::error::{clear_error, fail_with, failure, guard_status};
use crate::handles::{exec, scope_from_raw};
use crate::memory::bytes_to_ffi;
use crate::types::{StorkvBytes, StorkvScopeHandle, StorkvSlice, StorkvStatus};

use super::read::slice_bytes;

/// Store one key/value pair in a scope.
///
/// # Safety
///
/// `scope` must be a live scope handle and both slices must reference readable
/// bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_put(
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
    value: StorkvSlice,
) -> StorkvStatus {
    guard_status(|| store(scope, key, value, None))
}

/// Store one key/value pair with a time to live in milliseconds.
///
/// # Safety
///
/// `scope` must be a live scope handle and both slices must reference readable
/// bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_put_with_ttl(
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
    value: StorkvSlice,
    ttl_ms: u64,
) -> StorkvStatus {
    guard_status(|| store(scope, key, value, Some(Duration::from_millis(ttl_ms))))
}

fn store(
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
    value: StorkvSlice,
    ttl: Option<Duration>,
) -> StorkvStatus {
    if scope.is_null() {
        return failure(StorkvStatus::NullPointer, "scope handle is null");
    }
    let Some(key) = slice_bytes(key) else {
        return failure(StorkvStatus::InvalidArgument, "key pointer is null");
    };
    let Some(value) = slice_bytes(value) else {
        return failure(StorkvStatus::InvalidArgument, "value pointer is null");
    };
    let Some(handle) = scope_from_raw(scope) else {
        return failure(StorkvStatus::NullPointer, "scope handle is null");
    };
    let result = match ttl {
        Some(ttl) => exec(&handle.runtime, handle.scope.put_with_ttl(key, value, ttl)),
        None => exec(&handle.runtime, handle.scope.put(key, value)),
    };
    match result {
        Ok(()) => {
            clear_error();
            StorkvStatus::Ok
        }
        Err(error) => fail_with(error),
    }
}

/// Read one key and restart its time to live.
///
/// # Safety
///
/// `scope` must be a live scope handle, `key` must reference readable bytes,
/// and `out_value` must point to writable storage for one byte buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_get_and_touch(
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
    ttl_ms: u64,
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
        match exec(
            &handle.runtime,
            handle
                .scope
                .get_and_touch(key, Duration::from_millis(ttl_ms)),
        ) {
            Ok(value) => {
                clear_error();
                unsafe { *out_value = bytes_to_ffi(value) };
                StorkvStatus::Ok
            }
            Err(error) => fail_with(error),
        }
    })
}

/// Delete one key from a scope.
///
/// # Safety
///
/// `scope` must be a live scope handle and `key` must reference readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_delete(
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
) -> StorkvStatus {
    guard_status(|| {
        if scope.is_null() {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        }
        let Some(key) = slice_bytes(key) else {
            return failure(StorkvStatus::InvalidArgument, "key pointer is null");
        };
        let Some(handle) = scope_from_raw(scope) else {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        };
        match exec(&handle.runtime, handle.scope.delete(key)) {
            Ok(()) => {
                clear_error();
                StorkvStatus::Ok
            }
            Err(error) => fail_with(error),
        }
    })
}
