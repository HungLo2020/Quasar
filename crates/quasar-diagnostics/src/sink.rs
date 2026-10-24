use crate::Record;
use std::{
    collections::VecDeque,
    io::Write,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

/// Destination for completed records. Implementations must be thread-safe and
/// must not call back into the emitting [`crate::Diagnostics`] instance.
pub trait Sink: Send + Sync {
    fn record(&self, record: Record);
    fn flush(&self) {}
}

/// Intentionally discards records.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullSink;
impl Sink for NullSink {
    fn record(&self, _: Record) {}
}

/// Bounded in-memory ring buffer for tests, overlays and crash reports.
#[derive(Debug)]
pub struct MemorySink {
    capacity: usize,
    records: Mutex<VecDeque<Record>>,
    dropped: AtomicU64,
}
impl MemorySink {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            records: Mutex::new(VecDeque::new()),
            dropped: AtomicU64::new(0),
        }
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn len(&self) -> usize {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
    pub fn records(&self) -> Vec<Record> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }
    pub fn drain(&self) -> Vec<Record> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}
impl Sink for MemorySink {
    fn record(&self, record: Record) {
        if self.capacity == 0 {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if records.len() == self.capacity {
            records.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        records.push_back(record);
    }
}

/// Calls a user closure for each record. The closure runs on the emitting
/// thread and should avoid blocking.
pub struct CallbackSink<F>(pub F);
impl<F> Sink for CallbackSink<F>
where
    F: Fn(Record) + Send + Sync,
{
    fn record(&self, record: Record) {
        (self.0)(record);
    }
}

/// Human-readable append-only sink. Each record is one line.
pub struct TextSink<W> {
    writer: Mutex<W>,
}
impl<W> TextSink<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer: Mutex::new(writer),
        }
    }
}
impl<W: Write + Send> Sink for TextSink<W> {
    fn record(&self, record: Record) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writeln!(writer, "{}", record.format_text());
        }
    }
    fn flush(&self) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.flush();
        }
    }
}
