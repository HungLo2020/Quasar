//! Ordered GPU 2D drawing through VulkanicGAL's public API.
//! Draw lists, textures, and reference snapshots also work without the `gpu`
//! feature. Native games use instanced quads; software-rendered games can use
//! the optional persistent-framebuffer presenter.
mod atlas;
mod canvas;
mod font;
#[cfg(feature = "gpu")]
mod gpu;
mod indexed;
mod math;
mod pixel;
pub use indexed::{IndexedImage, Palette, PaletteBinding};
pub use pixel::{PixelState, Priority2D};
pub mod reference;
mod texture;
pub use atlas::{AnimationClip, AnimationFrame, SpriteAtlas};
pub use canvas::{Canvas, Quad, Sprite, SpriteTransform};
#[cfg(feature = "gpu")]
pub use gpu::{
    DynamicTexture, FramebufferPresenter, LoadOp, Prepared2D, PresentOutcome, RenderState2D,
    RenderStats, Renderer2D,
};
pub use math::{Camera2D, Color, Rect, Transform2D};
pub use quasar_core::{DisplayOptions, Filter, Scaling, Viewport};
use std::{error::Error as StdError, fmt};
#[cfg(feature = "assets")]
pub use texture::PngLoader;
#[cfg(feature = "png")]
pub use texture::write_png;
pub use texture::{Image, TextureId, Textures};
#[cfg(feature = "text")]
mod text;
#[cfg(feature = "gpu")]
pub use quasar_frame::{Gpu, GpuTiming, GraphicsError, GraphicsResult};
#[cfg(feature = "text")]
pub use text::{GpuTextAtlas, TextUploadStats};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl StdError for Error {}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error(message.into())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod retro_tests;
