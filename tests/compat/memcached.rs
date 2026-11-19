use crate::support::{MemcachedClient, spawn};

fn cas_token(reply: &[u8]) -> u64 {
    let header_end = reply
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap_or(0);
    let header = std::str::from_utf8(&reply[..header_end]).unwrap_or_default();
    header
        .split_whitespace()
        .last()
        .and_then(|token| token.parse().ok())
        .unwrap_or(0)
}

#[tokio::test]
async fn memcached_storage_and_retrieval() {
    let server = spawn("compat_memcached_storage").await;
    let mut client = MemcachedClient::connect(&server.memcached_addr).await;

    assert_eq!(
        client.send(b"set alpha 7 0 5\r\nhello\r\n", b"\r\n").await,
        b"STORED\r\n"
    );
    assert_eq!(
        client.send(b"get alpha\r\n", b"END\r\n").await,
        b"VALUE alpha 7 5\r\nhello\r\nEND\r\n"
    );
    assert_eq!(
        client.send(b"get alpha missing\r\n", b"END\r\n").await,
        b"VALUE alpha 7 5\r\nhello\r\nEND\r\n"
    );
    assert_eq!(
        client.send(b"add alpha 0 0 5\r\nother\r\n", b"\r\n").await,
        b"NOT_STORED\r\n"
    );
    assert_eq!(
        client.send(b"add beta 0 0 5\r\nvalue\r\n", b"\r\n").await,
        b"STORED\r\n"
    );
    assert_eq!(
        client
            .send(b"replace missing 0 0 1\r\nx\r\n", b"\r\n")
            .await,
        b"NOT_STORED\r\n"
    );
    assert_eq!(
        client
            .send(b"replace beta 0 0 7\r\nupdated\r\n", b"\r\n")
            .await,
        b"STORED\r\n"
    );
    assert_eq!(
        client.send(b"get beta\r\n", b"END\r\n").await,
        b"VALUE beta 0 7\r\nupdated\r\nEND\r\n"
    );
}

#[tokio::test]
async fn memcached_append_prepend_and_counters() {
    let server = spawn("compat_memcached_counters").await;
    let mut client = MemcachedClient::connect(&server.memcached_addr).await;

    client.send(b"set word 0 0 5\r\nworld\r\n", b"\r\n").await;
    assert_eq!(
        client.send(b"append word 0 0 1\r\n!\r\n", b"\r\n").await,
        b"STORED\r\n"
    );
    assert_eq!(
        client
            .send(b"prepend word 0 0 6\r\nhello \r\n", b"\r\n")
            .await,
        b"STORED\r\n"
    );
    assert_eq!(
        client.send(b"get word\r\n", b"END\r\n").await,
        b"VALUE word 0 12\r\nhello world!\r\nEND\r\n"
    );

    client.send(b"set counter 0 0 2\r\n10\r\n", b"\r\n").await;
    assert_eq!(client.send(b"incr counter 5\r\n", b"\r\n").await, b"15\r\n");
    assert_eq!(client.send(b"decr counter 3\r\n", b"\r\n").await, b"12\r\n");
    assert_eq!(
        client.send(b"incr missing 1\r\n", b"\r\n").await,
        b"NOT_FOUND\r\n"
    );

    client.send(b"set text 0 0 3\r\nabc\r\n", b"\r\n").await;
    match client.send(b"incr text 1\r\n", b"\r\n").await.as_slice() {
        b"CLIENT_ERROR cannot increment or decrement non-numeric value\r\n" => {}
        other => panic!("unexpected incr reply: {other:?}"),
    }
}

#[tokio::test]
async fn memcached_gets_cas_and_meta_commands() {
    let server = spawn("compat_memcached_cas").await;
    let mut client = MemcachedClient::connect(&server.memcached_addr).await;

    client.send(b"set item 0 0 5\r\nfirst\r\n", b"\r\n").await;
    let gets = client.send(b"gets item\r\n", b"END\r\n").await;
    let token = cas_token(&gets);
    assert!(token > 0, "expected a positive CAS token in {gets:?}");

    let wrong = format!("cas item 0 0 6 {}\r\nsecond\r\n", token + 1);
    assert_eq!(client.send(wrong.as_bytes(), b"\r\n").await, b"EXISTS\r\n");

    let correct = format!("cas item 0 0 6 {token}\r\nsecond\r\n");
    assert_eq!(
        client.send(correct.as_bytes(), b"\r\n").await,
        b"STORED\r\n"
    );
    assert_eq!(
        client.send(b"get item\r\n", b"END\r\n").await,
        b"VALUE item 0 6\r\nsecond\r\nEND\r\n"
    );

    assert_eq!(
        client.send(b"touch item 60\r\n", b"\r\n").await,
        b"TOUCHED\r\n"
    );
    assert_eq!(
        client.send(b"touch missing 60\r\n", b"\r\n").await,
        b"NOT_FOUND\r\n"
    );
    assert_eq!(
        client.send(b"delete item\r\n", b"\r\n").await,
        b"DELETED\r\n"
    );
    assert_eq!(
        client.send(b"delete item\r\n", b"\r\n").await,
        b"NOT_FOUND\r\n"
    );

    let version = client.send(b"version\r\n", b"\r\n").await;
    assert!(version.starts_with(b"VERSION "));
    let stats = client.send(b"stats\r\n", b"END\r\n").await;
    assert!(stats.starts_with(b"STAT "));
    assert_eq!(client.send(b"flush_all\r\n", b"\r\n").await, b"OK\r\n");
    assert_eq!(
        client.send(b"get item\r\n", b"END\r\n").await,
        b"END\r\n",
        "get must miss every key after flush_all"
    );
}

#[tokio::test]
async fn memcached_expiry_removes_entries() {
    let server = spawn("compat_memcached_expiry").await;
    let mut client = MemcachedClient::connect(&server.memcached_addr).await;

    client
        .send(b"set ephemeral 0 1 4\r\nsoon\r\n", b"\r\n")
        .await;
    assert_eq!(
        client.send(b"get ephemeral\r\n", b"END\r\n").await,
        b"VALUE ephemeral 0 4\r\nsoon\r\nEND\r\n"
    );
    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    assert_eq!(
        client.send(b"get ephemeral\r\n", b"END\r\n").await,
        b"END\r\n"
    );
}

#[tokio::test]
async fn memcached_flush_all_is_denied_under_strict_admin() {
    let server = spawn("compat_memcached_strict_flush").await;
    server.server.set_strict_admin(true);
    let mut client = MemcachedClient::connect(&server.memcached_addr).await;

    assert_eq!(
        client.send(b"set kept 0 0 5\r\nvalue\r\n", b"\r\n").await,
        b"STORED\r\n"
    );
    let reply = client.send(b"flush_all\r\n", b"\r\n").await;
    assert!(
        reply.starts_with(b"CLIENT_ERROR"),
        "flush_all must be denied: {:?}",
        String::from_utf8_lossy(&reply)
    );
    assert_eq!(
        client.send(b"get kept\r\n", b"END\r\n").await,
        b"VALUE kept 0 5\r\nvalue\r\nEND\r\n"
    );
}
