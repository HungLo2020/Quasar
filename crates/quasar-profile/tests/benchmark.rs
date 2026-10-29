use quasar_profile::*;
#[test]
fn sessions_bound_samples_skip_warmup_and_apply_live_limits() {
    let mut s = BenchmarkSession::new(BenchmarkConfig {
        warmup_frames: 2,
        sample_capacity: 2,
        thresholds: ProfileThresholds {
            max_draws_p99: Some(3),
            ..ProfileThresholds::default()
        },
    })
    .unwrap();
    assert_eq!(s.check(), Err(BenchmarkError::NoSamples));
    for frame in 0..5 {
        assert_eq!(
            s.record(FrameSample {
                frame,
                draws: frame,
                ..FrameSample::default()
            }),
            frame >= 2
        );
    }
    assert_eq!(s.summary().frames, 3);
    assert_eq!(s.summary().window_samples, 2);
    assert_eq!(s.summary().draws.p50, 3);
    assert!(matches!(s.check(), Err(BenchmarkError::Threshold(_))));
    assert!(
        BenchmarkSession::new(BenchmarkConfig {
            sample_capacity: 0,
            ..BenchmarkConfig::default()
        })
        .is_err()
    );
    assert!(
        BenchmarkSession::new(BenchmarkConfig {
            sample_capacity: usize::MAX,
            ..BenchmarkConfig::default()
        })
        .is_err()
    );
    assert!(s.set_metadata("context", &"x".repeat(257)).is_err());
}
#[test]
fn unavailable_gpu_measurements_cannot_pass_a_gpu_budget() {
    let mut s = BenchmarkSession::new(BenchmarkConfig {
        warmup_frames: 0,
        thresholds: ProfileThresholds {
            max_gpu_frame_p99_nanos: Some(10),
            ..ProfileThresholds::default()
        },
        ..BenchmarkConfig::default()
    })
    .unwrap();
    s.record(FrameSample::default());
    assert_eq!(s.check(), Err(BenchmarkError::MissingGpu));
}
#[cfg(feature = "config")]
#[test]
fn benchmark_config_uses_existing_precedence_and_rejects_invalid_types() {
    use quasar_config::Config;
    let mut c = Config::new();
    BenchmarkConfig::register_defaults(&mut c, "measure").unwrap();
    c.set_file("measure.sample_capacity", 20u64).unwrap();
    c.set_cli("measure.sample_capacity", 3u64).unwrap();
    c.set_file("measure.sample_capacity", 10u64).unwrap();
    c.set_cli("measure.max_draws_p99", 5u64).unwrap();
    let settings = BenchmarkConfig::from_config(&c, "measure").unwrap();
    assert_eq!(settings.sample_capacity, 3);
    assert_eq!(settings.thresholds.max_draws_p99, Some(5));
    c.set_cli("measure.sample_capacity", "wrong").unwrap();
    assert!(BenchmarkConfig::from_config(&c, "measure").is_err());
}
#[cfg(feature = "json")]
#[test]
fn benchmark_artifacts_preserve_summary_shape_and_optional_old_fields() {
    let mut s = BenchmarkSession::new(BenchmarkConfig {
        warmup_frames: 0,
        ..BenchmarkConfig::default()
    })
    .unwrap();
    s.set_metadata("application", "consumer").unwrap();
    s.record(FrameSample {
        validation_nanos: 15,
        renderer_prepare_nanos: 12,
        visible_instances: 10,
        culled_instances: 4,
        ..FrameSample::default()
    });
    let file = tempfile::NamedTempFile::new().unwrap();
    s.write_json(file.path()).unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(file.path()).unwrap()).unwrap();
    assert_eq!(value["benchmark"]["metadata"]["application"], "consumer");
    assert_eq!(value["renderer_prepare"]["p99"], 12);
    let mut old = serde_json::to_value(FrameSample::default()).unwrap();
    for k in [
        "validation_nanos",
        "renderer_prepare_nanos",
        "backend_encode_nanos",
        "backend_submit_nanos",
        "hazard_analysis_nanos",
        "visible_instances",
        "culled_instances",
        "instance_upload_bytes",
    ] {
        old.as_object_mut().unwrap().remove(k);
    }
    let _: FrameSample = serde_json::from_value(old).unwrap();
}
#[cfg(feature = "diagnostics")]
#[test]
fn benchmark_reports_use_explicit_diagnostics_handles() {
    use quasar_diagnostics::*;
    use std::sync::Arc;
    let sink = Arc::new(MemorySink::new(8));
    let diagnostics = Diagnostics::new(sink.clone());
    let mut s = BenchmarkSession::new(BenchmarkConfig {
        warmup_frames: 0,
        ..BenchmarkConfig::default()
    })
    .unwrap();
    assert!(s.report(&diagnostics).is_err());
    s.record(FrameSample::default());
    s.report(&diagnostics).unwrap();
    assert!(
        sink.records()
            .iter()
            .any(|r| r.name == "benchmark_complete")
    );
    s.report(&Diagnostics::disabled()).unwrap();
    assert_eq!(sink.len(), 2);
}
