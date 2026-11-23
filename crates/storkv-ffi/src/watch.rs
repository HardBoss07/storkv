use std::time::Duration;

use storkv_sdk::WatchEventKind;

use crate::error::{clear_error, fail_with, failure, guard_status, guard_void};
use crate::handles::{
    WatchHandle, drop_watch, exec, scope_from_raw, watch_from_raw, watch_into_raw,
};
use crate::memory::bytes_to_ffi;
use crate::types::{
    StorkvBytes, StorkvEventKind, StorkvScopeHandle, StorkvSlice, StorkvStatus, StorkvWatchEvent,
    StorkvWatchHandle,
};

fn slice_bytes(slice: StorkvSlice) -> Option<&'static [u8]> {
    if slice.data.is_null() {
        if slice.len == 0 {
            return Some(&[]);
        }
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(slice.data, slice.len) })
}

fn empty_event() -> StorkvWatchEvent {
    StorkvWatchEvent {
        kind: StorkvEventKind::Put,
        scope_id: 0,
        seq: 0,
        key: StorkvBytes::EMPTY,
        value: StorkvBytes::EMPTY,
    }
}

/// Open a watch stream for every key with a prefix.
///
/// # Safety
///
/// `scope` must be a live scope handle, `prefix` must reference readable bytes,
/// and `out_watch` must point to writable storage for one watch handle pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_watch_prefix(
    scope: *mut StorkvScopeHandle,
    prefix: StorkvSlice,
    out_watch: *mut *mut StorkvWatchHandle,
) -> StorkvStatus {
    guard_status(|| {
        if out_watch.is_null() {
            return failure(StorkvStatus::NullPointer, "out watch handle is null");
        }
        unsafe { *out_watch = std::ptr::null_mut() };
        if scope.is_null() {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        }
        let Some(prefix) = slice_bytes(prefix) else {
            return failure(StorkvStatus::InvalidArgument, "prefix pointer is null");
        };
        let Some(handle) = scope_from_raw(scope) else {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        };
        match exec(&handle.runtime, handle.scope.watch_prefix(prefix)) {
            Ok(stream) => {
                clear_error();
                unsafe {
                    *out_watch = watch_into_raw(WatchHandle {
                        runtime: handle.runtime.clone(),
                        stream: tokio::sync::Mutex::new(stream),
                    })
                };
                StorkvStatus::Ok
            }
            Err(error) => fail_with(error),
        }
    })
}

/// Receive the next watch event.
///
/// A `timeout_ms` of zero waits indefinitely. On success the caller owns
/// `out_event` and must release `key` and `value` with `storkv_bytes_free`.
///
/// # Safety
///
/// `watch` must be a live watch handle and `out_event` must point to writable
/// storage for one event.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_watch_next(
    watch: *mut StorkvWatchHandle,
    out_event: *mut StorkvWatchEvent,
    timeout_ms: u64,
) -> StorkvStatus {
    guard_status(|| {
        if watch.is_null() {
            return failure(StorkvStatus::NullPointer, "watch handle is null");
        }
        if out_event.is_null() {
            return failure(StorkvStatus::NullPointer, "out_event is null");
        }
        unsafe { *out_event = empty_event() };
        let Some(handle) = watch_from_raw(watch) else {
            return failure(StorkvStatus::NullPointer, "watch handle is null");
        };
        let future = async {
            let mut guard = handle.stream.lock().await;
            guard.next().await
        };
        let timed = async {
            if timeout_ms == 0 {
                Some(future.await)
            } else {
                tokio::time::timeout(Duration::from_millis(timeout_ms), future)
                    .await
                    .ok()
            }
        };
        match exec(&handle.runtime, timed) {
            Some(Some(Ok(event))) => {
                clear_error();
                let kind = match event.kind {
                    WatchEventKind::Put => StorkvEventKind::Put,
                    WatchEventKind::Delete => StorkvEventKind::Delete,
                    WatchEventKind::DropScope => StorkvEventKind::DropScope,
                    WatchEventKind::Expired => StorkvEventKind::Expired,
                };
                unsafe {
                    *out_event = StorkvWatchEvent {
                        kind,
                        scope_id: event.scope_id,
                        seq: event.seq,
                        key: bytes_to_ffi(Some(event.key)),
                        value: bytes_to_ffi(event.value),
                    }
                };
                StorkvStatus::Ok
            }
            Some(Some(Err(error))) => fail_with(error),
            Some(None) => failure(StorkvStatus::Watch, "watch stream ended"),
            None => failure(StorkvStatus::Timeout, "watch timed out"),
        }
    })
}

/// Close a watch handle.
///
/// # Safety
///
/// `watch` must come from [`storkv_watch_prefix`]. Do not use it after
/// this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_watch_close(watch: *mut StorkvWatchHandle) {
    guard_void(|| drop_watch(watch));
}
