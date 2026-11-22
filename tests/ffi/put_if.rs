use std::ffi::CString;
use std::path::PathBuf;
use std::ptr;

use storkv_ffi::{
    StorkvPoolHandle, StorkvPutCondition, StorkvPutConditionKind, StorkvPutIfResult,
    StorkvScopeHandle, StorkvSlice, StorkvStatus, storkv_pool_close, storkv_pool_open, storkv_put,
    storkv_put_if, storkv_scope_close, storkv_scope_open,
};

fn slice(bytes: &[u8]) -> StorkvSlice {
    StorkvSlice {
        data: bytes.as_ptr(),
        len: bytes.len(),
    }
}

fn condition(kind: StorkvPutConditionKind, expected_seq: u64) -> StorkvPutCondition {
    StorkvPutCondition {
        kind,
        expected_value: StorkvSlice {
            data: ptr::null_mut(),
            len: 0,
        },
        expected_seq,
    }
}

#[test]
fn put_if_applies_condition_through_c_abi() {
    let path = std::env::temp_dir().join(format!("storkv_ffi_put_if_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp dir");
    let uri = CString::new(format!("file://{}", path.display())).expect("uri");
    let mut pool: *mut StorkvPoolHandle = ptr::null_mut();
    assert_eq!(
        unsafe { storkv_pool_open(uri.as_ptr(), ptr::null(), &mut pool) },
        StorkvStatus::Ok
    );
    let name = CString::new("default").expect("scope name");
    let mut scope: *mut StorkvScopeHandle = ptr::null_mut();
    assert_eq!(
        unsafe { storkv_scope_open(pool, name.as_ptr(), &mut scope) },
        StorkvStatus::Ok
    );

    let mut result = StorkvPutIfResult::EMPTY;
    let absent = condition(StorkvPutConditionKind::Absent, 0);
    assert_eq!(
        unsafe {
            storkv_put_if(
                scope,
                slice(b"cas"),
                &absent,
                slice(b"first"),
                0,
                &mut result,
            )
        },
        StorkvStatus::Ok
    );
    assert_eq!(result.applied, 1);

    assert_eq!(
        unsafe {
            storkv_put_if(
                scope,
                slice(b"cas"),
                &absent,
                slice(b"second"),
                0,
                &mut result,
            )
        },
        StorkvStatus::Ok
    );
    assert_eq!(result.applied, 0);

    let expected_seq = condition(StorkvPutConditionKind::ExpectedSeq, result.seq);
    assert_eq!(
        unsafe {
            storkv_put_if(
                scope,
                slice(b"cas"),
                &expected_seq,
                slice(b"second"),
                0,
                &mut result,
            )
        },
        StorkvStatus::Ok
    );
    assert_eq!(result.applied, 1);

    assert_eq!(
        unsafe { storkv_put(scope, slice(b"zero"), slice(b"value")) },
        StorkvStatus::Ok
    );

    unsafe { storkv_scope_close(scope) };
    unsafe { storkv_pool_close(pool) };
    let _ = std::fs::remove_dir_all(PathBuf::from(&path));
}
