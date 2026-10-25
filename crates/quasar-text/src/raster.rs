use crate::layout::{FontId, Glyph, TextLayout};
use crate::{TextEngine, TextError, TextResult};
use cosmic_text::{CacheKey, CacheKeyFlags, SubpixelBin, SwashCache, SwashContent, Weight};
use std::collections::HashMap;

/// Stable key for a rasterized glyph at a specific size, weight and subpixel
/// position. Equivalent key values reuse one atlas entry and one swash bitmap.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlyphKey {
    pub font: FontId,
    pub glyph_id: u16,
    pub size_bits: u32,
    pub weight: u16,
    pub flags: u32,
    pub x_bin: u8,
    pub y_bin: u8,
}

impl GlyphKey {
    fn from_glyph(glyph: &Glyph) -> (Self, CacheKey, i32, i32) {
        let (cache_key, x, y) = CacheKey::new(
            glyph.font.0,
            glyph.glyph_id,
            glyph.font_size,
            (glyph.x, glyph.baseline),
            Weight(glyph.font_weight),
            CacheKeyFlags::from_bits_truncate(glyph.raster_flags),
        );
        (
            Self {
                font: glyph.font,
                glyph_id: glyph.glyph_id,
                size_bits: glyph.font_size.to_bits(),
                weight: glyph.font_weight,
                flags: glyph.raster_flags,
                x_bin: bin(cache_key.x_bin),
                y_bin: bin(cache_key.y_bin),
            },
            cache_key,
            x,
            y,
        )
    }
}

fn bin(bin: SubpixelBin) -> u8 {
    match bin {
        SubpixelBin::Zero => 0,
        SubpixelBin::One => 1,
        SubpixelBin::Two => 2,
        SubpixelBin::Three => 3,
    }
}

/// Raw glyph bitmap. Mask glyphs are grayscale coverage; color glyphs are RGBA.
#[derive(Clone, Debug, PartialEq)]
pub struct RasterGlyph {
    pub key: GlyphKey,
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub color: bool,
    pub data: Vec<u8>,
}

/// Rasterizes glyphs through swash while retaining swash's own key cache.
#[derive(Debug)]
pub struct GlyphRasterizer {
    cache: SwashCache,
    rasterized: u64,
    cache_hits: u64,
}

impl Default for GlyphRasterizer {
    fn default() -> Self {
        Self {
            cache: SwashCache::new(),
            rasterized: 0,
            cache_hits: 0,
        }
    }
}

impl GlyphRasterizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stats(&self) -> (u64, u64) {
        (self.rasterized, self.cache_hits)
    }

    pub fn rasterize(&mut self, engine: &mut TextEngine, glyph: &Glyph) -> Option<RasterGlyph> {
        let (key, cache_key, _, _) = GlyphKey::from_glyph(glyph);
        let existed = self.cache.image_cache.contains_key(&cache_key);
        let image = self
            .cache
            .get_image(engine.font_system_mut(), cache_key)
            .clone();
        if existed {
            self.cache_hits += 1;
        } else {
            self.rasterized += 1;
        }
        let image = image.as_ref()?;
        let (width, height) = (image.placement.width, image.placement.height);
        if width == 0 || height == 0 {
            return None;
        }
        let color = matches!(image.content, SwashContent::Color);
        let data = match image.content {
            SwashContent::Mask => image.data.clone(),
            SwashContent::Color => image.data.clone(),
            SwashContent::SubpixelMask => return None,
        };
        Some(RasterGlyph {
            key,
            left: image.placement.left,
            top: image.placement.top,
            width,
            height,
            color,
            data,
        })
    }
}

/// Configuration for a CPU glyph atlas.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AtlasConfig {
    pub size: u32,
    pub padding: u32,
    pub max_pages: usize,
}

impl Default for AtlasConfig {
    fn default() -> Self {
        Self {
            size: 1024,
            padding: 1,
            max_pages: 8,
        }
    }
}

impl AtlasConfig {
    fn validate(self) -> TextResult<Self> {
        if self.size < 64 || self.size > 8192 || !self.size.is_power_of_two() {
            return Err(TextError::InvalidStyle(
                "atlas size must be a power of two in 64..=8192".into(),
            ));
        }
        if self.padding > 8 {
            return Err(TextError::InvalidStyle(
                "atlas padding must be at most 8 pixels".into(),
            ));
        }
        if self.max_pages == 0 {
            return Err(TextError::InvalidStyle(
                "atlas must allow at least one page".into(),
            ));
        }
        Ok(self)
    }
}

