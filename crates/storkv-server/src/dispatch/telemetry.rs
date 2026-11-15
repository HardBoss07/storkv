use std::sync::atomic::Ordering;

use storkv_proto::{Response, WireTelemetrySnapshot};

use crate::server::Server;

impl Server {
    pub(super) fn handle_telemetry_query(&self) -> Response {
        let telemetry = self.db.telemetry();
        Response::Telemetry(WireTelemetrySnapshot {
            writes_total: telemetry.writes_total.load(Ordering::Relaxed),
            reads_total: telemetry.reads_total.load(Ordering::Relaxed),
            write_conflicts_total: telemetry.write_conflicts_total.load(Ordering::Relaxed),
            compactions_total: telemetry.compactions_total.load(Ordering::Relaxed),
            fsyncs_total: telemetry.fsyncs_total.load(Ordering::Relaxed),
        })
    }
}
