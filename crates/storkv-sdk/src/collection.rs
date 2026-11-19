//! Typed collections over one key prefix.

use std::marker::PhantomData;

use bytes::Bytes;

use crate::scope::Scope;

mod codec;
mod ops;

pub use codec::{TypedWatchEvent, TypedWatchStream};

pub(crate) use codec::CodecKind;

/// Typed view of one key prefix inside a scope.
pub struct Collection<T> {
    codec: CodecKind,
    scope: Scope,
    prefix: Bytes,
    marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Collection<T> {
    fn clone(&self) -> Self {
        Self {
            scope: self.scope.clone(),
            prefix: self.prefix.clone(),
            codec: self.codec,
            marker: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for Collection<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Collection")
            .field("scope", &self.scope)
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

impl<T> Collection<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    #[cfg(any(feature = "json", feature = "postcard"))]
    pub(crate) fn new(scope: Scope, prefix: &[u8]) -> Self {
        Self {
            scope,
            prefix: Bytes::copy_from_slice(prefix),
            codec: CodecKind::default(),
            marker: PhantomData,
        }
    }
}
