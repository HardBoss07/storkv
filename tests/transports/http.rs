use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::support;

async fn admin_compact_status(http_uri: &str, query: &str) -> u16 {
    let address = http_uri.strip_prefix("http://").expect("http uri");
    let mut stream = TcpStream::connect(address).await.expect("connect http");
    let request = format!(
        "POST /v1/admin/compact?{query} HTTP/1.1\r\nHost: {address}\r\n\
         Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    String::from_utf8_lossy(&response)
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("status code")
}

#[tokio::test]
async fn http_admin_compact_forwards_range_bounds() {
    let transports = support::spawn_all("transports_http_compact_range").await;
    assert_eq!(
        admin_compact_status(&transports.http_uri, "scope=default&start=a&end=z").await,
        200
    );
    // An inverted range must reach the engine and fail validation there.
    assert_eq!(
        admin_compact_status(&transports.http_uri, "scope=default&start=z&end=a").await,
        400
    );
}

#[tokio::test]
async fn http_crud_and_telemetry() {
    let transports = support::spawn_all("transports_http").await;
    support::crud_roundtrip(&transports.http_uri).await;
    support::assert_error_propagation(&transports.http_uri).await;

    let client = support::connect(&transports.http_uri).await;
    let stats = client.admin_stats().await.expect("admin stats over http");
    assert!(stats.transports.iter().any(|entry| entry.kind == 1));
}
