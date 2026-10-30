#![cfg(feature = "frame")]
use quasar_frame::{Gpu, record_and_submit};
use quasar_profile::{FrameProfiler, FrameSample};
#[test]
#[ignore = "native timestamp queries"]
fn profiler_resolves_the_original_frame_after_async_completion() {
    let gpu = Gpu::headless().unwrap();
    gpu.set_gpu_timestamps(true);
    let mut profiler = FrameProfiler::new(8);
    for frame in 0..8 {
        let (_, report) =
            record_and_submit(&gpu, |gpu| Ok(vec![gpu.encoder("profile frame").finish()])).unwrap();
        profiler.record(FrameSample::from_frame_report(frame, &report));
    }
    gpu.wait().unwrap();
    let summary = profiler.summary();
    let supported = gpu.device.features().contains(
        vulkanic_gal::Features::TIMESTAMP_QUERY
            | vulkanic_gal::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
    );
    assert_eq!(summary.gpu_frame.samples, if supported { 8 } else { 0 });
    assert_eq!(summary.blocking_waits.max, 0);
}
