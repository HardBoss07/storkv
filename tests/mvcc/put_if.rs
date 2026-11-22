use std::time::Duration;

use storkv_core::{PutCondition, PutIfResult, ScopeOptions, StorkvError};

use crate::common;

fn applied(result: PutIfResult) -> u64 {
    match result {
        PutIfResult::Applied { seq } => seq,
        other => panic!("expected applied, got {other:?}"),
    }
}

fn conflict_seq(result: PutIfResult) -> Option<u64> {
    match result {
        PutIfResult::Conflict { current_seq, .. } => current_seq,
        other => panic!("expected conflict, got {other:?}"),
    }
}

#[test]
fn absent_and_present_conditions() {
    let temp = common::TempDb::new("put_if_absent_present");
    let db = temp.db();
    let scope = db
        .create_scope("scope", ScopeOptions::default())
        .expect("create scope");

    assert_eq!(
        applied(
            db.put_if(&scope, "k", &PutCondition::Absent, Some("v1"), None)
                .expect("absent write"),
        ),
        1
    );
    assert_eq!(
        conflict_seq(
            db.put_if(&scope, "k", &PutCondition::Absent, Some("v2"), None)
                .expect("absent conflict"),
        ),
        Some(1)
    );
    assert_eq!(
        applied(
            db.put_if(&scope, "k", &PutCondition::Present, Some("v3"), None)
                .expect("present write"),
        ),
        2
    );
    assert_eq!(
        conflict_seq(
            db.put_if(&scope, "missing", &PutCondition::Present, Some("v"), None)
                .expect("present conflict")
        ),
        None
    );
    assert_eq!(db.get(&scope, "k").expect("get"), Some("v3".into()));
}

#[test]
fn expected_seq_and_value_conditions() {
    let temp = common::TempDb::new("put_if_expected");
    let db = temp.db();
    let scope = db
        .create_scope("scope", ScopeOptions::default())
        .expect("create scope");

    let first = applied(
        db.put_if(&scope, "k", &PutCondition::Absent, Some("v1"), None)
            .expect("first write"),
    );
    let second = applied(
        db.put_if(
            &scope,
            "k",
            &PutCondition::ExpectedSeq(first),
            Some("v2"),
            None,
        )
        .expect("sequence write"),
    );
    assert_eq!(second, first + 1);
    assert_eq!(
        conflict_seq(
            db.put_if(
                &scope,
                "k",
                &PutCondition::ExpectedSeq(first),
                Some("stale"),
                None,
            )
            .expect("stale sequence"),
        ),
        Some(second)
    );
    assert_eq!(
        applied(
            db.put_if(
                &scope,
                "k",
                &PutCondition::ExpectedValue("v2".into()),
                Some("v3"),
                None,
            )
            .expect("value write"),
        ),
        second + 1
    );
    assert_eq!(
        conflict_seq(
            db.put_if(
                &scope,
                "k",
                &PutCondition::ExpectedValue("wrong".into()),
                Some("v4"),
                None,
            )
            .expect("value conflict"),
        ),
        Some(second + 1)
    );
}

#[test]
fn expired_key_is_treated_as_absent() {
    let temp = common::TempDb::new("put_if_expiry");
    let db = temp.db();
    let scope = db
        .create_scope("scope", ScopeOptions::default())
        .expect("create scope");

    db.put_with_ttl(&scope, "k", "old", Duration::from_millis(5))
        .expect("ttl write");
    std::thread::sleep(Duration::from_millis(30));

    assert_eq!(
        conflict_seq(
            db.put_if(&scope, "k", &PutCondition::Present, Some("new"), None)
                .expect("present conflict"),
        ),
        None
    );
    applied(
        db.put_if(&scope, "k", &PutCondition::Absent, Some("new"), None)
            .expect("absent write"),
    );
}

#[test]
fn ttl_guardrails_and_watch_emission() {
    let temp = common::TempDb::new("put_if_guardrails");
    let db = temp.db();
    let scope = db
        .create_scope(
            "scope",
            ScopeOptions {
                max_ttl: Some(Duration::from_millis(10)),
                ..ScopeOptions::default()
            },
        )
        .expect("create scope");

    let error = db
        .put_if(
            &scope,
            "k",
            &PutCondition::Absent,
            Some("v"),
            Some(Duration::from_secs(1)),
        )
        .expect_err("ttl above maximum");
    assert!(matches!(error, StorkvError::InvalidTtl(_)));

    let receiver = db.watch().expect("watch");
    let seq = applied(
        db.put_if(&scope, "k", &PutCondition::Absent, Some("v"), None)
            .expect("absent write"),
    );
    let event = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("watch event");
    match event {
        storkv_core::watch::ChangeEvent::Put { seq: event_seq, .. } => {
            assert_eq!(event_seq, seq);
        }
        other => panic!("expected put event, got {other:?}"),
    }
}
