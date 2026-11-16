//! Voxel Blocks: a small block world on Quasar's voxel stack with
//! MattMC-style defaults (16³ chunks, per-block models, AO, block and sky
//! light, cutout and translucent layers, connectivity culling).
pub mod content;
pub mod render;
pub mod settings;
pub mod terrain;
pub mod world;

use quasar_app::{AppContext, Application, Flow, HeadlessConfig, run_headless as run_app_headless};
use quasar_diagnostics::{Diagnostics, DiagnosticsConfig, Level, TextSink};
use quasar_frame::OffscreenTarget;
use settings::Settings;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug)]
pub struct BlocksError(String);

impl BlocksError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl std::fmt::Display for BlocksError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for BlocksError {}
pub type Result<T> = std::result::Result<T, BlocksError>;

fn err(error: impl std::fmt::Display) -> BlocksError {
    BlocksError::invalid(error.to_string())
}

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
            settle_ms: 0.0,
        },
        config,
    )
    .map_err(err)
}

struct HeadlessApp {
    settings: Settings,
    diagnostics: Diagnostics,
    world: Option<world::BlockWorld>,
    samples: Vec<f64>,
    settle_ms: f64,
}

fn percentile(samples: &[f64], p: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() - 1) as f64 * p) as usize]
}

impl Application for HeadlessApp {
    type Error = BlocksError;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        let mut world = world::BlockWorld::new(
            self.settings.seed,
            self.settings.radius,
            self.settings.workers,
        );
        world.sync_meshing = true;
        let started = Instant::now();
        world.settle();
        self.settle_ms = started.elapsed().as_secs_f64() * 1e3;
        self.diagnostics.event_with(
            Level::Info,
            "voxel-blocks.app",
            "initialized",
            [
                quasar_diagnostics::Field::new("headless", true),
                quasar_diagnostics::Field::new("seed", self.settings.seed),
                quasar_diagnostics::Field::new("chunks", world.stats.chunks as u64),
            ],
        );
        self.world = Some(world);
        Ok(())
    }
    fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow> {
        let Some(world) = &mut self.world else {
            return Ok(Flow::Continue);
        };
        let input = if self.settings.autoplay {
            world::autoplay_input(world)
        } else {
            world::Input::default()
        };
        let started = Instant::now();
        world.update(input);
        self.samples.push(started.elapsed().as_secs_f64() * 1e3);
        Ok(Flow::Continue)
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        let Some(world) = &mut self.world else {
            return Ok(());
        };
        let s = world.stats;
        let p = world.player.position;
        println!(
            "voxel-blocks ticks={} columns={} chunks={} meshed={} quads={} placed={} removed={} light_changes={} player=[{:.2},{:.2},{:.2}] hash={:016x}",
            world.tick(),
            s.columns,
            s.chunks,
            s.meshed,
            s.quads,
            s.placed,
            s.removed,
            s.light_changes,
            p.x,
            p.y,
            p.z,
            world.state_hash()
        );
        let mean = self.samples.iter().sum::<f64>() / self.samples.len().max(1) as f64;
        let p99 = percentile(&self.samples, 0.99);
        println!(
            "voxel-blocks settle_ms={:.1} meshes_built={} tick_mean_ms={mean:.3} tick_p99_ms={p99:.3}",
            self.settle_ms, s.meshes_built
        );
        let mut prepare_us = None;
        if let Some(path) = &self.settings.screenshot {
            prepare_us = Some(render_screenshot(world, &self.settings, path)?);
            println!("screenshot={}", path.display());
        }
        if let Some(path) = &self.settings.benchmark_file {
            let json = format!(
                "{{\n  \"settle_ms\": {:.2},\n  \"chunks\": {},\n  \"meshed\": {},\n  \"quads\": {},\n  \"meshes_built\": {},\n  \"tick_mean_ms\": {mean:.4},\n  \"tick_p99_ms\": {p99:.4},\n  \"prepare_us\": {}\n}}\n",
                self.settle_ms,
                s.chunks,
                s.meshed,
                s.quads,
                s.meshes_built,
                prepare_us.map_or("null".into(), |v: f64| format!("{v:.1}"))
            );
            std::fs::write(path, json)
                .map_err(|e| BlocksError::invalid(format!("write {}: {e}", path.display())))?;
        }
        Ok(())
    }
}

/// Renders the world offscreen, writes a PNG and returns the mean CPU
/// time of preparing the voxel pass over several frames (µs).
fn render_screenshot(
    world: &mut world::BlockWorld,
    settings: &Settings,
    path: &std::path::Path,
) -> Result<f64> {
    let (width, height) = (settings.width, settings.height);
    use quasar_frame::Gpu;
    let gal = Gpu::headless().map_err(err)?;
    let mut graphics = render::Graphics::new(&gal).map_err(err)?;
    graphics.occlusion = settings.occlusion;
    let target = OffscreenTarget::new(&gal, width, height).map_err(err)?;
    let mut prepare = Vec::new();
    let mut image = Vec::new();
    for frame in 0..8 {
        let report = target
            .render(&gal, |gal, target| {
                graphics.commands(gal, target, world, true)
            })
            .map_err(err)?;
        graphics.submitted(&gal);
        if frame >= 3 {
            prepare.push(graphics.prepare_micros);
        }
        image = report.rgba;
    }
    let file = std::fs::File::create(path)
        .map_err(|e| BlocksError::invalid(format!("create {}: {e}", path.display())))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut header| header.write_image_data(&image))
        .map_err(|e| BlocksError::invalid(format!("write screenshot: {e}")))?;
    let stats = graphics.voxel_stats;
    println!(
        "voxel-blocks render records={} occlusion_culled={} gpu_culled={} cpu_culled={} batches={} direct={} triangles={}",
        stats.records,
        stats.occlusion_culled,
        stats.gpu_culled_records,
        stats.cpu_culled,
        stats.indirect_batches,
        stats.direct_draws,
        stats.triangles
    );
    target.release(&gal).map_err(err)?;
    graphics.release(&gal).map_err(err)?;
    gal.wait().map_err(err)?;
    Ok(prepare.iter().sum::<f64>() / prepare.len().max(1) as f64)
}
