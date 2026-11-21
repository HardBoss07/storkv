use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use storkv_server::{ReplicationRole, Server, TransportOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub struct CompatServer {
    pub server: Arc<Server>,
    pub _temp: crate::common::TempDb,
    pub tcp_uri: String,
    pub http_addr: String,
    pub redis_addr: String,
    pub memcached_addr: String,
    handles: Vec<JoinHandle<()>>,
}

impl Drop for CompatServer {
    fn drop(&mut self) {
        let _ = self.server.shutdown_handle().send(());
        for handle in &self.handles {
            handle.abort();
        }
    }
}

pub async fn spawn(prefix: &str) -> CompatServer {
    spawn_with_role(prefix, ReplicationRole::Standalone).await
}

pub async fn spawn_with_role(prefix: &str, role: ReplicationRole) -> CompatServer {
    let mut temp = crate::common::TempDb::new(prefix);
    let server =
        Arc::new(Server::with_db(Arc::new(temp.db.take().expect("test db"))).with_role(role));
    let mut handles = Vec::new();

    let tcp_listener = TcpListener::bind("127.0.0.1:0").await.expect("tcp bind");
    let tcp_addr = tcp_listener.local_addr().expect("tcp addr").to_string();
    let tcp_server = server.clone();
    handles.push(tokio::spawn(async move {
        let _ = tcp_server.run_tcp_listener(tcp_listener, true).await;
    }));

    let http_listener = TcpListener::bind("127.0.0.1:0").await.expect("http bind");
    let http_addr = http_listener.local_addr().expect("http addr").to_string();
    let http_server = server.clone();
    handles.push(tokio::spawn(async move {
        let _ = storkv_server::transports::serve_http_tcp(
            http_server,
            http_listener,
            TransportOptions::new(true, None),
        )
        .await;
    }));

    let redis_listener = TcpListener::bind("127.0.0.1:0").await.expect("redis bind");
    let redis_addr = redis_listener.local_addr().expect("redis addr").to_string();
    let redis_server = server.clone();
    handles.push(tokio::spawn(async move {
        let _ = storkv_server::compat::serve_redis_tcp(redis_server, redis_listener).await;
    }));

    let memcached_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("memcached bind");
    let memcached_addr = memcached_listener
        .local_addr()
        .expect("memcached addr")
        .to_string();
    let memcached_server = server.clone();
    handles.push(tokio::spawn(async move {
        let _ =
            storkv_server::compat::serve_memcached_tcp(memcached_server, memcached_listener).await;
    }));

    CompatServer {
        server,
        _temp: temp,
        tcp_uri: format!("storkv://{tcp_addr}"),
        http_addr,
        redis_addr,
        memcached_addr,
        handles,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resp {
    Status(String),
    Error(String),
    Integer(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Resp>>),
    Null,
    Boolean(bool),
    Map(Vec<(Resp, Resp)>),
    Set(Vec<Resp>),
}

pub struct RedisClient {
    stream: TcpStream,
}

impl RedisClient {
    pub async fn connect(address: &str) -> Self {
        let stream = TcpStream::connect(address).await.expect("redis connect");
        Self { stream }
    }

    pub async fn command(&mut self, arguments: &[&[u8]]) -> Resp {
        let mut encoded = format!("*{}\r\n", arguments.len()).into_bytes();
        for argument in arguments {
            encoded.extend_from_slice(format!("${}\r\n", argument.len()).as_bytes());
            encoded.extend_from_slice(argument);
            encoded.extend_from_slice(b"\r\n");
        }
        self.stream.write_all(&encoded).await.expect("redis write");
        read_reply(&mut self.stream).await
    }
}

fn read_reply<'a>(stream: &'a mut TcpStream) -> Pin<Box<dyn Future<Output = Resp> + Send + 'a>> {
    Box::pin(async move {
        let mut line = Vec::new();
        read_line(stream, &mut line).await;
        let prefix = line.first().copied().unwrap_or(b'-');
        let text = String::from_utf8_lossy(&line[1..]).to_string();
        match prefix {
            b'+' => Resp::Status(text),
            b'-' => Resp::Error(text),
            b':' => Resp::Integer(text.parse().unwrap_or(0)),
            b'$' => {
                let length: i64 = text.parse().unwrap_or(-1);
                if length < 0 {
                    Resp::Bulk(None)
                } else {
                    let mut data = vec![0u8; length as usize + 2];
                    stream.read_exact(&mut data).await.expect("bulk read");
                    data.truncate(length as usize);
                    Resp::Bulk(Some(data))
                }
            }
            b'*' | b'~' | b'>' => {
                let count: i64 = text.parse().unwrap_or(-1);
                if count < 0 {
                    Resp::Array(None)
                } else {
                    let mut values = Vec::new();
                    for _ in 0..count {
                        values.push(read_reply(stream).await);
                    }
                    if prefix == b'*' {
                        Resp::Array(Some(values))
                    } else {
                        Resp::Set(values)
                    }
                }
            }
            b'%' => {
                let count: i64 = text.parse().unwrap_or(-1);
                let mut pairs = Vec::new();
                for _ in 0..count.max(0) {
                    let key = read_reply(stream).await;
                    let value = read_reply(stream).await;
                    pairs.push((key, value));
                }
                Resp::Map(pairs)
            }
            b'_' => Resp::Null,
            b'#' => Resp::Boolean(text == "t"),
            _ => Resp::Error(format!("protocol error: {text}")),
        }
    })
}

async fn read_line(stream: &mut TcpStream, line: &mut Vec<u8>) {
    loop {
        let mut byte = [0u8; 1];
        let read = stream.read(&mut byte).await.expect("line read");
        if read == 0 || byte[0] == b'\n' {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return;
        }
        line.push(byte[0]);
    }
}

pub struct MemcachedClient {
    stream: TcpStream,
    buffer: Vec<u8>,
}

impl MemcachedClient {
    pub async fn connect(address: &str) -> Self {
        let stream = TcpStream::connect(address)
            .await
            .expect("memcached connect");
        Self {
            stream,
            buffer: Vec::new(),
        }
    }

    pub async fn send(&mut self, raw: &[u8], terminator: &[u8]) -> Vec<u8> {
        self.stream.write_all(raw).await.expect("memcached write");
        loop {
            if self.buffer.ends_with(terminator) {
                return std::mem::take(&mut self.buffer);
            }
            let mut chunk = [0u8; 4096];
            let read = self.stream.read(&mut chunk).await.expect("memcached read");
            assert!(read > 0, "memcached connection closed early");
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }
}

pub async fn http_post(server: &CompatServer, path: &str, body: &str) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(&server.http_addr)
        .await
        .expect("http connect");
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        server.http_addr,
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("http write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("http read");
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or(response.len());
    let head = String::from_utf8_lossy(&response[..split]).to_string();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = response.get(split + 4..).unwrap_or_default();
    let value = serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);
    (status, value)
}
