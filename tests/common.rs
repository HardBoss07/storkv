#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use storkv_core::{Db, DbOptions};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct TempDb {
    pub path: PathBuf,
    pub db: Option<Db>,
}

impl TempDb {
    pub fn new(prefix: &str) -> Self {
        let count = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "storkv_test_{}_{}_{}",
            prefix,
            std::process::id(),
            count
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("Failed to create temp db dir");

        let db = Db::open(&path, DbOptions::default()).expect("Failed to open test Db");
        Self { path, db: Some(db) }
    }

    pub fn open_with_options(prefix: &str, options: DbOptions) -> Self {
        let count = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "storkv_test_{}_{}_{}",
            prefix,
            std::process::id(),
            count
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("Failed to create temp db dir");

        let db = Db::open(&path, options).expect("Failed to open test Db");
        Self { path, db: Some(db) }
    }

    pub fn db(&self) -> &Db {
        self.db.as_ref().expect("Db is closed")
    }

    pub fn close(&mut self) {
        self.db.take();
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        self.close();
        let _ = fs::remove_dir_all(&self.path);
    }
}
