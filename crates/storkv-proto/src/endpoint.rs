/// Heuristic shared by server/client tooling: an address refers to a
/// Unix-domain socket when it uses the `unix:` scheme or looks like a
/// filesystem path rather than a `host:port` TCP endpoint.
pub fn is_unix_socket_addr(address: &str) -> bool {
    address.starts_with("unix:")
        || address.contains('/')
        || address.ends_with(".sock")
        || address.contains('\\')
}

/// Strip an optional `unix:` scheme from a socket path.
pub fn unix_socket_path(address: &str) -> &str {
    address.strip_prefix("unix:").unwrap_or(address)
}
