//! Terminal dashboard: polled telemetry/admin models, live polling, and
//! ratatui rendering.

mod model;
mod poller;
mod ui;

pub use model::{
    AdminPoll, DashboardSnapshot, LevelStats, ScopeStats, TelemetryPoll, write_amplification,
};
pub use poller::{fetch_admin_stats, fetch_dashboard, fetch_telemetry};
pub use ui::{render_dashboard, render_dashboard_with_stats};
