use std::time::Duration;
use storkv_tui::fetch_dashboard;

use super::support::spawn_test_server;

#[tokio::test]
async fn fetch_telemetry_from_live_server() {
    let (_server, addr, _temp_db) = spawn_test_server().await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let snapshot = storkv_tui::fetch_telemetry(&addr).await.expect("fetch");
    assert_eq!(snapshot.writes_total, 0);
    assert_eq!(snapshot.reads_total, 0);

    let dashboard = fetch_dashboard(&addr).await.expect("dashboard");
    assert_eq!(dashboard.telemetry.writes_total, 0);
    assert_eq!(dashboard.admin.levels.len(), 7);
    assert!(
        dashboard
            .admin
            .scopes
            .iter()
            .any(|scope| scope.name == "default"),
        "default scope should appear in admin stats"
    );
}

#[tokio::test]
async fn fetch_dashboard_after_writes_reports_counts_and_levels() {
    let (_server, addr, _temp_db) = spawn_test_server().await;
    let client = storkv_client::AsyncClient::connect(&addr).await.unwrap();
    let scope = storkv_client::ScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };
    for index in 0..200 {
        client
            .put(&scope, format!("tui:{:03}", index).as_bytes(), b"v")
            .await
            .unwrap();
    }

    let dashboard = fetch_dashboard(&addr).await.expect("dashboard");
    assert!(dashboard.telemetry.writes_total >= 200);
    assert_eq!(dashboard.admin.levels.len(), 7);
    let default = dashboard
        .admin
        .scopes
        .iter()
        .find(|scope| scope.scope_id == 0)
        .expect("default scope");
    assert!(default.writes_total >= 200);
}
