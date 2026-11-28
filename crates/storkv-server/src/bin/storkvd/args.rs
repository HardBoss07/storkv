//! Command-line arguments for the `storkvd` daemon.

use std::path::PathBuf;

use clap::Parser;

/// Command-line settings for `storkvd`.
#[derive(Parser, Debug)]
#[command(name = "storkvd", about = "StorKV daemon")]
pub(super) struct Args {
    /// Load settings from this TOML file.
    #[arg(long, env = "STORKV_CONFIG", value_name = "PATH")]
    pub(super) config: Option<PathBuf>,
    /// Ship sealed WAL segments continuously (point-in-time recovery).
    #[arg(
        long,
        env = "STORKV_BACKUP_SHIP_WAL",
        value_parser = clap::builder::FalseyValueParser::new()
    )]
    pub(super) backup_ship_wal: bool,
    /// Restrict admin-only requests to admin listeners or token holders.
    #[arg(
        long,
        env = "STORKV_STRICT_ADMIN",
        value_parser = clap::builder::FalseyValueParser::new()
    )]
    pub(super) strict_admin: bool,
    /// Treat every HTTP connection as admin (dedicated admin listener).
    #[arg(
        long,
        env = "STORKV_HTTP_ADMIN",
        value_parser = clap::builder::FalseyValueParser::new()
    )]
    pub(super) http_admin: bool,
    /// Treat every WebSocket connection as admin.
    #[arg(
        long,
        env = "STORKV_WS_ADMIN",
        value_parser = clap::builder::FalseyValueParser::new()
    )]
    pub(super) ws_admin: bool,
    /// Treat every gRPC connection as admin.
    #[arg(
        long,
        env = "STORKV_GRPC_ADMIN",
        value_parser = clap::builder::FalseyValueParser::new()
    )]
    pub(super) grpc_admin: bool,
    /// Seconds between automatic full backups.
    #[arg(long, env = "STORKV_BACKUP_INTERVAL_SECS", default_value_t = 3600)]
    pub(super) backup_interval_secs: u64,
    /// Number of full generations to retain.
    #[arg(long, env = "STORKV_BACKUP_RETENTION", default_value_t = 7)]
    pub(super) backup_retention: usize,
    #[arg(long, env = "STORKV_DATA_DIR", default_value = "./data/storkv")]
    pub(super) data_dir: PathBuf,
    #[arg(long, env = "STORKV_LISTEN", default_value = "127.0.0.1:7878")]
    pub(super) listen: String,
    #[arg(long, env = "STORKV_ADMIN_SOCK")]
    pub(super) admin_sock: Option<String>,
    /// Enable autonomous continuous backup into this directory.
    #[arg(long, env = "STORKV_BACKUP_DIR")]
    pub(super) backup_dir: Option<PathBuf>,
    /// Stream backup archives into this program instead of writing locally.
    #[arg(long, env = "STORKV_BACKUP_PIPE_CMD")]
    pub(super) backup_pipe_cmd: Option<String>,
    /// Repeatable argument for --backup-pipe-cmd.
    #[arg(
        long = "backup-pipe-arg",
        env = "STORKV_BACKUP_PIPE_ARGS",
        value_delimiter = ','
    )]
    pub(super) backup_pipe_args: Vec<String>,
    /// Archive compression for the pipe sink: none, zstd, lz4.
    #[arg(long, env = "STORKV_BACKUP_COMPRESSION", default_value = "none")]
    pub(super) backup_compression: String,
    /// Shared secret granting admin privileges on auxiliary transports.
    #[arg(long, env = "STORKV_ADMIN_TOKEN")]
    pub(super) admin_token: Option<String>,
    /// Read the admin token from this file. The file must contain one line.
    #[arg(long, env = "STORKV_ADMIN_TOKEN_FILE", value_name = "PATH")]
    pub(super) admin_token_file: Option<PathBuf>,
    /// Serve the HTTP/1.1 + HTTP/2 REST API on this address.
    #[arg(long, env = "STORKV_HTTP_LISTEN")]
    pub(super) http_listen: Option<String>,
    /// Serve the WebSocket multiplexed API on this address.
    #[arg(long, env = "STORKV_WS_LISTEN")]
    pub(super) ws_listen: Option<String>,
    /// Serve the gRPC API on this address.
    #[arg(long, env = "STORKV_GRPC_LISTEN")]
    pub(super) grpc_listen: Option<String>,
    /// Serve the Redis RESP2/RESP3 API on this address.
    #[arg(long, env = "STORKV_REDIS_LISTEN")]
    pub(super) redis_listen: Option<String>,
    /// Password for the Redis RESP listener. Falls back to --admin-token.
    #[arg(long, env = "STORKV_REDIS_PASSWORD")]
    pub(super) redis_password: Option<String>,
    /// Serve the Memcached ASCII API on this address.
    #[arg(long, env = "STORKV_MEMCACHED_LISTEN")]
    pub(super) memcached_listen: Option<String>,
    /// PEM server certificate that enables TLS on the native listener.
    #[arg(long, env = "STORKV_TLS_CERT", requires = "tls_key")]
    pub(super) tls_cert: Option<PathBuf>,
    /// PEM private key that enables TLS on the native listener.
    #[arg(long, env = "STORKV_TLS_KEY", requires = "tls_cert")]
    pub(super) tls_key: Option<PathBuf>,
    /// Replication role: standalone, primary, or replica.
    #[arg(long, env = "STORKV_ROLE", default_value = "standalone")]
    pub(super) role: String,
    /// Primary URI that a replica streams WAL frames from.
    #[arg(long, env = "STORKV_PRIMARY_URI")]
    pub(super) primary_uri: Option<String>,
}
