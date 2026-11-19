use std::ffi::{CString, c_void};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

use storkv_ffi::{
    StorkvBytes, StorkvEntries, StorkvEventKind, StorkvPoolHandle, StorkvScopeHandle, StorkvSlice,
    StorkvStatus, StorkvTxHandle, StorkvWatchEvent, StorkvWatchHandle, storkv_bytes_free,
    storkv_delete, storkv_entries_free, storkv_exists, storkv_get, storkv_get_and_touch,
    storkv_pool_close, storkv_pool_is_closed, storkv_pool_open, storkv_put, storkv_put_with_ttl,
    storkv_scan_prefix, storkv_scope_close, storkv_scope_open, storkv_tx_begin, storkv_tx_commit,
    storkv_tx_get, storkv_tx_put, storkv_tx_rollback, storkv_tx_run, storkv_watch_close,
    storkv_watch_next, storkv_watch_prefix,
};

static COUNTER: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let count = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "storkv_ffi_test_{}_{}_{}",
            prefix,
            std::process::id(),
            count
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn slice(bytes: &[u8]) -> StorkvSlice {
    StorkvSlice {
        data: bytes.as_ptr(),
        len: bytes.len(),
    }
}

unsafe fn take_bytes(bytes: StorkvBytes) -> Vec<u8> {
    if bytes.data.is_null() {
        return Vec::new();
    }
    let owned = unsafe { std::slice::from_raw_parts(bytes.data, bytes.len) }.to_vec();
    unsafe { storkv_bytes_free(bytes) };
    owned
}

unsafe fn take_entries(entries: StorkvEntries) -> Vec<(Vec<u8>, Vec<u8>)> {
    if entries.entries.is_null() {
        return Vec::new();
    }
    let slice = unsafe { std::slice::from_raw_parts(entries.entries, entries.len) };
    let mut owned = Vec::with_capacity(entries.len);
    for entry in slice {
        let key = unsafe { std::slice::from_raw_parts(entry.key.data, entry.key.len) }.to_vec();
        let value =
            unsafe { std::slice::from_raw_parts(entry.value.data, entry.value.len) }.to_vec();
        owned.push((key, value));
    }
    unsafe { storkv_entries_free(entries) };
    owned
}

