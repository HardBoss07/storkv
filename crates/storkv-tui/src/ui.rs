use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::model::{AdminPoll, BackupPoll, LevelStats, TelemetryPoll, write_amplification};

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

fn render_amplification_panel(frame: &mut Frame, telemetry: &TelemetryPoll, area: Rect) {
    let amplification_text = vec![
        Line::from(vec![
            Span::styled("Writes : ", Style::default().fg(Color::Yellow)),
            Span::raw(telemetry.writes_total.to_string()),
        ]),
        Line::from(vec![
            Span::styled("Reads  : ", Style::default().fg(Color::Cyan)),
            Span::raw(telemetry.reads_total.to_string()),
        ]),
        Line::from(vec![
            Span::styled(
                "W/R Amp: ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("{:.2}", write_amplification(telemetry))),
        ]),
    ];
    let paragraph = Paragraph::new(amplification_text).block(
        Block::default()
            .title("Write/Read Amplification")
            .borders(Borders::ALL),
    );
    frame.render_widget(paragraph, area);
}

fn render_levels_panel(frame: &mut Frame, admin: &AdminPoll, area: Rect) {
    let level_stats = if admin.levels.is_empty() {
        crate::model::empty_level_stats()
    } else {
        admin.levels.clone()
    };

    let header_cells = ["Level", "Files", "Size"].iter().map(|header| {
        Cell::from(*header).style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    });
    let rows: Vec<Row> = level_stats.iter().map(level_row).collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(14),
        ],
    )
    .header(Row::new(header_cells).height(1))
    .block(
        Block::default()
            .title("L0..L6 SSTable Levels")
            .borders(Borders::ALL),
    );
    frame.render_widget(table, area);
}

fn level_row(stats: &LevelStats) -> Row<'static> {
    Row::new(vec![
        Cell::from(format!("L{}", stats.level)),
        Cell::from(stats.file_count.to_string()),
        Cell::from(format_bytes(stats.size_bytes)),
    ])
}

fn render_wal_panel(frame: &mut Frame, telemetry: &TelemetryPoll, admin: &AdminPoll, area: Rect) {
    let latency_text = if admin.fsync_latency_samples == 0 {
        "n/a".to_string()
    } else {
        format!("{} us", admin.fsync_latency_p99_us)
    };
    let wal_text = vec![
        Line::from(vec![
            Span::styled("Fsyncs total : ", Style::default().fg(Color::Yellow)),
            Span::raw(telemetry.fsyncs_total.to_string()),
        ]),
        Line::from(vec![
            Span::styled("Fsync p99    : ", Style::default().fg(Color::Magenta)),
            Span::raw(latency_text),
        ]),
        Line::from(vec![
            Span::styled("Compactions  : ", Style::default().fg(Color::Cyan)),
            Span::raw(telemetry.compactions_total.to_string()),
        ]),
        Line::from(vec![
            Span::styled("Conflicts    : ", Style::default().fg(Color::Red)),
            Span::raw(telemetry.write_conflicts_total.to_string()),
        ]),
    ];
    let paragraph = Paragraph::new(wal_text)
        .block(Block::default().title("WAL / Fsyncs").borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

fn render_backup_panel(
    frame: &mut Frame,
    telemetry: &TelemetryPoll,
    backup: &BackupPoll,
    area: Rect,
) {
    let state = if backup.enabled {
        "enabled"
    } else {
        "disabled"
    };
    let root = backup
        .backup_root
        .as_deref()
        .map(|root| truncate(root, 28))
        .unwrap_or_else(|| "<none>".to_string());
    let mut text = vec![
        Line::from(vec![
            Span::styled("State        : ", Style::default().fg(Color::Green)),
            Span::raw(state),
        ]),
        Line::from(vec![
            Span::styled("Last seq     : ", Style::default().fg(Color::Yellow)),
            Span::raw(telemetry.last_backup_seq.to_string()),
        ]),
        Line::from(vec![
            Span::styled("Completed    : ", Style::default().fg(Color::Cyan)),
            Span::raw(format!(
                "{} (failed {}, wals {})",
                telemetry.backups_completed_total,
                telemetry.backups_failed_total,
                telemetry.wal_segments_shipped_total
            )),
        ]),
        Line::from(vec![
            Span::styled("Root         : ", Style::default().fg(Color::Magenta)),
            Span::raw(format!("{root} [{}]", backup.sink)),
        ]),
    ];
    if let Some(error) = &backup.last_error {
        text.push(Line::from(vec![
            Span::styled("Last error   : ", Style::default().fg(Color::Red)),
            Span::raw(truncate(error, 40)),
        ]));
    }
    let paragraph =
        Paragraph::new(text).block(Block::default().title("Backup").borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

fn render_scope_panel(frame: &mut Frame, admin: &AdminPoll, area: Rect) {
    let scope_text: Vec<Line> = if admin.scopes.is_empty() {
        vec![Line::from(
            "Per-scope metrics: connect via --sock to populate live data.",
        )]
    } else {
        admin.scopes.iter().map(scope_line).collect()
    };
    let paragraph = Paragraph::new(scope_text).block(
        Block::default()
            .title("Per-Scope Metrics")
            .borders(Borders::ALL),
    );
    frame.render_widget(paragraph, area);
}

fn scope_line(scope: &crate::model::ScopeStats) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("scope {:<3} ", scope.scope_id),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            format!("{:<14}", truncate(&scope.name, 14)),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            " W:{} R:{} SSTs:{}",
            scope.writes_total, scope.reads_total, scope.sst_files
        )),
    ])
}

/// Number of bytes per unit in the human-readable format.
const BYTES_PER_UNIT: u64 = 1024;

/// Units for human-readable byte representation.
const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];

fn format_bytes(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= BYTES_PER_UNIT as f64 && unit < UNITS.len() - 1 {
        value /= BYTES_PER_UNIT as f64;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let limit = std::cmp::min(value.chars().count(), max_chars);
    if value.chars().count() <= limit {
        value.to_string()
    } else {
        value.chars().take(limit).collect()
    }
}
