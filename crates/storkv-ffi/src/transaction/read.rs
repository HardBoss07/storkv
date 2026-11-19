//! Transaction read operations for the C ABI.

use crate::error::{clear_error, fail_with, failure};
use crate::handles::{exec, scope_from_raw, tx_from_raw};
use crate::types::{StorkvBytes, StorkvScopeHandle, StorkvSlice, StorkvStatus, StorkvTxHandle};

use super::{slice_bytes, transaction_ref};

/// Read one key inside a transaction and add it to the read set.
///
/// # Safety
///
/// `tx` must be a live transaction handle, `scope` must be a live scope handle,
/// and `out_value` must point to writable storage for one byte buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_tx_get(
    tx: *mut StorkvTxHandle,
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
    out_value: *mut StorkvBytes,
) -> StorkvStatus {
    if tx.is_null() || scope.is_null() {
        return failure(StorkvStatus::NullPointer, "handle is null");
    }
    if out_value.is_null() {
        return failure(StorkvStatus::NullPointer, "out_value is null");
    }
    unsafe { *out_value = StorkvBytes::EMPTY };
    let Some(key) = slice_bytes(key) else {
        return failure(StorkvStatus::InvalidArgument, "key pointer is null");
    };
    let Some(tx_handle) = tx_from_raw(tx) else {
        return failure(StorkvStatus::NullPointer, "transaction handle is null");
    };
    let Some(scope_handle) = scope_from_raw(scope) else {
        return failure(StorkvStatus::NullPointer, "scope handle is null");
    };
    let Some(transaction) = transaction_ref(tx_handle) else {
        return failure(StorkvStatus::Transaction, "transaction is finished");
    };
    match exec(
        &tx_handle.runtime,
        transaction.get(&scope_handle.scope, key),
    ) {
        Ok(value) => {
            clear_error();
            unsafe { *out_value = crate::memory::bytes_to_ffi(value) };
            StorkvStatus::Ok
        }
        Err(error) => fail_with(error),
    }
}
