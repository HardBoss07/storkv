//! Terminal dashboard: polled telemetry/admin models, live polling, and
//! ratatui rendering.
//!
//! The `storkv-top` binary polls a live server through [`storkv_client`] and
//! renders the result with ratatui. This library separates the polled model
//! ([`DashboardSnapshot`]) from the network polling ([`fetch_dashboard`]) and the
//! drawing code so each part is testable without a terminal.

#![warn(missing_docs)]

mod model;
mod poller;
mod ui;

pub use model::{
    AdminPoll, BackupPoll, DashboardSnapshot, LevelStats, ScopeStats, TelemetryPoll,
    write_amplification,
};
pub use poller::{fetch_admin_stats, fetch_backup_status, fetch_dashboard, fetch_telemetry};
pub use ui::{render_dashboard, render_dashboard_full, render_dashboard_with_stats};
