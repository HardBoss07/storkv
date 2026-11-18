use super::support::{RedisClient, Resp, spawn};

#[tokio::test]
async fn redis_watch_contention_aborts_only_the_stale_committer() {
    let server = spawn("compat_redis_watch_race").await;
    let mut first = RedisClient::connect(&server.redis_addr).await;
    let mut second = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        first.command(&[b"SET", b"contended", b"base"]).await,
        Resp::Status("OK".into())
    );

    assert_eq!(
        first.command(&[b"WATCH", b"contended"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        second.command(&[b"WATCH", b"contended"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(first.command(&[b"MULTI"]).await, Resp::Status("OK".into()));
    assert_eq!(
        first.command(&[b"SET", b"contended", b"first"]).await,
        Resp::Status("QUEUED".into())
    );
    assert_eq!(second.command(&[b"MULTI"]).await, Resp::Status("OK".into()));
    assert_eq!(
        second.command(&[b"SET", b"contended", b"second"]).await,
        Resp::Status("QUEUED".into())
    );

    match first.command(&[b"EXEC"]).await {
        Resp::Array(Some(values)) => assert_eq!(values, vec![Resp::Status("OK".into())]),
        other => panic!("first EXEC must commit, got {other:?}"),
    }
    assert_eq!(
        second.command(&[b"EXEC"]).await,
        Resp::Array(None),
        "stale WATCH must abort with a Null array reply"
    );

    assert_eq!(
        first.command(&[b"GET", b"contended"]).await,
        Resp::Bulk(Some(b"first".to_vec()))
    );
}

#[tokio::test]
async fn redis_multi_exec_increment_storm_has_no_lost_updates() {
    const WORKERS: usize = 8;
    const INCREMENTS: usize = 20;

    let server = spawn("compat_redis_multi_storm").await;
    let address = server.redis_addr.clone();
    let mut workers = Vec::new();
    for _ in 0..WORKERS {
        let address = address.clone();
        workers.push(tokio::spawn(async move {
            let mut client = RedisClient::connect(&address).await;
            let mut committed = 0;
            while committed < INCREMENTS {
                assert_eq!(client.command(&[b"MULTI"]).await, Resp::Status("OK".into()));
                assert_eq!(
                    client.command(&[b"INCR", b"storm"]).await,
                    Resp::Status("QUEUED".into())
                );
                match client.command(&[b"EXEC"]).await {
                    Resp::Array(Some(values)) => {
                        assert_eq!(values.len(), 1);
                        committed += 1;
                    }
                    Resp::Array(None) => continue,
                    other => panic!("unexpected EXEC reply: {other:?}"),
                }
            }
        }));
    }
    for worker in workers {
        worker.await.expect("worker");
    }

    let mut reader = RedisClient::connect(&address).await;
    assert_eq!(
        reader.command(&[b"GET", b"storm"]).await,
        Resp::Bulk(Some((WORKERS * INCREMENTS).to_string().into_bytes()))
    );
}
