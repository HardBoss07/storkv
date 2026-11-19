//! Explicit transaction entry points for the C ABI.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use storkv_sdk::{SdkError, StorkvPool, Transaction};

use crate::error::{clear_error, fail_with, failure};
use crate::handles::{TxHandle, exec, pool_from_raw, tx_into_raw};
use crate::types::{
    StorkvPoolHandle, StorkvRetryConfig, StorkvSlice, StorkvStatus, StorkvTxCallback,
    StorkvTxHandle,
};

mod read;
mod write;

pub use read::storkv_tx_get;
pub use write::{
    storkv_tx_commit, storkv_tx_delete, storkv_tx_put, storkv_tx_put_with_ttl, storkv_tx_rollback,
};

#[derive(Clone, Copy)]
struct UserPointer(*mut c_void);

unsafe impl Send for UserPointer {}
unsafe impl Sync for UserPointer {}

fn slice_bytes(slice: StorkvSlice) -> Option<&'static [u8]> {
    if slice.data.is_null() {
        if slice.len == 0 {
            return Some(&[]);
        }
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(slice.data, slice.len) })
}

/// Begin an explicit transaction on a pooled connection.
///
/// # Safety
///
/// `pool` must be a live pool handle and `out` must point to writable storage
/// for one transaction handle pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_tx_begin(
    pool: *mut StorkvPoolHandle,
    out: *mut *mut StorkvTxHandle,
) -> StorkvStatus {
    if out.is_null() {
        return failure(StorkvStatus::NullPointer, "out transaction handle is null");
    }
    unsafe { *out = std::ptr::null_mut() };
    let Some(handle) = pool_from_raw(pool) else {
        return failure(StorkvStatus::NullPointer, "pool handle is null");
    };
    let connection = match exec(&handle.runtime, handle.pool.acquire()) {
        Ok(connection) => connection,
        Err(error) => return fail_with(error),
    };
    let transaction = match exec(&handle.runtime, connection.begin()) {
        Ok(transaction) => transaction,
        Err(error) => return fail_with(error),
    };
    let transaction = Box::into_raw(Box::new(transaction));
    clear_error();
    unsafe {
        *out = tx_into_raw(TxHandle {
            runtime: handle.runtime.clone(),
            transaction,
            owned: true,
        })
    };
    StorkvStatus::Ok
}

// The handle owns a boxed `Transaction`; the FFI layer hands out a mutable
// reference so the caller can reuse it across calls. The raw pointer comes
// from `Box::into_raw`, so the pointee is uniquely owned by this handle.
#[allow(clippy::mut_from_ref)]
fn transaction_ref(handle: &TxHandle) -> Option<&mut Transaction> {
    if handle.transaction.is_null() {
        return None;
    }
    Some(unsafe { &mut *handle.transaction })
}

/// Run a callback inside a transaction with automatic OCC retries.
///
/// The callback must call [`storkv_tx_get`](crate::transaction::storkv_tx_get),
/// [`storkv_tx_put`](crate::transaction::storkv_tx_put), and
/// [`storkv_tx_delete`](crate::transaction::storkv_tx_delete) on the handle it
/// receives. It must not commit or roll back. The helper commits when the
/// callback returns [`StorkvStatus::Ok`] and rolls back otherwise.
///
/// # Safety
///
/// `pool` must be a live pool handle, `callback` must be a valid function
/// pointer, and the helper passes `user` through unchanged.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_tx_run(
    pool: *mut StorkvPoolHandle,
    config: *const StorkvRetryConfig,
    callback: StorkvTxCallback,
    user: *mut c_void,
    out_committed: *mut i32,
) -> StorkvStatus {
    if out_committed.is_null() {
        return failure(StorkvStatus::NullPointer, "out_committed is null");
    }
    unsafe { *out_committed = 0 };
    let Some(callback) = callback else {
        return failure(StorkvStatus::NullPointer, "callback is null");
    };
    let Some(handle) = pool_from_raw(pool) else {
        return failure(StorkvStatus::NullPointer, "pool handle is null");
    };
    let config = if config.is_null() {
        StorkvRetryConfig::default()
    } else {
        unsafe { *config }
    };
    let pool_ref: StorkvPool = handle.pool.clone();
    let runtime = handle.runtime.clone();
    let reported = Arc::new(AtomicI32::new(StorkvStatus::Ok as i32));
    let reported_in = reported.clone();
    let user = UserPointer(user);
    let runtime_in = runtime.clone();
    let outcome = exec(
        &runtime,
        pool_ref.transaction_with(config.to_policy(), move |transaction| {
            let reported = reported_in.clone();
            let runtime = runtime_in.clone();
            let user = user;
            let mut borrowed = TxHandle {
                runtime,
                transaction: transaction as *mut Transaction,
                owned: false,
            };
            let pointer = (&mut borrowed) as *mut TxHandle;
            let status = unsafe { callback(pointer.cast(), user.0) };
            reported.store(status as i32, Ordering::SeqCst);
            let outcome = if status == StorkvStatus::Ok {
                Ok(())
            } else {
                Err(SdkError::Unsupported(
                    "transaction callback reported failure".into(),
                ))
            };
            Box::pin(std::future::ready(outcome))
        }),
    );
    match outcome {
        Ok(()) => {
            clear_error();
            unsafe { *out_committed = 1 };
            StorkvStatus::Ok
        }
        Err(error) => {
            let reported = reported.load(Ordering::SeqCst);
            if reported == StorkvStatus::Ok as i32 {
                fail_with(error)
            } else {
                failure(
                    match reported {
                        0 => StorkvStatus::Ok,
                        1 => StorkvStatus::NullPointer,
                        2 => StorkvStatus::InvalidArgument,
                        3 => StorkvStatus::Pool,
                        4 => StorkvStatus::Scope,
                        5 => StorkvStatus::Transaction,
                        6 => StorkvStatus::Serialization,
                        7 => StorkvStatus::Watch,
                        8 => StorkvStatus::Timeout,
                        _ => StorkvStatus::Internal,
                    },
                    "transaction callback failed",
                )
            }
        }
    }
}
