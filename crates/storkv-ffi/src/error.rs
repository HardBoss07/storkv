//! Thread-local error recording and status conversion for the C ABI.
//!
//! Each exported call records its failure on the calling thread. The caller
//! reads the message with `storkv_last_error_message` and releases it with
//! `storkv_string_free`.

use std::cell::RefCell;
use std::ffi::CString;

use storkv_sdk::SdkError;

use crate::types::{StorkvStatus, StorkvString};

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

pub(crate) fn record_error(message: impl Into<String>) {
    let message = message.into();
    let sanitised = message.replace('\0', " ");
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = CString::new(sanitised).ok();
    });
}

pub(crate) fn clear_error() {
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

pub(crate) fn failure(status: StorkvStatus, message: impl Into<String>) -> StorkvStatus {
    record_error(message);
    status
}

/// Run an FFI entry point body and convert a panic into a status code.
///
/// A panic must never unwind across the `extern "C"` boundary because that is
/// undefined behavior in the C ABI.
pub(crate) fn guard_status<F>(body: F) -> StorkvStatus
where
    F: FnOnce() -> StorkvStatus,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(status) => status,
        Err(_) => failure(StorkvStatus::Internal, "panic in FFI call"),
    }
}

/// Run a void FFI entry point body and record a panic instead of unwinding.
pub(crate) fn guard_void<F>(body: F)
where
    F: FnOnce(),
{
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).is_err() {
        record_error("panic in FFI call");
    }
}

pub(crate) fn status_for(error: &SdkError) -> StorkvStatus {
    match error {
        SdkError::AcquireTimeout => StorkvStatus::Timeout,
        SdkError::PoolClosed => StorkvStatus::Pool,
        SdkError::ScopeNotFound(_) => StorkvStatus::Scope,
        SdkError::Serialization(_) => StorkvStatus::Serialization,
        SdkError::WatchClosed => StorkvStatus::Watch,
        SdkError::InvalidUri(_) | SdkError::Unsupported(_) => StorkvStatus::InvalidArgument,
        SdkError::Core(_) | SdkError::Client(_) | SdkError::Task(_) => StorkvStatus::Internal,
        _ => StorkvStatus::Internal,
    }
}

pub(crate) fn fail_with(error: SdkError) -> StorkvStatus {
    let status = status_for(&error);
    failure(status, error.to_string())
}

/// Return the last error message recorded on the calling thread.
///
/// # Safety
///
/// The caller must release the returned [`StorkvString`] with
/// [`storkv_string_free`]. Do not free the buffer any other way. The message is
/// thread-local, so call this function on the thread that recorded the error.
#[unsafe(no_mangle)]
pub extern "C" fn storkv_last_error_message() -> StorkvString {
    match std::panic::catch_unwind(|| {
        LAST_ERROR.with(|slot| {
            let borrowed = slot.borrow();
            let Some(message) = borrowed.as_ref() else {
                return StorkvString::EMPTY;
            };
            let Some(copy) = CString::new(message.as_bytes()).ok() else {
                return StorkvString::EMPTY;
            };
            let len = copy.as_bytes().len();
            let data = copy.into_raw();
            StorkvString { data, len }
        })
    }) {
        Ok(value) => value,
        Err(_) => StorkvString::EMPTY,
    }
}

/// Release a string returned by [`storkv_last_error_message`].
///
/// # Safety
///
/// `value` must come from [`storkv_last_error_message`]. Do not use it after
/// this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_string_free(value: StorkvString) {
    guard_void(|| {
        if !value.data.is_null() {
            drop(unsafe { CString::from_raw(value.data) });
        }
    });
}
