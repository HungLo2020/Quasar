//! Graveyard: a small walkable night scene built to show Quasar's fog.
//!
//! - [`world`]: the player, collision against graves, trees and walls, and
//!   the deterministic layout of the graveyard.
//! - [`render`]: the scene, lit by a cold moon and warm lanterns, in thick
//!   ray-marched volumetric ground fog (drifting banks, lantern halos and a
//!   flashlight beam with a ring cookie) and a graded, bloomed post chain.
pub mod render;
pub mod world;

/// Errors from setup and rendering.
#[derive(Debug)]
pub struct GraveError(pub String);

impl std::fmt::Display for GraveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for GraveError {}

impl GraveError {
    pub fn new(message: impl std::fmt::Display) -> Self {
        Self(message.to_string())
    }
}

pub type Result<T> = std::result::Result<T, GraveError>;

/// Renders `ticks` of the scripted walk headlessly and writes a PNG.
pub fn screenshot(
    path: &std::path::Path,
    width: u32,
    height: u32,
    ticks: u32,
    flashlight: bool,
) -> Result<()> {
    use quasar_frame::{Gpu, OffscreenTarget};
    let mut world = world::World::new();
    let gpu = Gpu::headless().map_err(GraveError::new)?;
    let mut graphics = render::Graphics::new(&gpu, &world).map_err(GraveError::new)?;
    let dt = 1.0 / world::TICK_RATE;
    for _ in 0..ticks {
        world.update(world::autoplay(&world));
    }
    world.flashlight = flashlight;
    // Let the mist banks drift in before the shot.
    graphics.prewarm(14.0 + ticks as f32 * dt);
    let target = OffscreenTarget::new(&gpu, width, height).map_err(GraveError::new)?;
    let image = target
        .render(&gpu, |gpu, target| graphics.frame(gpu, target, &world, dt))
        .map_err(GraveError::new)?;
    let file = std::fs::File::create(path).map_err(GraveError::new)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(&image.rgba))
        .map_err(GraveError::new)?;
    gpu.wait().map_err(GraveError::new)?;
    Ok(())
}
