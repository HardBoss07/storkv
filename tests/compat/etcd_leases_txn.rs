use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::support::{CompatServer, http_post, spawn};

fn encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

async fn post(server: &CompatServer, path: &str, body: Value) -> (u16, Value) {
    http_post(server, path, &body.to_string()).await
}

async fn grant(server: &CompatServer, ttl: u64) -> String {
    let (status, reply) = post(server, "/v3/lease/grant", json!({"TTL": ttl})).await;
    assert_eq!(status, 200, "grant reply: {reply}");
    reply["ID"].as_str().expect("lease ID").to_string()
}

#[tokio::test]
async fn etcd_lease_lifecycle() {
    let server = spawn("compat_etcd_lease").await;
    let id = grant(&server, 60).await;
    assert!(!id.is_empty());

    let (status, _) = post(
        &server,
        "/v3/kv/put",
        json!({"key": encode(b"alpha"), "value": encode(b"one"), "lease": id}),
    )
    .await;
    assert_eq!(status, 200);

    let (status, reply) = post(&server, "/v3/kv/range", json!({"key": encode(b"alpha")})).await;
    assert_eq!(status, 200);
    assert_eq!(reply["kvs"][0]["lease"], id.as_str());

    let (status, reply) = post(
        &server,
        "/v3/lease/timetolive",
        json!({"ID": id, "keys": true}),
    )
    .await;
    assert_eq!(status, 200, "timetolive reply: {reply}");
    assert_eq!(reply["grantedTTL"], "60");
    assert_eq!(reply["keys"], json!([encode(b"alpha")]));

    let (status, reply) = post(&server, "/v3/lease/keepalive", json!({"ID": id})).await;
    assert_eq!(status, 200, "keepalive reply: {reply}");
    assert_eq!(reply["TTL"], "60");

    let (status, reply) = post(
        &server,
        "/v3/kv/put",
        json!({"key": encode(b"beta"), "value": encode(b"two")}),
    )
    .await;
    assert_eq!(status, 200);
    assert!(reply["header"]["revision"].is_string());

    let (status, _) = post(&server, "/v3/lease/revoke", json!({"ID": id})).await;
    assert_eq!(status, 200);

    let (status, reply) = post(&server, "/v3/kv/range", json!({"key": encode(b"alpha")})).await;
    assert_eq!(status, 200);
    assert_eq!(reply["count"], "0");

    let (_, reply) = post(&server, "/v3/kv/range", json!({"key": encode(b"beta")})).await;
    assert_eq!(reply["count"], "1");

    let (status, reply) = post(&server, "/v3/lease/timetolive", json!({"ID": id})).await;
    assert_eq!(status, 404);
    assert_eq!(reply["code"], 5);
}

