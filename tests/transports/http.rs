use super::support;

#[tokio::test]
async fn http_crud_and_telemetry() {
    let transports = support::spawn_all("transports_http").await;
    support::crud_roundtrip(&transports.http_uri).await;
    support::assert_error_propagation(&transports.http_uri).await;

    let client = support::connect(&transports.http_uri).await;
    let stats = client.admin_stats().await.expect("admin stats over http");
    assert!(stats.transports.iter().any(|entry| entry.kind == 1));
}
