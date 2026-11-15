use storkv_proto::{WireAdminStats, WireTelemetrySnapshot};

/// A snapshot of telemetry data polled from the server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TelemetryPoll {
    pub writes_total: u64,
    pub reads_total: u64,
    pub write_conflicts_total: u64,
    pub compactions_total: u64,
    pub fsyncs_total: u64,
}

impl From<WireTelemetrySnapshot> for TelemetryPoll {
    fn from(snapshot: WireTelemetrySnapshot) -> Self {
        Self {
            writes_total: snapshot.writes_total,
            reads_total: snapshot.reads_total,
            write_conflicts_total: snapshot.write_conflicts_total,
            compactions_total: snapshot.compactions_total,
            fsyncs_total: snapshot.fsyncs_total,
        }
    }
}

/// Per-level SSTable distribution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelStats {
    pub level: u8,
    pub file_count: usize,
    pub size_bytes: u64,
}

/// Per-scope activity metrics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeStats {
    pub scope_id: u32,
    pub name: String,
    pub writes_total: u64,
    pub reads_total: u64,
    pub sst_files: u32,
}

/// Rich dashboard data polled from the admin socket.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminPoll {
    pub levels: Vec<LevelStats>,
    pub scopes: Vec<ScopeStats>,
    pub fsync_latency_p99_us: u64,
    pub fsync_latency_samples: u64,
}

impl From<WireAdminStats> for AdminPoll {
    fn from(stats: WireAdminStats) -> Self {
        let mut levels = empty_level_stats();
        for wire_level in stats.levels {
            if let Some(slot) = levels.get_mut(wire_level.level as usize) {
                slot.file_count = wire_level.file_count as usize;
                slot.size_bytes = wire_level.size_bytes;
            }
        }
        Self {
            levels,
            scopes: stats
                .scopes
                .into_iter()
                .map(scope_stats_from_wire)
                .collect(),
            fsync_latency_p99_us: stats.fsync_latency_p99_us,
            fsync_latency_samples: stats.fsync_latency_samples,
        }
    }
}

/// Combined dashboard snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardSnapshot {
    pub telemetry: TelemetryPoll,
    pub admin: AdminPoll,
}

/// Compute write amplification (writes / reads ratio), returning 0.0 when reads == 0.
pub fn write_amplification(telemetry: &TelemetryPoll) -> f64 {
    if telemetry.reads_total == 0 {
        0.0
    } else {
        telemetry.writes_total as f64 / telemetry.reads_total as f64
    }
}

pub(crate) fn empty_level_stats() -> Vec<LevelStats> {
    (0u8..=6u8)
        .map(|level| LevelStats {
            level,
            file_count: 0,
            size_bytes: 0,
        })
        .collect()
}

fn scope_stats_from_wire(scope: storkv_proto::WireScopeStat) -> ScopeStats {
    ScopeStats {
        scope_id: scope.scope_id,
        name: scope.name,
        writes_total: scope.writes_total,
        reads_total: scope.reads_total,
        sst_files: scope.sst_files,
    }
}
