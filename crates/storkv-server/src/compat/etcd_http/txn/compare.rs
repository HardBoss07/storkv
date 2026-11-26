//! Compare predicates for the etcd `/v3/kv/txn` handler.

use bytes::Bytes;
use serde_json::Value;
use storkv_core::{ScopeHandle, StorkvError, Transaction};

use super::{RANGE_END_TO_LAST_KEY, field, field_number, required_key};

pub(super) fn evaluate(
    tx: &mut Transaction,
    scope: &ScopeHandle,
    compares: &[Value],
) -> Result<bool, StorkvError> {
    for spec in compares {
        if !compare_matches(tx, scope, spec)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn compare_matches(
    tx: &mut Transaction,
    scope: &ScopeHandle,
    spec: &Value,
) -> Result<bool, StorkvError> {
    let key = required_key(spec)?;
    let target = string_field(spec, "target").unwrap_or("VALUE");
    let result = string_field(spec, "result").unwrap_or("EQUAL");
    let range_end = field(spec, "range_end").unwrap_or_default();
    let keys = if range_end.is_empty() {
        vec![Bytes::from(key)]
    } else {
        let end = (range_end != RANGE_END_TO_LAST_KEY).then_some(range_end.as_slice());
        let found: Vec<Bytes> = tx
            .scan(scope, Some(key.as_slice()), end)?
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        if found.is_empty() {
            vec![Bytes::from(key)]
        } else {
            found
        }
    };

    for key in keys {
        let matched = match target {
            "VALUE" => {
                let expected = field(spec, "value").unwrap_or_default();
                match tx.get(scope, &key)? {
                    Some(actual) => match result {
                        "EQUAL" => actual.as_ref() == expected.as_slice(),
                        "GREATER" => actual.as_ref() > expected.as_slice(),
                        "LESS" => actual.as_ref() < expected.as_slice(),
                        _ => return Err(invalid("unknown compare result")),
                    },
                    None => false,
                }
            }
            "CREATE" | "VERSION" | "MOD" => {
                let current = tx.key_meta(scope, &key)?.map(|meta| meta.seq).unwrap_or(0);
                let expected = field_number(spec, compare_field(target)).unwrap_or(0);
                compare_numbers(current, expected, result)?
            }
            _ => return Err(invalid("unknown compare target")),
        };
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
}

fn compare_field(target: &str) -> &'static str {
    match target {
        "CREATE" => "create_revision",
        "MOD" => "mod_revision",
        _ => "version",
    }
}

fn compare_numbers(current: u64, expected: u64, result: &str) -> Result<bool, StorkvError> {
    match result {
        "EQUAL" => Ok(current == expected),
        "GREATER" => Ok(current > expected),
        "LESS" => Ok(current < expected),
        _ => Err(invalid("unknown compare result")),
    }
}

fn string_field<'a>(spec: &'a Value, name: &str) -> Option<&'a str> {
    spec.get(name).and_then(Value::as_str)
}

fn invalid(message: &str) -> StorkvError {
    StorkvError::InvalidOperation(message.to_string())
}
