//! Terminal dashboard rendering.

mod format;
mod panels;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
};

use crate::model::{AdminPoll, BackupPoll, TelemetryPoll};

use panels::{
    render_amplification_panel, render_backup_panel, render_levels_panel, render_scope_panel,
    render_wal_panel,
};

/// Render the full dashboard into a [`ratatui::Frame`].
///
/// Layout (top-to-bottom):
/// ```text
/// ┌─── Amplification ───┐
/// ├─── Level Stats ─────┤
/// ├─── WAL / Fsyncs ────┤
/// ├─── Backup ──────────┤
/// └─── Scope Metrics ───┘
/// ```
pub fn render_dashboard(frame: &mut Frame, telemetry: &TelemetryPoll) {
    render_dashboard_with_stats(frame, telemetry, &AdminPoll::default());
}

/// Render the dashboard including live admin statistics (L0..L6 distribution,
/// per-scope counters, and WAL fsync latency p99).
pub fn render_dashboard_with_stats(
    frame: &mut Frame,
    telemetry: &TelemetryPoll,
    admin: &AdminPoll,
) {
    render_dashboard_full(frame, telemetry, admin, &BackupPoll::default());
}

/// Render every panel, including autonomous backup status.
pub fn render_dashboard_full(
    frame: &mut Frame,
    telemetry: &TelemetryPoll,
    admin: &AdminPoll,
    backup: &BackupPoll,
) {
    let chunks = dashboard_chunks(frame.area());
    render_amplification_panel(frame, telemetry, chunks[0]);
    render_levels_panel(frame, admin, chunks[1]);
    render_wal_panel(frame, telemetry, admin, chunks[2]);
    render_backup_panel(frame, telemetry, backup, chunks[3]);
    render_scope_panel(frame, admin, chunks[4]);
}

/// Width of the amplification panel.
const AMPLIFICATION_PANEL_WIDTH: u16 = 5;
/// Width of the levels panel.
const LEVELS_PANEL_WIDTH: u16 = 11;
/// Width of the WAL panel.
const WAL_PANEL_WIDTH: u16 = 6;
/// Width of the backup panel.
const BACKUP_PANEL_WIDTH: u16 = 7;
/// Minimum width of the scope panel.
const SCOPE_PANEL_MIN_WIDTH: u16 = 4;

fn dashboard_chunks(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(AMPLIFICATION_PANEL_WIDTH),
            Constraint::Length(LEVELS_PANEL_WIDTH),
            Constraint::Length(WAL_PANEL_WIDTH),
            Constraint::Length(BACKUP_PANEL_WIDTH),
            Constraint::Min(SCOPE_PANEL_MIN_WIDTH),
        ])
        .split(area)
}
