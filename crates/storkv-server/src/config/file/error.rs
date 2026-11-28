//! Configuration errors and listen address validation.

use std::net::ToSocketAddrs;
use std::path::PathBuf;

/// Configuration loading or validation failure.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The configuration file could not be read.
    #[error("cannot read configuration file {path}: {source}")]
    Read {
        /// Path that could not be read.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The configuration file is not valid TOML or has an unknown key.
    #[error("cannot parse configuration file: {0}")]
    Parse(#[from] toml::de::Error),
    /// A listen address is empty or cannot be parsed.
    #[error("{name} is not a valid listen address `{value}`: {detail}")]
    Listen {
        /// Flag or setting name.
        name: &'static str,
        /// Rejected address.
        value: String,
        /// Reason for the rejection.
        detail: String,
    },
    /// A `[[scopes]]` entry is invalid.
    #[error("invalid scope `{name}`: {detail}")]
    Scope {
        /// Scope name from the file.
        name: String,
        /// Reason for the rejection.
        detail: String,
    },
}

/// Validate one listen address.
///
/// A Unix socket address must hold a path. Any other value must resolve to a
/// TCP socket address.
///
/// # Errors
///
/// Returns [`ConfigError::Listen`] when the value is not a valid address.
pub fn validate_listen_address(name: &'static str, value: &str) -> Result<(), ConfigError> {
    if storkv_proto::is_unix_socket_addr(value) {
        return if storkv_proto::unix_socket_path(value).is_empty() {
            Err(listen_error(name, value, "the socket path is empty"))
        } else {
            Ok(())
        };
    }
    match value.to_socket_addrs() {
        Ok(mut addresses) => {
            if addresses.next().is_some() {
                Ok(())
            } else {
                Err(listen_error(name, value, "the address did not resolve"))
            }
        }
        Err(error) => Err(listen_error(name, value, &error.to_string())),
    }
}

fn listen_error(name: &'static str, value: &str, detail: &str) -> ConfigError {
    ConfigError::Listen {
        name,
        value: value.to_owned(),
        detail: detail.to_owned(),
    }
}
