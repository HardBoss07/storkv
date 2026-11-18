use std::sync::Arc;

use storkv_cli::CliRunner;
use storkv_server::{Server, TransportOptions};
use tokio::net::TcpListener;

use crate::common;

struct MultiTransport {
    server: Arc<Server>,
    tcp_uri: String,
    http_uri: String,
    ws_uri: String,
    _temp: common::TempDb,
}

async fn spawn_multi(prefix: &str) -> MultiTransport {
    let mut temp = common::TempDb::new(prefix);
    let server = Arc::new(Server::with_db(Arc::new(temp.db.take().unwrap())));

    let tcp_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tcp_addr = tcp_listener.local_addr().unwrap().to_string();
    let tcp_server = server.clone();
    tokio::spawn(async move {
        let _ = tcp_server.run_tcp_listener(tcp_listener, true).await;
    });

    let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap().to_string();
    let http_server = server.clone();
    tokio::spawn(async move {
        let _ = storkv_server::transports::serve_http_tcp(
            http_server,
            http_listener,
            TransportOptions::new(true, None),
        )
        .await;
    });

    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_addr = ws_listener.local_addr().unwrap().to_string();
    let ws_server = server.clone();
    tokio::spawn(async move {
        let _ = storkv_server::transports::serve_ws_tcp(
            ws_server,
            ws_listener,
            TransportOptions::new(true, None),
        )
        .await;
    });

    MultiTransport {
        server,
        tcp_uri: format!("storkv://{tcp_addr}"),
        http_uri: format!("http://{http_addr}"),
        ws_uri: format!("ws://{ws_addr}"),
        _temp: temp,
    }
}

#[tokio::test]
async fn cli_commands_accept_http_and_websocket_uris() {
    let transports = spawn_multi("cli_multi_uri").await;

    for uri in [
        &transports.tcp_uri,
        &transports.http_uri,
        &transports.ws_uri,
    ] {
        CliRunner::health(uri)
            .await
            .unwrap_or_else(|error| panic!("health over {uri} failed: {error}"));
    }

    let backup_dir = transports._temp.path.join("multi_uri_backup");
    CliRunner::backup(&transports.http_uri, backup_dir.to_str().unwrap())
        .await
        .expect("backup over HTTP URI");
    assert!(backup_dir.join("CURRENT").exists());

    let status = CliRunner::backup_status(&transports.ws_uri)
        .await
        .expect("backup status over WebSocket URI");
    assert!(!status.enabled);

    let _ = transports.server.shutdown_handle().send(());
}
