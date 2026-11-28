//! Owned buffer allocation and release helpers for the C ABI.

use bytes::Bytes;

use crate::error::guard_void;
use crate::types::{StorkvBytes, StorkvEntries, StorkvEntry};

pub(crate) fn bytes_to_ffi(value: Option<Bytes>) -> StorkvBytes {
    match value {
        Some(value) if !value.is_empty() => {
            let mut owned = value.to_vec().into_boxed_slice();
            let len = owned.len();
            let data = owned.as_mut_ptr();
            std::mem::forget(owned);
            StorkvBytes { data, len }
        }
        _ => StorkvBytes::EMPTY,
    }
}

pub(crate) fn entries_to_ffi(entries: Vec<(Bytes, Bytes)>) -> StorkvEntries {
    let mut owned: Vec<StorkvEntry> = entries
        .into_iter()
        .map(|(key, value)| StorkvEntry {
            key: bytes_to_ffi(Some(key)),
            value: bytes_to_ffi(Some(value)),
        })
        .collect();
    let len = owned.len();
    if len == 0 {
        return StorkvEntries::EMPTY;
    }
    let data = owned.as_mut_ptr();
    std::mem::forget(owned);
    StorkvEntries { entries: data, len }
}

unsafe fn free_bytes(value: StorkvBytes) {
    if value.data.is_null() {
        return;
    }
    let slice = std::ptr::slice_from_raw_parts_mut(value.data, value.len);
    drop(unsafe { Box::from_raw(slice) });
}

/// Release a byte buffer allocated by the FFI layer.
///
/// # Safety
///
/// `value` must come from this library. Do not use it after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_bytes_free(value: StorkvBytes) {
    guard_void(|| unsafe { free_bytes(value) });
}

/// Release an entry array allocated by the FFI layer.
///
/// # Safety
///
/// `entries` must come from this library. Do not use it after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_entries_free(entries: StorkvEntries) {
    guard_void(|| {
        if entries.entries.is_null() {
            return;
        }
        let slice = std::ptr::slice_from_raw_parts_mut(entries.entries, entries.len);
        let boxed = unsafe { Box::from_raw(slice) };
        for entry in boxed.iter() {
            unsafe {
                free_bytes(entry.key);
                free_bytes(entry.value);
            }
        }
        drop(boxed);
    });
}
