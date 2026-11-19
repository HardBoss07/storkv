//! REST data-plane handlers: keys, scans, merges, and transactions.

mod key;
mod scan;
mod support;
mod tx;

pub(super) use key::{delete_key, get_key, merge, put_key};
pub(super) use scan::{delete_range, scan};
pub(super) use tx::{tx_begin, tx_commit};
