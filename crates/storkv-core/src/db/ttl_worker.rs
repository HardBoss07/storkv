//! Background worker for active TTL expiration.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;

use crate::db::DbInner;
use crate::vfs::FileSystem;

/// Background expiration runner that wakes periodically and expires due keys.
pub struct TtlWorkerHandle {
    shutdown: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl TtlWorkerHandle {
    /// Start a worker loop running against the engine inner state.
    pub fn start<FS: FileSystem + 'static>(inner: Arc<DbInner<FS>>) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = shutdown.clone();
        let worker = std::thread::Builder::new()
            .name("storkv-ttl".into())
            .spawn(move || run_ttl_worker(inner, flag))
            .ok();
        Self {
            shutdown,
            worker: Mutex::new(worker),
        }
    }

    /// Signal the worker to stop and wait for it to finish.
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker.lock().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for TtlWorkerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_ttl_worker<FS: FileSystem + 'static>(inner: Arc<DbInner<FS>>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::SeqCst) {
        let _ = inner.expire_due_keys();
        std::thread::sleep(Duration::from_millis(25));
    }
}
