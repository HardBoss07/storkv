//! Human-readable formatting helpers for the dashboard.

/// Number of bytes per unit in the human-readable format.
const BYTES_PER_UNIT: u64 = 1024;

/// Units for human-readable byte representation.
const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];

pub(super) fn format_bytes(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= BYTES_PER_UNIT as f64 && unit < UNITS.len() - 1 {
        value /= BYTES_PER_UNIT as f64;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub(super) fn truncate(value: &str, max_chars: usize) -> String {
    let limit = std::cmp::min(value.chars().count(), max_chars);
    if value.chars().count() <= limit {
        value.to_string()
    } else {
        value.chars().take(limit).collect()
    }
}
