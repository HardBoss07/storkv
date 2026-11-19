use std::time::Duration;

use storkv_sdk::{SdkError, StorkvPoolOptions};

use crate::support;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_exhaustion_and_acquire_timeout() {
    for context in [
        support::TestContext::embedded(
            "exhaust_embedded",
            StorkvPoolOptions::new()
                .max_connections(1)
                .min_connections(0)
                .acquire_timeout(Duration::from_millis(80)),
        )
        .await,
        support::TestContext::networked(
            "exhaust_networked",
            StorkvPoolOptions::new()
                .max_connections(1)
                .min_connections(0)
                .acquire_timeout(Duration::from_millis(80)),
        )
        .await,
    ] {
        let first = context.pool.acquire().await.unwrap();
        let error = context.pool.acquire().await.unwrap_err();
        assert!(
            matches!(error, SdkError::AcquireTimeout),
            "unexpected error for {}: {error}",
            context.uri
        );
        drop(first);
        let second = context.pool.acquire().await.unwrap();
        drop(second);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_recovery_after_release() {
    for context in support::contexts("recovery").await {
        for _ in 0..5 {
            let connection = context.pool.acquire().await.unwrap();
            drop(connection);
        }
        let connection = context.pool.acquire().await.unwrap();
        drop(connection);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closed_pool_rejects_acquire() {
    for context in support::contexts("closed").await {
        context.pool.close().await;
        let error = context.pool.acquire().await.unwrap_err();
        assert!(matches!(error, SdkError::PoolClosed));
    }
}
