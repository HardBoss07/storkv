use crate::common;
use std::sync::Arc;
use storkv_proto::{Request, Response};
use storkv_server::Server;
use tokio::net::{TcpListener, TcpStream};

use super::support::send_request;

#[cfg(unix)]
#[test]
fn admin_socket_uses_private_permissions() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};

    let root = std::env::temp_dir().join(format!(
        "storkv_admin_sock_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    ));
    let data_dir = root.join("data");
    let socket_dir = root.join("admin");
    let socket = socket_dir.join("storkv.sock");
    let admin_arg = format!("unix:{}", socket.display());

    let mut child = Command::new(env!("CARGO_BIN_EXE_storkvd"))
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--listen")
        .arg("127.0.0.1:0")
        .arg("--admin-sock")
        .arg(&admin_arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn storkvd");

    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(socket.exists(), "the admin socket must be created");

    let socket_mode = std::fs::metadata(&socket)
        .expect("socket metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(socket_mode, 0o600, "the admin socket must be private");

    let dir_mode = std::fs::metadata(&socket_dir)
        .expect("directory metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700, "the socket directory must be private");

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn test_server_startup_shutdown_and_ping() {
    let mut temp_db = common::TempDb::new("server_ping");
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
    let response = send_request(&mut stream, 1, Request::Ping).await;
    assert_eq!(response, Response::Pong);

    let health = send_request(&mut stream, 2, Request::HealthCheck).await;
    assert_eq!(health, Response::HealthOk);

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
}
