use std::process::Command;

use storkv_cli::CliRunner;

use super::spawn_test_server;

#[test]
fn compact_help_lists_range_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_storkvctl"))
        .args(["compact", "--help"])
        .output()
        .expect("run storkvctl compact --help");
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--scope"));
    assert!(help.contains("--start"));
    assert!(help.contains("--end"));
}

#[tokio::test]
async fn compact_range_executes_over_the_daemon() {
    let (server, addr, _temp_db) = spawn_test_server("cli_compact_range").await;

    CliRunner::compact(&addr, Some("default"), Some(b"a"), Some(b"z"))
        .await
        .expect("range compaction");
    let error = CliRunner::compact(&addr, Some("default"), Some(b"z"), Some(b"a"))
        .await
        .expect_err("inverted range must fail");
    assert!(error.to_string().contains("start must be less than end"));

    server.shutdown_handle().send(()).unwrap();
}
