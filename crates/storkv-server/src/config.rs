use std::path::PathBuf;

/// Configuration for a [`crate::Server`] instance and its optional transports.
///
/// Native TCP/UDS listeners are started explicitly through `run_*_listener`.
/// The remaining fields record the addresses configured for the auxiliary
/// transports so callers (for example `storkvd`) can start them consistently.
pub struct ServerConfig {
    /// Directory that holds the database files.
    pub data_dir: PathBuf,
    /// Native TCP listen address, when configured.
    pub listen_addr: Option<String>,
    /// Native admin listen address, when configured.
    pub admin_addr: Option<String>,
    /// Reject admin-only requests on non-admin connections.
    pub strict_admin: bool,
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
        }
    }
}
