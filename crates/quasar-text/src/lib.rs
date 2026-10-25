//! Unicode font loading, shaping, text layout and CPU glyph atlases.
//!
//! `quasar-text` is independent of Vulkan, windows, ECS, editors and app
//! runners. It registers caller-owned font bytes, shapes with Harfrust through
//! `cosmic-text`, lays out wrapped and bidirectional text, and rasterizes glyphs
//! into CPU atlas pages. `quasar-render-2d` can upload those pages through its
//! optional `text` feature; games that only need the built-in bitmap font do not
//! depend on this crate.
//!
//! ```
//! use quasar_text::{FontData, FontStack, TextEngine, TextStyle};
//!
//! let mut fonts = FontStack::new();
//! fonts.add_bytes("demo", include_bytes!("../tests/fonts/DejaVuSans.ttf").as_slice());
//! let mut engine = fonts.build("en-US").unwrap();
//! let layout = engine.layout("Hello, world", &TextStyle::new(16.0), None).unwrap();
//! assert!(layout.width() > 0.0);
//! assert!(!layout.glyphs().is_empty());
//! ```
mod error;
mod font;
mod layout;
mod raster;
mod style;

pub use error::{TextError, TextResult};
pub use font::{FontData, FontFace, FontStack, TextEngine};
pub use layout::{
    Caret, FontId, Glyph, LayoutStats, Line, SelectionRect, SharedLayout, TextLayout,
};
pub use raster::{
    AtlasConfig, AtlasEntry, AtlasPage, GlyphAtlas, GlyphCache, GlyphFrameStats, GlyphKey,
    GlyphRasterizer, RasterGlyph, TextFrame, TextQuad,
};
pub use style::{TextAlign, TextStyle, TextWrap};

/// Re-export the font database used for loading and matching.
pub use cosmic_text::fontdb;
