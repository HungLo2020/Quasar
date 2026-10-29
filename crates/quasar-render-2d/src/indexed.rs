//! Portable indexed color: index bytes use the red channel of an RGBA8 texture.
//! Palette lookup always uses nearest texels, independently of RGBA filtering.
use crate::{Color, Image, Result, TextureId, invalid};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct IndexedImage {
    image: Image,
}
impl IndexedImage {
    pub fn new(width: u32, height: u32, indices: impl AsRef<[u8]>) -> Result<Self> {
        let indices = indices.as_ref();
        if width == 0
            || height == 0
            || width > 16384
            || height > 16384
            || indices.len() as u64 != u64::from(width) * u64::from(height)
        {
            return Err(invalid("invalid indexed image dimensions or byte count"));
        }
        let rgba: Vec<_> = indices.iter().flat_map(|&i| [i, 0, 0, 255]).collect();
        Ok(Self {
            image: Image::new(width, height, rgba)?,
        })
    }
    /// Shared immutable backing, ready for Textures::insert. No conversion per palette/frame.
    pub fn image(&self) -> &Image {
        &self.image
    }
}
/// A reusable bank of 1..256 RGBA colors. Indices beyond its length are transparent.
#[derive(Debug, Clone)]
pub struct Palette {
    colors: Arc<[Color]>,
}
impl Palette {
    pub fn new(colors: impl Into<Arc<[Color]>>) -> Result<Self> {
        let colors = colors.into();
        if colors.is_empty() || colors.len() > 256 {
            return Err(invalid("palette needs 1..256 colors"));
        }
        Ok(Self { colors })
    }
    pub fn colors(&self) -> &[Color] {
        &self.colors
    }
    pub fn image(&self) -> Image {
        let mut pixels = vec![0; 1024];
        for (p, c) in pixels.chunks_exact_mut(4).zip(self.colors.iter()) {
            p.copy_from_slice(&c.0);
        }
        Image::new(256, 1, pixels).unwrap()
    }
    /// Returns a new small palette; the indexed image is untouched. Positive steps
    /// rotate the selected half-open range to the right. Suitable for integer clocks.
    pub fn cycled(&self, range: std::ops::Range<usize>, steps: u64) -> Result<Self> {
        if range.start >= range.end || range.end > self.colors.len() {
            return Err(invalid("invalid palette cycle range"));
        }
        let mut colors = self.colors.to_vec();
        let len = range.len();
        colors[range].rotate_right((steps % len as u64) as usize);
        Self::new(colors)
    }
}
/// Captured per draw. Palette textures must be 256x1 RGBA8. The index source is
/// sampled nearest; transparent_index is discarded before palette/tint/priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteBinding {
    pub texture: TextureId,
    pub transparent_index: Option<u8>,
}
