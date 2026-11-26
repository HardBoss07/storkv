mod common;

#[path = "compat/support.rs"]
mod support;

#[path = "compat/adversarial.rs"]
mod adversarial;
#[path = "compat/etcd.rs"]
mod etcd;
#[path = "compat/memcached.rs"]
mod memcached;
#[path = "compat/migrate.rs"]
mod migrate;
#[path = "compat/mixed_protocol.rs"]
mod mixed_protocol;
#[path = "compat/redis.rs"]
mod redis;
#[path = "compat/redis_auth.rs"]
mod redis_auth;
#[path = "compat/redis_collections.rs"]
mod redis_collections;
#[path = "compat/redis_collections_keys.rs"]
mod redis_collections_keys;
#[path = "compat/redis_contention.rs"]
mod redis_contention;
#[path = "compat/redis_expiry.rs"]
mod redis_expiry;
#[path = "compat/redis_extended.rs"]
mod redis_extended;
#[path = "compat/redis_rename.rs"]
mod redis_rename;
#[path = "compat/replica.rs"]
mod replica;
#[path = "compat/tree.rs"]
mod tree;
