use storkv_client::{AsyncClient, ScopeHandle};
use storkv_proto::{WireDurability, WireRetention};

use super::support::{MemcachedClient, RedisClient, spawn};

async fn native_client(uri: &str) -> AsyncClient {
    AsyncClient::connect_uri(uri).await.expect("native connect")
}

#[tokio::test]
async fn mixed_protocol_clients_share_one_server_without_interference() {
    const PER_PROTOCOL: usize = 50;

    let server = spawn("compat_mixed_protocols").await;
    let native = native_client(&server.tcp_uri).await;
    let scope = native
        .create_scope(
            "mixed_scope",
            WireDurability::StrictFsync,
            WireRetention::Permanent,
            Some(8),
        )
        .await
        .expect("create mixed scope");

    let native_task = {
        let uri = server.tcp_uri.clone();
        let scope: ScopeHandle = scope.clone();
        tokio::spawn(async move {
            let client = native_client(&uri).await;
            for index in 0..PER_PROTOCOL {
                client
                    .put(
                        &scope,
                        format!("native:{index:03}"),
                        format!("native-value:{index:03}"),
                    )
                    .await
                    .expect("native put");
            }
        })
    };

    let http_task = {
        let uri = format!("http://{}", server.http_addr);
        let scope: ScopeHandle = scope.clone();
        tokio::spawn(async move {
            let client = native_client(&uri).await;
            for index in 0..PER_PROTOCOL {
                client
                    .put(
                        &scope,
                        format!("http:{index:03}"),
                        format!("http-value:{index:03}"),
                    )
                    .await
                    .expect("http put");
            }
        })
    };

    let redis_task = {
        let address = server.redis_addr.clone();
        tokio::spawn(async move {
            let mut client = RedisClient::connect(&address).await;
            for index in 0..PER_PROTOCOL {
                client
                    .command(&[
                        b"SET",
                        format!("redis:{index:03}").as_bytes(),
                        format!("redis-value:{index:03}").as_bytes(),
                    ])
                    .await;
            }
        })
    };

    let memcached_task = {
        let address = server.memcached_addr.clone();
        tokio::spawn(async move {
            let mut client = MemcachedClient::connect(&address).await;
            for index in 0..PER_PROTOCOL {
                let key = format!("memcached:{index:03}");
                let value = format!("memcached-value:{index:03}");
                let command = format!("set {key} 0 0 {}\r\n{value}\r\n", value.len());
                assert_eq!(
                    client.send(command.as_bytes(), b"\r\n").await,
                    b"STORED\r\n"
                );
            }
        })
    };

    for task in [native_task, http_task, redis_task, memcached_task] {
        task.await.expect("protocol worker");
    }

    let native_rows = native.scan_prefix(&scope, b"native:").await.expect("scan");
    assert_eq!(native_rows.len(), PER_PROTOCOL);
    let scope_id = scope.scope_id;
    let replayed = native
        .scan_prefix(
            &ScopeHandle {
                scope_id,
                name: scope.name.clone(),
            },
            b"http:",
        )
        .await
        .expect("scan http keys via native");
    assert_eq!(replayed.len(), PER_PROTOCOL);

    let mut redis = RedisClient::connect(&server.redis_addr).await;
    assert_eq!(
        redis.command(&[b"GET", b"redis:049"]).await,
        super::support::Resp::Bulk(Some(b"redis-value:049".to_vec()))
    );

    let mut memcached = MemcachedClient::connect(&server.memcached_addr).await;
    let reply = memcached.send(b"get memcached:049\r\n", b"END\r\n").await;
    assert!(
        reply
            .windows(b"memcached-value:049".len())
            .any(|window| window == b"memcached-value:049"),
        "unexpected memcached reply: {:?}",
        String::from_utf8_lossy(&reply)
    );

    let stats = native.admin_stats().await.expect("admin stats");
    let transports: Vec<u8> = stats.transports.iter().map(|entry| entry.kind).collect();
    assert!(transports.contains(&0), "native transport missing");
    assert!(transports.contains(&1), "http transport missing");
}