#[test]
fn embedded_ffi_round_trip() {
    let dir = TempDir::new("roundtrip");
    let uri = CString::new(format!("file://{}", dir.path.display())).unwrap();
    let mut pool: *mut StorkvPoolHandle = ptr::null_mut();
    assert_eq!(
        unsafe { storkv_pool_open(uri.as_ptr(), ptr::null(), &mut pool) },
        StorkvStatus::Ok
    );
    assert!(!pool.is_null());

    let name = CString::new("default").unwrap();
    let mut scope: *mut StorkvScopeHandle = ptr::null_mut();
    assert_eq!(
        unsafe { storkv_scope_open(pool, name.as_ptr(), &mut scope) },
        StorkvStatus::Ok
    );

    assert_eq!(
        unsafe { storkv_put(scope, slice(b"alpha"), slice(b"one")) },
        StorkvStatus::Ok
    );
    let mut value = StorkvBytes::EMPTY;
    assert_eq!(
        unsafe { storkv_get(scope, slice(b"alpha"), &mut value) },
        StorkvStatus::Ok
    );
    assert_eq!(unsafe { take_bytes(value) }, b"one");

    let mut exists = 0;
    assert_eq!(
        unsafe { storkv_exists(scope, slice(b"alpha"), &mut exists) },
        StorkvStatus::Ok
    );
    assert_eq!(exists, 1);

    assert_eq!(
        unsafe { storkv_put_with_ttl(scope, slice(b"session"), slice(b"live"), 150,) },
        StorkvStatus::Ok
    );
    std::thread::sleep(std::time::Duration::from_millis(350));
    let mut expired = StorkvBytes::EMPTY;
    assert_eq!(
        unsafe { storkv_get(scope, slice(b"session"), &mut expired) },
        StorkvStatus::Ok
    );
    assert!(unsafe { take_bytes(expired) }.is_empty());

    let mut touched = StorkvBytes::EMPTY;
    assert_eq!(
        unsafe { storkv_get_and_touch(scope, slice(b"alpha"), 5_000, &mut touched) },
        StorkvStatus::Ok
    );
    assert_eq!(unsafe { take_bytes(touched) }, b"one");

    for index in 0..3u8 {
        let key = format!("scan:{index}");
        let value = format!("value-{index}");
        assert_eq!(
            unsafe { storkv_put(scope, slice(key.as_bytes()), slice(value.as_bytes())) },
            StorkvStatus::Ok
        );
    }
    let mut entries = StorkvEntries::EMPTY;
    assert_eq!(
        unsafe { storkv_scan_prefix(scope, slice(b"scan:"), &mut entries) },
        StorkvStatus::Ok
    );
    assert_eq!(unsafe { take_entries(entries) }.len(), 3);

    let mut transaction: *mut StorkvTxHandle = ptr::null_mut();
    assert_eq!(
        unsafe { storkv_tx_begin(pool, &mut transaction) },
        StorkvStatus::Ok
    );
    assert_eq!(
        unsafe { storkv_tx_put(transaction, scope, slice(b"tx-key"), slice(b"tx-value")) },
        StorkvStatus::Ok
    );
    let mut tx_value = StorkvBytes::EMPTY;
    assert_eq!(
        unsafe { storkv_tx_get(transaction, scope, slice(b"tx-key"), &mut tx_value) },
        StorkvStatus::Ok
    );
    assert_eq!(unsafe { take_bytes(tx_value) }, b"tx-value");
    assert_eq!(unsafe { storkv_tx_commit(transaction) }, StorkvStatus::Ok);

    let mut transaction: *mut StorkvTxHandle = ptr::null_mut();
    assert_eq!(
        unsafe { storkv_tx_begin(pool, &mut transaction) },
        StorkvStatus::Ok
    );
    assert_eq!(
        unsafe { storkv_tx_put(transaction, scope, slice(b"rolled"), slice(b"back")) },
        StorkvStatus::Ok
    );
    unsafe { storkv_tx_rollback(transaction) };
    let mut rolled = StorkvBytes::EMPTY;
    assert_eq!(
        unsafe { storkv_get(scope, slice(b"rolled"), &mut rolled) },
        StorkvStatus::Ok
    );
    assert!(unsafe { take_bytes(rolled) }.is_empty());

    assert_eq!(
        unsafe { storkv_put(scope, slice(b"counter"), slice(b"0")) },
        StorkvStatus::Ok
    );
    let mut committed = 0;
    assert_eq!(
        unsafe {
            storkv_tx_run(
                pool,
                ptr::null(),
                Some(bump_counter),
                (&mut scope) as *mut *mut StorkvScopeHandle as *mut c_void,
                &mut committed,
            )
        },
        StorkvStatus::Ok
    );
    assert_eq!(committed, 1);
    let mut counter = StorkvBytes::EMPTY;
    assert_eq!(
        unsafe { storkv_get(scope, slice(b"counter"), &mut counter) },
        StorkvStatus::Ok
    );
    assert_eq!(unsafe { take_bytes(counter) }, b"1");

    let mut watch: *mut StorkvWatchHandle = ptr::null_mut();
    assert_eq!(
        unsafe { storkv_watch_prefix(scope, slice(b"wf:"), &mut watch) },
        StorkvStatus::Ok
    );
    assert_eq!(
        unsafe { storkv_put(scope, slice(b"wf:key"), slice(b"event")) },
        StorkvStatus::Ok
    );
    let mut event = std::mem::MaybeUninit::<StorkvWatchEvent>::uninit();
    assert_eq!(
        unsafe { storkv_watch_next(watch, event.as_mut_ptr(), 5_000) },
        StorkvStatus::Ok
    );
    let event = unsafe { event.assume_init() };
    assert_eq!(event.kind, StorkvEventKind::Put);
    assert_eq!(unsafe { take_bytes(event.key) }, b"wf:key");
    assert_eq!(unsafe { take_bytes(event.value) }, b"event");
    unsafe { storkv_watch_close(watch) };

    assert_eq!(
        unsafe { storkv_delete(scope, slice(b"alpha")) },
        StorkvStatus::Ok
    );
    let mut closed = 0;
    assert_eq!(
        unsafe { storkv_pool_is_closed(pool, &mut closed) },
        StorkvStatus::Ok
    );
    assert_eq!(closed, 0);
    unsafe { storkv_scope_close(scope) };
    unsafe { storkv_pool_close(pool) };
}

unsafe extern "C" fn bump_counter(
    transaction: *mut StorkvTxHandle,
    user: *mut c_void,
) -> StorkvStatus {
    let scope = unsafe { *(user as *const *mut StorkvScopeHandle) };
    let mut value = StorkvBytes::EMPTY;
    let status = unsafe { storkv_tx_get(transaction, scope, slice(b"counter"), &mut value) };
    if status != StorkvStatus::Ok {
        return status;
    }
    let current = String::from_utf8_lossy(&unsafe { take_bytes(value) })
        .parse::<u64>()
        .unwrap_or(0);
    let next = (current + 1).to_string();
    unsafe {
        storkv_tx_put(
            transaction,
            scope,
            slice(b"counter"),
            slice(next.as_bytes()),
        )
    }
}

#[test]
fn null_pointers_are_rejected() {
    let mut value = StorkvBytes::EMPTY;
    assert_eq!(
        unsafe { storkv_get(ptr::null_mut(), slice(b"k"), &mut value) },
        StorkvStatus::NullPointer
    );
    let mut pool: *mut StorkvPoolHandle = ptr::null_mut();
    assert_eq!(
        unsafe { storkv_pool_open(ptr::null(), ptr::null(), &mut pool) },
        StorkvStatus::NullPointer
    );
    assert!(pool.is_null());
}
