use super::support::{MemcachedClient, http_post, spawn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn malformed_http_json_is_rejected_without_stopping_the_listener() {
    let server = spawn("compat_adversarial_http").await;

    let (status, _) = http_post(&server, "/v1/scopes", "{\"name\":").await;
    assert!(status >= 400, "truncated JSON must be rejected: {status}");

    let (status, _) = http_post(&server, "/v1/rpc", "not-json-at-all").await;
    assert!(status >= 400, "non-JSON body must be rejected: {status}");

    let mut redis = super::support::RedisClient::connect(&server.redis_addr).await;
    assert_eq!(
        redis.command(&[b"SET", b"healthy", b"yes"]).await,
        super::support::Resp::Status("OK".into())
    );
    assert_eq!(
        redis.command(&[b"GET", b"healthy"]).await,
        super::support::Resp::Bulk(Some(b"yes".to_vec()))
    );
}

#[tokio::test]
async fn invalid_memcached_flags_and_ttl_return_client_errors() {
    let server = spawn("compat_adversarial_memcached").await;
    let mut client = MemcachedClient::connect(&server.memcached_addr).await;

    for command in [
        b"set bad flags 0 1\r\nx\r\n".as_slice(),
        b"set bad 0 -1 1\r\nx\r\n".as_slice(),
        b"set bad 0 0 nope\r\nx\r\n".as_slice(),
    ] {
        let reply = client.send(command, b"\r\n").await;
        assert!(
            reply.starts_with(b"CLIENT_ERROR") || reply.starts_with(b"ERROR"),
            "unexpected reply: {:?}",
            String::from_utf8_lossy(&reply)
        );
    }

    assert_eq!(
        client.send(b"set good 0 0 2\r\nok\r\n", b"\r\n").await,
        b"STORED\r\n"
    );
    assert_eq!(
        client.send(b"get good\r\n", b"END\r\n").await,
        b"VALUE good 0 2\r\nok\r\nEND\r\n"
    );
}

#[tokio::test]
async fn oversized_resp_bulk_length_is_rejected_and_service_continues() {
    let server = spawn("compat_adversarial_resp").await;
    let mut stream = TcpStream::connect(&server.redis_addr).await.unwrap();
    stream.write_all(b"$999999999999\r\n").await.unwrap();
    let mut response = [0u8; 256];
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.read(&mut response),
    )
    .await
    .expect("server must reply or close promptly")
    .expect("read");
    assert!(
        read == 0 || response[..read].starts_with(b"-"),
        "unexpected reply: {:?}",
        String::from_utf8_lossy(&response[..read])
    );

    let mut redis = super::support::RedisClient::connect(&server.redis_addr).await;
    assert_eq!(
        redis.command(&[b"PING"]).await,
        super::support::Resp::Status("PONG".into())
    );
}