#[tokio::test]
async fn etcd_lease_errors() {
    let server = spawn("compat_etcd_lease_errors").await;

    let (status, reply) = post(&server, "/v3/lease/grant", json!({"TTL": 0})).await;
    assert_eq!(status, 400);
    assert_eq!(reply["code"], 3);

    let (status, reply) = post(
        &server,
        "/v3/kv/put",
        json!({"key": encode(b"alpha"), "value": encode(b"one"), "lease": "999"}),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(reply["code"], 3);

    for path in ["/v3/lease/keepalive", "/v3/lease/revoke"] {
        let (status, reply) = post(&server, path, json!({"ID": "999"})).await;
        assert_eq!(status, 404, "{path} reply: {reply}");
        assert_eq!(reply["code"], 5);
    }

    let (status, reply) = post(&server, "/v3/lease/timetolive", json!({})).await;
    assert_eq!(status, 400);
    assert_eq!(reply["code"], 3);
}

#[tokio::test]
async fn etcd_lease_expires_attached_keys() {
    let server = spawn("compat_etcd_lease_expiry").await;
    let id = grant(&server, 1).await;
    let (status, _) = post(
        &server,
        "/v3/kv/put",
        json!({"key": encode(b"short"), "value": encode(b"lived"), "lease": id}),
    )
    .await;
    assert_eq!(status, 200);

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let (status, reply) = post(&server, "/v3/kv/range", json!({"key": encode(b"short")})).await;
    assert_eq!(status, 200);
    assert_eq!(reply["count"], "0");

    let (status, _) = post(&server, "/v3/lease/timetolive", json!({"ID": id})).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn etcd_txn_compares_and_branches() {
    let server = spawn("compat_etcd_txn").await;
    let (status, _) = post(
        &server,
        "/v3/kv/put",
        json!({"key": encode(b"alpha"), "value": encode(b"one")}),
    )
    .await;
    assert_eq!(status, 200);
    let (_, reply) = post(&server, "/v3/kv/range", json!({"key": encode(b"alpha")})).await;
    let version = reply["kvs"][0]["version"]
        .as_str()
        .expect("version")
        .to_string();

    let txn = json!({
        "compare": [{"target": "VERSION", "result": "EQUAL", "key": encode(b"alpha"), "version": version}],
        "success": [{"request_put": {"key": encode(b"alpha"), "value": encode(b"two")}}],
        "failure": [{"request_put": {"key": encode(b"alpha"), "value": encode(b"three")}}],
    });
    let (status, reply) = post(&server, "/v3/kv/txn", txn.clone()).await;
    assert_eq!(status, 200, "txn reply: {reply}");
    assert_eq!(reply["succeeded"], true);
    assert!(reply["responses"][0]["response_put"].is_object());

    let (_, reply) = post(&server, "/v3/kv/range", json!({"key": encode(b"alpha")})).await;
    assert_eq!(reply["kvs"][0]["value"], encode(b"two"));

    let (status, reply) = post(&server, "/v3/kv/txn", txn).await;
    assert_eq!(status, 200);
    assert_eq!(reply["succeeded"], false);
    let (_, reply) = post(&server, "/v3/kv/range", json!({"key": encode(b"alpha")})).await;
    assert_eq!(reply["kvs"][0]["value"], encode(b"three"));

    let create = json!({
        "compare": [{"target": "CREATE", "result": "EQUAL", "key": encode(b"gamma"), "create_revision": "0"}],
        "success": [{"request_put": {"key": encode(b"gamma"), "value": encode(b"new")}}],
    });
    let (status, reply) = post(&server, "/v3/kv/txn", create.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(reply["succeeded"], true);
    let (_, reply) = post(&server, "/v3/kv/txn", create).await;
    assert_eq!(reply["succeeded"], false);

    let value_compare = json!({
        "compare": [{"target": "VALUE", "result": "EQUAL", "key": encode(b"alpha"), "value": encode(b"three")}],
        "success": [{"request_delete_range": {"key": encode(b"alpha")}}],
    });
    let (status, reply) = post(&server, "/v3/kv/txn", value_compare).await;
    assert_eq!(status, 200);
    assert_eq!(reply["succeeded"], true);
    assert_eq!(
        reply["responses"][0]["response_delete_range"]["deleted"],
        "1"
    );
}

#[tokio::test]
async fn etcd_txn_range_and_atomic_rollback() {
    let server = spawn("compat_etcd_txn_range").await;
    for (key, value) in [("a", "1"), ("b", "2"), ("c", "3")] {
        let (status, _) = post(
            &server,
            "/v3/kv/put",
            json!({"key": encode(key.as_bytes()), "value": encode(value.as_bytes())}),
        )
        .await;
        assert_eq!(status, 200);
    }

    let txn = json!({
        "compare": [{"target": "VERSION", "result": "GREATER", "key": encode(b"a"), "range_end": encode(b"z"), "version": "0"}],
        "success": [{"request_range": {"key": encode(b"a"), "range_end": encode(b"z"), "limit": "2"}}],
    });
    let (status, reply) = post(&server, "/v3/kv/txn", txn).await;
    assert_eq!(status, 200, "txn reply: {reply}");
    assert_eq!(reply["succeeded"], true);
    assert_eq!(reply["responses"][0]["response_range"]["count"], "2");

    let broken = json!({
        "success": [
            {"request_put": {"key": encode(b"delta"), "value": encode(b"4")}},
            {"request_txn": {}},
        ],
    });
    let (status, reply) = post(&server, "/v3/kv/txn", broken).await;
    assert_eq!(status, 400);
    assert_eq!(reply["code"], 3);
    let (_, reply) = post(&server, "/v3/kv/range", json!({"key": encode(b"delta")})).await;
    assert_eq!(reply["count"], "0");

    let (status, _) = post(&server, "/v3/kv/txn", json!({"compare": [{}]})).await;
    assert_eq!(status, 400);
    let (status, _) = post(
        &server,
        "/v3/kv/txn",
        json!({"compare": [{"target": "NOPE", "key": encode(b"a")}]}),
    )
    .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn etcd_txn_put_with_lease_attaches_key() {
    let server = spawn("compat_etcd_txn_lease").await;
    let id = grant(&server, 30).await;
    let txn = json!({
        "success": [{"request_put": {"key": encode(b"locked"), "value": encode(b"x"), "lease": id}}],
    });
    let (status, reply) = post(&server, "/v3/kv/txn", txn).await;
    assert_eq!(status, 200, "txn reply: {reply}");

    let (_, reply) = post(&server, "/v3/kv/range", json!({"key": encode(b"locked")})).await;
    assert_eq!(reply["kvs"][0]["lease"], id.as_str());

    let (_, reply) = post(
        &server,
        "/v3/lease/timetolive",
        json!({"ID": id, "keys": true}),
    )
    .await;
    assert_eq!(reply["keys"], json!([encode(b"locked")]));
}
