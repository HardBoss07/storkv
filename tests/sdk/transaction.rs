use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bytes::Bytes;
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
async fn transactional_typed_merges_support_read_your_writes() {
    for context in support::contexts("tx_merges").await {
        let pool = &context.pool;
        pool.provision_scopes(&[ScopeDefinition::new("tx_merge")])
            .await
            .unwrap();
        let scope = pool.scope("tx_merge");

        scope.put("log", b"01").await.unwrap();
        scope.put("max", b"5").await.unwrap();
        scope.put("min", b"5").await.unwrap();
        scope.put("or_bits", b"3").await.unwrap();
        scope.put("and_bits", b"15").await.unwrap();

        let connection = pool.acquire().await.unwrap();
        let mut tx = connection.begin().await.unwrap();
        tx.append_bytes(&scope, "log", b"234", Some(4))
            .await
            .unwrap();
        assert_eq!(
            tx.get(&scope, "log").await.unwrap(),
            Some(Bytes::from_static(b"1234"))
        );

        tx.max_i64(&scope, "max", 19).await.unwrap();
        assert_eq!(
            tx.get(&scope, "max").await.unwrap(),
            Some(Bytes::from_static(b"19"))
        );

        tx.min_i64(&scope, "min", -4).await.unwrap();
        assert_eq!(
            tx.get(&scope, "min").await.unwrap(),
            Some(Bytes::from_static(b"-4"))
        );

        tx.bit_or(&scope, "or_bits", 0b1100).await.unwrap();
        assert_eq!(
            tx.get(&scope, "or_bits").await.unwrap(),
            Some(Bytes::from_static(b"15"))
        );

        tx.bit_and(&scope, "and_bits", 0b1010).await.unwrap();
        assert_eq!(
            tx.get(&scope, "and_bits").await.unwrap(),
            Some(Bytes::from_static(b"10"))
        );

        tx.commit().await.unwrap();

        assert_eq!(
            scope.get("log").await.unwrap(),
            Some(Bytes::from_static(b"1234"))
        );
        assert_eq!(
            scope.get("max").await.unwrap(),
            Some(Bytes::from_static(b"19"))
        );
        assert_eq!(
            scope.get("min").await.unwrap(),
            Some(Bytes::from_static(b"-4"))
        );
        assert_eq!(
            scope.get("or_bits").await.unwrap(),
            Some(Bytes::from_static(b"15"))
        );
        assert_eq!(
            scope.get("and_bits").await.unwrap(),
            Some(Bytes::from_static(b"10"))
        );

        let connection = pool.acquire().await.unwrap();
        let mut followup = connection.begin().await.unwrap();
        followup.bit_and(&scope, "or_bits", 0b1010).await.unwrap();
        assert_eq!(
            followup.get(&scope, "or_bits").await.unwrap(),
            Some(Bytes::from_static(b"10"))
        );
        followup.commit().await.unwrap();
        assert_eq!(
            scope.get("or_bits").await.unwrap(),
            Some(Bytes::from_static(b"10"))
        );
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
