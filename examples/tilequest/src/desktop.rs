use super::*;
use quasar_app::{
    AppConfig, AppContext, Application, DesktopApplication, Flow, RenderPolicy, TimingConfig,
    run as run_app,
};
use quasar_core::{DisplayOptions, Filter, FixedStepConfig, Scaling};
use quasar_frame::{CompletionPolicy, FrameSurface, Gpu, GraphicsError, OffscreenTarget};
use quasar_input::BindingError;
use quasar_render_2d::{Canvas, LoadOp, RenderState2D, Textures};
use quasar_window::{DesktopConfig, WindowConfig};
use std::time::{Duration, Instant};
use vulkanic_gal::*;

#[derive(Debug)]
struct AppError(Box<dyn std::error::Error>);
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}
impl From<Box<dyn std::error::Error>> for AppError {
    fn from(error: Box<dyn std::error::Error>) -> Self {
        Self(error)
    }
}
impl From<&str> for AppError {
    fn from(error: &str) -> Self {
        Self(Box::new(std::io::Error::other(error.to_owned())))
    }
}
impl From<String> for AppError {
    fn from(error: String) -> Self {
        Self(Box::new(std::io::Error::other(error)))
    }
}
impl From<BindingError> for AppError {
    fn from(error: BindingError) -> Self {
        Self(Box::new(error))
    }
}
impl From<quasar_render_2d::Error> for AppError {
    fn from(error: quasar_render_2d::Error) -> Self {
        Self(Box::new(error))
    }
}
impl From<GraphicsError> for AppError {
    fn from(error: GraphicsError) -> Self {
        Self(Box::new(error))
    }
}
type AppResult<T> = std::result::Result<T, AppError>;

pub fn run(options: Options, session: Session, controls: tilequest::controls::Controls) -> Result {
    let config = AppConfig {
        window: WindowConfig {
            title: "Tilequest / Quasar".into(),
            application_id: "quasar-tilequest".into(),
            size: (960, 640),
            minimum_size: (480, 320),
            ..Default::default()
        },
        desktop: DesktopConfig::for_application("quasar-tilequest"),
        timing: TimingConfig::Fixed(FixedStepConfig::for_rate(tilequest::HZ, 8)),
        render: RenderPolicy::EveryFrame,
        max_frame_wait: Duration::from_millis(16),
        frame_interval: None,
    };
    let app = App {
        options,
        session,
        controls,
        gal: None,
        renderer: None,
        surface: None,
        display: DisplayOptions {
            scaling: Scaling::Integer,
            ..Default::default()
        },
        size: (960, 640),
        frames: 0,
        stage: 0,
        started: Instant::now(),
        samples: Vec::new(),
        draw_stats: quasar_tilemap::render::DrawStats::default(),
        baseline: None,
        gpu: Vec::new(),
        gpu_pending: Vec::new(),
        last_gpu: 0,
        pause_pending: false,
    };
    run_app(app, config)?;
    Ok(())
}

struct App {
    options: Options,
    session: Session,
    controls: tilequest::controls::Controls,
    gal: Option<Gpu>,
    renderer: Option<RenderState2D>,
    surface: Option<FrameSurface>,
    display: DisplayOptions,
    size: (u32, u32),
    frames: u64,
    stage: u8,
    started: Instant,
    samples: Vec<u64>,
    draw_stats: quasar_tilemap::render::DrawStats,
    baseline: Option<(u64, u64, u64, u64)>,
    gpu: Vec<u64>,
    gpu_pending: Vec<quasar_frame::GpuTiming>,
    last_gpu: u64,
    pause_pending: bool,
}

impl App {
    fn initialize(&mut self, context: &mut AppContext<'_>) -> Result {
        let window = context
            .window
            .ok_or("desktop runner did not provide a window")?;
        let size = window.native().inner_size();
        let (gal, mut surface) =
            FrameSurface::for_window(window.shared_native(), size.width, size.height)?;
        if self.options.benchmark.is_some() {
            surface.config.present_mode = PresentMode::AutoNoVsync;
            surface.reconfigure(&gal)?;
        }
        self.renderer = Some(RenderState2D::new(&gal)?);
        self.surface = Some(surface);
        gal.set_gpu_timestamps(self.options.benchmark.is_some());
        self.gal = Some(gal);
        self.size = (size.width, size.height);
        self.started = Instant::now();
        Ok(())
    }

