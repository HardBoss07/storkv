//! Atomic conditional write for the C ABI.

use std::time::Duration;

use crate::error::{clear_error, fail_with, failure, guard_status};
use crate::handles::{exec, scope_from_raw};
use crate::types::{
    StorkvPutCondition, StorkvPutIfResult, StorkvScopeHandle, StorkvSlice, StorkvStatus,
};

use super::read::slice_bytes;

/// Store one key/value pair when a condition matches.
///
/// `ttl_ms` of `0` writes a permanent key. `out_result.seq` is `0` on a
/// mismatch when the key is absent.
///
/// # Safety
///
/// `scope` must be a live scope handle. `key`, `condition`, and `value` must
/// reference readable memory. `out_result` must point to writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_put_if(
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
    condition: *const StorkvPutCondition,
    value: StorkvSlice,
    ttl_ms: u64,
    out_result: *mut StorkvPutIfResult,
) -> StorkvStatus {
    guard_status(|| {
        if scope.is_null() {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        }
        if condition.is_null() {
            return failure(StorkvStatus::NullPointer, "condition is null");
        }
        if out_result.is_null() {
            return failure(StorkvStatus::NullPointer, "out_result is null");
        }
        unsafe { *out_result = StorkvPutIfResult::EMPTY };
        let Some(key) = slice_bytes(key) else {
            return failure(StorkvStatus::InvalidArgument, "key pointer is null");
        };
        let Some(value) = slice_bytes(value) else {
            return failure(StorkvStatus::InvalidArgument, "value pointer is null");
        };
        let Some(handle) = scope_from_raw(scope) else {
            return failure(StorkvStatus::NullPointer, "scope handle is null");
        };
        let Some(core_condition) = unsafe { *condition }.to_core() else {
            return failure(StorkvStatus::InvalidArgument, "unsupported put condition");
        };
        let ttl = (ttl_ms != 0).then(|| Duration::from_millis(ttl_ms));
        match exec(
            &handle.runtime,
            handle.scope.put_if(key, &core_condition, value, ttl),
        ) {
            Ok(result) => {
                clear_error();
                let (applied, seq) = match result {
                    storkv_sdk::PutIfResult::Applied { seq } => (1u8, seq),
                    storkv_sdk::PutIfResult::Conflict { current_seq, .. } => {
                        (0u8, current_seq.unwrap_or(0))
                    }
                    _ => (0u8, 0u64),
                };
                unsafe { *out_result = StorkvPutIfResult { applied, seq } };
                StorkvStatus::Ok
            }
            Err(error) => fail_with(error),
        }
    })
}
