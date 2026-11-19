//! Pool, retry, cache, and scope configuration types.

mod policy;
mod pool;

pub use policy::{CacheOptions, RetryPolicy, ScopeDefinition};
pub use pool::StorkvPoolOptions;
