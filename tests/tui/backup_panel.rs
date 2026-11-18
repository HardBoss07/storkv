use ratatui::{Terminal, backend::TestBackend};
use storkv_tui::{AdminPoll, BackupPoll, TelemetryPoll, render_dashboard_full};

#[test]
fn backup_panel_renders_counters() {
    let backend = TestBackend::new(100, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    let telemetry = TelemetryPoll {
        writes_total: 10,
        reads_total: 5,
        fsyncs_total: 3,
        backups_completed_total: 7,
        backups_failed_total: 2,
        last_backup_seq: 4242,
        wal_segments_shipped_total: 9,
        ..Default::default()
    };
    let backup = BackupPoll {
        enabled: true,
        backup_root: Some("/srv/backups".into()),
        last_backup_timestamp_ms: 1_760_000_000_000,
        last_backup_seq: 4242,
        backups_completed_total: 7,
        backups_failed_total: 2,
        wal_segments_shipped_total: 9,
        next_full_backup_in_ms: 3_600_000,
        last_error: None,
        sink: "local".into(),
    };

    terminal
        .draw(|frame| render_dashboard_full(frame, &telemetry, &AdminPoll::default(), &backup))
        .unwrap();

    let rendered = buffer_text(&terminal);
    assert!(rendered.contains("Backup"), "backup panel title missing");
    assert!(rendered.contains("4242"), "last backup sequence missing");
    assert!(
        rendered.contains("7 (failed 2, wals 9)"),
        "counters missing"
    );
    assert!(rendered.contains("/srv/backups"), "backup root missing");
    assert!(rendered.contains("enabled"), "enabled state missing");
}

#[test]
fn backup_panel_handles_zero_values_and_long_errors() {
    let backend = TestBackend::new(60, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    let backup = BackupPoll {
        enabled: false,
        backup_root: None,
        last_error: Some("x".repeat(500)),
        ..Default::default()
    };
    terminal
        .draw(|frame| {
            render_dashboard_full(
                frame,
                &TelemetryPoll::default(),
                &AdminPoll::default(),
                &backup,
            )
        })
        .unwrap();

    let rendered = buffer_text(&terminal);
    assert!(rendered.contains("disabled"));
    assert!(rendered.contains("<none>"));
    assert!(rendered.contains("Last error"));
}

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol().to_string())
        .collect()
}
