use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::model::{AdminPoll, LevelStats, TelemetryPoll, write_amplification};

/// Render the full dashboard into a [`ratatui::Frame`].
///
/// Layout (top-to-bottom):
/// ```text
/// ┌─── Amplification ───┐
/// ├─── Level Stats ─────┤
/// ├─── WAL / Fsyncs ────┤
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
    let chunks = dashboard_chunks(frame.area());
    render_amplification_panel(frame, telemetry, chunks[0]);
    render_levels_panel(frame, admin, chunks[1]);
    render_wal_panel(frame, telemetry, admin, chunks[2]);
    render_scope_panel(frame, admin, chunks[3]);
}

fn dashboard_chunks(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(11),
            Constraint::Length(6),
            Constraint::Min(4),
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

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        value.chars().take(max_chars).collect()
    }
}
