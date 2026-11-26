use std::path::PathBuf;

mod file;
mod scopes;

pub use file::{ConfigError, FileConfig, validate_listen_address};
pub use scopes::ScopeConfig;

/// Replication role of one server node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplicationRole {
    /// Serve reads and writes from the local database.
    #[default]
    Standalone,
    /// Serve reads and writes, and stream WAL frames to replicas.
    Primary,
    /// Apply WAL frames from a primary and reject writes.
    Replica,
}

impl ReplicationRole {
    /// Parse a role name from the command line.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is not `standalone`, `primary`, or
    /// `replica`.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "standalone" => Ok(ReplicationRole::Standalone),
            "primary" => Ok(ReplicationRole::Primary),
            "replica" => Ok(ReplicationRole::Replica),
            other => Err(format!(
                "Unknown role `{other}`; expected standalone, primary, or replica"
            )),
        }
    }
}

/// Configuration for a [`crate::Server`] instance and its optional transports.
///
/// Native TCP/UDS listeners are started explicitly through `run_*_listener`.
/// The remaining fields record the addresses configured for the auxiliary
/// transports so callers (for example `storkvd`) can start them consistently.
pub struct ServerConfig {
    /// Reject admin-only requests on non-admin connections.
    pub strict_admin: bool,
    /// Directory that holds the database files.
    pub data_dir: PathBuf,
    /// Native TCP listen address, when configured.
    pub listen_addr: Option<String>,
    /// Native admin listen address, when configured.
    pub admin_addr: Option<String>,
    /// Optional shared token accepted as `x-storkv-admin-token` or a bearer token.
    pub admin_token: Option<String>,
    /// HTTP listen address, when configured.
    pub http_listen: Option<String>,
    /// WebSocket listen address, when configured.
    pub ws_listen: Option<String>,
    /// gRPC listen address, when configured.
    pub grpc_listen: Option<String>,
    /// Redis-compatible listen address, when configured.
    pub redis_listen: Option<String>,
    /// Memcached-compatible listen address, when configured.
    pub memcached_listen: Option<String>,
    /// Replication role of this node.
    pub role: ReplicationRole,
    /// Primary URI that a replica streams from, when configured.
    pub primary_uri: Option<String>,
}

impl ServerConfig {
    /// Build a configuration with every optional address unset.
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            listen_addr: None,
            admin_addr: None,
            strict_admin: false,
            admin_token: None,
            http_listen: None,
            ws_listen: None,
            grpc_listen: None,
            redis_listen: None,
            memcached_listen: None,
            role: ReplicationRole::Standalone,
            primary_uri: None,
        }
    }

    /// Set the replication role.
    pub fn with_role(mut self, role: ReplicationRole) -> Self {
        self.role = role;
        self
    }

    /// Set the primary URI that a replica streams from.
    pub fn with_primary_uri(mut self, primary_uri: Option<String>) -> Self {
        self.primary_uri = primary_uri;
        self
    }
}
