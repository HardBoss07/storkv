//! Opaque handle allocation and ownership helpers for the C ABI.

use std::future::Future;
use std::sync::Arc;

use storkv_sdk::{Scope, StorkvPool, Transaction, WatchStream};
use tokio::runtime::Runtime;

use crate::types::{StorkvPoolHandle, StorkvScopeHandle, StorkvTxHandle, StorkvWatchHandle};

pub(crate) struct PoolHandle {
    pub(crate) runtime: Arc<Runtime>,
    pub(crate) pool: StorkvPool,
}

pub(crate) struct ScopeHandle {
    pub(crate) runtime: Arc<Runtime>,
    pub(crate) scope: Scope,
}

pub(crate) struct TxHandle {
    pub(crate) runtime: Arc<Runtime>,
    pub(crate) transaction: *mut Transaction,
    pub(crate) owned: bool,
}

pub(crate) struct WatchHandle {
    pub(crate) runtime: Arc<Runtime>,
    pub(crate) stream: tokio::sync::Mutex<WatchStream>,
}

pub(crate) fn exec<F: Future>(runtime: &Runtime, future: F) -> F::Output {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(|| runtime.block_on(future))
    } else {
        runtime.block_on(future)
    }
}

pub(crate) fn pool_from_raw<'a>(pointer: *mut StorkvPoolHandle) -> Option<&'a PoolHandle> {
    unsafe { pointer.cast::<PoolHandle>().as_ref() }
}

pub(crate) fn scope_from_raw<'a>(pointer: *mut StorkvScopeHandle) -> Option<&'a ScopeHandle> {
    unsafe { pointer.cast::<ScopeHandle>().as_ref() }
}

pub(crate) fn tx_from_raw<'a>(pointer: *mut StorkvTxHandle) -> Option<&'a TxHandle> {
    unsafe { pointer.cast::<TxHandle>().as_ref() }
}

pub(crate) fn watch_from_raw<'a>(pointer: *mut StorkvWatchHandle) -> Option<&'a WatchHandle> {
    unsafe { pointer.cast::<WatchHandle>().as_ref() }
}

pub(crate) fn pool_into_raw(handle: PoolHandle) -> *mut StorkvPoolHandle {
    Box::into_raw(Box::new(handle)).cast::<StorkvPoolHandle>()
}

pub(crate) fn scope_into_raw(handle: ScopeHandle) -> *mut StorkvScopeHandle {
    Box::into_raw(Box::new(handle)).cast::<StorkvScopeHandle>()
}

pub(crate) fn tx_into_raw(handle: TxHandle) -> *mut StorkvTxHandle {
    Box::into_raw(Box::new(handle)).cast::<StorkvTxHandle>()
}

pub(crate) fn watch_into_raw(handle: WatchHandle) -> *mut StorkvWatchHandle {
    Box::into_raw(Box::new(handle)).cast::<StorkvWatchHandle>()
}

pub(crate) fn drop_pool(pointer: *mut StorkvPoolHandle) {
    if !pointer.is_null() {
        drop(unsafe { Box::from_raw(pointer.cast::<PoolHandle>()) });
    }
}

pub(crate) fn drop_scope(pointer: *mut StorkvScopeHandle) {
    if !pointer.is_null() {
        drop(unsafe { Box::from_raw(pointer.cast::<ScopeHandle>()) });
    }
}

pub(crate) fn drop_tx(pointer: *mut StorkvTxHandle) {
    if pointer.is_null() {
        return;
    }
    let handle = unsafe { Box::from_raw(pointer.cast::<TxHandle>()) };
    if handle.owned && !handle.transaction.is_null() {
        drop(unsafe { Box::from_raw(handle.transaction) });
    }
}

pub(crate) fn drop_watch(pointer: *mut StorkvWatchHandle) {
    if !pointer.is_null() {
        drop(unsafe { Box::from_raw(pointer.cast::<WatchHandle>()) });
    }
}

pub(crate) fn build_runtime() -> std::io::Result<Arc<Runtime>> {
    Ok(Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("storkv-ffi")
            .build()?,
    ))
}
