use std::time::Duration;

use bytes::Bytes;
use storkv_sdk::{ScopeDefinition, StorkvPoolOptions};

use crate::support;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_crud_behavior() {
    for context in support::contexts("basic").await {
        let pool = &context.pool;
        pool.provision_scopes(&[
            ScopeDefinition::new("kv").with_prefix_bloom_len(16),
            ScopeDefinition::new("ttl"),
        ])
        .await
        .unwrap_or_else(|error| panic!("provision {}: {error}", context.uri));

        let scope = pool.scope("kv");
        assert!(scope.get("missing").await.unwrap().is_none());
        scope.put("alpha", b"one").await.unwrap();
        scope.put("beta", b"two").await.unwrap();
        scope.put("beta", b"two-updated").await.unwrap();

        assert_eq!(
            scope.get("alpha").await.unwrap(),
            Some(Bytes::from_static(b"one"))
        );
        assert_eq!(
            scope.get("beta").await.unwrap(),
            Some(Bytes::from_static(b"two-updated"))
        );
        assert!(scope.exists("alpha").await.unwrap());
        assert!(!scope.exists("missing").await.unwrap());

        let scanned = scope.scan_prefix("").await.unwrap();
        assert_eq!(scanned.len(), 2);
        assert_eq!(scanned[0].0, Bytes::from_static(b"alpha"));
        assert_eq!(scanned[1].0, Bytes::from_static(b"beta"));

        let ranged = scope
            .scan_range(Some(&b"alpha"[..]), Some(&b"beta"[..]))
            .await
            .unwrap();
        assert_eq!(ranged.len(), 1);
        assert_eq!(ranged[0].0, Bytes::from_static(b"alpha"));

        scope.delete("alpha").await.unwrap();
        assert!(scope.get("alpha").await.unwrap().is_none());

        let ttl_scope = pool.scope("ttl");
        ttl_scope
            .put_with_ttl("session", b"live", Duration::from_millis(200))
            .await
            .unwrap();
        assert_eq!(
            ttl_scope.get("session").await.unwrap(),
            Some(Bytes::from_static(b"live"))
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(ttl_scope.get("session").await.unwrap().is_none());

        pool.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provisioning_is_idempotent() {
    for context in support::contexts("provision").await {
        let pool = &context.pool;
        let definitions = [
            ScopeDefinition::new("users").with_prefix_bloom_len(8),
            ScopeDefinition::new("sessions")
                .with_ttl(Duration::from_secs(60), storkv_sdk::CompactionStyle::Fifo),
        ];
        pool.provision_scopes(&definitions).await.unwrap();
        pool.provision_scopes(&definitions).await.unwrap();

        let users = pool.scope("users");
        users.put("u1", b"present").await.unwrap();
        assert_eq!(
            users.get("u1").await.unwrap(),
            Some(Bytes::from_static(b"present"))
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_scope_reports_error() {
    for context in support::contexts("missing").await {
        let scope = context.pool.scope("absent");
        let error = scope.get("key").await.unwrap_err();
        assert!(matches!(error, storkv_sdk::SdkError::ScopeNotFound(_)));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_and_touch_extends_expiry() {
    for context in support::contexts("touch").await {
        let pool = &context.pool;
        pool.provision_scopes(&[ScopeDefinition::new("sessions")])
            .await
            .unwrap();
        let sessions = pool.scope("sessions");
        sessions
            .put_with_ttl("token", b"alive", Duration::from_millis(300))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            sessions
                .get_and_touch("token", Duration::from_millis(900))
                .await
                .unwrap(),
            Some(Bytes::from_static(b"alive"))
        );
        tokio::time::sleep(Duration::from_millis(450)).await;
        assert_eq!(
            sessions.get("token").await.unwrap(),
            Some(Bytes::from_static(b"alive"))
        );
        assert!(
            sessions
                .get_and_touch("absent", Duration::from_secs(1))
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn options_are_reported() {
    let context = support::TestContext::embedded(
        "options",
        StorkvPoolOptions::new()
            .max_connections(3)
            .min_connections(0),
    )
    .await;
    assert_eq!(context.pool.options().max_connections, 3);
    assert_eq!(context.pool.options().min_connections, 0);
    assert!(!context.pool.is_closed());
    context.pool.close().await;
    assert!(context.pool.is_closed());
}
