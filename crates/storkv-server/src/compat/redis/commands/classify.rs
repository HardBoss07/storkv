//! Redis command classification helpers.

/// Return `true` for commands that a connection may run before `AUTH`.
pub(super) fn allows_before_auth(command: &str) -> bool {
    matches!(command, "AUTH" | "HELLO" | "PING")
}

/// Return `true` for commands that change stored data.
pub(super) fn is_write_command(command: &str) -> bool {
    matches!(
        command,
        "SET"
            | "SETEX"
            | "PSETEX"
            | "SETNX"
            | "MSET"
            | "MSETNX"
            | "GETDEL"
            | "GETEX"
            | "RENAME"
            | "RENAMENX"
            | "DEL"
            | "UNLINK"
            | "INCR"
            | "DECR"
            | "INCRBY"
            | "DECRBY"
            | "APPEND"
            | "EXPIRE"
            | "PEXPIRE"
            | "EXPIREAT"
            | "PEXPIREAT"
            | "PERSIST"
            | "HSET"
            | "HDEL"
            | "HINCRBY"
            | "SADD"
            | "SREM"
            | "FLUSHDB"
            | "FLUSHALL"
            | "EXEC"
    )
}

/// Return `true` for commands that increment the per-transport request count.
pub(super) fn records_transport_request(command: &str) -> bool {
    matches!(
        command,
        "PING"
            | "ECHO"
            | "GET"
            | "SET"
            | "MGET"
            | "MSET"
            | "DEL"
            | "UNLINK"
            | "EXISTS"
            | "SCAN"
            | "KEYS"
            | "HSET"
            | "HGET"
            | "HGETALL"
            | "SADD"
            | "SMEMBERS"
            | "INFO"
            | "DBSIZE"
            | "TTL"
            | "PTTL"
            | "HELLO"
    )
}
