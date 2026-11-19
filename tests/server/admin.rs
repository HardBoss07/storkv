use crate::common;
use bytes::Bytes;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use storkv_proto::{
    Request, Response, WireAdminStats, WireDurability, WireRetention, WireScopeOptions,
};
use storkv_server::Server;
use tokio::net::{TcpListener, TcpStream};

use super::support::send_request;

const ADMIN_TOKEN: &str = "correct-horse-battery-staple";

#[tokio::test]
async fn test_admin_stats_reports_levels_scopes_and_latency() {
    let mut temp_db = common::TempDb::new("server_admin_stats");
    let server = Arc::new(Server::with_db(Arc::new(temp_db.db.take().unwrap())));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_clone = server.clone();
    let handle = tokio::spawn(async move {
        server_clone
            .run_tcp_listener(listener, false)
            .await
            .unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.unwrap();

    let create_response = send_request(
        &mut stream,
        1,
        Request::CreateScope {
            name: "metrics".into(),
            options: WireScopeOptions {
                durability: WireDurability::StrictFsync,
                retention: WireRetention::Permanent,
                prefix_bloom_len: None,
            },
        },
    )
    .await;
    let scope_id = match create_response {
        Response::Scope(scope) => scope.scope_id,
        other => panic!("Expected scope handle, got {:?}", other),
    };

    assert_eq!(
        send_request(
            &mut stream,
            2,
            Request::Put {
                scope_id,
                key: Bytes::from_static(b"k"),
                val: Bytes::from_static(b"v"),
            },
        )
        .await,
        Response::Empty
    );
    assert_eq!(
        send_request(
            &mut stream,
            3,
            Request::Get {
                scope_id,
                key: Bytes::from_static(b"k"),
            },
        )
        .await,
        Response::Value(Some(Bytes::from_static(b"v")))
    );

    match send_request(&mut stream, 4, Request::AdminStats).await {
        Response::AdminStats(WireAdminStats {
            levels,
            scopes,
            fsync_latency_p99_us,
            fsync_latency_samples,
            ..
        }) => {
            assert_eq!(levels.len(), 7);
            assert!(levels.iter().any(|level| level.level == 0));
            let metrics = scopes
                .iter()
                .find(|scope| scope.scope_id == scope_id)
                .expect("new scope present in admin stats");
            assert_eq!(metrics.name, "metrics");
            assert!(metrics.writes_total >= 1);
            assert!(metrics.reads_total >= 1);
            assert!(fsync_latency_samples >= 1);
            let _ = fsync_latency_p99_us;
        }
        other => panic!("Expected AdminStats, got {:?}", other),
    }

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
}

fn temporary_root(prefix: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "storkv_{prefix}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create temporary root");
    root
}

fn free_local_address() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind free port");
    listener.local_addr().expect("free address").to_string()
}

fn wait_for_listener(address: &str) {
    for _ in 0..200 {
        if std::net::TcpStream::connect(address).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("listener {address} did not start");
}

fn admin_compact_status(address: &str, token: Option<&str>) -> u16 {
    let mut stream = std::net::TcpStream::connect(address).expect("connect http listener");
    let authorization = token
        .map(|token| format!("X-StorkV-Admin-Token: {token}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST /v1/admin/compact HTTP/1.1\r\nHost: {address}\r\n\
         Content-Length: 0\r\n{authorization}Connection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .expect("write admin request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read admin response");
    response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("unexpected HTTP response: {response}"))
}

fn spawn_daemon(
    data_dir: &Path,
    http_address: &str,
    configure: impl FnOnce(&mut Command),
) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_storkvd"));
    command
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--listen")
        .arg("127.0.0.1:0")
        .arg("--http-listen")
        .arg(http_address)
        .arg("--strict-admin")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure(&mut command);
    command.spawn().expect("spawn storkvd")
}

fn stop_daemon(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn admin_token_file_authenticates_admin_requests() {
    let root = temporary_root("admin_token_file");
    let token_file = root.join("admin.token");
    std::fs::write(&token_file, format!("{ADMIN_TOKEN}\n")).expect("write token file");
    let http_address = free_local_address();
    let child = spawn_daemon(&root.join("data"), &http_address, |command| {
        command
            .env_remove("STORKV_ADMIN_TOKEN")
            .arg("--admin-token-file")
            .arg(&token_file);
    });
    wait_for_listener(&http_address);

    assert_eq!(admin_compact_status(&http_address, None), 403);
    assert_eq!(admin_compact_status(&http_address, Some(ADMIN_TOKEN)), 200);

    stop_daemon(child);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn admin_token_environment_authenticates_admin_requests() {
    let root = temporary_root("admin_token_env");
    let http_address = free_local_address();
    let child = spawn_daemon(&root.join("data"), &http_address, |command| {
        command.env("STORKV_ADMIN_TOKEN", ADMIN_TOKEN);
    });
    wait_for_listener(&http_address);

    assert_eq!(admin_compact_status(&http_address, None), 403);
    assert_eq!(admin_compact_status(&http_address, Some(ADMIN_TOKEN)), 200);

    stop_daemon(child);
    let _ = std::fs::remove_dir_all(&root);
}
