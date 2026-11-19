//! Voxel Drift: a Space Engineers-style scene on Quasar's voxel stack: a
//! smooth, diggable asteroid with LOD and seam skirts, and ships built from
//! oriented, multi-cell and sloped blocks on dynamic grids that split apart.
pub mod asteroid;
pub mod content;
pub mod render;
pub mod settings;
pub mod ship;
pub mod world;

use quasar_app::{AppContext, Application, Flow, HeadlessConfig, run_headless as run_app_headless};
use quasar_diagnostics::{Diagnostics, DiagnosticsConfig, Level, TextSink};
use quasar_frame::OffscreenTarget;
use quasar_math::IVec3;
use quasar_voxel::{GridId, GridTransform};
use quasar_voxel_mesh::MeshBuffers;
use settings::Settings;
use std::sync::Arc;
use std::time::Instant;

/// Renderer-facing changes produced by the simulation.
pub enum GeometryUpdate {
    Grid {
        grid: GridId,
        transform: GridTransform,
        cell_size: f64,
    },
    Upload {
        grid: GridId,
        chunk: IVec3,
        level: u32,
        origin: IVec3,
        scale: f32,
        mesh: MeshBuffers,
    },
    Remove {
        grid: GridId,
        chunk: IVec3,
        level: u32,
    },
}

#[derive(Debug)]
pub struct DriftError(String);

impl DriftError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl std::fmt::Display for DriftError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for DriftError {}
pub type Result<T> = std::result::Result<T, DriftError>;

fn err(error: impl std::fmt::Display) -> DriftError {
    DriftError::invalid(error.to_string())
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
            setup_ms: 0.0,
        },
        config,
    )
    .map_err(err)
}

struct HeadlessApp {
    settings: Settings,
    diagnostics: Diagnostics,
    world: Option<world::DriftWorld>,
    samples: Vec<f64>,
    setup_ms: f64,
}

impl Application for HeadlessApp {
    type Error = DriftError;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        let started = Instant::now();
        let world = world::DriftWorld::new(self.settings.seed, self.settings.workers);
        self.setup_ms = started.elapsed().as_secs_f64() * 1e3;
        self.diagnostics.event_with(
            Level::Info,
            "voxel-drift.app",
            "initialized",
            [
                quasar_diagnostics::Field::new("headless", true),
                quasar_diagnostics::Field::new("seed", self.settings.seed),
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
            world::autoplay(world)
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
        println!(
            "voxel-drift ticks={} ships={} ship_blocks={} splits={} digs={} fills={} placed={} removed={} nodes={} levels={:?} triangles={} collider_chunks={} hash={:016x}",
            world.tick(),
            s.ships,
            s.ship_blocks,
            s.splits,
            s.digs,
            s.fills,
            s.placed,
            s.removed,
            s.nodes,
            s.levels,
            s.triangles,
            s.collider_chunks,
            world.state_hash()
        );
        let mut sorted = self.samples.clone();
        sorted.sort_by(f64::total_cmp);
        let mean = sorted.iter().sum::<f64>() / sorted.len().max(1) as f64;
        let p99 = sorted
            .get(((sorted.len().max(1) - 1) as f64 * 0.99) as usize)
            .copied()
            .unwrap_or(0.0);
        println!(
            "voxel-drift setup_ms={:.1} tick_mean_ms={mean:.3} tick_p99_ms={p99:.3}",
            self.setup_ms
        );
        let mut prepare_us = None;
        if let Some(path) = &self.settings.screenshot {
            prepare_us = Some(render_screenshot(world, &self.settings, path)?);
            println!("screenshot={}", path.display());
        }
        if let Some(path) = &self.settings.benchmark_file {
            let json = format!(
                "{{\n  \"setup_ms\": {:.2},\n  \"tick_mean_ms\": {mean:.4},\n  \"tick_p99_ms\": {p99:.4},\n  \"nodes\": {},\n  \"triangles\": {},\n  \"ships\": {},\n  \"prepare_us\": {}\n}}\n",
                self.setup_ms,
                s.nodes,
                s.triangles,
                s.ships,
                prepare_us.map_or("null".into(), |v: f64| format!("{v:.1}"))
            );
            std::fs::write(path, json)
                .map_err(|e| DriftError::invalid(format!("write {}: {e}", path.display())))?;
        }
        Ok(())
    }
}

fn render_screenshot(
    world: &mut world::DriftWorld,
    settings: &Settings,
    path: &std::path::Path,
) -> Result<f64> {
    use quasar_frame::Gpu;
    let (width, height) = (settings.width, settings.height);
    let gal = Gpu::headless().map_err(err)?;
    let mut graphics = render::Graphics::new(&gal).map_err(err)?;
    graphics.set_toon(settings.toon);
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
        .map_err(|e| DriftError::invalid(format!("create {}: {e}", path.display())))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut header| header.write_image_data(&image))
        .map_err(|e| DriftError::invalid(format!("write screenshot: {e}")))?;
    let stats = graphics.voxel_stats;
    println!(
        "voxel-drift render records={} cpu_culled={} batches={} direct={} triangles={}",
        stats.records,
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
