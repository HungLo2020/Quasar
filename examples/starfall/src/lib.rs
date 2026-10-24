//! Starfall: a small but complete Quasar 3D reference game.
//!
//! The library owns deterministic game rules, content loading, journaling and
//! the shared-GAL renderer. Native/headless entry points live in `main` and the
//! optional `desktop` module.

pub mod content;
pub mod render;
pub mod session;
pub mod settings;
pub mod world;

use content::Content;
use quasar_diagnostics::{Diagnostics, DiagnosticsConfig, Level, TextSink};
use quasar_jobs::JobPool;
use settings::GameSettings;
use std::sync::Arc;
use world::{GameError, GameRules, GameWorld};

pub fn diagnostics(settings: &GameSettings) -> Diagnostics {
    let sink = Arc::new(TextSink::new(std::io::stderr()));
    Diagnostics::with_config(
        sink,
        DiagnosticsConfig {
            min_level: settings.log_level,
            ..DiagnosticsConfig::default()
        },
    )
}

/// Load assets and generated sounds on a caller-owned manual job pool, then
/// return the joined result. This is the deterministic path shared by native
/// setup and headless tests.
pub fn load_content_blocking(
    settings: &GameSettings,
    diagnostics: &Diagnostics,
) -> Result<Content, GameError> {
    let pool = JobPool::manual();
    let root = settings.assets_root.clone();
    let level = settings.level_path.clone();
    let diagnostics = diagnostics.clone();
    let job = pool
        .spawn(move |_| content::load_content(root, level, 44_100, diagnostics))
        .map_err(|error| GameError::invalid(format!("job submission: {error}")))?;
    pool.run_all();
    let content = job
        .wait()
        .map_err(|error| GameError::invalid(format!("content job: {error}")))?;
    pool.shutdown()
        .map_err(|error| GameError::invalid(format!("job shutdown: {error}")))?;
    Ok(content)
}

pub fn world_from_content(
    content: &Content,
    settings: &GameSettings,
) -> Result<GameWorld, GameError> {
    GameWorld::new(
        content.level.clone(),
        GameRules {
            hz: 60.0,
            time_limit: settings.time_limit.min(content.level.duration),
            player_speed: settings.player_speed,
            turn_speed: settings.turn_speed,
            camera_distance: settings.camera_distance,
            camera_height: settings.camera_height,
        },
    )
}

/// Render one final frame to an offscreen target and write a PNG. Used by
/// headless screenshot verification; never called by the native runner.
pub fn render_screenshot(
    settings: &GameSettings,
    content: &Content,
    world: &GameWorld,
) -> Result<(), GameError> {
    use quasar_frame::OffscreenTarget;
    use quasar_profile::RendererStats;
    use vulkanic_gal::{BackendChoice, VulkanicGal};
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "Starfall offscreen")
        .map_err(|error| GameError::invalid(error.to_string()))?;
    let mut graphics = render::Graphics::new(&mut gal, &content.texture)
        .map_err(|error| GameError::invalid(error.to_string()))?;
    let mut target = OffscreenTarget::new(&mut gal, settings.window_width, settings.window_height)
        .map_err(|error| GameError::invalid(error.to_string()))?;
    let report = target
        .render(&mut gal, |gal, target| {
            graphics.commands(gal, target, world)
        })
        .map_err(|error| GameError::invalid(error.to_string()))?;
    let _ = RendererStats::default();
    let path = settings
        .screenshot
        .clone()
        .ok_or_else(|| GameError::invalid("internal screenshot path missing"))?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(&path).map_err(|error| {
            GameError::invalid(format!("create screenshot {}: {error}", path.display()))
        })?),
        settings.window_width,
        settings.window_height,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut header| header.write_image_data(&report.rgba))
        .map_err(|error| GameError::invalid(format!("write screenshot: {error}")))?;
    target
        .release(&mut gal)
        .map_err(|error| GameError::invalid(error.to_string()))?;
    graphics
        .release(&mut gal)
        .map_err(|error| GameError::invalid(error.to_string()))?;
    gal.retire_through(gal.latest_submission_id())
        .map_err(|error| GameError::invalid(error.to_string()))?;
    Ok(())
}

pub fn log_level_name(level: Level) -> &'static str {
    match level {
        Level::Trace => "trace",
        Level::Debug => "debug",
        Level::Info => "info",
        Level::Warn => "warn",
        Level::Error => "error",
    }
}
