//! Native server state and configuration.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use storkv_core::{Db, DbOptions, StorkvError};
use tokio::sync::broadcast;

use crate::ReplicationRole;
use crate::metrics::ServerStats;

mod consistency;
mod listener;
mod session;

pub(crate) use consistency::ConnectionConsistency;

const READ_BUFFER_CAPACITY: usize = 8192;
const WATCH_CHANNEL_CAPACITY: usize = 128;

/// One network server bound to a single database.
pub struct Server {
    pub(crate) role: ReplicationRole,
    pub(crate) consistency: AtomicU8,
    pub(crate) strict_admin: AtomicBool,
    pub(crate) consistency_max_lag_ms: AtomicU64,
    pub(crate) last_applied_seq: AtomicU64,
    pub(crate) last_frame_sent_at_ms: AtomicU64,
    pub(crate) primary_uri: Option<String>,
    pub(crate) db: Arc<Db>,
    pub(crate) shutdown_sender: broadcast::Sender<()>,
    pub(crate) stats: Arc<ServerStats>,
}

impl Server {
    /// Open the database at `data_dir` and wrap it in a server.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be opened.
    pub fn new(data_dir: PathBuf) -> Result<Self, StorkvError> {
        let db = Db::open(&data_dir, DbOptions::default())?;
        Ok(Self::with_db(Arc::new(db)))
    }

    /// Wrap an existing database handle in a server.
    pub fn with_db(db: Arc<Db>) -> Self {
        let (shutdown_sender, _) = broadcast::channel(1);
        Self {
            role: ReplicationRole::Standalone,
            consistency: AtomicU8::new(consistency_code(storkv_proto::Consistency::Strong)),
            strict_admin: AtomicBool::new(false),
            consistency_max_lag_ms: AtomicU64::new(0),
            last_applied_seq: AtomicU64::new(db.newest_sequence()),
            last_frame_sent_at_ms: AtomicU64::new(0),
            primary_uri: None,
            db,
            shutdown_sender,
            stats: Arc::new(ServerStats::default()),
        }
    }

    /// Set the replication role.
    pub fn with_role(mut self, role: ReplicationRole) -> Self {
        self.role = role;
        if role == ReplicationRole::Replica {
            self.set_consistency(storkv_proto::Consistency::Eventual);
        }
        self
    }

    /// Set the primary URI that a replica streams from.
    pub fn with_primary_uri(mut self, primary_uri: Option<String>) -> Self {
        self.primary_uri = primary_uri;
        self
    }

    /// Return the configured replication role.
    pub fn role(&self) -> ReplicationRole {
        self.role
    }

    /// Return the configured primary URI, when set.
    pub fn primary_uri(&self) -> Option<&str> {
        self.primary_uri.as_deref()
    }

    /// Set the consistency level that later requests observe.
    pub fn set_consistency(&self, consistency: storkv_proto::Consistency) {
        let (mode, max_lag_ms) = consistency.to_wire();
        self.consistency.store(mode, Ordering::Relaxed);
        self.consistency_max_lag_ms
            .store(max_lag_ms, Ordering::Relaxed);
    }

    /// Return the current consistency level.
    pub fn consistency(&self) -> storkv_proto::Consistency {
        let mode = self.consistency.load(Ordering::Relaxed);
        let max_lag_ms = self.consistency_max_lag_ms.load(Ordering::Relaxed);
        storkv_proto::Consistency::from_wire(mode, max_lag_ms)
            .unwrap_or(storkv_proto::Consistency::Strong)
    }

    /// Return the newest sequence number that the replica applied.
    pub fn last_applied_seq(&self) -> u64 {
        self.last_applied_seq.load(Ordering::Relaxed)
    }

    /// Return the current replication lag in milliseconds.
    ///
    /// The lag is the age of the most recent frame from the primary. A node
    /// that never received a frame reports zero lag.
    pub fn replication_lag_ms(&self) -> u64 {
        let sent_at_ms = self.last_frame_sent_at_ms.load(Ordering::Relaxed);
        if sent_at_ms == 0 {
            return 0;
        }
        now_millis().saturating_sub(sent_at_ms)
    }

    /// Builder-style strict-admin toggle. When enabled, admin-only requests are
    /// rejected with [`storkv_proto::StorkvWireError::Unauthorized`] unless the
    /// connection was accepted on an admin listener or presented a valid token.
    pub fn with_strict_admin(self, strict_admin: bool) -> Self {
        self.strict_admin.store(strict_admin, Ordering::Relaxed);
        self
    }

    /// Change the strict-admin flag after construction.
    pub fn set_strict_admin(&self, strict_admin: bool) {
        self.strict_admin.store(strict_admin, Ordering::Relaxed);
    }

    /// Return the current strict-admin flag.
    pub fn strict_admin(&self) -> bool {
        self.strict_admin.load(Ordering::Relaxed)
    }

    /// Return the shared database handle.
    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    /// Return a sender that stops every listener when used.
    pub fn shutdown_handle(&self) -> broadcast::Sender<()> {
        self.shutdown_sender.clone()
    }
}

fn consistency_code(consistency: storkv_proto::Consistency) -> u8 {
    consistency.to_wire().0
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
