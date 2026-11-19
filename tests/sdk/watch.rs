use std::time::Duration;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use storkv_sdk::{ScopeDefinition, WatchEventKind};

use crate::support;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Reading {
    value: i64,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_delivers_put_then_delete() {
    for context in support::contexts("watch").await {
        let pool = &context.pool;
        pool.provision_scopes(&[ScopeDefinition::new("events")])
            .await
            .unwrap();
        let scope = pool.scope("events");
        let mut stream = scope.watch_prefix("").await.unwrap();

        scope.put("key", b"value").await.unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("watch put timed out")
            .expect("watch stream ended")
            .expect("watch error");
        assert_eq!(event.kind, WatchEventKind::Put);
        assert_eq!(event.key, Bytes::from_static(b"key"));
        assert_eq!(event.value, Some(Bytes::from_static(b"value")));

        scope.delete("key").await.unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("watch delete timed out")
            .expect("watch stream ended")
            .expect("watch error");
        assert_eq!(event.kind, WatchEventKind::Delete);
        assert_eq!(event.key, Bytes::from_static(b"key"));
        assert!(event.value.is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_prefix_filters_keys() {
    for context in support::contexts("watch_filter").await {
        let pool = &context.pool;
        pool.provision_scopes(&[ScopeDefinition::new("events")])
            .await
            .unwrap();
        let scope = pool.scope("events");
        let mut stream = scope.watch_prefix("hot:").await.unwrap();

        scope.put("cold:key", b"ignored").await.unwrap();
        scope.put("hot:key", b"delivered").await.unwrap();

        let event = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("watch timed out")
            .expect("watch stream ended")
            .expect("watch error");
        assert_eq!(event.key, Bytes::from_static(b"hot:key"));
        assert_eq!(event.value, Some(Bytes::from_static(b"delivered")));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_watch_decodes_values() {
    for context in support::contexts("watch_typed").await {
        let pool = &context.pool;
        pool.provision_scopes(&[ScopeDefinition::new("sensors")])
            .await
            .unwrap();
        let readings = pool.scope("sensors").collection::<Reading>("room:");
        let mut stream = readings.watch_prefix("").await.unwrap();

        readings.put("1", &Reading { value: 21 }).await.unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("typed watch timed out")
            .expect("typed watch ended")
            .expect("typed watch error");
        assert_eq!(event.kind, WatchEventKind::Put);
        assert_eq!(event.key, Bytes::from_static(b"1"));
        assert_eq!(event.value, Some(Reading { value: 21 }));
    }
}
