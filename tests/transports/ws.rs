use super::support;

#[tokio::test]
async fn websocket_binary_crud() {
    let transports = support::spawn_all("transports_ws").await;
    support::crud_roundtrip(&transports.ws_uri).await;
    support::assert_error_propagation(&transports.ws_uri).await;
}

#[tokio::test]
async fn websocket_http_upgrade_path_is_served() {
    let transports = support::spawn_all("transports_ws_http").await;
    let upgrade_uri = transports.http_uri.replacen("http://", "ws://", 1) + "/v1/ws";
    support::crud_roundtrip(&upgrade_uri).await;
}
