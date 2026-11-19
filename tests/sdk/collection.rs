use std::time::Duration;

use serde::{Deserialize, Serialize};
use storkv_sdk::ScopeDefinition;

use crate::support;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct User {
    age: u32,
    name: String,
}

fn user(name: &str, age: u32) -> User {
    User {
        name: name.to_string(),
        age,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_collection_round_trip() {
    for context in support::contexts("typed").await {
        let pool = &context.pool;
        pool.provision_scopes(&[ScopeDefinition::new("app")])
            .await
            .unwrap();
        let users = pool.scope("app").collection::<User>("user:");

        users.put("1", &user("alice", 30)).await.unwrap();
        users.put("2", &user("bob", 41)).await.unwrap();
        users.put("3", &user("carol", 25)).await.unwrap();

        assert_eq!(users.get("1").await.unwrap(), Some(user("alice", 30)));
        assert!(users.exists("2").await.unwrap());

        let mut scanned = users.scan().await.unwrap();
        scanned.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(scanned.len(), 3);
        assert_eq!(scanned[0].0.as_ref(), b"1");
        assert_eq!(scanned[0].1, user("alice", 30));

        let ranged = users
            .scan_range(Some(&b"2"[..]), Some(&b"3"[..]))
            .await
            .unwrap();
        assert_eq!(ranged.len(), 1);
        assert_eq!(ranged[0].1, user("bob", 41));

        assert_eq!(
            users
                .get_and_touch("1", Duration::from_secs(30))
                .await
                .unwrap(),
            Some(user("alice", 30))
        );

        users.delete("2").await.unwrap();
        assert!(users.get("2").await.unwrap().is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_collection_ttl_expires() {
    for context in support::contexts("typed_ttl").await {
        let pool = &context.pool;
        pool.provision_scopes(&[ScopeDefinition::new("app")])
            .await
            .unwrap();
        let cache = pool.scope("app").collection::<User>("cache:");
        cache
            .put_with_ttl("session", &user("temp", 1), Duration::from_millis(150))
            .await
            .unwrap();
        assert_eq!(cache.get("session").await.unwrap(), Some(user("temp", 1)));
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert!(cache.get("session").await.unwrap().is_none());
    }
}
