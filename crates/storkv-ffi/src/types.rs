use std::ffi::c_void;
use std::time::Duration;

mod put_if;

pub use put_if::{StorkvPutCondition, StorkvPutConditionKind, StorkvPutIfResult};

/// Status code returned by every FFI function.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorkvStatus {
    /// The operation succeeded.
    Ok = 0,
    /// A required pointer argument was null.
    NullPointer = 1,
    /// The caller supplied a malformed argument.
    InvalidArgument = 2,
    /// The pool cannot open or is closed.
    Pool = 3,
    /// The scope does not resolve.
    Scope = 4,
    /// The transaction cannot start or commit.
    Transaction = 5,
    /// Encoding or decoding a value failed.
    Serialization = 6,
    /// The watch stream failed.
    Watch = 7,
    /// The operation timed out.
    Timeout = 8,
    /// Any other internal failure.
    Internal = 9,
}

/// Kind of change carried by [`StorkvWatchEvent`].
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorkvEventKind {
    /// A key received a new value.
    Put = 0,
    /// A delete event for one key.
    Delete = 1,
    /// A drop event for one scope.
    DropScope = 2,
}

/// Owned byte buffer allocated by the FFI layer.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StorkvBytes {
    /// Pointer to the first byte. Null when `len` is zero.
    pub data: *mut u8,
    /// Number of bytes.
    pub len: usize,
}

impl StorkvBytes {
    /// A buffer with no data.
    pub const EMPTY: Self = Self {
        data: std::ptr::null_mut(),
        len: 0,
    };
}

/// Borrowed byte slice passed into the FFI layer.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StorkvSlice {
    /// Pointer to the first byte. Null means an open range bound.
    pub data: *const u8,
    /// Number of bytes.
    pub len: usize,
}

impl StorkvSlice {
    /// An empty slice.
    pub const EMPTY: Self = Self {
        data: std::ptr::null(),
        len: 0,
    };

    /// Borrow the slice as bytes.
    ///
    /// # Safety
    ///
    /// The pointer must reference `len` initialised bytes for the lifetime of
    /// the returned slice.
    pub unsafe fn as_bytes(&self) -> &[u8] {
        if self.data.is_null() || self.len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(self.data, self.len) }
    }

    /// Return `true` when the slice is null and therefore an open bound.
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.data.is_null()
    }
}

/// Owned, NUL-terminated string allocated by the FFI layer.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StorkvString {
    /// Pointer to the first character.
    pub data: *mut std::ffi::c_char,
    /// Number of characters before the terminating NUL.
    pub len: usize,
}

impl StorkvString {
    /// An empty string.
    pub const EMPTY: Self = Self {
        data: std::ptr::null_mut(),
        len: 0,
    };
}

/// Configuration for a pooled FFI connection.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StorkvPoolConfig {
    /// Probe idle connections before reuse.
    pub health_check: bool,
    /// Maximum number of simultaneous connections.
    pub max_connections: u32,
    /// Number of connections opened eagerly.
    pub min_connections: u32,
    /// Acquire timeout in milliseconds. Zero selects the SDK default.
    pub acquire_timeout_ms: u64,
    /// Idle timeout in milliseconds. Zero disables idle expiry.
    pub idle_timeout_ms: u64,
}

impl Default for StorkvPoolConfig {
    fn default() -> Self {
        Self {
            health_check: true,
            max_connections: 10,
            min_connections: 1,
            acquire_timeout_ms: 5_000,
            idle_timeout_ms: 300_000,
        }
    }
}

impl StorkvPoolConfig {
    pub(crate) fn to_options(self) -> storkv_sdk::StorkvPoolOptions {
        let mut options = storkv_sdk::StorkvPoolOptions::new()
            .health_check(self.health_check)
            .max_connections(self.max_connections)
            .min_connections(self.min_connections);
        if self.acquire_timeout_ms > 0 {
            options = options.acquire_timeout(Duration::from_millis(self.acquire_timeout_ms));
        }
        options = options.idle_timeout(if self.idle_timeout_ms == 0 {
            None
        } else {
            Some(Duration::from_millis(self.idle_timeout_ms))
        });
        options
    }
}

/// Retry configuration for the automatic transaction helper.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StorkvRetryConfig {
    /// Add random jitter to each backoff delay.
    pub jitter: bool,
    /// Maximum number of retries after the first attempt.
    pub max_retries: u32,
    /// Delay before the first retry, in milliseconds.
    pub base_delay_ms: u64,
    /// Maximum backoff delay, in milliseconds.
    pub max_delay_ms: u64,
}

impl Default for StorkvRetryConfig {
    fn default() -> Self {
        Self {
            jitter: true,
            max_retries: 5,
            base_delay_ms: 2,
            max_delay_ms: 250,
        }
    }
}

impl StorkvRetryConfig {
    pub(crate) fn to_policy(self) -> storkv_sdk::RetryPolicy {
        storkv_sdk::RetryPolicy::default()
            .with_jitter(self.jitter)
            .with_max_retries(self.max_retries)
            .with_base_delay(Duration::from_millis(self.base_delay_ms))
            .with_max_delay(Duration::from_millis(self.max_delay_ms))
    }
}

/// One key/value pair returned by a scan.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StorkvEntry {
    /// Owned key bytes.
    pub key: StorkvBytes,
    /// Owned value bytes.
    pub value: StorkvBytes,
}

/// Owned array of scan entries.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StorkvEntries {
    /// Pointer to the first entry.
    pub entries: *mut StorkvEntry,
    /// Number of entries.
    pub len: usize,
}

impl StorkvEntries {
    /// An array with no entries.
    pub const EMPTY: Self = Self {
        entries: std::ptr::null_mut(),
        len: 0,
    };
}

/// One change event returned by a watch stream.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StorkvWatchEvent {
    /// Kind of change.
    pub kind: StorkvEventKind,
    /// Identifier of the scope that changed.
    pub scope_id: u32,
    /// Commit sequence number of the change.
    pub seq: u64,
    /// Owned key bytes.
    pub key: StorkvBytes,
    /// Owned value bytes. Empty for deletes and scope drops.
    pub value: StorkvBytes,
}

/// Callback invoked by the automatic transaction retry helper.
pub type StorkvTxCallback =
    Option<unsafe extern "C" fn(tx: *mut StorkvTxHandle, user: *mut c_void) -> StorkvStatus>;

/// cbindgen:no-export
/// Opaque pool handle.
#[repr(C)]
pub struct StorkvPoolHandle {
    _private: [u8; 0],
}

/// cbindgen:no-export
/// Opaque scope handle.
#[repr(C)]
pub struct StorkvScopeHandle {
    _private: [u8; 0],
}

/// cbindgen:no-export
/// Opaque transaction handle.
#[repr(C)]
pub struct StorkvTxHandle {
    _private: [u8; 0],
}

/// cbindgen:no-export
/// Opaque watch handle.
#[repr(C)]
pub struct StorkvWatchHandle {
    _private: [u8; 0],
}
