//! Wire type definitions for protocol requests.

mod condition;
mod enum_def;
mod methods;
mod options;

pub use condition::WirePutCondition;
pub(crate) use condition::encode_put_condition;
pub use enum_def::{Request, WireMergeOp, WireTxMutation};
pub use options::{WireCompactionStyle, WireDurability, WireRetention, WireScopeOptions};
