//! REST gateway handlers, grouped by data, scope, and admin surfaces.

pub(super) use super::rest_admin::{
    admin_stats, checkpoint, compact, create_snapshot, health, list_backups, ping, telemetry,
    trigger_backup, unpin_snapshot, verify_backup,
};
pub(super) use super::rest_data::{
    delete_key, delete_range, get_key, merge, put_key, scan, tx_begin, tx_commit,
};
pub(super) use super::rest_scopes::{create_scope, drop_scope};
