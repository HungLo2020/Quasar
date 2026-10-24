//! Structured diagnostics without a compulsory runtime, global logger, or
//! background thread.
//!
//! A [`Diagnostics`] handle owns an explicit [`Sink`]. Clone the handle into the
//! subsystems that should emit records; use [`Diagnostics::disabled`] or omit an
//! optional hook when diagnostics are not wanted. Quasar integrations are
//! feature-gated, so disabled builds do not format records or call clocks.
//!
//! ```
//! use quasar_diagnostics::{Diagnostics, Field, Level, MemorySink};
//! use std::sync::Arc;
//!
//! let sink = Arc::new(MemorySink::new(16));
//! let diagnostics = Diagnostics::new(sink.clone());
//! diagnostics.event_with(
//!     Level::Info,
//!     "game",
//!     "ready",
//!     [Field::new("assets", 12_u64)],
//! );
//! assert_eq!(sink.records().len(), 1);
//! ```
mod diagnostics;
mod record;
mod sink;

pub use diagnostics::{Counter, Diagnostics, DiagnosticsConfig, Gauge, Span};
#[cfg(feature = "timing")]
pub use diagnostics::{FrameStats, FrameTimer};
pub use record::{Failure, Field, Level, Record, SpanId, Value};
pub use sink::{CallbackSink, MemorySink, NullSink, Sink, TextSink};
