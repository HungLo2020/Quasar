//! Versioned tilemap data, Tiled JSON interchange and indexed CPU queries.
//! Optional 2D drawing and previews never require a GPU or a GUI.
mod format;
mod io;
#[cfg(feature = "render-2d")]
pub mod render;
mod runtime;
pub use format::*;
pub use io::{AssetRoot, load, parse, to_tiled_json, write_bundle};
pub use runtime::{Bounds, Cell, EditStats, QueryStats, Runtime, TileEdit};
use std::fmt;
#[derive(Debug, Clone, serde::Serialize)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn error(code: &'static str, message: impl Into<String>) -> Error {
    Error {
        code,
        message: message.into(),
    }
}
pub(crate) fn json_error(e: serde_json::Error) -> Error {
    error("json", e.to_string())
}
