//! ABI layout checks for the public C structs.

use std::mem::{align_of, size_of};

use storkv_ffi::{
    StorkvBytes, StorkvEntries, StorkvEntry, StorkvEventKind, StorkvPoolConfig, StorkvRetryConfig,
    StorkvSlice, StorkvStatus, StorkvString, StorkvWatchEvent,
};

#[test]
fn abi_struct_sizes_are_stable() {
    assert_eq!(size_of::<StorkvStatus>(), 4);
    assert_eq!(size_of::<StorkvEventKind>(), 4);
    assert_eq!(size_of::<StorkvBytes>(), 16);
    assert_eq!(align_of::<StorkvBytes>(), 8);
    assert_eq!(size_of::<StorkvSlice>(), 16);
    assert_eq!(size_of::<StorkvString>(), 16);
    assert_eq!(size_of::<StorkvPoolConfig>(), 32);
    assert_eq!(align_of::<StorkvPoolConfig>(), 8);
    assert_eq!(size_of::<StorkvRetryConfig>(), 24);
    assert_eq!(size_of::<StorkvEntry>(), 32);
    assert_eq!(size_of::<StorkvEntries>(), 16);
    assert_eq!(size_of::<StorkvWatchEvent>(), 48);
    assert_eq!(align_of::<StorkvWatchEvent>(), 8);
}
