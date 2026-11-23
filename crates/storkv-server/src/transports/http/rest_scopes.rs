//! REST scope lifecycle handlers.

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response as HttpResponse;
use serde_json::Value;
use storkv_proto::{Request, WireCompactionStyle, WireDurability, WireRetention, WireScopeOptions};

use super::HttpState;
use super::json::{json_error, response_payload};

pub(super) async fn create_scope(
    State(state): State<HttpState>,
    headers: HeaderMap,
    body: Bytes,
) -> HttpResponse {
    let Some(options) = parse_scope_body(&body) else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid_body",
            "invalid scope options",
        );
    };
    response_payload(state.dispatch(
        Request::CreateScope {
            name: options.0,
            options: options.1,
        },
        &headers,
    ))
}

pub(super) async fn drop_scope(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(scope): Path<String>,
) -> HttpResponse {
    response_payload(state.dispatch(Request::DropScope { name: scope }, &headers))
}

fn parse_scope_body(body: &[u8]) -> Option<(String, WireScopeOptions)> {
    let parsed = serde_json::from_slice::<Value>(body).ok()?;
    let name = parsed.get("name").and_then(Value::as_str)?.to_owned();
    let durability = parse_durability(parsed.get("durability"))?;
    let retention = parse_retention(parsed.get("retention"))?;
    let prefix_bloom_len = parsed
        .get("prefix_bloom_len")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    Some((
        name,
        WireScopeOptions {
            durability,
            retention,
            prefix_bloom_len,
            min_ttl: ttl_bound(&parsed, "min_ttl_ms")?,
            max_ttl: ttl_bound(&parsed, "max_ttl_ms")?,
            allow_permanent: parsed
                .get("allow_permanent")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            active_ttl_tracking: parsed
                .get("active_ttl_tracking")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
    ))
}

fn ttl_bound(parsed: &Value, key: &str) -> Option<Option<Duration>> {
    match parsed.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(value) => value.as_u64().map(|ms| Some(Duration::from_millis(ms))),
    }
}

fn parse_durability(value: Option<&Value>) -> Option<WireDurability> {
    match value {
        None | Some(Value::Null) => Some(WireDurability::StrictFsync),
        Some(Value::String(mode)) if mode == "strict_fsync" => Some(WireDurability::StrictFsync),
        Some(Value::String(mode)) if mode == "buffered" => Some(WireDurability::Buffered {
            sync_interval: Duration::from_millis(100),
        }),
        Some(Value::Object(map)) => {
            let buffered = map.get("buffered")?;
            Some(WireDurability::Buffered {
                sync_interval: Duration::from_millis(
                    buffered
                        .get("sync_interval_ms")
                        .and_then(Value::as_u64)
                        .unwrap_or(100),
                ),
            })
        }
        _ => None,
    }
}

fn parse_retention(value: Option<&Value>) -> Option<WireRetention> {
    match value {
        None | Some(Value::Null) => Some(WireRetention::Permanent),
        Some(Value::String(mode)) if mode == "permanent" => Some(WireRetention::Permanent),
        Some(Value::Object(map)) => {
            let ttl = map.get("ttl")?;
            let style = match ttl
                .get("compaction_style")
                .and_then(Value::as_str)
                .unwrap_or("leveled")
            {
                "fifo" => WireCompactionStyle::Fifo,
                _ => WireCompactionStyle::Leveled,
            };
            Some(WireRetention::Ttl {
                default_ttl: Duration::from_millis(
                    ttl.get("default_ttl_ms")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                ),
                compaction_style: style,
            })
        }
        _ => None,
    }
}
