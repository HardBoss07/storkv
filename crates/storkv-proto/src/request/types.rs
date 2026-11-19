//! Wire type definitions for protocol requests.

mod enum_def;
mod methods;

pub use enum_def::{
    Request, WireCompactionStyle, WireDurability, WireMergeOp, WireRetention, WireScopeOptions,
    WireTxMutation,
};
