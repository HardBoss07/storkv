//! Metadata for every supported Redis command.

/// Arity and key positions of one supported Redis command.
pub(crate) struct CommandSpec {
    /// Uppercase command name.
    pub name: &'static str,
    /// Redis arity: a positive value is exact, a negative value is a minimum.
    pub arity: i8,
    /// Index of the first key argument, or zero when the command has no keys.
    pub first_key: u8,
    /// Index of the last key argument, or zero when the command has no keys.
    pub last_key: i8,
    /// Step between key arguments, or zero when the command has no keys.
    pub step: u8,
}

/// Every supported Redis command with its metadata.
#[rustfmt::skip]
pub(crate) const COMMAND_SPECS: &[CommandSpec] = &[
    CommandSpec { name: "AUTH", arity: -2, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "CLIENT", arity: -2, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "COMMAND", arity: -1, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "PING", arity: -1, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "ECHO", arity: 2, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "QUIT", arity: 1, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "HELLO", arity: -1, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "SELECT", arity: 2, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "INFO", arity: -1, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "DBSIZE", arity: 1, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "FLUSHDB", arity: -1, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "FLUSHALL", arity: -1, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "GET", arity: 2, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "SET", arity: -3, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "SETEX", arity: 4, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "PSETEX", arity: 4, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "SETNX", arity: 3, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "MGET", arity: -2, first_key: 1, last_key: -1, step: 1 },
    CommandSpec { name: "MSET", arity: -3, first_key: 1, last_key: -1, step: 2 },
    CommandSpec { name: "DEL", arity: -2, first_key: 1, last_key: -1, step: 1 },
    CommandSpec { name: "UNLINK", arity: -2, first_key: 1, last_key: -1, step: 1 },
    CommandSpec { name: "EXISTS", arity: -2, first_key: 1, last_key: -1, step: 1 },
    CommandSpec { name: "INCR", arity: 2, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "DECR", arity: 2, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "INCRBY", arity: 3, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "DECRBY", arity: 3, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "APPEND", arity: 3, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "STRLEN", arity: 2, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "EXPIRE", arity: -3, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "PEXPIRE", arity: -3, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "PERSIST", arity: 2, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "TTL", arity: 2, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "PTTL", arity: 2, first_key: 1, last_key: 1, step: 1 },
    CommandSpec { name: "KEYS", arity: 2, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "SCAN", arity: -2, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "MULTI", arity: 1, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "EXEC", arity: 1, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "DISCARD", arity: 1, first_key: 0, last_key: 0, step: 0 },
    CommandSpec { name: "WATCH", arity: -2, first_key: 1, last_key: -1, step: 1 },
    CommandSpec { name: "UNWATCH", arity: 1, first_key: 0, last_key: 0, step: 0 },
];

pub(crate) fn is_known_command(command: &str) -> bool {
    COMMAND_SPECS.iter().any(|spec| spec.name == command)
}
