use std::ffi::{CStr, c_char};

use crate::error::{clear_error, fail_with, failure};
use crate::handles::{
    PoolHandle, ScopeHandle, build_runtime, drop_pool, drop_scope, exec, pool_from_raw,
    pool_into_raw, scope_into_raw,
};
use crate::types::{StorkvPoolConfig, StorkvPoolHandle, StorkvScopeHandle, StorkvStatus};

fn c_string(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    Some(
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// Open a pooled connection to an embedded or networked StorKV backend.
///
/// # Safety
///
/// `uri` must be a valid NUL-terminated string. `config` may be null. `out`
/// must point to writable storage for one handle pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_pool_open(
    uri: *const c_char,
    config: *const StorkvPoolConfig,
    out: *mut *mut StorkvPoolHandle,
) -> StorkvStatus {
    if out.is_null() {
        return failure(StorkvStatus::NullPointer, "out pool handle is null");
    }
    unsafe { *out = std::ptr::null_mut() };
    let Some(uri) = c_string(uri) else {
        return failure(StorkvStatus::NullPointer, "uri is null");
    };
    let config = if config.is_null() {
        StorkvPoolConfig::default()
    } else {
        unsafe { *config }
    };
    let runtime = match build_runtime() {
        Ok(runtime) => runtime,
        Err(error) => return failure(StorkvStatus::Internal, error.to_string()),
    };
    match exec(
        &runtime,
        storkv_sdk::StorkvPool::connect_with(&uri, config.to_options()),
    ) {
        Ok(pool) => {
            clear_error();
            let handle = pool_into_raw(PoolHandle { runtime, pool });
            unsafe { *out = handle };
            StorkvStatus::Ok
        }
        Err(error) => fail_with(error),
    }
}

/// Close a pool handle.
///
/// # Safety
///
/// `pool` must come from [`storkv_pool_open`]. Do not use it after this
/// call. Scope handles opened from the pool remain valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_pool_close(pool: *mut StorkvPoolHandle) {
    if let Some(handle) = pool_from_raw(pool) {
        exec(&handle.runtime, handle.pool.close());
    }
    drop_pool(pool);
}

/// Report whether a pool is closed.
///
/// # Safety
///
/// `pool` must be a live pool handle and `out_closed` must point to writable
/// storage for one integer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_pool_is_closed(
    pool: *mut StorkvPoolHandle,
    out_closed: *mut i32,
) -> StorkvStatus {
    if pool.is_null() {
        return failure(StorkvStatus::NullPointer, "pool handle is null");
    }
    if out_closed.is_null() {
        return failure(StorkvStatus::NullPointer, "out_closed is null");
    }
    let Some(handle) = pool_from_raw(pool) else {
        return failure(StorkvStatus::NullPointer, "pool handle is null");
    };
    unsafe { *out_closed = if handle.pool.is_closed() { 1 } else { 0 } };
    clear_error();
    StorkvStatus::Ok
}

/// Open a lightweight handle for one scope.
///
/// # Safety
///
/// `pool` must be a live pool handle, `name` must be a valid NUL-terminated
/// string, and `out` must point to writable storage for one handle pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_scope_open(
    pool: *mut StorkvPoolHandle,
    name: *const c_char,
    out: *mut *mut StorkvScopeHandle,
) -> StorkvStatus {
    if out.is_null() {
        return failure(StorkvStatus::NullPointer, "out scope handle is null");
    }
    unsafe { *out = std::ptr::null_mut() };
    if pool.is_null() {
        return failure(StorkvStatus::NullPointer, "pool handle is null");
    }
    let Some(name) = c_string(name) else {
        return failure(StorkvStatus::NullPointer, "scope name is null");
    };
    let Some(handle) = pool_from_raw(pool) else {
        return failure(StorkvStatus::NullPointer, "pool handle is null");
    };
    let scope = handle.pool.scope(name);
    clear_error();
    unsafe {
        *out = scope_into_raw(ScopeHandle {
            runtime: handle.runtime.clone(),
            scope,
        })
    };
    StorkvStatus::Ok
}

/// Close a scope handle.
///
/// # Safety
///
/// `scope` must come from [`storkv_scope_open`]. Do not use it after this
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn storkv_scope_close(scope: *mut StorkvScopeHandle) {
    drop_scope(scope);
}
