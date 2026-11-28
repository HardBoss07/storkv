//! Compaction range selection and entry filtering.
use crate::format::{InternalKey, ValueType};
use crate::manifest::SstFileMeta;
use crate::scope::Scope;
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;
use bytes::Bytes;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Collect every SSTable whose key bounds overlap the half-open range.
///
/// The compacted output is one file at the highest selected level and spans the
/// union of the selected bounds. A file inside that span must join the
/// selection, or two files in one level would overlap.
pub(super) fn select_overlapping<FS: FileSystem>(
    scope: &Arc<Scope<FS>>,
    start: Option<&[u8]>,
    end: Option<&[u8]>,
) -> Vec<(u8, SstFileMeta)> {
    let levels = scope.sst_levels.read();
    let all: Vec<(u8, SstFileMeta)> = levels
        .iter()
        .flat_map(|(level, files)| files.iter().map(move |file| (*level, file.clone())))
        .collect();
    let bounds: Vec<Option<(Bytes, Bytes)>> = all
        .iter()
        .map(|(_, metadata)| user_key_bounds(metadata))
        .collect();
    let mut selected: Vec<bool> = all
        .iter()
        .zip(&bounds)
        .map(|(_, bounds)| overlaps(bounds.as_ref(), start, end))
        .collect();
    if !selected.iter().any(|selected| *selected) {
        return Vec::new();
    }

    loop {
        let span = selected_span(&bounds, &selected);
        let mut changed = false;
        for index in 0..all.len() {
            if !selected[index] && intersects_span(bounds[index].as_ref(), span.as_ref()) {
                selected[index] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    all.into_iter()
        .zip(selected)
        .filter_map(|(file, selected)| selected.then_some(file))
        .collect()
}

fn user_key_bounds(metadata: &SstFileMeta) -> Option<(Bytes, Bytes)> {
    let min_key = InternalKey::decode(metadata.min_key.clone())?;
    let max_key = InternalKey::decode(metadata.max_key.clone())?;
    Some((min_key.user_key, max_key.user_key))
}

fn overlaps(bounds: Option<&(Bytes, Bytes)>, start: Option<&[u8]>, end: Option<&[u8]>) -> bool {
    // Malformed bounds must not hide data. Compact the file conservatively.
    let Some((min_key, max_key)) = bounds else {
        return true;
    };
    if end.is_some_and(|end| min_key.as_ref() >= end) {
        return false;
    }
    if start.is_some_and(|start| max_key.as_ref() < start) {
        return false;
    }
    true
}

/// Return the union of the selected bounds. `None` means unbounded.
fn selected_span(bounds: &[Option<(Bytes, Bytes)>], selected: &[bool]) -> Option<(Bytes, Bytes)> {
    let mut span: Option<(Bytes, Bytes)> = None;
    for (bounds, selected) in bounds.iter().zip(selected) {
        if !selected {
            continue;
        }
        let Some((min_key, max_key)) = bounds else {
            return None;
        };
        span = Some(match span {
            Some((span_min, span_max)) => {
                (span_min.min(min_key.clone()), span_max.max(max_key.clone()))
            }
            None => (min_key.clone(), max_key.clone()),
        });
    }
    span
}

fn intersects_span(bounds: Option<&(Bytes, Bytes)>, span: Option<&(Bytes, Bytes)>) -> bool {
    // An unbounded span or malformed bounds must not hide data.
    let (Some((min_key, max_key)), Some((span_min, span_max))) = (bounds, span) else {
        return true;
    };
    min_key <= span_max && max_key >= span_min
}

/// Drop obsolete versions inside the range. Versions outside the range stay
/// untouched because lower-level files outside the range can still hold data.
pub(super) fn filter_entries<FS: FileSystem>(
    scope: &Arc<Scope<FS>>,
    entries: BTreeMap<InternalKey, Bytes>,
    start: Option<&[u8]>,
    end: Option<&[u8]>,
    gc_safe_seq: SequenceNumber,
    now_ms: TimestampMs,
) -> Vec<(InternalKey, Bytes)> {
    let mut kept = Vec::new();
    let mut current_user_key: Option<Bytes> = None;
    let mut seen_terminal = false;
    for (internal_key, value) in entries {
        let in_range = start.is_none_or(|start| internal_key.user_key.as_ref() >= start)
            && end.is_none_or(|end| internal_key.user_key.as_ref() < end);
        if !in_range {
            kept.push((internal_key, value));
            current_user_key = None;
            seen_terminal = false;
            continue;
        }
        // The persistent range-tombstone index keeps the deletion, so the SST
        // copy of a range tombstone is obsolete after this compaction.
        if internal_key.value_type == ValueType::RangeTombstone {
            continue;
        }
        if is_expired(&internal_key, now_ms) {
            continue;
        }
        if current_user_key.as_ref() != Some(&internal_key.user_key) {
            current_user_key = Some(internal_key.user_key.clone());
            seen_terminal = false;
        }
        // A range tombstone at or below the GC-safe sequence hides this
        // version from every reader that compaction must preserve.
        if internal_key.seq < gc_safe_seq.get()
            && scope.is_range_deleted(&internal_key.user_key, internal_key.seq, gc_safe_seq.get())
        {
            continue;
        }
        let is_terminal = matches!(
            internal_key.value_type,
            ValueType::Value | ValueType::Tombstone
        );
        if is_terminal && seen_terminal && internal_key.seq < gc_safe_seq.get() {
            continue;
        }
        kept.push((internal_key, value));
        if is_terminal {
            seen_terminal = true;
        }
    }
    kept
}

fn is_expired(internal_key: &InternalKey, now_ms: TimestampMs) -> bool {
    internal_key.expires_at_ms != 0 && internal_key.expires_at_ms <= now_ms.get()
}