/// One immutable atlas page. Pages grow before the frame is drawn and are only
/// uploaded again when their version changes.
#[derive(Clone, Debug)]
pub struct AtlasPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub version: u64,
    pub dirty: bool,
    shelf_x: u32,
    shelf_y: u32,
    shelf_height: u32,
}

impl AtlasPage {
    fn new(config: AtlasConfig) -> Self {
        Self {
            width: config.size,
            height: config.size,
            rgba: vec![0; (config.size * config.size * 4) as usize],
            version: 0,
            dirty: false,
            shelf_x: config.padding,
            shelf_y: config.padding,
            shelf_height: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtlasEntry {
    pub page: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub color: bool,
}

/// CPU-only shelf glyph atlas. It owns no GPU object; callers can upload changed
/// pages once per frame or use the optional `quasar-render-2d` GPU adapter.
#[derive(Clone, Debug)]
pub struct GlyphAtlas {
    config: AtlasConfig,
    pages: Vec<AtlasPage>,
    entries: HashMap<GlyphKey, AtlasEntry>,
    rasterized: u64,
    cache_hits: u64,
}

impl GlyphAtlas {
    pub fn new(config: AtlasConfig) -> TextResult<Self> {
        let config = config.validate()?;
        Ok(Self {
            config,
            pages: Vec::new(),
            entries: HashMap::new(),
            rasterized: 0,
            cache_hits: 0,
        })
    }

    pub fn config(&self) -> AtlasConfig {
        self.config
    }

    pub fn pages(&self) -> &[AtlasPage] {
        &self.pages
    }

    pub fn entries(&self) -> usize {
        self.entries.len()
    }

    pub fn rasterized(&self) -> u64 {
        self.rasterized
    }

    pub fn cache_hits(&self) -> u64 {
        self.cache_hits
    }

    /// Marks one page clean after its pixels have been uploaded.
    pub fn mark_clean(&mut self, page: u32) {
        if let Some(page) = self.pages.get_mut(page as usize) {
            page.dirty = false;
        }
    }

    /// Returns the pixels of one atlas page, or `None` when out of range.
    pub fn page_rgba(&self, page: u32) -> Option<&[u8]> {
        self.pages
            .get(page as usize)
            .map(|page| page.rgba.as_slice())
    }

    pub fn page_version(&self, page: u32) -> Option<u64> {
        self.pages.get(page as usize).map(|page| page.version)
    }

    pub fn bytes(&self) -> usize {
        self.pages.iter().map(|page| page.rgba.len()).sum()
    }

    pub fn clear(&mut self) {
        self.pages.clear();
        self.entries.clear();
    }

    /// Inserts a rasterized glyph, returning its existing entry when unchanged.
    pub fn insert(&mut self, glyph: RasterGlyph) -> TextResult<AtlasEntry> {
        if let Some(entry) = self.entries.get(&glyph.key) {
            self.cache_hits += 1;
            return Ok(*entry);
        }
        let (entry, inserted) = self.allocate(&glyph)?;
        self.entries.insert(glyph.key, entry);
        if inserted {
            self.rasterized += 1;
        }
        Ok(entry)
    }

    fn allocate(&mut self, glyph: &RasterGlyph) -> TextResult<(AtlasEntry, bool)> {
        let padding = self.config.padding;
        let width = glyph.width + padding * 2;
        let height = glyph.height + padding * 2;
        if width > self.config.size || height > self.config.size {
            return Err(TextError::AtlasFull);
        }
        loop {
            if let Some(page_index) = self.pages.len().checked_sub(1) {
                let page = &mut self.pages[page_index];
                if page.shelf_x + width > page.width {
                    page.shelf_y += page.shelf_height;
                    page.shelf_x = padding;
                    page.shelf_height = 0;
                }
                if page.shelf_y + height <= page.height {
                    let x = page.shelf_x;
                    let y = page.shelf_y;
                    page.shelf_x += width;
                    page.shelf_height = page.shelf_height.max(height);
                    page.version += 1;
                    page.dirty = true;
                    write_glyph(page, glyph, x + padding, y + padding);
                    return Ok((
                        AtlasEntry {
                            page: page_index as u32,
                            x: x + padding,
                            y: y + padding,
                            width: glyph.width,
                            height: glyph.height,
                            left: glyph.left,
                            top: glyph.top,
                            color: glyph.color,
                        },
                        true,
                    ));
                }
            }
            if self.pages.len() >= self.config.max_pages {
                return Err(TextError::AtlasFull);
            }
            self.pages.push(AtlasPage::new(self.config));
        }
    }
}

fn write_glyph(page: &mut AtlasPage, glyph: &RasterGlyph, x: u32, y: u32) {
    let page_width = page.width as usize;
    if glyph.color {
        for row in 0..glyph.height as usize {
            let source = row * glyph.width as usize * 4;
            let destination = ((y as usize + row) * page_width + x as usize) * 4;
            page.rgba[destination..destination + glyph.width as usize * 4]
                .copy_from_slice(&glyph.data[source..source + glyph.width as usize * 4]);
        }
    } else {
        for row in 0..glyph.height as usize {
            let source = row * glyph.width as usize;
            for column in 0..glyph.width as usize {
                let destination = ((y as usize + row) * page_width + x as usize + column) * 4;
                let alpha = glyph.data[source + column];
                page.rgba[destination] = 255;
                page.rgba[destination + 1] = 255;
                page.rgba[destination + 2] = 255;
                page.rgba[destination + 3] = alpha;
            }
        }
    }
}

/// One tinted atlas quad in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextQuad {
    pub page: u32,
    pub source: [f32; 4],
    pub destination: [f32; 4],
    pub color: [u8; 4],
}

/// Glyph cache plus CPU atlas. Reuses rasterized glyphs across frames.
#[derive(Debug)]
pub struct GlyphCache {
    rasterizer: GlyphRasterizer,
    atlas: GlyphAtlas,
    cache_hits: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GlyphFrameStats {
    pub glyphs: usize,
    pub quads: usize,
    pub rasterized: u64,
    pub cache_hits: u64,
    pub pages_dirty: usize,
}

#[derive(Clone, Debug, Default)]
pub struct TextFrame {
    pub quads: Vec<TextQuad>,
    pub pages_dirty: Vec<u32>,
    pub stats: GlyphFrameStats,
}

impl GlyphCache {
    pub fn new(config: AtlasConfig) -> TextResult<Self> {
        Ok(Self {
            rasterizer: GlyphRasterizer::new(),
            atlas: GlyphAtlas::new(config)?,
            cache_hits: 0,
        })
    }

    pub fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    pub fn atlas_mut(&mut self) -> &mut GlyphAtlas {
        &mut self.atlas
    }

    pub fn clear(&mut self) {
        self.atlas.clear();
    }

    /// Warms the cache for a layout without producing quads. UI renderers call
    /// this before uploading atlas pages, then use [`Self::frame`] while drawing.
    pub fn warm(&mut self, engine: &mut TextEngine, layout: &TextLayout) -> GlyphFrameStats {
        let mut stats = GlyphFrameStats::default();
        for glyph in layout.glyphs() {
            if self.ensure(engine, glyph).is_some() {
                stats.glyphs += 1;
            }
        }
        stats.rasterized = self.atlas.rasterized();
        stats.cache_hits = self.cache_hits.saturating_add(self.atlas.cache_hits());
        stats.pages_dirty = self.atlas.pages().iter().filter(|page| page.dirty).count();
        stats
    }

    /// Builds tinted quads for one layout. Missing glyphs are rasterized on
    /// demand, so call [`Self::warm`] before a GPU upload phase.
    pub fn frame(
        &mut self,
        engine: &mut TextEngine,
        layout: &TextLayout,
        color: [u8; 4],
    ) -> TextFrame {
        let mut frame = TextFrame::default();
        for glyph in layout.glyphs() {
            let Some(entry) = self.ensure(engine, glyph) else {
                continue;
            };
            let tint = if entry.color { [255; 4] } else { color };
            frame.quads.push(TextQuad {
                page: entry.page,
                source: [
                    entry.x as f32,
                    entry.y as f32,
                    entry.width as f32,
                    entry.height as f32,
                ],
                destination: [
                    glyph.x + entry.left as f32,
                    glyph.baseline - entry.top as f32,
                    entry.width as f32,
                    entry.height as f32,
                ],
                color: tint,
            });
        }
        frame.pages_dirty = self
            .atlas
            .pages()
            .iter()
            .enumerate()
            .filter_map(|(index, page)| page.dirty.then_some(index as u32))
            .collect();
        frame.stats = GlyphFrameStats {
            glyphs: layout.glyphs().len(),
            quads: frame.quads.len(),
            rasterized: self.atlas.rasterized(),
            cache_hits: self.cache_hits.saturating_add(self.atlas.cache_hits()),
            pages_dirty: frame.pages_dirty.len(),
        };
        frame
    }

    fn ensure(&mut self, engine: &mut TextEngine, glyph: &Glyph) -> Option<AtlasEntry> {
        let (key, _, _, _) = GlyphKey::from_glyph(glyph);
        if let Some(entry) = self.atlas.entries.get(&key) {
            self.cache_hits += 1;
            return Some(*entry);
        }
        let raster = self.rasterizer.rasterize(engine, glyph)?;
        self.atlas.insert(raster).ok()
    }
}
