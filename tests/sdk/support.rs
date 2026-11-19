use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use storkv_sdk::{StorkvPool, StorkvPoolOptions};
use storkv_server::Server;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(prefix: &str) -> Self {
        let count = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "storkv_sdk_test_{}_{}_{}",
            prefix,
            std::process::id(),
            count
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub struct TestContext {
    pub pool: StorkvPool,
    pub uri: String,
    _server: Option<Arc<Server>>,
    _handles: Vec<JoinHandle<()>>,
    _temp: Option<crate::common::TempDb>,
    _dir: Option<TempDir>,
}

impl TestContext {
    pub async fn embedded(prefix: &str, options: StorkvPoolOptions) -> Self {
        let dir = TempDir::new(prefix);
        let uri = format!("file://{}", dir.path().display());
        let pool = options
            .connect(&uri)
            .await
            .unwrap_or_else(|error| panic!("connect embedded: {error}"));
        Self {
            pool,
            uri,
            _server: None,
            _handles: Vec::new(),
            _temp: None,
            _dir: Some(dir),
        }
    }

    pub async fn networked(prefix: &str, options: StorkvPoolOptions) -> Self {
        let mut temp = crate::common::TempDb::new(prefix);
        let db = temp.db.take().expect("test db");
        let server = Arc::new(Server::with_db(Arc::new(db)));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address").to_string();
        let serving = server.clone();
        let handle = tokio::spawn(async move {
            let _ = serving.run_tcp_listener(listener, true).await;
        });
        let uri = format!("storkv://{address}");
        let pool = options
            .connect(&uri)
            .await
            .unwrap_or_else(|error| panic!("connect networked: {error}"));
        Self {
            pool,
            uri,
            _server: Some(server),
            _handles: vec![handle],
            _temp: Some(temp),
            _dir: None,
        }
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        if let Some(server) = &self._server {
            let _ = server.shutdown_handle().send(());
        }
        for handle in &self._handles {
            handle.abort();
        }
    }
}

pub async fn contexts(prefix: &str) -> Vec<TestContext> {
    vec![
        TestContext::embedded(prefix, StorkvPoolOptions::default()).await,
        TestContext::networked(prefix, StorkvPoolOptions::default()).await,
    ]
}
