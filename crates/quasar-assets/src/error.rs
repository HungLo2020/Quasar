use crate::AssetPath;
use std::{error::Error, fmt};

/// Stable error category. Messages are diagnostic; match on the kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetErrorKind {
    InvalidPath,
    NotMounted,
    NotFound,
    Io,
    Schema,
    Loader,
    Budget,
    Dependency,
    Busy,
    Configuration,
    DuplicateMount,
    WrongType,
    QueueFull,
    Cancelled,
}

/// Explicit failure from path validation, mounting, loading or cache management.
#[derive(Debug)]
pub struct AssetError {
    kind: AssetErrorKind,
    path: Option<AssetPath>,
    message: String,
}
impl AssetError {
    pub fn new(kind: AssetErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            path: None,
            message: message.into(),
        }
    }
    pub fn at(kind: AssetErrorKind, path: &AssetPath, message: impl Into<String>) -> Self {
        Self {
            kind,
            path: Some(path.clone()),
            message: message.into(),
        }
    }
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new(AssetErrorKind::InvalidPath, message)
    }
    pub fn kind(&self) -> AssetErrorKind {
        self.kind
    }
    pub fn path(&self) -> Option<&AssetPath> {
        self.path.as_ref()
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}
impl fmt::Display for AssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(
                formatter,
                "{} @ {path}: {}",
                kind_name(self.kind),
                self.message
            )
        } else {
            write!(formatter, "{}: {}", kind_name(self.kind), self.message)
        }
    }
}
impl Error for AssetError {}

fn kind_name(kind: AssetErrorKind) -> &'static str {
    match kind {
        AssetErrorKind::InvalidPath => "invalid path",
        AssetErrorKind::NotMounted => "not mounted",
        AssetErrorKind::NotFound => "not found",
        AssetErrorKind::Io => "io",
        AssetErrorKind::Schema => "schema",
        AssetErrorKind::Loader => "loader",
        AssetErrorKind::Budget => "budget",
        AssetErrorKind::Dependency => "dependency",
        AssetErrorKind::Busy => "busy",
        AssetErrorKind::Configuration => "configuration",
        AssetErrorKind::DuplicateMount => "duplicate mount",
        AssetErrorKind::WrongType => "wrong type",
        AssetErrorKind::QueueFull => "queue full",
        AssetErrorKind::Cancelled => "cancelled",
    }
}
