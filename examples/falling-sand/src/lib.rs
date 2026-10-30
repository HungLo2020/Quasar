//! Falling Sand: cellular sand, Rapier 2D obstacles and `quasar-spatial` queries.
pub mod menu;
pub mod render;
pub mod sand;
pub mod settings;

use quasar_app::{AppContext, Application, Flow, HeadlessConfig, run_headless as run_app_headless};
use quasar_diagnostics::{Diagnostics, DiagnosticsConfig, Level, TextSink};
use quasar_frame::OffscreenTarget;
use settings::Settings;
use std::sync::Arc;

#[derive(Debug)]
pub struct SandError(String);

impl SandError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for SandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for SandError {}

pub type Result<T> = std::result::Result<T, SandError>;

pub fn diagnostics(settings: &Settings) -> Diagnostics {
    let sink = Arc::new(TextSink::new(std::io::stderr()));
    Diagnostics::with_config(
        sink,
        DiagnosticsConfig {
            min_level: settings.log_level,
            ..DiagnosticsConfig::default()
        },
    )
}

pub fn run_headless(settings: Settings, diagnostics: Diagnostics) -> Result<()> {
    let config = HeadlessConfig {
        ticks: settings.ticks,
        step: std::time::Duration::from_secs_f64(1.0 / 60.0),
    };
    run_headless_app(
        HeadlessApp {
            settings,
            diagnostics,
            world: None,
            samples: Vec::new(),
        },
        config,
    )
}

fn run_headless_app(app: HeadlessApp, config: HeadlessConfig) -> Result<()> {
    run_app_headless(app, config).map_err(|error| SandError::invalid(error.to_string()))
}

struct HeadlessApp {
    settings: Settings,
    diagnostics: Diagnostics,
    world: Option<sand::SandWorld>,
    samples: Vec<f64>,
}

impl Application for HeadlessApp {
    type Error = SandError;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        self.world = Some(sand::SandWorld::new(self.settings.seed));
        self.diagnostics.event_with(
            Level::Info,
            "falling-sand.app",
            "initialized",
            [
                quasar_diagnostics::Field::new("headless", true),
                quasar_diagnostics::Field::new("seed", self.settings.seed),
            ],
        );
        Ok(())
    }
    fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow> {
        let Some(world) = &mut self.world else {
            return Ok(Flow::Continue);
        };
        let input = if self.settings.autoplay {
            sand::autoplay_input(world)
        } else {
            sand::SandInput::default()
        };
        let started = std::time::Instant::now();
        world.update(input);
        self.samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        Ok(Flow::Continue)
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        let Some(world) = &self.world else {
            return Ok(());
        };
        let stats = world.stats();
        println!(
            "falling-sand ticks={} sand={} obstacles={} ball=[{:.2},{:.2}] hash={:016x}",
            world.tick(),
            stats.sand,
            stats.obstacles,
            stats.ball[0],
            stats.ball[1],
            world.state_hash()
        );
        if !self.samples.is_empty() {
            let mean = self.samples.iter().sum::<f64>() / self.samples.len() as f64;
            let mut sorted = self.samples.clone();
            sorted.sort_by(f64::total_cmp);
            let p99 = sorted[((sorted.len() - 1) as f64 * 0.99) as usize];
            println!(
                "falling-sand simulation mean_ms={mean:.4} p99_ms={p99:.4} ticks={}",
                self.samples.len()
            );
        }
        if let Some(path) = &self.settings.screenshot {
            render_screenshot(world, self.settings.width, self.settings.height, path)?;
            println!("screenshot={}", path.display());
        }
        Ok(())
    }
}

fn render_screenshot(
    world: &sand::SandWorld,
    width: u32,
    height: u32,
    path: &std::path::Path,
) -> Result<()> {
    use quasar_frame::Gpu;
    let gal = Gpu::headless().map_err(|error| SandError::invalid(error.to_string()))?;
    let mut graphics =
        render::Graphics::new(&gal).map_err(|error| SandError::invalid(error.to_string()))?;
    let target = OffscreenTarget::new(&gal, width, height)
        .map_err(|error| SandError::invalid(error.to_string()))?;
    let report = target
        .render(&gal, |gal, target| {
            graphics.commands(gal, target, world, None, sand::Tool::Sand, None)
        })
        .map_err(|error| SandError::invalid(error.to_string()))?;
    let file = std::fs::File::create(path)
        .map_err(|error| SandError::invalid(format!("create {}: {error}", path.display())))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut header| header.write_image_data(&report.rgba))
        .map_err(|error| SandError::invalid(format!("write screenshot: {error}")))?;
    target
        .release(&gal)
        .map_err(|error| SandError::invalid(error.to_string()))?;
    graphics
        .release(&gal)
        .map_err(|error| SandError::invalid(error.to_string()))?;
    gal.wait()
        .map_err(|error| SandError::invalid(error.to_string()))?;
    Ok(())
}
