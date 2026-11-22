//! C ABI for the StorKV SDK.
//!
//! The crate builds a `cdylib` and a `staticlib`. Every exported function
//! returns [`StorkvStatus`]. Each function records a message for the calling
//! thread. Callers fetch the message with [`storkv_last_error_message`].
//!
//! # Layout
//!
//! Every public struct uses `#[repr(C)]`. Fields go from the smallest binary
//! field to the largest. This rule keeps alignment simple and stable. The
//! generated header is `bindings/c/storkv.h`.
//!
//! ```
//! use std::mem::{align_of, size_of};
//! use storkv_ffi::{StorkvBytes, StorkvWatchEvent};
//!
//! assert_eq!(size_of::<StorkvBytes>(), 16);
//! assert_eq!(align_of::<StorkvBytes>(), 8);
//! assert_eq!(size_of::<StorkvWatchEvent>(), 48);
//! assert_eq!(align_of::<StorkvWatchEvent>(), 8);
//! ```
//!
//! # Buffer ownership
//!
//! An input [`StorkvSlice`] is a borrow. Keep the bytes alive during the call.
//! The library copies the bytes before the call returns.
//!
//! An output buffer belongs to the caller. Release a [`StorkvBytes`] with
//! [`storkv_bytes_free`]. Release a [`StorkvEntries`] with
//! [`storkv_entries_free`]. Release a [`StorkvString`] with
//! [`storkv_string_free`]. Do not call `free` on a library buffer.
//!
//! # Thread safety
//!
//! Each pool owns a multithreaded Tokio runtime. Foreign callers never create
//! or poll Rust futures.
//!
//! A pool handle can serve more than one thread. Do not use one transaction
//! handle or one watch handle from two threads at the same time. The last
//! error message is thread-local. Read it on the calling thread.
//!
//! # Handle lifecycle
//!
//! 1. [`storkv_pool_open`] creates a pool.
//! 2. [`storkv_scope_open`] creates a lightweight scope handle.
//! 3. Data functions, transactions, and watches use those handles.
//! 4. Close each watch and each transaction first.
//! 5. [`storkv_scope_close`] and [`storkv_pool_close`] release the handles.
//!
//! # Documentation
//!
//! - [C ABI reference](https://github.com/HardBoss07/storkv/blob/main/docs/reference/ffi-c-abi.md)
//! - [SDK overview](https://github.com/HardBoss07/storkv/blob/main/docs/storkv-sdk/index.md)

#![warn(missing_docs)]

mod error;
mod handles;
mod memory;
mod pool;
mod scope;
mod transaction;
mod types;
mod watch;

pub use error::{storkv_last_error_message, storkv_string_free};
pub use memory::{storkv_bytes_free, storkv_entries_free};
pub use pool::{
    storkv_pool_close, storkv_pool_is_closed, storkv_pool_open, storkv_scope_close,
    storkv_scope_open,
};
pub use scope::{
    storkv_delete, storkv_exists, storkv_get, storkv_get_and_touch, storkv_put, storkv_put_if,
    storkv_put_with_ttl, storkv_scan_prefix, storkv_scan_range,
};
pub use transaction::{
    storkv_tx_begin, storkv_tx_commit, storkv_tx_delete, storkv_tx_get, storkv_tx_put,
    storkv_tx_put_with_ttl, storkv_tx_rollback, storkv_tx_run,
};
pub use types::{
    StorkvBytes, StorkvEntries, StorkvEntry, StorkvEventKind, StorkvPoolConfig, StorkvPoolHandle,
    StorkvPutCondition, StorkvPutConditionKind, StorkvPutIfResult, StorkvRetryConfig,
    StorkvScopeHandle, StorkvSlice, StorkvStatus, StorkvString, StorkvTxCallback, StorkvTxHandle,
    StorkvWatchEvent, StorkvWatchHandle,
};
pub use watch::{storkv_watch_close, storkv_watch_next, storkv_watch_prefix};
