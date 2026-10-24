use crate::{Failure, Field, Level, Record, Sink, SpanId};
use std::{
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
    time::Instant,
};

/// Bounds applied before a record reaches a sink. These limits protect log size
/// and avoid retaining unbounded caller-provided strings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticsConfig {
    pub min_level: Level,
    pub span_level: Level,
    pub max_fields: usize,
    pub max_field_chars: usize,
    pub max_name_chars: usize,
    pub max_failure_chain: usize,
}
impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            min_level: Level::Info,
            span_level: Level::Info,
            max_fields: 32,
            max_field_chars: 256,
            max_name_chars: 512,
            max_failure_chain: 16,
        }
    }
}

struct Inner {
    sink: Arc<dyn Sink>,
    config: DiagnosticsConfig,
    next_span: AtomicU64,
}

/// Cheap cloneable diagnostics handle. There is no global logger.
///
/// A [`Diagnostics::disabled`] handle has no sink and every emitter is a cheap
/// branch. Hooks in Quasar crates are feature-gated and use `Option<Diagnostics>`
/// or an equivalent disabled handle, so omitted instrumentation creates no
/// record or string formatting work.
#[derive(Clone)]
pub struct Diagnostics {
    inner: Option<Arc<Inner>>,
}
impl fmt::Debug for Diagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Diagnostics")
            .field("enabled", &self.inner.is_some())
            .field("config", &self.inner.as_ref().map(|inner| inner.config))
            .finish()
    }
}
impl Default for Diagnostics {
    fn default() -> Self {
        Self::disabled()
    }
}
impl Diagnostics {
    pub fn disabled() -> Self {
        Self { inner: None }
    }
    pub fn new(sink: Arc<dyn Sink>) -> Self {
        Self::with_config(sink, DiagnosticsConfig::default())
    }
    pub fn with_config(sink: Arc<dyn Sink>, config: DiagnosticsConfig) -> Self {
        Self {
            inner: Some(Arc::new(Inner {
                sink,
                config,
                next_span: AtomicU64::new(1),
            })),
        }
    }
    pub fn is_enabled(&self, level: Level) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|inner| level >= inner.config.min_level)
    }
    pub fn config(&self) -> DiagnosticsConfig {
        self.inner
            .as_ref()
            .map_or_else(DiagnosticsConfig::default, |inner| inner.config)
    }
    pub fn sink(&self) -> Option<&Arc<dyn Sink>> {
        self.inner.as_ref().map(|inner| &inner.sink)
    }
    pub fn flush(&self) {
        if let Some(inner) = &self.inner {
            inner.sink.flush();
        }
    }
    /// Emits a pre-built record if the level is enabled.
    pub fn record(&self, mut record: Record) {
        let Some(inner) = &self.inner else {
            return;
        };
        if record.level < inner.config.min_level {
            return;
        }
        record.truncate(
            inner.config.max_fields,
            inner.config.max_field_chars,
            inner.config.max_name_chars,
        );
        if let Some(failure) = &mut record.failure {
            failure.kind.truncate_chars(inner.config.max_name_chars);
            failure.message.truncate_chars(inner.config.max_name_chars);
            failure.chain.truncate(inner.config.max_failure_chain);
            for cause in &mut failure.chain {
                cause.truncate_chars(inner.config.max_name_chars);
            }
        }
        inner.sink.record(record);
    }
    pub fn event(&self, level: Level, target: &str, name: &str) {
        if self.is_enabled(level) {
            self.record(Record::new(level, target, name));
        }
    }
    pub fn event_with(
        &self,
        level: Level,
        target: &str,
        name: &str,
        fields: impl IntoIterator<Item = Field>,
    ) {
        if self.is_enabled(level) {
            let mut record = Record::new(level, target, name);
            record.fields.extend(fields);
            self.record(record);
        }
    }
    pub fn trace(&self, target: &str, name: &str) {
        self.event(Level::Trace, target, name);
    }
    pub fn debug(&self, target: &str, name: &str) {
        self.event(Level::Debug, target, name);
    }
    pub fn info(&self, target: &str, name: &str) {
        self.event(Level::Info, target, name);
    }
    pub fn warn(&self, target: &str, name: &str) {
        self.event(Level::Warn, target, name);
    }
    pub fn error(&self, target: &str, name: &str) {
        self.event(Level::Error, target, name);
    }
    /// Captures a failure with its source chain without retaining a borrow.
    pub fn failure(&self, target: &str, kind: &str, error: &dyn Error) {
        if !self.is_enabled(Level::Error) {
            return;
        }
        let chain = self
            .inner
            .as_ref()
            .map_or(16, |inner| inner.config.max_failure_chain);
        self.record(
            Record::new(Level::Error, target, "failure")
                .failure(Failure::from_error(kind, error, chain)),
        );
    }
    pub fn failure_with_fields(
        &self,
        target: &str,
        kind: &str,
        error: &dyn Error,
        fields: impl IntoIterator<Item = Field>,
    ) {
        if !self.is_enabled(Level::Error) {
            return;
        }
        let chain = self
            .inner
            .as_ref()
            .map_or(16, |inner| inner.config.max_failure_chain);
        let mut record = Record::new(Level::Error, target, "failure")
            .failure(Failure::from_error(kind, error, chain));
        record.fields.extend(fields);
        self.record(record);
    }
    /// Starts a span. The span emits one record with duration when dropped or
    /// finished. Disabled spans allocate no id and call no clock.
    pub fn span(&self, target: &str, name: impl Into<String>) -> Span {
        let enabled = self.is_enabled(self.config().span_level);
        let id = enabled.then(|| {
            let inner = self
                .inner
                .as_ref()
                .expect("enabled diagnostics has an inner");
            SpanId(inner.next_span.fetch_add(1, Ordering::Relaxed))
        });
        Span {
            diagnostics: self.clone(),
            target: target.to_owned(),
            name: name.into(),
            id,
            started: id.map(|_| Instant::now()),
            fields: Vec::new(),
        }
    }
    /// Creates an independent counter; counters are not globally registered.
    pub fn counter(&self, name: impl Into<String>) -> Counter {
        Counter::new(name)
    }
    pub fn gauge(&self, name: impl Into<String>) -> Gauge {
        Gauge::new(name)
    }
}

