//! Small local fixed-step overhead probe; not a statistically rigorous
//! benchmark. Run with:
//!
//! ```sh
//! cargo run --release -p quasar-core --example timing_overhead
//! ```
use quasar_core::{BacklogPolicy, FixedStep, FixedStepConfig};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

const FRAMES: u64 = 2_000_000;

fn measure(label: &str, frame: Duration, reported: bool, mut clock: FixedStep) {
    let start = Instant::now();
    let mut ticks = 0u64;
    for _ in 0..FRAMES {
        if reported {
            let report = clock.advance_with_report(frame);
            black_box(report);
        } else {
            clock.advance(frame);
        }
        while clock.take_tick() {
            ticks += 1;
        }
        black_box(clock.interpolation());
    }
    let elapsed = start.elapsed();
    let per_frame = elapsed.as_nanos() as f64 / FRAMES as f64;
    println!(
        "{label:<28} {per_frame:>7.2} ns/frame  {ticks:>9} ticks  {:.1} Mframe/s",
        FRAMES as f64 / elapsed.as_secs_f64() / 1_000_000.0
    );
}

fn main() {
    let frame = Duration::from_nanos(1_000_000_000 / 60);
    measure(
        "advance+consume",
        frame,
        false,
        FixedStep::new(60.0, 8).unwrap(),
    );
    measure(
        "advance_with_report",
        frame,
        true,
        FixedStep::new(60.0, 8).unwrap(),
    );
    measure(
        "250ms clamp+drop",
        Duration::from_millis(400),
        true,
        FixedStep::with_config(FixedStepConfig {
            max_elapsed: Some(Duration::from_millis(250)),
            ..FixedStepConfig::for_rate(60.0, 8)
        })
        .unwrap(),
    );
    measure(
        "preserve backlog",
        frame / 4,
        true,
        FixedStep::with_config(FixedStepConfig {
            backlog: BacklogPolicy::Preserve,
            ..FixedStepConfig::for_rate(60.0, 8)
        })
        .unwrap(),
    );
}
