use storkv_tui::{TelemetryPoll, write_amplification};

#[test]
fn write_amplification_calculation() {
    let zero = TelemetryPoll::default();
    assert_eq!(write_amplification(&zero), 0.0, "zero reads => 0.0 amp");

    let telemetry = TelemetryPoll {
        writes_total: 100,
        reads_total: 50,
        ..Default::default()
    };
    let amplification = write_amplification(&telemetry);
    assert!(
        (amplification - 2.0).abs() < f64::EPSILON,
        "expected amp 2.0, got {amplification}"
    );
}

#[test]
fn write_amplification_edge_cases() {
    assert_eq!(
        write_amplification(&TelemetryPoll {
            writes_total: u64::MAX,
            reads_total: 0,
            ..Default::default()
        }),
        0.0
    );
    assert_eq!(
        write_amplification(&TelemetryPoll {
            writes_total: 0,
            reads_total: 10,
            ..Default::default()
        }),
        0.0
    );
    let high = write_amplification(&TelemetryPoll {
        writes_total: 1_000_000,
        reads_total: 3,
        ..Default::default()
    });
    assert!(high > 100_000.0);
    assert!(high.is_finite());
}
