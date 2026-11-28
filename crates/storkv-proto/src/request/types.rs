//! Wire type definitions for protocol requests.

mod condition;
mod enum_def;
mod merge_ops;
mod methods;
mod options;

pub use condition::WirePutCondition;
pub(crate) use condition::encode_put_condition;
pub use enum_def::Request;
pub use merge_ops::{WireMergeOp, WireTxMutation};
pub use options::{WireCompactionStyle, WireDurability, WireRetention, WireScopeOptions};
