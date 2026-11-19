use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use storkv_sdk::{RetryPolicy, ScopeDefinition, SdkError};

use crate::support;

fn counter_value(bytes: Option<bytes::Bytes>) -> u64 {
    bytes
        .as_deref()
        .and_then(|raw| std::str::from_utf8(raw).ok())
        .and_then(|text| text.parse::<u64>().ok())
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn occ_contention_is_retried() {
    for context in support::contexts("occ").await {
        let pool = context.pool.clone();
        pool.provision_scopes(&[ScopeDefinition::new("counters")])
            .await
            .unwrap();
        let counters = pool.scope("counters");
        counters.put("counter", b"0").await.unwrap();

        let policy = RetryPolicy::default()
            .with_max_retries(64)
            .with_base_delay(Duration::from_millis(1))
            .with_max_delay(Duration::from_millis(20));

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let pool = pool.clone();
            let scope = counters.clone();
            let policy = policy.clone();
            tasks.push(tokio::spawn(async move {
                pool.transaction_with(policy, move |transaction| {
                    let scope = scope.clone();
                    Box::pin(async move {
                        let current = counter_value(transaction.get(&scope, "counter").await?);
                        tokio::time::sleep(Duration::from_millis(2)).await;
                        transaction
                            .put(&scope, "counter", (current + 1).to_string())
                            .await?;
                        Ok(())
                    })
                })
                .await
                .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }

        assert_eq!(counter_value(counters.get("counter").await.unwrap()), 8);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_transaction_commits_and_rolls_back() {
    for context in support::contexts("explicit_tx").await {
        let pool = &context.pool;
        pool.provision_scopes(&[ScopeDefinition::new("tx")])
            .await
            .unwrap();
        let scope = pool.scope("tx");

        let connection = pool.acquire().await.unwrap();
        let mut transaction = connection.begin().await.unwrap();
        transaction.put(&scope, "committed", b"yes").await.unwrap();
        transaction.commit().await.unwrap();
        assert_eq!(
            scope.get("committed").await.unwrap(),
            Some(bytes::Bytes::from_static(b"yes"))
        );

        let connection = pool.acquire().await.unwrap();
        let mut transaction = connection.begin().await.unwrap();
        transaction.put(&scope, "rolled", b"back").await.unwrap();
        transaction.rollback().await;
        assert!(scope.get("rolled").await.unwrap().is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_retryable_errors_abort_immediately() {
    for context in support::contexts("non_retryable").await {
        let pool = context.pool.clone();
        let attempts = Arc::new(AtomicU32::new(0));
        let counted = attempts.clone();
        let error = pool
            .transaction(move |_| {
                let counted = counted.clone();
                Box::pin(async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                    Err::<(), SdkError>(SdkError::Unsupported("application failure".into()))
                })
            })
            .await
            .unwrap_err();
        assert!(matches!(error, SdkError::Unsupported(_)));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }
}
