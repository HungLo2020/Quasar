use std::fmt;

/// Errors from font loading, shaping, layout and glyph rasterization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextError {
    NoFonts,
    InvalidFont(String),
    InvalidStyle(String),
    InvalidLayout(String),
    Io(String),
    AtlasFull,
}

impl fmt::Display for TextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFonts => formatter.write_str("at least one font must be registered"),
            Self::InvalidFont(message) => write!(formatter, "invalid font: {message}"),
            Self::InvalidStyle(message) => write!(formatter, "invalid text style: {message}"),
            Self::InvalidLayout(message) => write!(formatter, "invalid text layout: {message}"),
            Self::Io(message) => write!(formatter, "font I/O: {message}"),
            Self::AtlasFull => formatter.write_str("glyph atlas has no space left"),
        }
    }
}

impl std::error::Error for TextError {}

pub type TextResult<T> = Result<T, TextError>;
