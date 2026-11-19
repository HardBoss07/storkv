//! Single-scope operations for the C ABI.

mod read;
mod write;

pub use read::{storkv_exists, storkv_get, storkv_scan_prefix, storkv_scan_range};
pub use write::{storkv_delete, storkv_get_and_touch, storkv_put, storkv_put_with_ttl};
