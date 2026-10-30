//! Physics Playground: Rapier 3D dynamics, character movement and spatial queries.
pub mod render;
pub mod settings;
pub mod world;

use quasar_app::{AppContext, Application, Flow, HeadlessConfig, run_headless as run_app_headless};
use quasar_diagnostics::{Diagnostics, DiagnosticsConfig, Level, TextSink};
use quasar_frame::OffscreenTarget;
use settings::Settings;
use std::sync::Arc;

#[derive(Debug)]
pub struct PlaygroundError(String);

impl PlaygroundError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl std::fmt::Display for PlaygroundError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for PlaygroundError {}
pub type Result<T> = std::result::Result<T, PlaygroundError>;

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
    run_app_headless(
        HeadlessApp {
            settings,
            diagnostics,
            world: None,
            samples: Vec::new(),
        },
        config,
    )
    .map_err(|error| PlaygroundError::invalid(error.to_string()))
}

struct HeadlessApp {
    settings: Settings,
    diagnostics: Diagnostics,
    world: Option<world::Playground>,
    samples: Vec<f64>,
}

impl Application for HeadlessApp {
    type Error = PlaygroundError;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        self.world = Some(world::Playground::new(self.settings.seed));
        self.diagnostics.event_with(
            Level::Info,
            "physics-playground.app",
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
            world::autoplay_input(world)
        } else {
            world::PlaygroundInput::default()
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
            "physics-playground ticks={} props={} balls={} blocks={} agents={} nav_gen={} player=[{:.2},{:.2},{:.2}] grounded={} zone={} events={} hash={:016x}",
            world.tick(),
            stats.props,
            stats.balls,
            stats.blocks,
            stats.agents,
            stats.nav_generation,
            stats.player[0],
            stats.player[1],
            stats.player[2],
            stats.grounded,
            stats.zone_active,
            stats.collision_events,
            world.state_hash()
        );
        if !self.samples.is_empty() {
            let mean = self.samples.iter().sum::<f64>() / self.samples.len() as f64;
            let mut sorted = self.samples.clone();
            sorted.sort_by(f64::total_cmp);
            let p99 = sorted[((sorted.len() - 1) as f64 * 0.99) as usize];
            println!(
                "physics-playground simulation mean_ms={mean:.4} p99_ms={p99:.4} ticks={}",
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
    world: &world::Playground,
    width: u32,
    height: u32,
    path: &std::path::Path,
) -> Result<()> {
    use quasar_frame::Gpu;
    let gal = Gpu::headless().map_err(|error| PlaygroundError::invalid(error.to_string()))?;
    let mut graphics =
        render::Graphics::new(&gal).map_err(|error| PlaygroundError::invalid(error.to_string()))?;
    let target = OffscreenTarget::new(&gal, width, height)
        .map_err(|error| PlaygroundError::invalid(error.to_string()))?;
    let report = target
        .render(&gal, |gal, target| graphics.commands(gal, target, world))
        .map_err(|error| PlaygroundError::invalid(error.to_string()))?;
    let file = std::fs::File::create(path)
        .map_err(|error| PlaygroundError::invalid(format!("create {}: {error}", path.display())))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut header| header.write_image_data(&report.rgba))
        .map_err(|error| PlaygroundError::invalid(format!("write screenshot: {error}")))?;
    target
        .release(&gal)
        .map_err(|error| PlaygroundError::invalid(error.to_string()))?;
    graphics
        .release(&gal)
        .map_err(|error| PlaygroundError::invalid(error.to_string()))?;
    gal.wait()
        .map_err(|error| PlaygroundError::invalid(error.to_string()))?;
    Ok(())
}
