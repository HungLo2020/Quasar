use quasar_diagnostics::{
    CallbackSink, Diagnostics, DiagnosticsConfig, Failure, Field, Level, MemorySink, Record, Sink,
};
use std::{
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Debug)]
struct Inner;
impl fmt::Display for Inner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("inner cause")
    }
}
impl Error for Inner {}

#[derive(Debug)]
struct Outer(Inner);
impl fmt::Display for Outer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("outer failure")
    }
}
impl Error for Outer {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

#[test]
fn diagnostics_and_bounded_sinks_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Diagnostics>();
    assert_send_sync::<MemorySink>();
}

#[test]
fn structured_records_filter_levels_and_bound_fields() {
    let sink = Arc::new(MemorySink::new(8));
    let diagnostics = Diagnostics::with_config(
        sink.clone(),
        DiagnosticsConfig {
            min_level: Level::Warn,
            max_fields: 2,
            max_field_chars: 4,
            ..DiagnosticsConfig::default()
        },
    );
    diagnostics.info("test", "hidden");
    diagnostics.event_with(
        Level::Warn,
        "test",
        "visible",
        [
            Field::new("one", "abcdef"),
            Field::new("two", 2_u64),
            Field::new("three", true),
        ],
    );
    let records = sink.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].name, "visible");
    assert_eq!(records[0].fields.len(), 2);
    assert_eq!(records[0].fields[0].value.to_string(), "abcd");
    assert_eq!(records[0].fields[1].key, "two");
}

#[test]
fn memory_sink_is_bounded_and_reports_drops() {
    let sink = MemorySink::new(2);
    for index in 0..3 {
        sink.record(Record::new(Level::Info, "test", format!("record-{index}")));
    }
    assert_eq!(sink.len(), 2);
    assert_eq!(sink.dropped(), 1);
    let drained = sink.drain();
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[1].name, "record-2");
    assert!(sink.is_empty());
}

#[test]
fn spans_emit_duration_and_disabled_handles_are_noops() {
    let sink = Arc::new(MemorySink::new(4));
    let diagnostics = Diagnostics::new(sink.clone());
    let span = diagnostics.span("test", "work").field("job", 7_u64);
    assert!(span.id().is_some());
    span.finish();
    let records = sink.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].name, "work");
    assert!(records[0].duration_nanos.is_some());
    assert_eq!(records[0].fields[0].key, "job");

    let disabled = Diagnostics::disabled();
    assert!(!disabled.is_enabled(Level::Error));
    let span = disabled.span("test", "ignored");
    assert!(span.id().is_none());
    span.finish();
    assert_eq!(sink.records().len(), 1);
}

#[test]
fn failures_preserve_a_bounded_source_chain() {
    let sink = Arc::new(MemorySink::new(4));
    let diagnostics = Diagnostics::new(sink.clone());
    diagnostics.failure("test", "backend", &Outer(Inner));
    let record = &sink.records()[0];
    assert_eq!(record.level, Level::Error);
    let failure = record.failure.as_ref().unwrap();
    assert_eq!(failure.kind, "backend");
    assert_eq!(failure.message, "outer failure");
    assert_eq!(failure.chain, vec!["inner cause"]);
    let direct = Failure::from_error("x", &Outer(Inner), 1);
    assert_eq!(direct.chain, vec!["inner cause"]);
}

#[test]
fn callback_sink_and_counters_work_across_threads() {
    let count = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&count);
    let diagnostics = Diagnostics::new(Arc::new(CallbackSink(move |record: Record| {
        assert_eq!(record.level, Level::Info);
        observed.fetch_add(1, Ordering::Relaxed);
    })));
    let counter = diagnostics.counter("events");
    let shared = counter.clone();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let diagnostics = diagnostics.clone();
            let counter = shared.clone();
            scope.spawn(move || {
                for _ in 0..250 {
                    counter.increment();
                    diagnostics.info("worker", "tick");
                }
            });
        }
    });
    assert_eq!(counter.get(), 1000);
    assert_eq!(count.load(Ordering::Relaxed), 1000);

    let gauge = diagnostics.gauge("queue");
    gauge.set(4);
    gauge.add(-2);
    assert_eq!(gauge.get(), 2);
}

#[test]
fn text_format_contains_structured_context() {
    let record = Record::new(Level::Warn, "audio", "underrun")
        .field("frames", 12_u64)
        .duration_nanos(99);
    let text = record.format_text();
    assert!(text.contains("warn"));
    assert!(text.contains("audio: underrun"));
    assert!(text.contains("frames=12"));
    assert!(text.contains("duration_ns=99"));
}

#[cfg(feature = "timing")]
#[test]
fn frame_timer_reports_bounded_cpu_samples() {
    let mut timer = quasar_diagnostics::FrameTimer::new();
    assert_eq!(timer.begin_frame(), None);
    std::thread::sleep(std::time::Duration::from_millis(1));
    let elapsed = timer.begin_frame().expect("second frame has a duration");
    assert!(elapsed > 0);
    let stats = timer.stats();
    assert_eq!(stats.frames, 1);
    assert_eq!(stats.last_nanos, elapsed);
    assert_eq!(stats.min_nanos, elapsed);
    assert_eq!(stats.max_nanos, elapsed);
    timer.reset();
    assert_eq!(timer.stats().frames, 0);
}

#[cfg(feature = "serde")]
#[test]
fn records_serialize_structurally() {
    let record = Record::new(Level::Info, "test", "event").field("ok", true);
    let json = serde_json::to_string(&record).unwrap();
    let restored: Record = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, record);
    assert!(json.contains("\"kind\":\"bool\""));
}

#[test]
fn reusable_level_parser_rejects_invalid_filter_names() {
    use quasar_diagnostics::Level;
    for level in [
        Level::Trace,
        Level::Debug,
        Level::Info,
        Level::Warn,
        Level::Error,
    ] {
        assert_eq!(level.as_str().parse::<Level>().unwrap(), level);
    }
    assert_eq!("WARNING".parse::<Level>().unwrap(), Level::Warn);
    assert!("verbose".parse::<Level>().is_err());
}
