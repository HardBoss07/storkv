use crate::common;
use std::sync::Arc;
use storkv_core::ScopeOptions;

#[test]
fn test_concurrent_scope_creation_across_16_threads() {
    let mut temp = common::TempDb::new("scope_concurrent_create");
    let db = Arc::new(temp.db.take().unwrap());

    let threads = 16;
    let scopes_per_thread = 4;
    let mut handles = Vec::new();
    for thread in 0..threads {
        let db = db.clone();
        handles.push(std::thread::spawn(move || {
            for index in 0..scopes_per_thread {
                let name = format!("concurrent_{:02}_{}", thread, index);
                let scope = db.create_scope(name, ScopeOptions::default()).unwrap();
                db.put(&scope, b"k", b"v").unwrap();
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(
        db.inner().scope_names.read().len(),
        threads * scopes_per_thread + 1
    );
}
