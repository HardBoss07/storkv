use std::time::Duration;

use storkv_proto::WireChangeEvent;

use super::support;

#[tokio::test]
async fn websocket_watch_stream_receives_put_events() {
    let transports = support::spawn_all("transports_watch").await;
    let client = support::connect(&transports.ws_uri).await;
    let scope = support::make_scope(&client, "watch_scope").await;

    let mut watch = client.subscribe_watch().await.expect("subscribe watch");
    // A watch subscription owns its connection while waiting for events, so
    // writes use an independent client (the normal production pattern).
    let writer = support::connect(&transports.ws_uri).await;
    let writer_scope = scope.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        writer
            .put(&writer_scope, b"watched", b"value")
            .await
            .expect("watch put");
    });

    let event = tokio::time::timeout(Duration::from_secs(5), watch.next())
        .await
        .expect("watch timeout")
        .expect("watch transport")
        .expect("watch event");
    match event {
        WireChangeEvent::Put { key, .. } => {
            assert_eq!(key.as_ref(), b"watched");
        }
        other => panic!("expected put event, got {other:?}"),
    }
}
