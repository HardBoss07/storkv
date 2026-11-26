//! Internal key encoding and metadata for Redis hashes and sets.
//!
//! Composite values live under reserved keys that start with `0x01`. A user
//! key that starts with that byte is therefore reserved by the adapter.

use bytes::Bytes;
use storkv_core::{ScopeHandle, StorkvError};

use super::super::codec::Resp;
use super::view::{ReadView, WriteView};

const INTERNAL_PREFIX: u8 = 0x01;
const META_TAG: u8 = b'm';
const KEY_LEN_BYTES: usize = 4;
const META_LEN: usize = 9;

/// Composite value type stored by the adapter.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Redis hash.
    Hash,
    /// Redis set.
    Set,
}

impl Kind {
    fn tag(self) -> u8 {
        match self {
            Self::Hash => b'h',
            Self::Set => b's',
        }
    }

    /// Redis `TYPE` name.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Hash => "hash",
            Self::Set => "set",
        }
    }
}

/// Metadata entry for one composite key.
pub(crate) struct Meta {
    pub(crate) kind: Kind,
    pub(crate) count: u64,
}

/// Result of looking up a composite key while expecting one type.
pub(crate) enum Lookup {
    /// The key does not exist.
    Absent,
    /// The key exists with the expected type.
    Meta(Meta),
    /// The key exists with another type.
    WrongType,
}

/// Return the reply for a key that holds another type.
pub(crate) fn wrong_type() -> Resp {
    Resp::error("WRONGTYPE Operation against a key holding the wrong kind of value")
}

/// Return the key prefix that holds every field or member of one composite key.
pub(crate) fn data_prefix(kind: Kind, key: &[u8]) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(2 + KEY_LEN_BYTES + key.len());
    prefix.push(INTERNAL_PREFIX);
    prefix.push(kind.tag());
    prefix.extend_from_slice(&(key.len() as u32).to_be_bytes());
    prefix.extend_from_slice(key);
    prefix
}

/// Return the internal key for one hash field or set member.
pub(crate) fn data_key(kind: Kind, key: &[u8], member: &[u8]) -> Vec<u8> {
    let mut encoded = data_prefix(kind, key);
    encoded.extend_from_slice(member);
    encoded
}

/// Return the internal key for the metadata entry of one composite key.
pub(crate) fn meta_key(key: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(2 + key.len());
    encoded.push(INTERNAL_PREFIX);
    encoded.push(META_TAG);
    encoded.extend_from_slice(key);
    encoded
}

/// Convert one raw internal key to the user-visible key.
///
/// Returns `None` for internal data keys, which are never visible.
pub(crate) fn logical_key(raw: &Bytes) -> Option<Bytes> {
    if raw.first() != Some(&INTERNAL_PREFIX) {
        return Some(raw.clone());
    }
    if raw.get(1) == Some(&META_TAG) {
        return Some(raw.slice(2..));
    }
    None
}

/// Read the metadata entry of one key.
pub(crate) fn read_meta<V: ReadView>(
    view: &mut V,
    scope: &ScopeHandle,
    key: &[u8],
) -> Result<Option<Meta>, StorkvError> {
    let Some(value) = view.get(scope, &meta_key(key))? else {
        return Ok(None);
    };
    Ok(parse_meta(&value))
}

/// Read the type recorded in the metadata entry of one key.
pub(crate) fn read_kind<V: ReadView>(
    view: &mut V,
    scope: &ScopeHandle,
    key: &[u8],
) -> Result<Option<Kind>, StorkvError> {
    Ok(read_meta(view, scope, key)?.map(|meta| meta.kind))
}

/// Look up a composite key that must have the expected type.
pub(crate) fn lookup<V: ReadView>(
    view: &mut V,
    scope: &ScopeHandle,
    key: &[u8],
    expected: Kind,
) -> Result<Lookup, StorkvError> {
    if let Some(meta) = read_meta(view, scope, key)? {
        return Ok(if meta.kind == expected {
            Lookup::Meta(meta)
        } else {
            Lookup::WrongType
        });
    }
    if view.get(scope, key)?.is_some() {
        return Ok(Lookup::WrongType);
    }
    Ok(Lookup::Absent)
}

/// Write the metadata entry of one composite key.
pub(crate) fn write_meta<V: WriteView>(
    view: &mut V,
    scope: &ScopeHandle,
    key: &[u8],
    kind: Kind,
    count: u64,
) -> Result<(), StorkvError> {
    let mut value = Vec::with_capacity(META_LEN);
    value.push(kind.tag());
    value.extend_from_slice(&count.to_be_bytes());
    view.put(scope, &meta_key(key), &value, 0)
}

/// Delete the metadata entry of one composite key.
pub(crate) fn delete_meta<V: WriteView>(
    view: &mut V,
    scope: &ScopeHandle,
    key: &[u8],
) -> Result<(), StorkvError> {
    view.delete(scope, &meta_key(key))
}

fn parse_meta(value: &[u8]) -> Option<Meta> {
    let kind = match value.first()? {
        b'h' => Kind::Hash,
        b's' => Kind::Set,
        _ => return None,
    };
    let count = u64::from_be_bytes(value.get(1..META_LEN)?.try_into().ok()?);
    Some(Meta { kind, count })
}
