use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::json;

use crate::support::{http_post, spawn};

fn encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

#[tokio::test]
async fn etcd_put_range_and_revisions() {
    let server = spawn("compat_etcd_range").await;

    let (status, reply) = http_post(
        &server,
        "/v3/kv/put",
        &json!({"key": encode(b"alpha"), "value": encode(b"one")}).to_string(),
    )
    .await;
    assert_eq!(status, 200);
    assert!(reply["header"]["revision"].is_string());

    for (key, value) in [("beta", "two"), ("gamma", "three")] {
        let (status, _) = http_post(
            &server,
            "/v3/kv/put",
            &json!({"key": encode(key.as_bytes()), "value": encode(value.as_bytes())}).to_string(),
        )
        .await;
        assert_eq!(status, 200);
    }

    let (status, reply) = http_post(
        &server,
        "/v3/kv/range",
        &json!({"key": encode(b"alpha")}).to_string(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(reply["count"], "1");
    assert_eq!(reply["kvs"][0]["key"], encode(b"alpha"));
    assert_eq!(reply["kvs"][0]["value"], encode(b"one"));

    let (status, reply) = http_post(
        &server,
        "/v3/kv/range",
        &json!({"key": encode(b"alpha"), "range_end": encode(b"z")}).to_string(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(reply["count"], "3");
    assert_eq!(reply["kvs"].as_array().map(Vec::len), Some(3));

    let (status, reply) = http_post(
        &server,
        "/v3/kv/range",
        &json!({
            "key": encode(b"alpha"),
            "range_end": encode(b"z"),
            "count_only": true,
        })
        .to_string(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(reply["count"], "3");
    assert_eq!(reply["kvs"].as_array().map(Vec::len), Some(0));

    let (status, reply) = http_post(
        &server,
        "/v3/kv/range",
        &json!({
            "key": encode(b"alpha"),
            "range_end": encode(b"z"),
            "keys_only": true,
        })
        .to_string(),
    )
    .await;
    assert_eq!(status, 200);
    let kvs = reply["kvs"].as_array().expect("kvs array");
    assert!(kvs.iter().all(|entry| entry.get("value").is_none()));
}

#[tokio::test]
async fn etcd_delete_range_and_errors() {
    let server = spawn("compat_etcd_delete").await;

    for key in [b"a".as_slice(), b"b", b"c", b"d"] {
        let (status, _) = http_post(
            &server,
            "/v3/kv/put",
            &json!({"key": encode(key), "value": encode(key)}).to_string(),
        )
        .await;
        assert_eq!(status, 200);
    }

    let (status, reply) = http_post(
        &server,
        "/v3/kv/deleterange",
        &json!({"key": encode(b"b"), "range_end": encode(b"d")}).to_string(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(reply["deleted"], "2");

    let (status, reply) = http_post(
        &server,
        "/v3/kv/range",
        &json!({"key": encode(b"a"), "range_end": encode(&[0])}).to_string(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(reply["count"], "2");

    let (status, _) = http_post(
        &server,
        "/v3/kv/put",
        &json!({"key": encode(b"x"), "value": encode(b"y"), "lease": 7}).to_string(),
    )
    .await;
    assert_eq!(status, 400);

    let (status, _) = http_post(&server, "/v3/kv/put", &json!({"value": "eA=="}).to_string()).await;
    assert_eq!(status, 400);

    let (status, _) = http_post(&server, "/v3/kv/range", &json!({}).to_string()).await;
    assert_eq!(status, 400);
}
