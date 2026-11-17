use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use storkv_core::Db;
use storkv_proto::{WireAdminStats, WireLevelStat, WireScopeStat, WireTransportStat};

const MAX_LATENCY_SAMPLES: usize = 2048;
const LEVEL_COUNT: usize = 7;

#[derive(Debug, Default, Clone, Copy)]
struct ScopeCounters {
    writes: u64,
    reads: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct TransportCounters {
    connections_total: u64,
    active_connections: u64,
    requests_total: u64,
}

#[derive(Default)]
pub(crate) struct ServerStats {
    scopes: Mutex<HashMap<u32, ScopeCounters>>,
    latency_us: Mutex<VecDeque<u64>>,
    transports: Mutex<HashMap<u8, TransportCounters>>,
}

const _: () = assert!(std::mem::size_of::<ServerStats>() <= 256);

impl ServerStats {
    pub(crate) fn record_read(&self, scope_id: u32) {
        if let Ok(mut scopes) = self.scopes.lock() {
            scopes.entry(scope_id).or_default().reads += 1;
        }
    }

    pub(crate) fn record_write(&self, scope_id: u32) {
        if let Ok(mut scopes) = self.scopes.lock() {
            scopes.entry(scope_id).or_default().writes += 1;
        }
    }

    pub(crate) fn record_latency(&self, micros: u64) {
        if let Ok(mut samples) = self.latency_us.lock() {
            if samples.len() >= MAX_LATENCY_SAMPLES {
                samples.pop_front();
            }
            samples.push_back(micros);
        }
    }

    pub(crate) fn record_transport_connection(&self, kind: u8) {
        if let Ok(mut transports) = self.transports.lock() {
            let counters = transports.entry(kind).or_default();
            counters.connections_total += 1;
            counters.active_connections += 1;
        }
    }

    pub(crate) fn record_transport_disconnection(&self, kind: u8) {
        if let Ok(mut transports) = self.transports.lock() {
            let counters = transports.entry(kind).or_default();
            counters.active_connections = counters.active_connections.saturating_sub(1);
        }
    }

    pub(crate) fn record_transport_request(&self, kind: u8) {
        if let Ok(mut transports) = self.transports.lock() {
            transports.entry(kind).or_default().requests_total += 1;
        }
    }

    fn transport_snapshot(&self) -> Vec<WireTransportStat> {
        let mut counters: Vec<(u8, TransportCounters)> = self
            .transports
            .lock()
            .map(|transports| {
                transports
                    .iter()
                    .map(|(kind, value)| (*kind, *value))
                    .collect()
            })
            .unwrap_or_default();
        counters.sort_by_key(|(kind, _)| *kind);
        counters
            .into_iter()
            .map(|(kind, value)| WireTransportStat {
                kind,
                connections_total: value.connections_total,
                active_connections: value.active_connections,
                requests_total: value.requests_total,
            })
            .collect()
    }

    fn p99_latency(&self) -> (u64, u64) {
        let Ok(samples) = self.latency_us.lock() else {
            return (0, 0);
        };
        if samples.is_empty() {
            return (0, 0);
        }
        let mut sorted_samples: Vec<u64> = samples.iter().copied().collect();
        let sample_count = sorted_samples.len() as u64;
        sorted_samples.sort_unstable();
        let rank = sorted_samples.len().saturating_sub(1);
        let p99_index = ((sorted_samples.len() * 99).div_ceil(100)).max(1) - 1;
        (sorted_samples[p99_index.min(rank)], sample_count)
    }

    pub(crate) fn snapshot(&self, db: &Db) -> WireAdminStats {
        let (levels, scopes) = self.collect_scope_totals(db);
        let (p99_latency_us, sample_count) = self.p99_latency();
        let telemetry = db.telemetry();
        WireAdminStats {
            levels,
            scopes,
            fsync_latency_p99_us: p99_latency_us,
            fsync_latency_samples: sample_count,
            backups_completed_total: telemetry
                .backups_completed_total
                .load(std::sync::atomic::Ordering::Relaxed),
            backups_failed_total: telemetry
                .backups_failed_total
                .load(std::sync::atomic::Ordering::Relaxed),
            last_backup_timestamp_ms: telemetry
                .last_backup_timestamp_ms
                .load(std::sync::atomic::Ordering::Relaxed),
            last_backup_seq: telemetry
                .last_backup_seq
                .load(std::sync::atomic::Ordering::Relaxed),
            wal_segments_shipped_total: telemetry
                .wal_segments_shipped_total
                .load(std::sync::atomic::Ordering::Relaxed),
            transports: self.transport_snapshot(),
        }
    }

    fn collect_scope_totals(&self, db: &Db) -> (Vec<WireLevelStat>, Vec<WireScopeStat>) {
        let mut level_counts = [(0u32, 0u64); LEVEL_COUNT];
        let counters = self
            .scopes
            .lock()
            .map(|scopes| scopes.clone())
            .unwrap_or_default();
        let scope_registry = db.inner().scopes.read();
        let mut ordered_scopes: Vec<_> = scope_registry.values().collect();
        ordered_scopes.sort_by_key(|scope| scope.scope_id);

        let mut scope_stats = Vec::with_capacity(ordered_scopes.len());
        for scope in ordered_scopes {
            let sst_file_count = accumulate_level_counts(scope, &mut level_counts);
            let counters = counters.get(&scope.scope_id).copied().unwrap_or_default();
            scope_stats.push(WireScopeStat {
                scope_id: scope.scope_id,
                name: scope.name.clone(),
                writes_total: counters.writes,
                reads_total: counters.reads,
                sst_files: sst_file_count,
            });
        }

        let level_stats = level_counts
            .iter()
            .enumerate()
            .map(|(level, (file_count, size_bytes))| WireLevelStat {
                level: level as u8,
                file_count: *file_count,
                size_bytes: *size_bytes,
            })
            .collect();
        (level_stats, scope_stats)
    }
}

fn accumulate_level_counts<FS: storkv_core::vfs::FileSystem>(
    scope: &storkv_core::scope::Scope<FS>,
    level_counts: &mut [(u32, u64); LEVEL_COUNT],
) -> u32 {
    let mut sst_file_count = 0u32;
    let levels = scope.sst_levels.read();
    for (level, files) in levels.iter() {
        if let Some(slot) = level_counts.get_mut(*level as usize) {
            slot.0 += files.len() as u32;
            slot.1 += files.iter().map(|file| file.file_size).sum::<u64>();
        }
        sst_file_count += files.len() as u32;
    }
    sst_file_count
}
