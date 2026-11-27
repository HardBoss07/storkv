use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use storkv_proto::WirePutCondition;

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

async fn http_get(http_uri: &str, path: &str) -> (u16, String, String) {
    let address = http_uri.strip_prefix("http://").expect("http uri");
    let mut stream = TcpStream::connect(address).await.expect("connect http");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let response = String::from_utf8_lossy(&response).into_owned();
    let (head, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("status code");
    (status, head.to_owned(), body.to_owned())
}

fn metric_value(body: &str, metric: &str) -> Option<f64> {
    body.lines()
        .find_map(|line| line.strip_prefix(metric))
        .and_then(|rest| rest.trim().parse().ok())
}

#[tokio::test]
async fn http_metrics_reports_operations_and_engine_state() {
    let transports = support::spawn_all("transports_http_metrics").await;
    support::crud_roundtrip(&transports.http_uri).await;

    let client = support::connect(&transports.http_uri).await;
    let scope = support::make_scope(&client, "metrics_ops").await;
    client
        .put_if(&scope, b"cond", &WirePutCondition::Absent, b"value", None)
        .await
        .expect("put if over http");
    client
        .delete(&scope, b"cond")
        .await
        .expect("delete over http");

    let _watch = transports.server.db().watch().expect("watch subscription");
    let (status, head, body) = http_get(&transports.http_uri, "/metrics").await;
    assert_eq!(status, 200);
    assert!(head.contains("text/plain"));

    assert!(metric_value(&body, "storkv_requests_total{op=\"get\"} ").unwrap_or(0.0) > 0.0);
    assert!(metric_value(&body, "storkv_requests_total{op=\"put\"} ").unwrap_or(0.0) > 0.0);
    assert!(metric_value(&body, "storkv_requests_total{op=\"put_if\"} ").unwrap_or(0.0) > 0.0);
    assert!(metric_value(&body, "storkv_requests_total{op=\"delete\"} ").unwrap_or(0.0) > 0.0);
    assert!(metric_value(&body, "storkv_requests_total{op=\"scan\"} ").unwrap_or(0.0) > 0.0);
    assert!(metric_value(&body, "storkv_requests_total{op=\"txn\"} ").unwrap_or(0.0) > 0.0);
    assert!(body.contains("storkv_request_duration_seconds_bucket{op=\"get\",le=\"+Inf\"}"));
    assert!(
        metric_value(
            &body,
            "storkv_request_duration_seconds_bucket{op=\"put_if\",le=\"+Inf\"} "
        )
        .unwrap_or(0.0)
            > 0.0
    );
    assert!(
        metric_value(
            &body,
            "storkv_request_duration_seconds_bucket{op=\"delete\",le=\"+Inf\"} "
        )
        .unwrap_or(0.0)
            > 0.0
    );
    assert!(metric_value(&body, "storkv_request_duration_seconds_sum{op=\"put_if\"} ").is_some());
    assert!(
        metric_value(
            &body,
            "storkv_request_duration_seconds_count{op=\"put_if\"} "
        )
        .is_some()
    );
    assert!(metric_value(&body, "storkv_request_duration_seconds_sum{op=\"delete\"} ").is_some());
    assert!(
        metric_value(
            &body,
            "storkv_request_duration_seconds_count{op=\"delete\"} "
        )
        .is_some()
    );
    assert!(metric_value(&body, "storkv_watch_subscribers ").unwrap_or(0.0) >= 1.0);
    assert!(body.contains("storkv_wal_pinned_bytes "));
    assert!(metric_value(&body, "storkv_wal_total_bytes ").unwrap_or(0.0) > 0.0);
    assert!(body.contains("storkv_sst_files{level=\"L0\"} "));
    assert!(body.contains("storkv_sst_bytes{level=\"L6\"} "));
    assert!(body.contains("storkv_compaction_bytes_written_total "));
    assert!(body.contains("storkv_block_cache_hits_total "));
    assert!(body.contains("storkv_block_cache_misses_total "));
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
