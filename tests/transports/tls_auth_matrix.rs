//! Authentication matrix for every access-controlled transport.
//!
//! The native listener is exercised over both plain TCP and real Rustls TLS.
//! HTTP and WebSocket share the same admin-token policy; gRPC uses the same
//! `TransportOptions` policy.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use storkv_proto::{Message, MessagePayload, Request, Response, StorkvWireError};
use storkv_server::tls::{rustls, tokio_rustls};
use storkv_server::transports::TransportOptions;
use storkv_server::{Server, transports};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const TOKEN: &str = "matrix-admin-token";

struct Running {
    server: Arc<Server>,
    _temp: crate::common::TempDb,
    handle: JoinHandle<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.server.shutdown_handle().send(());
        self.handle.abort();
    }
}

async fn spawn_http(prefix: &str, options: TransportOptions) -> (Running, String) {
    let mut temp = crate::common::TempDb::new(prefix);
    let server = Arc::new(
        Server::with_db(Arc::new(temp.db.take().expect("test db"))).with_strict_admin(true),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind http");
    let address = listener.local_addr().expect("http addr").to_string();
    let task_server = server.clone();
    let handle = tokio::spawn(async move {
        let _ = transports::serve_http_tcp(task_server, listener, options).await;
    });
    (
        Running {
            server,
            _temp: temp,
            handle,
        },
        address,
    )
}

async fn spawn_ws(prefix: &str, options: TransportOptions) -> (Running, String) {
    let mut temp = crate::common::TempDb::new(prefix);
    let server = Arc::new(
        Server::with_db(Arc::new(temp.db.take().expect("test db"))).with_strict_admin(true),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ws");
    let address = listener.local_addr().expect("ws addr").to_string();
    let task_server = server.clone();
    let handle = tokio::spawn(async move {
        let _ = transports::serve_ws_tcp(task_server, listener, options).await;
    });
    (
        Running {
            server,
            _temp: temp,
            handle,
        },
        address,
    )
}

async fn spawn_native(prefix: &str, strict_admin: bool) -> (Running, String, String) {
    let mut temp = crate::common::TempDb::new(prefix);
    let server = Arc::new(
        Server::with_db(Arc::new(temp.db.take().expect("test db"))).with_strict_admin(strict_admin),
    );
    let client_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind client");
    let client_addr = client_listener
        .local_addr()
        .expect("client addr")
        .to_string();
    let admin_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind admin");
    let admin_addr = admin_listener.local_addr().expect("admin addr").to_string();
    let client_server = server.clone();
    let admin_server = server.clone();
    let handle = tokio::spawn(async move {
        let client = tokio::spawn(async move {
            let _ = client_server.run_tcp_listener(client_listener, false).await;
        });
        let admin = tokio::spawn(async move {
            let _ = admin_server.run_tcp_listener(admin_listener, true).await;
        });
        let _ = tokio::join!(client, admin);
    });
    (
        Running {
            server,
            _temp: temp,
            handle,
        },
        client_addr,
        admin_addr,
    )
}

struct TestPki {
    directory: PathBuf,
    certificate_path: PathBuf,
    key_path: PathBuf,
    certificate: rustls::pki_types::CertificateDer<'static>,
}

impl TestPki {
    fn generate(prefix: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "storkv_{prefix}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&directory).expect("create tls directory");
        let generated = rcgen::generate_simple_self_signed(vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
        ])
        .expect("generate certificate");
        let certificate_path = directory.join("server.pem");
        let key_path = directory.join("server.key");
        std::fs::write(&certificate_path, generated.cert.pem()).expect("write certificate");
        std::fs::write(&key_path, generated.key_pair.serialize_pem()).expect("write key");
        Self {
            directory,
            certificate_path,
            key_path,
            certificate: generated.cert.der().clone(),
        }
    }
}

impl Drop for TestPki {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

async fn spawn_native_tls(prefix: &str, strict_admin: bool) -> (Running, String, String, TestPki) {
    let pki = TestPki::generate(prefix);
    let acceptor = storkv_server::tls::load_acceptor(&pki.certificate_path, &pki.key_path)
        .expect("tls accept");
    let mut temp = crate::common::TempDb::new(prefix);
    let server = Arc::new(
        Server::with_db(Arc::new(temp.db.take().expect("test db"))).with_strict_admin(strict_admin),
    );
    let client_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind client");
    let client_addr = client_listener
        .local_addr()
        .expect("client addr")
        .to_string();
    let admin_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind admin");
    let admin_addr = admin_listener.local_addr().expect("admin addr").to_string();
    let client_server = server.clone();
    let admin_server = server.clone();
    let handle = tokio::spawn(async move {
        let client = tokio::spawn(async move {
            let _ = client_server
                .run_tls_listener(client_listener, acceptor, false)
                .await;
        });
        let admin = tokio::spawn(async move {
            let _ = admin_server.run_tcp_listener(admin_listener, true).await;
        });
        let _ = tokio::join!(client, admin);
    });
    (
        Running {
            server,
            _temp: temp,
            handle,
        },
        client_addr,
        admin_addr,
        pki,
    )
}

async fn tls_connect(
    address: &str,
    certificate: rustls::pki_types::CertificateDer<'static>,
) -> tokio_rustls::client::TlsStream<TcpStream> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate).expect("trust test certificate");
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let server_name = rustls::pki_types::ServerName::try_from("localhost").expect("server name");
    let stream = TcpStream::connect(address).await.expect("connect tls");
    connector
        .connect(server_name, stream)
        .await
        .expect("tls handshake")
}