/// Completed span. Fields added with [`Span::field`] are emitted with the
/// duration record.
pub struct Span {
    diagnostics: Diagnostics,
    target: String,
    name: String,
    id: Option<SpanId>,
    started: Option<Instant>,
    fields: Vec<Field>,
}
impl Span {
    pub fn id(&self) -> Option<SpanId> {
        self.id
    }
    pub fn field(mut self, key: impl Into<String>, value: impl Into<crate::Value>) -> Self {
        if self.id.is_some() {
            self.fields.push(Field::new(key, value));
        }
        self
    }
    pub fn finish(mut self) {
        self.emit();
    }
    fn emit(&mut self) {
        let (Some(id), Some(started)) = (self.id.take(), self.started.take()) else {
            return;
        };
        let mut record = Record::new(
            self.diagnostics.config().span_level,
            &self.target,
            &self.name,
        )
        .span(id)
        .duration_nanos(started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
        record.fields = std::mem::take(&mut self.fields);
        self.diagnostics.record(record);
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        self.emit();
    }
}

/// Monotonic unsigned counter safe to share between threads.
#[derive(Clone, Debug)]
pub struct Counter {
    name: Arc<str>,
    value: Arc<AtomicU64>,
}
impl Counter {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into().into(),
            value: Arc::new(AtomicU64::new(0)),
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn add(&self, value: u64) -> u64 {
        self.value
            .fetch_add(value, Ordering::Relaxed)
            .saturating_add(value)
    }
    pub fn increment(&self) -> u64 {
        self.add(1)
    }
    pub fn set(&self, value: u64) {
        self.value.store(value, Ordering::Relaxed);
    }
    pub fn reset(&self) {
        self.value.store(0, Ordering::Relaxed);
    }
    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }
}

/// Signed gauge for queue depths and other values that can rise and fall.
#[derive(Clone, Debug)]
pub struct Gauge {
    name: Arc<str>,
    value: Arc<AtomicI64>,
}
impl Gauge {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into().into(),
            value: Arc::new(AtomicI64::new(0)),
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn set(&self, value: i64) {
        self.value.store(value, Ordering::Relaxed);
    }
    pub fn add(&self, value: i64) -> i64 {
        self.value.fetch_add(value, Ordering::Relaxed) + value
    }
    pub fn get(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
    }
}

/// Process-local CPU frame timing. Enabled with the `timing` feature so builds
/// that do not opt in perform no clock calls from frame loops.
#[cfg(feature = "timing")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameStats {
    pub frames: u64,
    pub last_nanos: u64,
    pub min_nanos: u64,
    pub max_nanos: u64,
    pub total_nanos: u64,
    /// Integer exponential moving average with a 1/8 new-sample weight.
    pub ema_nanos: u64,
}
#[cfg(feature = "timing")]
#[derive(Clone, Debug, Default)]
pub struct FrameTimer {
    last: Option<Instant>,
    stats: FrameStats,
}
#[cfg(feature = "timing")]
impl FrameTimer {
    pub fn new() -> Self {
        Self::default()
    }
    /// Marks the start of the next frame and returns the previous frame's
    /// duration. The first call returns `None`.
    pub fn begin_frame(&mut self) -> Option<u64> {
        let now = Instant::now();
        let last = self.last.replace(now)?;
        let nanos = now
            .duration_since(last)
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        self.stats.frames = self.stats.frames.saturating_add(1);
        self.stats.last_nanos = nanos;
        self.stats.total_nanos = self.stats.total_nanos.saturating_add(nanos);
        if self.stats.frames == 1 {
            self.stats.min_nanos = nanos;
            self.stats.max_nanos = nanos;
            self.stats.ema_nanos = nanos;
        } else {
            self.stats.min_nanos = self.stats.min_nanos.min(nanos);
            self.stats.max_nanos = self.stats.max_nanos.max(nanos);
            self.stats.ema_nanos = self.stats.ema_nanos - self.stats.ema_nanos / 8 + nanos / 8;
        }
        Some(nanos)
    }
    pub fn stats(&self) -> FrameStats {
        self.stats
    }
    pub fn reset(&mut self) {
        self.last = None;
        self.stats = FrameStats::default();
    }
}

trait TruncateChars {
    fn truncate_chars(&mut self, max: usize);
}
impl TruncateChars for String {
    fn truncate_chars(&mut self, max: usize) {
        if self.len() <= max {
            return;
        }
        let mut boundary = max;
        while boundary > 0 && !self.is_char_boundary(boundary) {
            boundary -= 1;
        }
        self.truncate(boundary);
    }
}
