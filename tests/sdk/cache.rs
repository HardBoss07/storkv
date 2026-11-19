use std::time::Duration;

use bytes::Bytes;
use storkv_proto::{WireDurability, WireMergeOp, WireRetention};
use storkv_sdk::{CacheOptions, ScopeDefinition, StorkvPoolOptions};

use crate::support;

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const POLL_ATTEMPTS: u32 = 250;

fn enabled_cache() -> CacheOptions {
    CacheOptions::default().enable()
}

async fn wait_for_value(scope: &storkv_sdk::Scope, expected: Option<&[u8]>) -> bool {
    wait_for_key(scope, "key", expected).await
}

async fn wait_for_key(scope: &storkv_sdk::Scope, key: &str, expected: Option<&[u8]>) -> bool {
    for _ in 0..POLL_ATTEMPTS {
        let current = scope.get(key).await.unwrap_or(None);
        if current.as_deref() == expected {
            return true;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    false
}

#[test]
fn cache_is_disabled_by_default() {
    let options = StorkvPoolOptions::default();
    assert!(!options.cache.enabled);
    assert_eq!(options.cache.max_entries_per_scope, 1024);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cache_invalidates_after_remote_put_and_delete() {
    let context =
        support::TestContext::networked("cache_invalidate", StorkvPoolOptions::default()).await;
    context
        .pool
        .provision_scopes(&[ScopeDefinition::new("cache_scope")])
        .await
        .unwrap();

    let cached = StorkvPoolOptions::default()
        .cache(enabled_cache())
        .connect(&context.uri)
        .await
        .unwrap();
    let cached_scope = cached.scope("cache_scope");
    let writer = context.pool.scope("cache_scope");

    assert_eq!(cached_scope.get("key").await.unwrap(), None);

    writer.put("key", b"first").await.unwrap();
    assert!(
        wait_for_value(&cached_scope, Some(b"first")).await,
        "remote put must reach the cache"
    );

    writer.put("key", b"second").await.unwrap();
    assert!(
        wait_for_value(&cached_scope, Some(b"second")).await,
        "remote update must invalidate the cached value"
    );

    writer.delete("key").await.unwrap();
    assert!(
        wait_for_value(&cached_scope, None).await,
        "remote delete must invalidate the cached value"
    );

    cached.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_cache_reads_own_writes() {
    let options = StorkvPoolOptions::default().cache(enabled_cache());
    let context = support::TestContext::embedded("cache_embedded", options).await;
    context
        .pool
        .provision_scopes(&[ScopeDefinition::new("cache_scope")])
        .await
        .unwrap();
    let scope = context.pool.scope("cache_scope");

    scope.put("key", b"value").await.unwrap();
    assert_eq!(
        scope.get("key").await.unwrap(),
        Some(Bytes::from_static(b"value"))
    );
    assert_eq!(
        scope.get("key").await.unwrap(),
        Some(Bytes::from_static(b"value"))
    );

    scope.put("key", b"next").await.unwrap();
    assert_eq!(
        scope.get("key").await.unwrap(),
        Some(Bytes::from_static(b"next"))
    );

    scope.delete("key").await.unwrap();
    assert_eq!(scope.get("key").await.unwrap(), None);

    context.pool.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_cache_expires_ttl_entries() {
    let options = StorkvPoolOptions::default().cache(enabled_cache());
    let context = support::TestContext::embedded("cache_ttl", options).await;
    context
        .pool
        .provision_scopes(&[ScopeDefinition::new("cache_scope")])
        .await
        .unwrap();
    let scope = context.pool.scope("cache_scope");

    scope
        .put_with_ttl("key", b"value", Duration::from_millis(150))
        .await
        .unwrap();
    assert_eq!(
        scope.get("key").await.unwrap(),
        Some(Bytes::from_static(b"value"))
    );

    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        scope.get("key").await.unwrap(),
        None,
        "an expired entry must be a cache miss"
    );

    context.pool.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_cache_clears_after_delete_range() {
    let options = StorkvPoolOptions::default().cache(enabled_cache());
    let context = support::TestContext::embedded("cache_range", options).await;
    context
        .pool
        .provision_scopes(&[ScopeDefinition::new("cache_scope")])
        .await
        .unwrap();
    let scope = context.pool.scope("cache_scope");

    scope.put("a", b"one").await.unwrap();
    scope.put("b", b"two").await.unwrap();
    assert_eq!(
        scope.get("a").await.unwrap(),
        Some(Bytes::from_static(b"one"))
    );
    assert_eq!(
        scope.get("b").await.unwrap(),
        Some(Bytes::from_static(b"two"))
    );

    let tx_scope = scope.clone();
    context
        .pool
        .transaction(move |transaction| {
            let tx_scope = tx_scope.clone();
            Box::pin(async move {
                transaction.delete_range(&tx_scope, b"a", b"z").await?;
                Ok(())
            })
        })
        .await
        .unwrap();

    assert!(
        wait_for_key(&scope, "a", None).await,
        "a range delete must clear cached keys"
    );
    assert!(
        wait_for_key(&scope, "b", None).await,
        "a range delete must clear cached keys"
    );

    context.pool.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn networked_cache_clears_after_merge() {
    let context =
        support::TestContext::networked("cache_merge", StorkvPoolOptions::default()).await;
    let client = storkv_client::AsyncClient::connect_uri(&context.uri)
        .await
        .expect("raw client");
    let handle = client
        .create_scope(
            "cache_scope",
            WireDurability::StrictFsync,
            WireRetention::Permanent,
            None,
        )
        .await
        .expect("create scope");

    let cached = StorkvPoolOptions::default()
        .cache(enabled_cache())
        .connect(&context.uri)
        .await
        .unwrap();
    let cached_scope = cached.scope("cache_scope");

    client.put(&handle, "key", b"41").await.unwrap();
    assert_eq!(
        cached_scope.get("key").await.unwrap(),
        Some(Bytes::from_static(b"41"))
    );

    client
        .merge(&handle, "key", WireMergeOp::CounterAdd(1))
        .await
        .unwrap();
    assert!(
        wait_for_value(&cached_scope, Some(b"42")).await,
        "a merge must clear cached keys"
    );

    cached.close().await;
}
