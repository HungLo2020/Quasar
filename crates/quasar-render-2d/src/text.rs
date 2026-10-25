//! GPU upload adapter for `quasar-text` CPU glyph atlases.
//!
//! The text crate owns shaping, rasterization, atlas packing and quads. This
//! module only maps its pages onto persistent `DynamicTexture` objects on the
//! caller's `RenderState2D`. Unchanged pages are never uploaded again.
use crate::{TextureId, Textures};
use quasar_text::GlyphAtlas;
use vulkanic_gal::{GalResult, VulkanicGal};

/// Upload counters for one or more `sync` calls.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextUploadStats {
    pub pages_created: u64,
    pub uploads: u64,
    pub upload_bytes: u64,
}

/// Persistent GPU pages for a CPU glyph atlas.
///
/// Pages are created lazily on the caller's `RenderState2D`; their
/// `TextureId` is stable for the life of the page. `sync` uploads only pages
/// whose `GlyphAtlas` version changed since the previous call.
#[derive(Debug, Default)]
pub struct GpuTextAtlas {
    pages: Vec<crate::DynamicTexture>,
    versions: Vec<u64>,
    stats: TextUploadStats,
}

impl GpuTextAtlas {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Stable texture identity for one atlas page, if it has been created.
    pub fn texture_id(&self, page: u32) -> Option<TextureId> {
        self.pages.get(page as usize).map(|page| page.id())
    }

    pub fn stats(&self) -> TextUploadStats {
        self.stats
    }

    pub fn take_stats(&mut self) -> TextUploadStats {
        std::mem::take(&mut self.stats)
    }

    /// Creates missing pages and uploads dirty pages. Marked-clean pages remain
    /// reusable across frames and are not uploaded again.
    pub fn sync(
        &mut self,
        state: &mut crate::RenderState2D,
        gal: &mut VulkanicGal,
        textures: &mut Textures,
        atlas: &mut GlyphAtlas,
    ) -> GalResult<()> {
        for index in 0..atlas.pages().len() {
            let page = &atlas.pages()[index];
            if self.pages.len() <= index {
                let created =
                    state.create_dynamic_texture(gal, textures, page.width, page.height)?;
                self.pages.push(created);
                self.versions.push(u64::MAX);
                self.stats.pages_created += 1;
            }
            let version = page.version;
            if self.versions[index] != version {
                let dynamic = self.pages[index];
                let bytes = page.rgba.len() as u64;
                state.update_dynamic_texture(dynamic, &page.rgba)?;
                atlas.mark_clean(index as u32);
                self.versions[index] = version;
                self.stats.uploads += 1;
                self.stats.upload_bytes += bytes;
            }
        }
        Ok(())
    }

    /// Destroys all page textures. Call before dropping the render state.
    pub fn release(
        &mut self,
        state: &mut crate::RenderState2D,
        gal: &mut VulkanicGal,
        textures: &mut Textures,
    ) -> GalResult<()> {
        for page in std::mem::take(&mut self.pages) {
            state.destroy_dynamic_texture(gal, textures, page)?;
        }
        self.versions.clear();
        Ok(())
    }
}
