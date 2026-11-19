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
/// Release the returned string with [`storkv_string_free`].
#[unsafe(no_mangle)]
pub extern "C" fn storkv_last_error_message() -> StorkvString {
    LAST_ERROR.with(|slot| {
        let borrowed = slot.borrow();
        let Some(message) = borrowed.as_ref() else {
            return StorkvString::EMPTY;
        };
        let bytes = message.as_bytes();
        let copy = CString::new(bytes).unwrap_or_default();
        let len = copy.as_bytes().len();
        let data = copy.into_raw();
        StorkvString { data, len }
    })
}

/// Release a string returned by [`storkv_last_error_message`].
///
/// # Safety
///
/// `value` must come from [`storkv_last_error_message`]. Do not use it after
/// this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_string_free(value: StorkvString) {
    if !value.data.is_null() {
        drop(unsafe { CString::from_raw(value.data) });
    }
}
