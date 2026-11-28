//! Database handle and scope handle exports.
mod handle;
mod inner;
mod open;

pub use handle::{Db, ScopeHandle};
pub use inner::{DbInner, MAX_SCOPES};
