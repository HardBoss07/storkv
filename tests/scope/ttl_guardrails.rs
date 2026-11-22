use crate::common;
use std::time::Duration;
use storkv_core::{ScopeOptions, StorkvError};

#[test]
fn test_permanent_writes_can_be_rejected() {
    let temp = common::TempDb::new("ttl_no_permanent");
    let scope = temp
        .db()
        .create_scope(
            "guarded",
            ScopeOptions {
                allow_permanent: false,
                ..ScopeOptions::default()
            },
        )
        .unwrap();

    let error = temp
        .db()
        .put(&scope, b"key", b"value")
        .expect_err("permanent write must fail");
    assert!(matches!(error, StorkvError::InvalidTtl(_)), "{error:?}");

    temp.db()
        .put_with_ttl(&scope, b"key", b"value", Duration::from_secs(1))
        .expect("write with a TTL must succeed");
}

#[test]
fn test_ttl_limits_reject_out_of_bounds_values() {
    let temp = common::TempDb::new("ttl_bounds");
    let scope = temp
        .db()
        .create_scope(
            "bounded",
            ScopeOptions {
                min_ttl: Some(Duration::from_millis(100)),
                max_ttl: Some(Duration::from_secs(1)),
                ..ScopeOptions::default()
            },
        )
        .unwrap();

    let below = temp
        .db()
        .put_with_ttl(&scope, b"key", b"value", Duration::from_millis(10))
        .expect_err("TTL below the minimum must fail");
    assert!(matches!(below, StorkvError::InvalidTtl(_)), "{below:?}");

    let above = temp
        .db()
        .put_with_ttl(&scope, b"key", b"value", Duration::from_secs(2))
        .expect_err("TTL above the maximum must fail");
    assert!(matches!(above, StorkvError::InvalidTtl(_)), "{above:?}");

    temp.db()
        .put_with_ttl(&scope, b"key", b"value", Duration::from_millis(500))
        .expect("TTL inside the bounds must succeed");
}
