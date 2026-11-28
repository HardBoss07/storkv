//! `storkv.toml` file model and command-line overlay.
//!
//! Loading rejects unknown keys and validates every listen address before a
//! value reaches the daemon.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::Command;
use serde::Deserialize;
use storkv_core::ScopeOptions;

mod error;

pub use error::{ConfigError, validate_listen_address};

use super::scopes::{self, ScopeConfig};

/// Daemon settings read from a `storkv.toml` file.
///
/// Every field is optional. An unset field leaves the built-in default or a
/// value from the environment or the command line in effect.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    /// Directory that holds the database.
    pub data_dir: Option<PathBuf>,
    /// Native TCP listen address.
    pub listen: Option<String>,
    /// Native admin listen address, as TCP or a Unix socket path.
    pub admin_sock: Option<String>,
    /// HTTP REST and binary RPC listen address.
    pub http_listen: Option<String>,
    /// WebSocket listen address.
    pub ws_listen: Option<String>,
    /// gRPC listen address.
    pub grpc_listen: Option<String>,
    /// Redis RESP listen address.
    pub redis_listen: Option<String>,
    /// Memcached ASCII listen address.
    pub memcached_listen: Option<String>,
    /// Replication role name.
    pub role: Option<String>,
    /// Primary URI that a replica streams from.
    pub primary_uri: Option<String>,
    /// Shared admin secret for auxiliary transports.
    pub admin_token: Option<String>,
    /// File that holds the admin secret.
    pub admin_token_file: Option<PathBuf>,
    /// Password for the Redis RESP listener.
    pub redis_password: Option<String>,
    /// Restrict admin-only requests to admin listeners or token holders.
    pub strict_admin: Option<bool>,
    /// Treat every HTTP connection as admin.
    pub http_admin: Option<bool>,
    /// Treat every WebSocket connection as admin.
    pub ws_admin: Option<bool>,
    /// Treat every gRPC connection as admin.
    pub grpc_admin: Option<bool>,
    /// Enable autonomous continuous backup into a directory.
    pub backup_dir: Option<PathBuf>,
    /// Stream backup archives into a child process.
    pub backup_pipe_cmd: Option<String>,
    /// One argument for the backup pipe command.
    pub backup_pipe_args: Option<Vec<String>>,
    /// Archive compression for the pipe sink.
    pub backup_compression: Option<String>,
    /// Seconds between automatic full backups.
    pub backup_interval_secs: Option<u64>,
    /// Number of full backup generations to keep.
    pub backup_retention: Option<usize>,
    /// Ship sealed WAL segments between full backups.
    pub backup_ship_wal: Option<bool>,
    /// PEM server certificate that enables TLS on the native listener.
    pub tls_cert: Option<PathBuf>,
    /// PEM private key that enables TLS on the native listener.
    pub tls_key: Option<PathBuf>,
    /// Scopes to create or verify before the server accepts client traffic.
    pub scopes: Vec<ScopeConfig>,
}

impl FileConfig {
    /// Parse a configuration from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Parse`] when the TOML is malformed or contains an
    /// unknown key.
    pub fn parse(contents: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(contents)?)
    }

    /// Load a configuration from a TOML file.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Read`] when the file cannot be read, or
    /// [`ConfigError::Parse`] when its contents are invalid.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&contents)
    }

    /// Convert the declared `[[scopes]]` entries to engine options.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Scope`] when an entry is invalid or a name is
    /// declared more than once.
    pub fn scope_options(&self) -> Result<Vec<(String, ScopeOptions)>, ConfigError> {
        scopes::collect(&self.scopes)
    }

    /// Install file values as command defaults.
    ///
    /// Command-line values and environment variables keep their own precedence
    /// because clap resolves them before default values.
    ///
    /// # Panics
    ///
    /// Panics when `command` lacks an argument for a setting that this file
    /// sets. The `storkvd` command always declares every argument.
    pub fn apply_to(&self, mut command: Command) -> Command {
        command = set_default(command, "data_dir", self.data_dir.clone());
        command = set_default(command, "listen", self.listen.clone());
        command = set_default(command, "admin_sock", self.admin_sock.clone());
        command = set_default(command, "http_listen", self.http_listen.clone());
        command = set_default(command, "ws_listen", self.ws_listen.clone());
        command = set_default(command, "grpc_listen", self.grpc_listen.clone());
        command = set_default(command, "redis_listen", self.redis_listen.clone());
        command = set_default(command, "memcached_listen", self.memcached_listen.clone());
        command = set_default(command, "role", self.role.clone());
        command = set_default(command, "primary_uri", self.primary_uri.clone());
        command = set_default(command, "admin_token", self.admin_token.clone());
        command = set_default(command, "admin_token_file", self.admin_token_file.clone());
        command = set_default(command, "redis_password", self.redis_password.clone());
        command = set_default(command, "strict_admin", bool_value(self.strict_admin));
        command = set_default(command, "http_admin", bool_value(self.http_admin));
        command = set_default(command, "ws_admin", bool_value(self.ws_admin));
        command = set_default(command, "grpc_admin", bool_value(self.grpc_admin));
        command = set_default(command, "backup_dir", self.backup_dir.clone());
        command = set_default(command, "backup_pipe_cmd", self.backup_pipe_cmd.clone());
        // An empty array must not become a default value of "", because clap
        // would split that into one empty argument.
        let pipe_args = self
            .backup_pipe_args
            .as_ref()
            .filter(|args| !args.is_empty())
            .map(|args| args.join(","));
        command = set_default(command, "backup_pipe_args", pipe_args);
        command = set_default(
            command,
            "backup_compression",
            self.backup_compression.clone(),
        );
        command = set_default(
            command,
            "backup_interval_secs",
            self.backup_interval_secs.map(|value| value.to_string()),
        );
        command = set_default(
            command,
            "backup_retention",
            self.backup_retention.map(|value| value.to_string()),
        );
        command = set_default(command, "backup_ship_wal", bool_value(self.backup_ship_wal));
        command = set_default(command, "tls_cert", self.tls_cert.clone());
        command = set_default(command, "tls_key", self.tls_key.clone());
        command
    }
}

fn bool_value(value: Option<bool>) -> Option<OsString> {
    value.map(|flag| OsString::from(if flag { "true" } else { "false" }))
}

fn set_default(command: Command, id: &'static str, value: Option<impl Into<OsString>>) -> Command {
    match value {
        Some(value) => command.mut_arg(id, |arg| arg.default_value(value.into())),
        None => command,
    }
}
