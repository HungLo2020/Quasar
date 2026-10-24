//! Small local overhead probe; not a statistically rigorous benchmark.
use quasar_diagnostics::{Diagnostics, Level, MemorySink};
use std::{sync::Arc, time::Instant};

fn main() {
    let disabled = Diagnostics::disabled();
    let start = Instant::now();
    for _ in 0..1_000_000 {
        std::hint::black_box(&disabled);
        disabled.event(Level::Info, "bench", "event");
    }
    let disabled_ns = start.elapsed().as_nanos() as f64 / 1_000_000.0;

    let sink = Arc::new(MemorySink::new(1024));
    let enabled = Diagnostics::new(sink.clone());
    let start = Instant::now();
    for index in 0..100_000_u64 {
        enabled.event_with(
            Level::Info,
            "bench",
            "event",
            [quasar_diagnostics::Field::new("index", index)],
        );
    }
    let enabled_ns = start.elapsed().as_nanos() as f64 / 100_000.0;
    println!(
        "disabled_ns_per_event={disabled_ns} enabled_ns_per_event={enabled_ns} retained={} dropped={}",
        sink.len(),
        sink.dropped()
    );
}
