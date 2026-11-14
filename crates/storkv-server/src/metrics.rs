use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use storkv_core::Db;
use storkv_proto::{WireAdminStats, WireLevelStat, WireScopeStat};

const MAX_LATENCY_SAMPLES: usize = 2048;
const LEVEL_COUNT: usize = 7;

#[derive(Debug, Default, Clone, Copy)]
struct ScopeCounters {
    writes: u64,
    reads: u64,
}

#[derive(Default)]
pub(crate) struct ServerStats {
    scopes: Mutex<HashMap<u32, ScopeCounters>>,
    latency_us: Mutex<VecDeque<u64>>,
}

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

    fn p99_latency(&self) -> (u64, u64) {
        let samples = match self.latency_us.lock() {
            Ok(samples) => samples.clone(),
            Err(_) => return (0, 0),
        };
        if samples.is_empty() {
            return (0, 0);
        }
        let mut sorted_samples: Vec<u64> = samples.into_iter().collect();
        sorted_samples.sort_unstable();
        let rank = sorted_samples.len().saturating_sub(1);
        let p99_index = ((sorted_samples.len() * 99).div_ceil(100)).max(1) - 1;
        (
            sorted_samples[p99_index.min(rank)],
            sorted_samples.len() as u64,
        )
    }

    pub(crate) fn snapshot(&self, db: &Db) -> WireAdminStats {
        let (levels, scopes) = self.collect_scope_totals(db);
        let (p99_latency_us, sample_count) = self.p99_latency();
        WireAdminStats {
            levels,
            scopes,
            fsync_latency_p99_us: p99_latency_us,
            fsync_latency_samples: sample_count,
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
