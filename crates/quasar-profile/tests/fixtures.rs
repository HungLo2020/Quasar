//! The historical composition artifact remains a checked rendering fixture.
//! Its absolute numbers are machine-specific, so this guard checks the schema
//! and statistical invariants rather than imposing a cross-GPU threshold.

#[cfg(feature = "json")]
#[test]
fn composition_fixture_keeps_reproducible_metric_shape() {
    let text =
        include_str!("../../../examples/composition/benchmarks/2026-10-23-rtx2070/summary.json");
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(value["frames_per_run"], 600);
    assert_eq!(value["gpu_timestamps"], true);
    for variant in ["sync", "poll"] {
        let metrics = &value["results"][variant];
        for metric in [
            "cpu_frame_ms",
            "gpu_frame_ms",
            "cpu_submit_ms",
            "submission_to_observed_completion_ms",
            "engine_frame_to_observed_completion_ms",
        ] {
            let distribution = &metrics[metric];
            let p50 = distribution["p50"].as_f64().unwrap();
            let p95 = distribution["p95"].as_f64().unwrap();
            let p99 = distribution["p99"].as_f64().unwrap();
            assert!(p50.is_finite() && p95.is_finite() && p99.is_finite());
            assert!(p50 <= p95 + f64::EPSILON);
            assert!(p95 <= p99 + f64::EPSILON);
            assert!(p99 >= 0.0);
        }
        assert!(metrics["host_upload_bytes"].as_u64().unwrap() > 0);
        assert!(metrics["resource_creates"].as_u64().is_some());
        assert!(metrics["blocking_timeline_waits"].as_u64().is_some());
    }
}
