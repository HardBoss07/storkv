use ratatui::{Terminal, backend::TestBackend};
use storkv_tui::{
    AdminPoll, LevelStats, ScopeStats, TelemetryPoll, render_dashboard, render_dashboard_with_stats,
};

#[test]
fn render_dashboard_zero_telemetry_no_panic() {
    let backend = TestBackend::new(80, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let telemetry = TelemetryPoll::default();
    terminal
        .draw(|frame| render_dashboard(frame, &telemetry))
        .expect("draw");
}

#[test]
fn render_dashboard_panel_titles_present() {
    let backend = TestBackend::new(80, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let telemetry = TelemetryPoll::default();
    terminal
        .draw(|frame| render_dashboard(frame, &telemetry))
        .expect("draw");

    let buffer = terminal.backend().buffer().clone();
    let rendered: String = buffer
        .content
        .iter()
        .map(|cell| cell.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("Amplification") || rendered.contains("Write"),
        "amplification panel missing from rendered output"
    );
    assert!(
        rendered.contains("SSTable") || rendered.contains("Level"),
        "level stats panel missing from rendered output"
    );
    assert!(
        rendered.contains("WAL") || rendered.contains("Fsync"),
        "WAL panel missing from rendered output"
    );
}

#[test]
fn render_dashboard_nonzero_values_visible() {
    let backend = TestBackend::new(80, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let telemetry = TelemetryPoll {
        writes_total: 1234,
        reads_total: 567,
        write_conflicts_total: 3,
        compactions_total: 2,
        fsyncs_total: 42,
        ..Default::default()
    };
    terminal
        .draw(|frame| render_dashboard(frame, &telemetry))
        .expect("draw");

    let buffer = terminal.backend().buffer().clone();
    let rendered: String = buffer
        .content
        .iter()
        .map(|cell| cell.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("1234"),
        "writes_total 1234 not visible in rendered buffer"
    );
    assert!(
        rendered.contains("567"),
        "reads_total 567 not visible in rendered buffer"
    );
    assert!(
        rendered.contains("42"),
        "fsyncs_total 42 not visible in rendered buffer"
    );
}

#[test]
fn render_dashboard_with_admin_stats() {
    let backend = TestBackend::new(100, 34);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let telemetry = TelemetryPoll {
        writes_total: 10,
        reads_total: 5,
        fsyncs_total: 3,
        ..Default::default()
    };
    let admin = AdminPoll {
        levels: (0u8..=6)
            .map(|level| LevelStats {
                level,
                file_count: if level == 0 { 2 } else { 0 },
                size_bytes: if level == 0 { 2048 } else { 0 },
            })
            .collect(),
        scopes: vec![ScopeStats {
            scope_id: 3,
            name: "sessions".into(),
            writes_total: 7,
            reads_total: 4,
            sst_files: 2,
        }],
        fsync_latency_p99_us: 250,
        fsync_latency_samples: 12,
    };

    terminal
        .draw(|frame| render_dashboard_with_stats(frame, &telemetry, &admin))
        .expect("draw");

    let buffer = terminal.backend().buffer().clone();
    let rendered: String = buffer
        .content
        .iter()
        .map(|cell| cell.symbol().to_string())
        .collect();

    assert!(rendered.contains("L0"), "level 0 row missing");
    assert!(rendered.contains("L6"), "level 6 row missing");
    assert!(rendered.contains("2.0 KiB"), "level size missing");
    assert!(rendered.contains("sessions"), "per-scope name missing");
    assert!(rendered.contains("250 us"), "fsync p99 missing");
}

#[test]
fn render_dashboard_in_tiny_terminals_never_panics() {
    for (width, height) in [(1, 1), (4, 3), (12, 6), (20, 8)] {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let telemetry = TelemetryPoll {
            writes_total: u64::MAX,
            reads_total: 1,
            fsyncs_total: u64::MAX,
            ..Default::default()
        };
        let admin = AdminPoll {
            levels: (0u8..=6)
                .map(|level| LevelStats {
                    level,
                    file_count: usize::MAX,
                    size_bytes: u64::MAX,
                })
                .collect(),
            scopes: vec![ScopeStats {
                scope_id: u32::MAX,
                name: "x".repeat(64),
                writes_total: u64::MAX,
                reads_total: u64::MAX,
                sst_files: u32::MAX,
            }],
            fsync_latency_p99_us: u64::MAX,
            fsync_latency_samples: 1,
        };
        terminal
            .draw(|frame| render_dashboard_with_stats(frame, &telemetry, &admin))
            .unwrap_or_else(|error| panic!("render failed at {width}x{height}: {error}"));
    }
}

#[test]
fn render_dashboard_heavy_levels_and_long_scope_names() {
    let backend = TestBackend::new(140, 46);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let telemetry = TelemetryPoll {
        writes_total: 9_000_000,
        reads_total: 3_000_000,
        write_conflicts_total: 17,
        compactions_total: 42,
        fsyncs_total: 8_000_000,
        ..Default::default()
    };
    let admin = AdminPoll {
        levels: (0u8..=6)
            .map(|level| LevelStats {
                level,
                file_count: (level as usize + 1) * 10,
                size_bytes: (level as u64 + 1) * 1024 * 1024 * 1024,
            })
            .collect(),
        scopes: vec![
            ScopeStats {
                scope_id: 0,
                name: "default".into(),
                writes_total: 100,
                reads_total: 200,
                sst_files: 5,
            },
            ScopeStats {
                scope_id: 7,
                name: "an-extremely-long-scope-name-that-must-be-truncated".into(),
                writes_total: 1,
                reads_total: 2,
                sst_files: 3,
            },
        ],
        fsync_latency_p99_us: 123_456,
        fsync_latency_samples: 500,
    };

    terminal
        .draw(|frame| render_dashboard_with_stats(frame, &telemetry, &admin))
        .expect("draw");

    let buffer = terminal.backend().buffer().clone();
    let rendered: String = buffer
        .content
        .iter()
        .map(|cell| cell.symbol().to_string())
        .collect();

    for level in 0..=6 {
        assert!(rendered.contains(&format!("L{level}")), "missing L{level}");
    }
    assert!(rendered.contains("GiB"), "gigabyte sizes missing");
    assert!(rendered.contains("an-extremely-l"), "scope name missing");
    assert!(
        !rendered.contains("an-extremely-long-scope-name-that-must-be-truncated"),
        "long scope name must be truncated"
    );
    assert!(rendered.contains("123456 us"), "p99 latency missing");
    assert!(rendered.contains("9000000"), "write count missing");
}
