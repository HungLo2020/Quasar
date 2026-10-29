use quasar_profile::{Distribution, FrameProfiler, FrameSample, ProfileThresholds, RendererStats};

#[test]
fn distribution_reports_order_statistics_and_percentiles() {
    let distribution = Distribution::from_values(1..=100);
    assert_eq!(distribution.samples, 100);
    assert_eq!(distribution.min, 1);
    assert_eq!(distribution.p50, 50);
    assert_eq!(distribution.p95, 95);
    assert_eq!(distribution.p99, 99);
    assert_eq!(distribution.max, 100);
    assert!((distribution.mean - 50.5).abs() < 1e-9);
    assert_eq!(Distribution::from_values([]), Distribution::default());
}

#[test]
fn profiler_ring_retains_window_but_counts_total_frames() {
    let mut profiler = FrameProfiler::new(2);
    for frame in 1..=5 {
        profiler.record(FrameSample {
            frame,
            cpu_frame_nanos: frame * 100,
            ..FrameSample::default()
        });
    }
    assert_eq!(profiler.len(), 2);
    assert_eq!(profiler.total_frames(), 5);
    let summary = profiler.summary();
    assert_eq!(summary.frames, 5);
    assert_eq!(summary.window_samples, 2);
    assert_eq!(summary.cpu_frame.min, 400);
    assert_eq!(summary.cpu_frame.max, 500);
    assert_eq!(summary.cpu_frame.p99, 500);
    profiler.clear();
    assert!(profiler.is_empty());
    assert_eq!(profiler.total_frames(), 0);
}

#[test]
fn optional_gpu_and_renderer_fields_are_aggregated() {
    let mut profiler = FrameProfiler::new(4);
    for (frame, gpu) in [(1, None), (2, Some(1_000)), (3, Some(2_000))] {
        let mut sample = FrameSample {
            frame,
            gpu_frame_nanos: gpu,
            gpu_scope_nanos: gpu.map_or_else(Vec::new, |_| vec![100, 200]),
            ..FrameSample::default()
        };
        sample.with_renderer_stats(RendererStats {
            draws: 10,
            geometry_upload_bytes: 2_000,
            texture_upload_bytes: 100,
            uniform_upload_bytes: 64,
        });
        profiler.record(sample);
    }
    let summary = profiler.summary();
    assert_eq!(summary.gpu_frame.samples, 2);
    assert_eq!(summary.gpu_frame.p50, 1_000);
    assert_eq!(summary.gpu_frame.p99, 2_000);
    assert_eq!(summary.gpu_scopes.len(), 2);
    assert_eq!(summary.gpu_scopes[0].p99, 100);
    assert_eq!(summary.draws.p99, 10);
    assert_eq!(summary.geometry_upload_bytes.p99, 2_000);
}

#[test]
fn thresholds_only_check_supplied_metrics() {
    let mut profiler = FrameProfiler::new(10);
    for frame in 0..10 {
        profiler.record(FrameSample {
            frame,
            cpu_frame_nanos: 1_000,
            gpu_frame_nanos: Some(500),
            acquire_nanos: 10,
            present_nanos: 20,
            draws: 30,
            resource_creates: 2,
            host_write_bytes: 4_000,
            ..FrameSample::default()
        });
    }
    let summary = profiler.summary();
    ProfileThresholds::default().check(&summary).unwrap();
    let thresholds = ProfileThresholds {
        max_cpu_frame_p99_nanos: Some(999),
        ..ProfileThresholds::default()
    };
    let violation = thresholds.check(&summary).unwrap_err();
    assert_eq!(violation.metric, "cpu_frame.p99");
    assert_eq!(violation.observed, 1_000);
    assert_eq!(violation.limit, 999);
    let thresholds = ProfileThresholds {
        max_cpu_frame_p99_nanos: None,
        max_gpu_frame_p99_nanos: Some(499),
        ..ProfileThresholds::default()
    };
    assert_eq!(
        thresholds.check(&summary).unwrap_err().metric,
        "gpu_frame.p99"
    );
}

#[cfg(feature = "json")]
#[test]
fn json_summary_is_machine_readable() {
    let mut profiler = FrameProfiler::new(4);
    profiler.record(FrameSample {
        frame: 1,
        cpu_frame_nanos: 123,
        gpu_frame_nanos: Some(456),
        ..FrameSample::default()
    });
    let json = profiler.summary_json().unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["cpu_frame"]["p99"], 123);
    assert_eq!(value["gpu_frame"]["p99"], 456);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profile.json");
    profiler.write_json(&path).unwrap();
    assert!(path.exists());
}

#[cfg(feature = "frame")]
#[test]
fn frame_reports_map_into_samples() {
    use quasar_frame::FrameReport;
    use vulkanic_gal::SubmissionId;
    use vulkanic_gal::metrics::SubmitProfile;
    let profile = SubmitProfile {
        gal_submit_total_nanos: 500,
        gal_command_generation_nanos: 29,
        gal_validate_ops_nanos: 11,
        gal_validate_handles_nanos: 13,
        gal_hazard_analysis_nanos: 17,
        backend_encode_nanos: 19,
        backend_submit_nanos: 23,
        draw_ops: 2,
        draw_indexed_ops: 3,
        host_write_bytes: 4_000,
        host_write_ops: 2,
        resource_creates_delta: 1,
        resource_destroys_delta: 2,
        native_wait_count: 3,
        native_device_wait_idle_count: 4,
        gpu_timestamp_status: 9,
        gpu_frame_total_nanos: 6_000,
        ..SubmitProfile::default()
    };
    let report = FrameReport {
        submission: SubmissionId(1),
        profile,
        acquire_nanos: 10,
        present_nanos: 20,
        cpu_frame_nanos: 30,
    };
    let sample = FrameSample::from_frame_report(7, &report);
    assert_eq!(sample.frame, 7);
    assert_eq!(sample.cpu_frame_nanos, 30);
    assert_eq!(sample.submit_nanos, 500);
    assert_eq!(sample.renderer_prepare_nanos, 29);
    assert_eq!(sample.validation_nanos, 24);
    assert_eq!(sample.hazard_analysis_nanos, 17);
    assert_eq!(sample.backend_encode_nanos, 19);
    assert_eq!(sample.backend_submit_nanos, 23);
    assert_eq!(sample.gpu_frame_nanos, Some(6_000));
    assert_eq!(sample.draws, 5);
    assert_eq!(sample.host_write_bytes, 4_000);
    assert_eq!(sample.resource_creates, 1);
    assert_eq!(sample.resource_destroys, 2);
    assert_eq!(sample.blocking_waits, 3);
    assert_eq!(sample.device_idle_waits, 4);
}