async fn http_status(address: &str, authorization: Option<&str>) -> u16 {
    let mut stream = TcpStream::connect(address).await.expect("connect http");
    let header = authorization
        .map(|value| format!("X-StorkV-Admin-Token: {value}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST /v1/admin/compact HTTP/1.1\r\nHost: {address}\r\n\
         Content-Length: 0\r\n{header}Connection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write admin request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .expect("read admin response");
    response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("HTTP status code")
}

async fn native_admin_compact(address: &str) -> Response {
    let stream = TcpStream::connect(address).await.expect("connect native");
    native_admin_compact_stream(stream).await
}

async fn native_admin_compact_stream<Stream>(mut stream: Stream) -> Response
where
    Stream: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let request = Message::new_request(1, Request::Compact).encode();
    stream
        .write_all(&request)
        .await
        .expect("write native request");
    let mut buffer = BytesMut::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = stream.read(&mut chunk).await.expect("read native");
        assert!(read > 0, "server closed the connection");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(message) = Message::decode(&mut buffer).expect("decode native") {
            let MessagePayload::Response(response) = message.payload else {
                panic!("expected a response payload");
            };
            return response;
        }
    }
}

async fn ws_admin_compact(address: &str, token: Option<&str>) -> Response {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let url = format!("ws://{address}/v1/ws");
    let mut request = url.into_client_request().expect("ws request");
    if let Some(token) = token {
        request
            .headers_mut()
            .insert("x-storkv-admin-token", token.parse().expect("header value"));
    }
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("ws connect");
    let frame = Message::new_request(1, Request::Compact).encode();
    socket
        .send(WsMessage::Binary(frame.to_vec().into()))
        .await
        .expect("ws send");
    let reply = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("ws timeout")
        .expect("ws reply")
        .expect("ws frame");
    let WsMessage::Binary(bytes) = reply else {
        panic!("expected a binary ws frame");
    };
    let mut buffer = BytesMut::from(bytes.as_ref());
    let message = Message::decode(&mut buffer)
        .expect("decode ws")
        .expect("frame");
    let MessagePayload::Response(response) = message.payload else {
        panic!("expected a response payload");
    };
    response
}

fn assert_unauthorized(response: Response) {
    assert_eq!(
        response,
        Response::Error(StorkvWireError::Unauthorized),
        "admin request without a token must fail"
    );
}

#[test]
fn tls_certificate_generation_probe() {
    let trusted = TestPki::generate("cert_probe");
    let other = TestPki::generate("cert_probe_other");

    let cert_pem = std::fs::read_to_string(&trusted.certificate_path).expect("read certificate");
    let key_pem = std::fs::read_to_string(&trusted.key_path).expect("read key");
    assert!(cert_pem.starts_with("-----BEGIN CERTIFICATE-----"));
    assert!(key_pem.starts_with("-----BEGIN PRIVATE KEY-----"));
    assert_ne!(
        trusted.certificate.as_ref(),
        other.certificate.as_ref(),
        "distinct identities"
    );
}

#[tokio::test]
async fn native_tls_listener_completes_a_rustls_handshake() {
    let (_running, client, admin, pki) = spawn_native_tls("matrix_native_tls", true).await;

    // The encrypted client listener is not an admin listener, so a TLS request
    // must still be rejected by the shared policy.
    let tls_stream = tls_connect(&client, pki.certificate.clone()).await;
    assert_unauthorized(native_admin_compact_stream(tls_stream).await);

    // The plain admin listener keeps working alongside the TLS listener.
    assert_eq!(native_admin_compact(&admin).await, Response::Empty);
}

#[test]
fn transport_options_grant_admin_for_exact_tokens_only() {
    let options = TransportOptions::new(false, Some(TOKEN.to_string()));
    assert!(options.grants_admin(Some(TOKEN)));
    assert!(!options.grants_admin(Some("matrix-admin-tokeN")));
    assert!(!options.grants_admin(Some("")));
    assert!(!options.grants_admin(None));
    assert!(TransportOptions::new(true, None).grants_admin(None));
    assert!(!TransportOptions::new(false, None).grants_admin(Some(TOKEN)));
}

#[tokio::test]
async fn native_strict_admin_rejects_client_listener_and_allows_admin_listener() {
    let (_running, client, admin) = spawn_native("matrix_native", true).await;
    assert_unauthorized(native_admin_compact(&client).await);
    assert_eq!(native_admin_compact(&admin).await, Response::Empty);
}

#[tokio::test]
async fn http_admin_token_matrix() {
    let options = TransportOptions::new(false, Some(TOKEN.to_string()));
    let (_running, address) = spawn_http("matrix_http", options).await;
    assert_eq!(http_status(&address, None).await, 403);
    assert_eq!(http_status(&address, Some("wrong-token")).await, 403);
    assert_eq!(http_status(&address, Some(TOKEN)).await, 200);
}

#[tokio::test]
async fn websocket_admin_token_matrix() {
    let options = TransportOptions::new(false, Some(TOKEN.to_string()));
    let (_running, address) = spawn_ws("matrix_ws", options).await;
    assert_unauthorized(ws_admin_compact(&address, None).await);
    assert_unauthorized(ws_admin_compact(&address, Some("wrong-token")).await);
    assert_eq!(
        ws_admin_compact(&address, Some(TOKEN)).await,
        Response::Empty
    );
}

#[tokio::test]
async fn native_crud_stays_available_when_strict_admin_is_on() {
    let (_running, client, _admin) = spawn_native("matrix_native_crud", true).await;
    let uri = format!("storkv://{client}");
    let scope = super::support::make_scope(&super::support::connect(&uri).await, "matrix").await;
    let handle = super::support::connect(&uri).await;
    handle
        .put(&scope, b"k", b"v")
        .await
        .expect("put without admin");
    assert_eq!(
        handle.get(&scope, b"k").await.expect("get without admin"),
        Some(Bytes::from_static(b"v"))
    );
}
