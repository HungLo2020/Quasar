//! Error types shared by decoding, mixing, scheduling and device output.

use std::{error::Error, fmt, io};

/// Result alias used throughout `quasar-audio`.
pub type AudioResult<T> = Result<T, AudioError>;

/// Stable, backend-independent audio errors.
#[derive(Debug)]
pub enum AudioError {
    /// A caller supplied an invalid configuration or handle.
    InvalidArgument(String),
    /// An asset could not be decoded or violated a resource limit.
    Decode(String),
    /// The requested capability is not supported by this build or asset.
    Unsupported(String),
    /// The operating system has no usable output device.
    NoDevice,
    /// A previously opened device was lost.
    DeviceLost(String),
    /// The native backend returned an error.
    Device(String),
    /// The command queue is full; the caller may retry later.
    QueueFull,
    /// The requested voice could not start under the configured limits.
    VoiceLimit,
    /// A sound, bus, or handle was not found.
    NotFound,
    /// File I/O failed.
    Io(io::Error),
    /// An internal invariant was violated.
    Internal(String),
}
impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(message) => write!(f, "invalid audio argument: {message}"),
            Self::Decode(message) => write!(f, "audio decode failed: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported audio capability: {message}"),
            Self::NoDevice => f.write_str("no audio output device is available"),
            Self::DeviceLost(message) => write!(f, "audio device lost: {message}"),
            Self::Device(message) => write!(f, "audio device error: {message}"),
            Self::QueueFull => f.write_str("audio command queue is full"),
            Self::VoiceLimit => f.write_str("audio voice limit reached"),
            Self::NotFound => f.write_str("audio resource was not found"),
            Self::Io(error) => write!(f, "audio I/O failed: {error}"),
            Self::Internal(message) => write!(f, "internal audio error: {message}"),
        }
    }
}
impl Error for AudioError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
impl From<io::Error> for AudioError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<hound::Error> for AudioError {
    fn from(error: hound::Error) -> Self {
        match error {
            hound::Error::IoError(error) => Self::Io(error),
            other => Self::Decode(other.to_string()),
        }
    }
}
impl From<lewton::VorbisError> for AudioError {
    fn from(error: lewton::VorbisError) -> Self {
        Self::Decode(error.to_string())
    }
}