    fn redraw(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        let gal = self.gal.as_mut().ok_or("missing GAL")?;
        let renderer = self.renderer.as_mut().ok_or("missing renderer")?;
        let session = &self.session;
        let mut draw_stats = quasar_tilemap::render::DrawStats::default();
        let report =
            self.surface
                .as_mut()
                .unwrap()
                .render(gal, CompletionPolicy::Poll, |gal, target| {
                    let (canvas, stats) = tilequest::scene(
                        &session.world,
                        &session.state,
                        &session.art,
                        self.display.filter,
                    )
                    .map_err(|e| GraphicsError::new(e.to_string()))?;
                    draw_stats = stats;
                    let prepared = renderer.prepare(
                        gal,
                        &canvas,
                        &session.art.textures,
                        target,
                        self.display,
                        LoadOp::Clear,
                    )?;
                    Ok(vec![prepared.commands])
                })?;
        let Some(report) = report else {
            return Ok(Flow::Continue);
        };
        self.frames += 1;
        self.draw_stats = draw_stats;
        if self.frames == 64 {
            self.baseline = Some((0, 0, 0, 0));
        }
        if self.options.benchmark.is_some() && self.frames > 64 {
            self.samples.push(report.cpu_frame_nanos);
            self.last_gpu += report.blocking_waits;
            if let Some(t) = &report.gpu_timing {
                self.gpu_pending.push(t.clone());
            } else if let Some(n) = report.gpu_frame_nanos {
                self.gpu.push(n);
            }
        }
        if self.options.exercise {
            self.exercise(context)?;
        }
        if self.options.frames.is_some_and(|n| self.frames >= n) {
            if self.options.exercise && self.stage != 4 {
                return Err("native exercise did not finish".into());
            }
            println!(
                "presented_frames={} window_exercise_stages={} visited_cells={} emitted_tiles={} stored_cells={} render_stats={:?}",
                self.frames,
                self.stage,
                self.draw_stats.visited_cells,
                self.draw_stats.emitted_tiles,
                self.session.world.runtime.stored_cells(),
                self.renderer.as_ref().unwrap().stats()
            );
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    fn exercise(&mut self, context: &AppContext<'_>) -> Result {
        let window = context
            .window
            .ok_or("desktop runner did not provide a window")?;
        match self.stage {
            0 if self.frames >= 2 => {
                let _ = window
                    .native()
                    .request_inner_size(quasar_window::winit::dpi::LogicalSize::new(1000, 700));
                self.stage = 1;
            }
            1 if self.frames >= 6 => {
                window.set_fullscreen(true);
                self.stage = 2;
            }
            2 if self.frames >= 12 => {
                window.set_fullscreen(false);
                self.stage = 3;
            }
            3 if self.frames >= 18 => {
                self.surface.as_mut().unwrap().resize(0, 0);
                let gal = self.gal.as_mut().unwrap();
                if self.surface.as_mut().unwrap().acquire(gal)?.is_some() {
                    return Err("zero-sized surface must defer".into());
                }
                let size = window.native().inner_size();
                self.surface
                    .as_mut()
                    .unwrap()
                    .resize(size.width, size.height);
                self.display.filter = Filter::Linear;
                self.display.scaling = Scaling::Fit;
                self.stage = 4;
            }
            _ => {}
        }
        Ok(())
    }

    fn teardown(&mut self, outputs: bool) -> Result {
        if let Some(gal) = self.gal.take() {
            self.surface.as_mut().unwrap().finish(&gal)?;
            if let Some(mut renderer) = self.renderer.take() {
                if outputs {
                    let pixels = if self.options.screenshot.is_some() {
                        let (canvas, _) = tilequest::scene(
                            &self.session.world,
                            &self.session.state,
                            &self.session.art,
                            Filter::Nearest,
                        )?;
                        Some(capture(
                            &gal,
                            &mut renderer,
                            &canvas,
                            &self.session.art.textures,
                        )?)
                    } else {
                        None
                    };
                    self.session.outputs(&self.options, pixels.as_deref())?;
                    if let Some(path) = &self.options.benchmark {
                        self.baseline.ok_or("benchmark needs more than 64 frames")?;
                        let mut cpu = self.samples.clone();
                        cpu.sort_unstable();
                        let mut gpu: Vec<_> = self
                            .gpu
                            .iter()
                            .copied()
                            .chain(
                                self.gpu_pending
                                    .iter()
                                    .filter_map(quasar_frame::GpuTiming::nanos),
                            )
                            .collect();
                        gpu.sort_unstable();
                        quasar_storage::write_json(
                            path,
                            &serde_json::json!({"frames":self.frames,"measured_frames":cpu.len(),"cpu_frame_p50_ms":cpu.get(cpu.len()/2).map(|v|*v as f64/1e6),"gpu_frame_p50_ms":gpu.get(gpu.len()/2).map(|v|*v as f64/1e6),"visited_cells":self.draw_stats.visited_cells,"emitted_tiles":self.draw_stats.emitted_tiles,"stored_cells":self.session.world.runtime.stored_cells(),"blocking_waits":self.last_gpu,"render_stats":{"quads":renderer.stats().quads,"batches":renderer.stats().batches,"geometry_bytes":renderer.stats().geometry_upload_bytes,"texture_bytes":renderer.stats().texture_upload_bytes},"scope":"native map traversal, draw-list construction, GPU rendering and presentation; first 64 frames excluded; one simulation tick per attempted frame"}),
                        )?;
                    }
                }
                renderer.release(&gal)?;
            }
            self.surface.take();
            gal.wait()?;
            if let Some(report) = gal.instance.generate_report() {
                let h = report.hub;
                println!(
                    "live_buffers={} live_textures={} live_views={}",
                    h.buffers.num_kept_from_user,
                    h.textures.num_kept_from_user,
                    h.texture_views.num_kept_from_user
                );
                if h.buffers.num_kept_from_user
                    + h.textures.num_kept_from_user
                    + h.texture_views.num_kept_from_user
                    != 0
                {
                    return Err("resource ownership leaked".into());
                }
            }
        }
        Ok(())
    }
}

impl Application for App {
    type Error = AppError;

    fn setup(&mut self, context: &mut AppContext<'_>) -> AppResult<()> {
        self.initialize(context).map_err(AppError::from)
    }

    fn frame_update(&mut self, context: &mut AppContext<'_>) -> AppResult<Flow> {
        if self.options.frames.is_some() && self.started.elapsed() > Duration::from_secs(60) {
            return Err("native smoke run timed out".into());
        }
        if self.size != context.size {
            if let (Some(renderer), Some(gal)) = (&mut self.renderer, &mut self.gal) {
                let _ = renderer.release_targets(gal);
            }
            self.surface
                .as_mut()
                .unwrap()
                .resize(context.size.0, context.size.1);
            self.size = context.size;
        }
        if context.focus_changed == Some(false) {
            self.pause_pending = true;
        }
        if context.input.action("quit").pressed {
            context.input.consume_action("quit");
            return Ok(Flow::Exit);
        }
        if context.input.action("fullscreen").pressed {
            context.input.consume_action("fullscreen");
            if let Some(window) = context.window {
                window.toggle_fullscreen();
            }
        }
        if context.input.action("filter").pressed {
            context.input.consume_action("filter");
            self.display.filter = if self.display.filter == Filter::Nearest {
                Filter::Linear
            } else {
                Filter::Nearest
            };
        }
        if context.input.action("scale").pressed {
            context.input.consume_action("scale");
            self.display.scaling = if self.display.scaling == Scaling::Integer {
                Scaling::Fit
            } else {
                Scaling::Integer
            };
        }
        Ok(Flow::Continue)
    }

    fn tick(&mut self, context: &mut AppContext<'_>) -> AppResult<Flow> {
        let mut input = self.controls.input(context.input);
        if self.pause_pending {
            input.pause = true;
            self.pause_pending = false;
        }
        if !self
            .session
            .step(input, self.options.autoplay)
            .map_err(AppError::from)?
        {
            return Ok(Flow::Exit);
        }
        if self.session.done(&self.options) {
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    fn render(&mut self, context: &mut AppContext<'_>) -> AppResult<Flow> {
        self.redraw(context).map_err(AppError::from)
    }

    fn suspended(&mut self, _: &mut AppContext<'_>) -> AppResult<()> {
        self.teardown(false).map_err(AppError::from)
    }

    fn shutdown(&mut self, _: &mut AppContext<'_>) -> AppResult<()> {
        self.teardown(true).map_err(AppError::from)
    }
}

impl DesktopApplication for App {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> AppResult<()> {
        if self.gal.is_none() {
            self.initialize(context).map_err(AppError::from)?;
        }
        Ok(())
    }

    fn configure_window(&mut self, window: &mut quasar_window::GameWindow) -> AppResult<()> {
        window.input.set_bindings(self.controls.bindings.clone())?;
        Ok(())
    }
}

fn capture(
    gpu: &Gpu,
    state: &mut RenderState2D,
    canvas: &Canvas,
    textures: &Textures,
) -> Result<Vec<u8>> {
    let target = OffscreenTarget::new(gpu, canvas.width(), canvas.height())?;
    Ok(target
        .render(gpu, |gpu, target| {
            Ok(vec![
                state
                    .prepare(
                        gpu,
                        canvas,
                        textures,
                        target,
                        DisplayOptions::default(),
                        LoadOp::Clear,
                    )?
                    .commands,
            ])
        })?
        .rgba)
}
