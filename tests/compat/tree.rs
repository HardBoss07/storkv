use crate::common::TempDb;

#[test]
fn embedded_tree_basic_operations_and_isolation() {
    let temp = TempDb::new("compat_tree_basic");
    let db = temp.db();

    let users = db.open_tree("users").expect("open users tree");
    let users_again = db.open_tree("users").expect("reopen users tree");
    assert_eq!(users.scope_id(), users_again.scope_id());
    assert_eq!(users.name(), "users");

    assert_eq!(users.insert("alice", "one").expect("insert"), None);
    assert_eq!(
        users.insert("alice", "two").expect("replace"),
        Some(bytes("one"))
    );
    assert_eq!(users.get("alice").expect("get"), Some(bytes("two")));
    assert_eq!(users.get("missing").expect("get missing"), None);

    let audit = db.open_tree("audit").expect("open audit tree");
    assert_ne!(users.scope_id(), audit.scope_id());
    assert_eq!(audit.get("alice").expect("isolated get"), None);

    assert_eq!(users.remove("alice").expect("remove"), Some(bytes("two")));
    assert_eq!(users.remove("alice").expect("remove again"), None);
    users.flush().expect("flush tree");
    db.flush().expect("flush db");
}

#[test]
fn embedded_tree_compare_and_swap() {
    let temp = TempDb::new("compat_tree_cas");
    let db = temp.db();
    let tree = db.open_tree("cas").expect("open tree");

    assert!(
        tree.compare_and_swap("key", None, b"first")
            .expect("cas create")
    );
    assert!(
        !tree
            .compare_and_swap("key", None, b"again")
            .expect("cas stale create")
    );
    assert!(
        !tree
            .compare_and_swap("key", Some(b"wrong"), b"second")
            .expect("cas mismatch")
    );
    assert!(
        tree.compare_and_swap("key", Some(b"first"), b"second")
            .expect("cas update")
    );
    assert_eq!(tree.get("key").expect("get"), Some(bytes("second")));
    assert!(
        tree.compare_and_swap("key", Some(b"second"), b"")
            .expect("cas delete")
    );
    assert_eq!(tree.get("key").expect("get deleted"), None);
}

#[test]
fn embedded_tree_prefix_range_batch_and_merge() {
    let temp = TempDb::new("compat_tree_scan");
    let db = temp.db();
    let tree = db.open_tree("scan").expect("open tree");

    tree.insert("aa", "1").expect("insert aa");
    tree.insert("ab", "2").expect("insert ab");
    tree.insert("ba", "3").expect("insert ba");

    let prefix = tree.scan_prefix("a").expect("scan prefix");
    assert_eq!(prefix.len(), 2);
    assert_eq!(prefix[0].0.as_ref(), b"aa");
    assert_eq!(prefix[1].0.as_ref(), b"ab");

    let range = tree.range("a", "b").expect("range");
    assert_eq!(range.len(), 2);
    let range_all = tree.range("a", "zz").expect("range all");
    assert_eq!(range_all.len(), 3);

    let mut batch = tree.batch();
    batch.insert("bb", "22").remove("ba");
    tree.apply_batch(batch).expect("apply batch");
    assert_eq!(tree.get("bb").expect("get bb"), Some(bytes("22")));
    assert_eq!(tree.get("ba").expect("get ba"), None);

    tree.insert("counter", "10").expect("insert counter");
    tree.merge("counter", 5).expect("merge counter");
    assert_eq!(tree.get("counter").expect("get counter"), Some(bytes("15")));
}

fn bytes(value: &str) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(value.as_bytes())
}
