//! Redis authentication, handshake, CLIENT, and COMMAND tests.

use crate::support::{RedisClient, Resp, spawn, spawn_with_password};

const PASSWORD: &str = "s3cret";

fn map_value<'a>(response: &'a Resp, key: &str) -> Option<&'a Resp> {
    let Resp::Map(pairs) = response else {
        return None;
    };
    pairs.iter().find_map(|(entry_key, value)| match entry_key {
        Resp::Bulk(Some(entry)) if entry == key.as_bytes() => Some(value),
        _ => None,
    })
}

fn array_value<'a>(response: &'a Resp, key: &str) -> Option<&'a Resp> {
    let Resp::Array(Some(values)) = response else {
        return None;
    };
    values.chunks_exact(2).find_map(|pair| match &pair[0] {
        Resp::Bulk(Some(entry)) if entry == key.as_bytes() => Some(&pair[1]),
        _ => None,
    })
}

#[tokio::test]
async fn unauthenticated_commands_are_rejected_with_noauth() {
    let server = spawn_with_password("compat_redis_noauth", Some(PASSWORD)).await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        client.command(&[b"PING"]).await,
        Resp::Status("PONG".into())
    );
    for command in [
        vec![&b"GET"[..], b"key"],
        vec![&b"SET"[..], b"key", b"value"],
        vec![&b"CLIENT"[..], b"ID"],
    ] {
        match client.command(&command).await {
            Resp::Error(message) => {
                assert_eq!(message, "NOAUTH Authentication required.");
            }
            other => panic!("command must require auth, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn auth_with_and_without_username_unlocks_commands() {
    let server = spawn_with_password("compat_redis_auth", Some(PASSWORD)).await;
    let mut password_only = RedisClient::connect(&server.redis_addr).await;
    let mut named = RedisClient::connect(&server.redis_addr).await;

    match password_only.command(&[b"AUTH", b"wrong"]).await {
        Resp::Error(message) => {
            assert!(
                message.starts_with("WRONGPASS"),
                "unexpected error: {message}"
            );
        }
        other => panic!("bad password must fail, got {other:?}"),
    }
    assert_eq!(
        password_only.command(&[b"GET", b"key"]).await,
        Resp::Error("NOAUTH Authentication required.".into())
    );

    assert_eq!(
        password_only.command(&[b"AUTH", PASSWORD.as_bytes()]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        password_only.command(&[b"SET", b"key", b"value"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        password_only.command(&[b"GET", b"key"]).await,
        Resp::Bulk(Some(b"value".to_vec()))
    );

    match named.command(&[b"AUTH", b"bob", PASSWORD.as_bytes()]).await {
        Resp::Error(message) => assert!(message.starts_with("WRONGPASS")),
        other => panic!("unknown user must fail, got {other:?}"),
    }
    assert_eq!(
        named
            .command(&[b"AUTH", b"default", PASSWORD.as_bytes()])
            .await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        named.command(&[b"GET", b"key"]).await,
        Resp::Bulk(Some(b"value".to_vec()))
    );
}

#[tokio::test]
async fn servers_without_a_password_accept_commands_and_reject_auth() {
    let server = spawn("compat_redis_no_password").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        client.command(&[b"SET", b"key", b"value"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        client.command(&[b"GET", b"key"]).await,
        Resp::Bulk(Some(b"value".to_vec()))
    );
    match client.command(&[b"AUTH", b"anything"]).await {
        Resp::Error(message) => assert!(message.starts_with("ERR AUTH"), "{message}"),
        other => panic!("AUTH without a password must fail, got {other:?}"),
    }
}

#[tokio::test]
async fn hello_authenticates_and_negotiates_the_protocol() {
    let server = spawn_with_password("compat_redis_hello", Some(PASSWORD)).await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    let before = client.command(&[b"HELLO"]).await;
    assert_eq!(
        array_value(&before, "proto"),
        Some(&Resp::Integer(2)),
        "HELLO without arguments must report RESP2"
    );
    assert_eq!(
        client.command(&[b"GET", b"key"]).await,
        Resp::Error("NOAUTH Authentication required.".into()),
        "HELLO without AUTH must not authenticate"
    );

    let reply = client
        .command(&[
            b"HELLO",
            b"3",
            b"AUTH",
            b"default",
            PASSWORD.as_bytes(),
            b"SETNAME",
            b"tool",
        ])
        .await;
    assert_eq!(map_value(&reply, "proto"), Some(&Resp::Integer(3)));
    assert_eq!(
        map_value(&reply, "server"),
        Some(&Resp::Bulk(Some(b"storkv".to_vec())))
    );
    assert!(matches!(map_value(&reply, "id"), Some(Resp::Integer(id)) if *id > 0));
    assert_eq!(
        client.command(&[b"CLIENT", b"GETNAME"]).await,
        Resp::Bulk(Some(b"tool".to_vec()))
    );
    assert_eq!(client.command(&[b"GET", b"key"]).await, Resp::Null);
}

#[tokio::test]
async fn hello_with_bad_credentials_changes_no_connection_state() {
    let server = spawn_with_password("compat_redis_hello_bad", Some(PASSWORD)).await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    match client
        .command(&[b"HELLO", b"3", b"AUTH", b"default", b"wrong"])
        .await
    {
        Resp::Error(message) => assert!(message.starts_with("WRONGPASS"), "{message}"),
        other => panic!("bad HELLO credentials must fail, got {other:?}"),
    }
    let state = client.command(&[b"HELLO"]).await;
    assert_eq!(
        array_value(&state, "proto"),
        Some(&Resp::Integer(2)),
        "a failed HELLO must not switch the protocol"
    );
    assert_eq!(
        client.command(&[b"GET", b"key"]).await,
        Resp::Error("NOAUTH Authentication required.".into()),
        "a failed HELLO must not authenticate"
    );
}

#[tokio::test]
async fn client_id_and_name_track_each_connection() {
    let server = spawn("compat_redis_client").await;
    let mut first = RedisClient::connect(&server.redis_addr).await;
    let mut second = RedisClient::connect(&server.redis_addr).await;

    let first_id = match first.command(&[b"CLIENT", b"ID"]).await {
        Resp::Integer(id) if id > 0 => id,
        other => panic!("unexpected CLIENT ID reply: {other:?}"),
    };
    let second_id = match second.command(&[b"CLIENT", b"ID"]).await {
        Resp::Integer(id) => id,
        other => panic!("unexpected CLIENT ID reply: {other:?}"),
    };
    assert_ne!(first_id, second_id, "client ids must be unique");

    assert_eq!(
        first.command(&[b"CLIENT", b"GETNAME"]).await,
        Resp::Bulk(None)
    );
    assert_eq!(
        first.command(&[b"CLIENT", b"SETNAME", b"worker"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        first.command(&[b"CLIENT", b"GETNAME"]).await,
        Resp::Bulk(Some(b"worker".to_vec()))
    );
    assert_eq!(
        second.command(&[b"CLIENT", b"GETNAME"]).await,
        Resp::Bulk(None)
    );

    match first.command(&[b"CLIENT", b"SETNAME", b"bad name"]).await {
        Resp::Error(message) => assert!(
            message.contains("Client names cannot contain"),
            "unexpected error: {message}"
        ),
        other => panic!("a name with a space must fail, got {other:?}"),
    }
    assert_eq!(
        first.command(&[b"CLIENT", b"GETNAME"]).await,
        Resp::Bulk(Some(b"worker".to_vec())),
        "a rejected name must not replace the stored name"
    );
}

#[tokio::test]
async fn command_and_command_count_describe_supported_commands() {
    let server = spawn("compat_redis_command").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    let Resp::Integer(count) = client.command(&[b"COMMAND", b"COUNT"]).await else {
        panic!("COMMAND COUNT must return an integer");
    };
    assert!(count > 0);

    let Resp::Array(Some(entries)) = client.command(&[b"COMMAND"]).await else {
        panic!("COMMAND must return an array");
    };
    assert_eq!(entries.len() as i64, count);

    let mut names = Vec::new();
    for entry in entries {
        let Resp::Array(Some(fields)) = entry else {
            panic!("each COMMAND entry must be an array");
        };
        assert_eq!(fields.len(), 6, "unexpected metadata size: {fields:?}");
        let Resp::Bulk(Some(name)) = &fields[0] else {
            panic!("a command name must be a bulk string");
        };
        names.push(String::from_utf8_lossy(name).to_string());
    }
    for expected in ["auth", "client", "command", "get", "hello", "set"] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing {expected}"
        );
    }
}
