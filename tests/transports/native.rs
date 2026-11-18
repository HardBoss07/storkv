use super::support;

#[tokio::test]
async fn native_tcp_crud_and_errors() {
    let transports = support::spawn_all("transports_native").await;
    support::crud_roundtrip(&transports.tcp_uri).await;
    support::assert_error_propagation(&transports.tcp_uri).await;
}

#[tokio::test]
async fn native_bare_address_still_connects() {
    let transports = support::spawn_all("transports_native_bare").await;
    let address = transports
        .tcp_uri
        .strip_prefix("storkv://")
        .expect("tcp uri");
    let client = support::connect(address).await;
    client.ping().await.expect("ping over bare address");
}
