//! Codec selection and typed watch decoding.

use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Result, SdkError};
use crate::watch::{WatchEvent, WatchEventKind, WatchStream};

#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub(crate) enum CodecKind {
    #[cfg(feature = "json")]
    #[default]
    Json,
    #[cfg(feature = "postcard")]
    #[cfg_attr(not(feature = "json"), default)]
    Postcard,
}

impl CodecKind {
    pub(crate) fn encode<T: Serialize>(self, value: &T) -> Result<Vec<u8>> {
        match self {
            #[cfg(feature = "json")]
            CodecKind::Json => serde_json::to_vec(value)
                .map_err(|error| SdkError::Serialization(error.to_string())),
            #[cfg(feature = "postcard")]
            CodecKind::Postcard => postcard::to_allocvec(value)
                .map_err(|error| SdkError::Serialization(error.to_string())),
        }
    }

    pub(crate) fn decode<T: DeserializeOwned>(self, bytes: &[u8]) -> Result<T> {
        match self {
            #[cfg(feature = "json")]
            CodecKind::Json => serde_json::from_slice(bytes)
                .map_err(|error| SdkError::Serialization(error.to_string())),
            #[cfg(feature = "postcard")]
            CodecKind::Postcard => postcard::from_bytes(bytes)
                .map_err(|error| SdkError::Serialization(error.to_string())),
        }
    }
}

/// One decoded change event from a collection watch stream.
#[derive(Debug, Clone)]
pub struct TypedWatchEvent<T> {
    /// Commit sequence number of the change.
    pub seq: u64,
    /// Kind of change.
    pub kind: WatchEventKind,
    /// Key relative to the collection prefix.
    pub key: Bytes,
    /// Decoded new value. Present only for puts.
    pub value: Option<T>,
}

/// Stream of decoded change events for one typed collection prefix.
pub struct TypedWatchStream<T> {
    pub(super) codec: CodecKind,
    pub(super) inner: WatchStream,
    pub(super) strip: Bytes,
    pub(super) marker: PhantomData<fn() -> T>,
}

impl<T> TypedWatchStream<T> {
    /// Receive the next decoded event.
    ///
    /// # Errors
    ///
    /// Returns `Some(Err(..))` when the watch stream or the value decode fails.
    pub async fn next(&mut self) -> Option<Result<TypedWatchEvent<T>>>
    where
        T: DeserializeOwned,
    {
        let event = self.inner.next().await?;
        Some(self.decode(event))
    }

    fn decode(&self, event: Result<WatchEvent>) -> Result<TypedWatchEvent<T>>
    where
        T: DeserializeOwned,
    {
        let event = event?;
        let key = event
            .key
            .strip_prefix(self.strip.as_ref())
            .map(Bytes::copy_from_slice)
            .unwrap_or_else(|| event.key.clone());
        let value = match event.value {
            Some(bytes) => Some(self.codec.decode::<T>(&bytes)?),
            None => None,
        };
        Ok(TypedWatchEvent {
            kind: event.kind,
            seq: event.seq,
            key,
            value,
        })
    }
}

impl<T> Stream for TypedWatchStream<T>
where
    T: DeserializeOwned + Unpin,
{
    type Item = Result<TypedWatchEvent<T>>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(context) {
            Poll::Ready(Some(event)) => Poll::Ready(Some(self.decode(event))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
