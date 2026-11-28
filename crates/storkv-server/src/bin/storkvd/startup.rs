//! Startup helpers for `storkvd`: address validation, admin token resolution,
//! and scope provisioning.

use storkv_core::{ScopeOptions, StorkvError};
use storkv_server::{ConfigError, Server, validate_listen_address};

use super::args::Args;

/// Validate every final listen address.
pub(super) fn validate_listen_addrs(args: &Args) -> Result<(), ConfigError> {
    let addresses = [
        ("--listen", Some(args.listen.as_str())),
        ("--admin-sock", args.admin_sock.as_deref()),
        ("--http-listen", args.http_listen.as_deref()),
        ("--ws-listen", args.ws_listen.as_deref()),
        ("--grpc-listen", args.grpc_listen.as_deref()),
        ("--redis-listen", args.redis_listen.as_deref()),
        ("--memcached-listen", args.memcached_listen.as_deref()),
    ];
    for (name, value) in addresses {
        if let Some(value) = value {
            validate_listen_address(name, value)?;
        }
    }
    Ok(())
}

/// Resolve the admin token from the flag or the token file.
pub(super) fn resolve_admin_token(
    args: &Args,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let Some(path) = &args.admin_token_file else {
        return Ok(args.admin_token.clone());
    };
    if args.admin_token.is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "use --admin-token or --admin-token-file, not both",
        )
        .into());
    }
    let contents = std::fs::read_to_string(path)?;
    let token = contents.trim();
    if token.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the admin token file is empty",
        )
        .into());
    }
    Ok(Some(token.to_string()))
}

/// Create every declared scope and refresh guardrails of existing scopes.
pub(super) fn provision_scopes(
    server: &Server,
    scopes: &[(String, ScopeOptions)],
) -> Result<(), Box<dyn std::error::Error>> {
    for (name, options) in scopes {
        match server.db().create_scope(name.clone(), options.clone()) {
            Ok(_) => {}
            Err(StorkvError::ScopeAlreadyExists(_)) => {
                server.db().update_scope_guardrails(name, options)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
