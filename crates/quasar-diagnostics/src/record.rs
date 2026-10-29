use std::{
    error::Error,
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

/// Severity ordered from most verbose to most severe.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
impl Level {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}
impl fmt::Display for Level {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Structured field value. Strings are owned so a record can cross threads.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(tag = "kind", content = "value", rename_all = "snake_case")
)]
pub enum Value {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
}
impl Value {
    pub fn str(value: impl Into<String>) -> Self {
        Self::Str(value.into())
    }
}
impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => formatter.write_str("null"),
            Self::Bool(value) => value.fmt(formatter),
            Self::I64(value) => value.fmt(formatter),
            Self::U64(value) => value.fmt(formatter),
            Self::F64(value) => value.fmt(formatter),
            Self::Str(value) => value.fmt(formatter),
        }
    }
}
impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}
impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}
impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Self::U64(value)
    }
}
impl From<usize> for Value {
    fn from(value: usize) -> Self {
        Self::U64(value as u64)
    }
}
impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::F64(value)
    }
}
impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Str(value.to_owned())
    }
}
impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

/// A named structured field.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Field {
    pub key: String,
    pub value: Value,
}
impl Field {
    pub fn new(key: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// Error information preserved as a diagnostic without keeping a borrowed
/// error alive. The chain is bounded by the diagnostics configuration.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Failure {
    pub kind: String,
    pub message: String,
    pub chain: Vec<String>,
}
impl Failure {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            chain: Vec::new(),
        }
    }
    /// Captures `Display` and the source chain, stopping at `max_chain`.
    pub fn from_error(kind: impl Into<String>, error: &dyn Error, max_chain: usize) -> Self {
        let mut chain = Vec::new();
        let mut source = error.source();
        while let Some(error) = source {
            if chain.len() >= max_chain {
                break;
            }
            chain.push(error.to_string());
            source = error.source();
        }
        Self {
            kind: kind.into(),
            message: error.to_string(),
            chain,
        }
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind, self.message)?;
        for cause in &self.chain {
            write!(formatter, " <- {cause}")?;
        }
        Ok(())
    }
}

/// Identity assigned by a [`crate::Diagnostics`] instance.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct SpanId(pub u64);

/// Structured diagnostic record delivered to a [`crate::Sink`].
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Record {
    pub timestamp_unix_nanos: u64,
    pub level: Level,
    pub target: String,
    pub name: String,
    pub fields: Vec<Field>,
    pub span: Option<SpanId>,
    /// Present for completed spans.
    pub duration_nanos: Option<u64>,
    pub failure: Option<Failure>,
}
impl Record {
    pub fn new(level: Level, target: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            timestamp_unix_nanos: now_unix_nanos(),
            level,
            target: target.into(),
            name: name.into(),
            fields: Vec::new(),
            span: None,
            duration_nanos: None,
            failure: None,
        }
    }
    pub fn field(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.fields.push(Field::new(key, value));
        self
    }
    pub fn failure(mut self, failure: Failure) -> Self {
        self.failure = Some(failure);
        self
    }
    pub fn span(mut self, span: SpanId) -> Self {
        self.span = Some(span);
        self
    }
    pub fn duration_nanos(mut self, duration: u64) -> Self {
        self.duration_nanos = Some(duration);
        self
    }
    /// Human-readable single line used by the text sink. Structured fields remain
    /// available to custom sinks.
    pub fn format_text(&self) -> String {
        let mut text = format!(
            "{} {:<5} {}: {}",
            self.timestamp_unix_nanos, self.level, self.target, self.name
        );
        for field in &self.fields {
            text.push(' ');
            text.push_str(&field.key);
            text.push('=');
            text.push_str(&field.value.to_string());
        }
        if let Some(span) = self.span {
            text.push_str(&format!(" span={}", span.0));
        }
        if let Some(duration) = self.duration_nanos {
            text.push_str(&format!(" duration_ns={duration}"));
        }
        if let Some(failure) = &self.failure {
            text.push_str(&format!(" failure={failure}"));
        }
        text
    }
    pub(crate) fn truncate(&mut self, max_fields: usize, max_field_chars: usize, max_name: usize) {
        if self.name.len() > max_name {
            self.name.truncate_floor(max_name);
        }
        if self.target.len() > max_name {
            self.target.truncate_floor(max_name);
        }
        self.fields.truncate(max_fields);
        for field in &mut self.fields {
            if field.key.len() > max_field_chars {
                field.key.truncate_floor(max_field_chars);
            }
            if let Value::Str(value) = &mut field.value
                && value.len() > max_field_chars
            {
                value.truncate_floor(max_field_chars);
            }
        }
    }
}

fn now_unix_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map_or(0, |duration| {
            duration.as_nanos().min(u128::from(u64::MAX)) as u64
        })
}

trait TruncateFloor {
    fn truncate_floor(&mut self, max: usize);
}
impl TruncateFloor for String {
    fn truncate_floor(&mut self, max: usize) {
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

/// Invalid textual diagnostics level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseLevelError;
impl std::fmt::Display for ParseLevelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("expected trace, debug, info, warn or error diagnostics level")
    }
}
impl std::error::Error for ParseLevelError {}
impl std::str::FromStr for Level {
    type Err = ParseLevelError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "trace" => Ok(Self::Trace),
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" | "warning" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            _ => Err(ParseLevelError),
        }
    }
}
