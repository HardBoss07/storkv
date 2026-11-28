//! storkv-top: live terminal diagnostics dashboard for StorKV.

use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::time::Duration;
use storkv_tui::{AdminPoll, BackupPoll, TelemetryPoll, fetch_dashboard, render_dashboard_full};

/// storkv-top: live terminal diagnostics dashboard for StorKV.
#[derive(Parser, Debug)]
#[command(
    name = "storkv-top",
    version,
    about = "Live StorKV diagnostics dashboard"
)]
struct Args {
    /// Polling interval in milliseconds
    #[arg(long, default_value_t = 1000)]
    interval_ms: u64,
    /// Address of the storkvd admin socket (TCP host:port)
    #[arg(long, default_value = "127.0.0.1:7878")]
    sock: String,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let args = Args::parse();

    // Set up crossterm raw mode and alternate screen
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut telemetry = TelemetryPoll::default();
    let mut admin = AdminPoll::default();
    let mut backup = BackupPoll::default();
    let poll_duration = Duration::from_millis(args.interval_ms);

    loop {
        // Attempt to fetch fresh telemetry; keep the last snapshot on failure
        if let Ok(snapshot) = fetch_dashboard(&args.sock).await {
            telemetry = snapshot.telemetry;
            admin = snapshot.admin;
            backup = snapshot.backup;
        }

        terminal.draw(|frame| render_dashboard_full(frame, &telemetry, &admin, &backup))?;

        // Poll for keyboard events with the configured interval
        if event::poll(poll_duration)?
            && let Event::Key(key_event) = event::read()?
        {
            match key_event.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => break,
                _ => {}
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
