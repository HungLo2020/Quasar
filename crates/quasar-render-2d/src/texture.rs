use crate::{Result, invalid};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
static NEXT: AtomicU64 = AtomicU64::new(2);
/// Opaque, non-reused texture identity. Builtins are available in every store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextureId(pub(crate) u64);
impl TextureId {
    pub const WHITE: Self = Self(0);
    pub const FONT: Self = Self(1);
}
#[derive(Debug, Clone)]
pub struct Image {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Arc<[u8]>,
}
impl Image {
    pub fn new(width: u32, height: u32, rgba: impl Into<Arc<[u8]>>) -> Result<Self> {
        let rgba = rgba.into();
        if width == 0
            || height == 0
            || width > 16384
            || height > 16384
            || rgba.len() as u64 != u64::from(width) * u64::from(height) * 4
        {
            return Err(invalid("invalid RGBA image dimensions or byte count"));
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}
/// Immutable image assets; GPU renderers cache uploads by identity, not per frame.
/// Removing an asset invalidates future draw lists referencing it.
#[derive(Clone, Debug)]
pub struct Textures {
    pub(crate) images: BTreeMap<TextureId, Image>,
    external: BTreeMap<TextureId, (u32, u32)>,
}
impl Default for Textures {
    fn default() -> Self {
        Self {
            external: BTreeMap::new(),
            images: BTreeMap::from([
                (TextureId::WHITE, Image::new(1, 1, vec![255; 4]).unwrap()),
                (TextureId::FONT, crate::font::atlas()),
            ]),
        }
    }
}
impl Textures {
    pub fn insert(&mut self, image: Image) -> TextureId {
        let id = TextureId(NEXT.fetch_add(1, Ordering::Relaxed));
        self.images.insert(id, image);
        id
    }
    /// Declares GPU-only image metadata. A GPU renderer must register the image.
    pub fn declare_external(&mut self, width: u32, height: u32) -> Result<TextureId> {
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(invalid("invalid external image dimensions"));
        }
        let id = TextureId(NEXT.fetch_add(1, Ordering::Relaxed));
        self.external.insert(id, (width, height));
        Ok(id)
    }
    pub fn dimensions(&self, id: TextureId) -> Option<(u32, u32)> {
        self.images
            .get(&id)
            .map(|i| (i.width, i.height))
            .or_else(|| self.external.get(&id).copied())
    }
    pub fn get(&self, id: TextureId) -> Option<&Image> {
        self.images.get(&id)
    }
    pub fn remove(&mut self, id: TextureId) -> bool {
        if id.0 < 2 {
            false
        } else {
            self.images.remove(&id).is_some() | self.external.remove(&id).is_some()
        }
    }
}

#[cfg(feature = "png")]
impl Image {
    /// Bounded CPU decode of PNG RGB/RGBA/grayscale/indexed images to RGBA8.    /// No graphics device is required. Both decoder and output have 64 MiB caps.
    pub fn decode_png(reader: impl std::io::Read) -> Result<Self> {
        let mut decoder = png::Decoder::new(reader);
        decoder.set_limits(png::Limits {
            bytes: 64 * 1024 * 1024,
        });
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .map_err(|e| invalid(format!("PNG header: {e}")))?;
        let info = reader.info();
        let bytes = u64::from(info.width) * u64::from(info.height) * 4;
        if info.width == 0
            || info.height == 0
            || info.width > 16384
            || info.height > 16384
            || bytes > 64 * 1024 * 1024
        {
            return Err(invalid("PNG exceeds image bounds or 64 MiB decoded limit"));
        }
        if info.animation_control.is_some() {
            return Err(invalid(
                "APNG is not supported; use an explicit sprite atlas animation",
            ));
        }
        let mut buffer = vec![0; reader.output_buffer_size()];
        let frame = reader
            .next_frame(&mut buffer)
            .map_err(|e| invalid(format!("PNG pixels: {e}")))?;
        let source = &buffer[..frame.buffer_size()];
        let rgba = match frame.color_type {
            png::ColorType::Rgba => source.to_vec(),
            png::ColorType::Rgb => source
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::Grayscale => source.iter().flat_map(|v| [*v, *v, *v, 255]).collect(),
            png::ColorType::GrayscaleAlpha => source
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Indexed => return Err(invalid("PNG palette was not expanded")),
        };
        Self::new(frame.width, frame.height, rgba)
    }
    pub fn load_png(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let file = std::fs::File::open(path.as_ref())
            .map_err(|e| invalid(format!("{}: {e}", path.as_ref().display())))?;
        Self::decode_png(std::io::BufReader::new(file))
    }
}

/// Saves tightly packed, top-to-bottom RGBA8 pixels without a graphics device.
#[cfg(feature = "png")]
pub fn write_png(
    path: impl AsRef<std::path::Path>,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<()> {
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(invalid("PNG dimensions must be 1..16384"));
    }
    let expected = u64::from(width) * u64::from(height) * 4;
    if rgba.len() as u64 != expected {
        return Err(invalid("RGBA length does not match PNG dimensions"));
    }
    let file = std::fs::File::create(path.as_ref())
        .map_err(|e| invalid(format!("{}: {e}", path.as_ref().display())))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(rgba))
        .map_err(|e| invalid(format!("PNG write: {e}")))?;
    Ok(())
}
