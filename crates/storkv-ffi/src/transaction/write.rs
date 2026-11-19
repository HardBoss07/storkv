//! Transaction write, commit, and rollback operations for the C ABI.

use std::time::Duration;

use crate::error::{clear_error, fail_with, failure};
use crate::handles::{TxHandle, drop_tx, exec, scope_from_raw, tx_from_raw};
use crate::types::{StorkvScopeHandle, StorkvSlice, StorkvStatus, StorkvTxHandle};

use super::{slice_bytes, transaction_ref};

/// Buffer a key/value write inside a transaction.
///
/// # Safety
///
/// Both handles must be live and both slices must reference readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_tx_put(
    tx: *mut StorkvTxHandle,
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
    value: StorkvSlice,
) -> StorkvStatus {
    write(tx, scope, key, value, None)
}

/// Buffer a key/value write with a time to live in milliseconds.
///
/// # Safety
///
/// Both handles must be live and both slices must reference readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_tx_put_with_ttl(
    tx: *mut StorkvTxHandle,
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
    value: StorkvSlice,
    ttl_ms: u64,
) -> StorkvStatus {
    write(tx, scope, key, value, Some(Duration::from_millis(ttl_ms)))
}

fn write(
    tx: *mut StorkvTxHandle,
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
    value: StorkvSlice,
    ttl: Option<Duration>,
) -> StorkvStatus {
    if tx.is_null() || scope.is_null() {
        return failure(StorkvStatus::NullPointer, "handle is null");
    }
    let Some(key) = slice_bytes(key) else {
        return failure(StorkvStatus::InvalidArgument, "key pointer is null");
    };
    let Some(value) = slice_bytes(value) else {
        return failure(StorkvStatus::InvalidArgument, "value pointer is null");
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
    let result = match ttl {
        Some(ttl) => exec(
            &tx_handle.runtime,
            transaction.put_with_ttl(&scope_handle.scope, key, value, ttl),
        ),
        None => exec(
            &tx_handle.runtime,
            transaction.put(&scope_handle.scope, key, value),
        ),
    };
    match result {
        Ok(()) => {
            clear_error();
            StorkvStatus::Ok
        }
        Err(error) => fail_with(error),
    }
}

/// Buffer a delete inside a transaction.
///
/// # Safety
///
/// Both handles must be live and `key` must reference readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_tx_delete(
    tx: *mut StorkvTxHandle,
    scope: *mut StorkvScopeHandle,
    key: StorkvSlice,
) -> StorkvStatus {
    if tx.is_null() || scope.is_null() {
        return failure(StorkvStatus::NullPointer, "handle is null");
    }
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
        transaction.delete(&scope_handle.scope, key),
    ) {
        Ok(()) => {
            clear_error();
            StorkvStatus::Ok
        }
        Err(error) => fail_with(error),
    }
}

/// Commit a transaction and release its handle.
///
/// # Safety
///
/// `tx` must come from [`storkv_tx_begin`](crate::transaction::storkv_tx_begin).
/// Do not use it after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_tx_commit(tx: *mut StorkvTxHandle) -> StorkvStatus {
    if tx.is_null() {
        return failure(StorkvStatus::NullPointer, "transaction handle is null");
    }
    let mut handle = unsafe { Box::from_raw(tx.cast::<TxHandle>()) };
    if !handle.owned {
        return failure(
            StorkvStatus::InvalidArgument,
            "a callback transaction is committed by the retry helper",
        );
    }
    if handle.transaction.is_null() {
        return failure(StorkvStatus::Transaction, "transaction is finished");
    }
    let transaction = unsafe { *Box::from_raw(handle.transaction) };
    handle.transaction = std::ptr::null_mut();
    let result = exec(&handle.runtime, transaction.commit());
    match result {
        Ok(()) => {
            clear_error();
            StorkvStatus::Ok
        }
        Err(error) => fail_with(error),
    }
}

/// Roll back a transaction and release its handle.
///
/// # Safety
///
/// `tx` must come from [`storkv_tx_begin`](crate::transaction::storkv_tx_begin).
/// Do not use it after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_tx_rollback(tx: *mut StorkvTxHandle) {
    drop_tx(tx);
}
