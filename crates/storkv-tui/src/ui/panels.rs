//! Dashboard panels for amplification, levels, WAL, backup, and scopes.

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use super::format::{format_bytes, truncate};
use crate::model::{AdminPoll, BackupPoll, LevelStats, TelemetryPoll, write_amplification};

pub(super) fn render_amplification_panel(frame: &mut Frame, telemetry: &TelemetryPoll, area: Rect) {
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

pub(super) fn render_levels_panel(frame: &mut Frame, admin: &AdminPoll, area: Rect) {
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

pub(super) fn render_wal_panel(
    frame: &mut Frame,
    telemetry: &TelemetryPoll,
    admin: &AdminPoll,
    area: Rect,
) {
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

pub(super) fn render_backup_panel(
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

pub(super) fn render_scope_panel(frame: &mut Frame, admin: &AdminPoll, area: Rect) {
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
