//! Prometheus text exposition for server and engine statistics.

use std::fmt::Write;

use storkv_core::Db;

use super::ServerStats;
use super::ops::{LATENCY_BUCKETS, OpSnapshot};

/// Render every metric in the Prometheus text exposition format.
pub(crate) fn render(stats: &ServerStats, db: &Db) -> String {
    let mut out = String::new();
    render_operations(&mut out, &stats.operation_snapshot());
    render_engine(&mut out, stats, db);
    out
}

fn render_operations(out: &mut String, operations: &[OpSnapshot]) {
    let _ = writeln!(
        out,
        "# HELP storkv_requests_total Data requests handled since start."
    );
    let _ = writeln!(out, "# TYPE storkv_requests_total counter");
    for operation in operations {
        let _ = writeln!(
            out,
            "storkv_requests_total{{op=\"{}\"}} {}",
            operation.kind.name(),
            operation.count
        );
    }

    let _ = writeln!(
        out,
        "# HELP storkv_request_duration_seconds Data request latency."
    );
    let _ = writeln!(out, "# TYPE storkv_request_duration_seconds histogram");
    for operation in operations {
        let name = operation.kind.name();
        for (index, (label, _)) in LATENCY_BUCKETS.iter().enumerate() {
            let _ = writeln!(
                out,
                "storkv_request_duration_seconds_bucket{{op=\"{name}\",le=\"{label}\"}} {}",
                operation.bucket_count(index)
            );
        }
        let _ = writeln!(
            out,
            "storkv_request_duration_seconds_bucket{{op=\"{name}\",le=\"+Inf\"}} {}",
            operation.count
        );
        let _ = writeln!(
            out,
            "storkv_request_duration_seconds_sum{{op=\"{name}\"}} {}",
            seconds(operation.sum_us)
        );
        let _ = writeln!(
            out,
            "storkv_request_duration_seconds_count{{op=\"{name}\"}} {}",
            operation.count
        );
    }
}

fn render_engine(out: &mut String, stats: &ServerStats, db: &Db) {
    let _ = writeln!(
        out,
        "# HELP storkv_watch_subscribers Active watch subscribers."
    );
    let _ = writeln!(out, "# TYPE storkv_watch_subscribers gauge");
    let _ = writeln!(
        out,
        "storkv_watch_subscribers {}",
        db.watch_subscriber_count()
    );

    let (pinned, total) = db.wal_byte_stats().unwrap_or_default();
    let _ = writeln!(
        out,
        "# HELP storkv_wal_pinned_bytes WAL bytes held by un-flushed MemTables."
    );
    let _ = writeln!(out, "# TYPE storkv_wal_pinned_bytes gauge");
    let _ = writeln!(out, "storkv_wal_pinned_bytes {pinned}");
    let _ = writeln!(
        out,
        "# HELP storkv_wal_total_bytes Total bytes of WAL segment files."
    );
    let _ = writeln!(out, "# TYPE storkv_wal_total_bytes gauge");
    let _ = writeln!(out, "storkv_wal_total_bytes {total}");

    let admin = stats.snapshot(db);
    let _ = writeln!(out, "# HELP storkv_sst_files SSTable files per level.");
    let _ = writeln!(out, "# TYPE storkv_sst_files gauge");
    for level in &admin.levels {
        let _ = writeln!(
            out,
            "storkv_sst_files{{level=\"L{}\"}} {}",
            level.level, level.file_count
        );
    }
    let _ = writeln!(out, "# HELP storkv_sst_bytes SSTable bytes per level.");
    let _ = writeln!(out, "# TYPE storkv_sst_bytes gauge");
    for level in &admin.levels {
        let _ = writeln!(
            out,
            "storkv_sst_bytes{{level=\"L{}\"}} {}",
            level.level, level.size_bytes
        );
    }

    let telemetry = db.telemetry();
    let _ = writeln!(
        out,
        "# HELP storkv_compaction_bytes_written_total Bytes written by compaction."
    );
    let _ = writeln!(out, "# TYPE storkv_compaction_bytes_written_total counter");
    let _ = writeln!(
        out,
        "storkv_compaction_bytes_written_total {}",
        telemetry
            .compaction_bytes_written_total
            .load(std::sync::atomic::Ordering::Relaxed)
    );
    let _ = writeln!(
        out,
        "# HELP storkv_block_cache_hits_total SSTable block cache hits."
    );
    let _ = writeln!(out, "# TYPE storkv_block_cache_hits_total counter");
    let _ = writeln!(
        out,
        "storkv_block_cache_hits_total {}",
        telemetry
            .block_cache_hits_total
            .load(std::sync::atomic::Ordering::Relaxed)
    );
    let _ = writeln!(
        out,
        "# HELP storkv_block_cache_misses_total SSTable block cache misses."
    );
    let _ = writeln!(out, "# TYPE storkv_block_cache_misses_total counter");
    let _ = writeln!(
        out,
        "storkv_block_cache_misses_total {}",
        telemetry
            .block_cache_misses_total
            .load(std::sync::atomic::Ordering::Relaxed)
    );
}

fn seconds(micros: u64) -> String {
    const MICROS_PER_SECOND: f64 = 1_000_000.0;
    format!("{:.6}", micros as f64 / MICROS_PER_SECOND)
}
