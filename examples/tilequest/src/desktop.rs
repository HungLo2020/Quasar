use super::*;
use quasar_app::{
    AppConfig, AppContext, Application, DesktopApplication, Flow, RenderPolicy, TimingConfig,
    run as run_app,
};
use quasar_core::{DisplayOptions, Filter, FixedStepConfig, Scaling};
use quasar_frame::{CompletionPolicy, FrameSurface};
use quasar_input::BindingError;
use quasar_render_2d::{Canvas, LoadOp, RenderState2D, Textures};
use quasar_window::{DesktopConfig, WindowConfig};
use std::time::{Duration, Instant};
use vulkanic_gal::{resources::*, *};

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
impl From<GalError> for AppError {
    fn from(error: GalError) -> Self {
        Self(Box::new(error))
    }
}
type AppResult<T> = std::result::Result<T, AppError>;

pub fn run(options: Options, session: Session, controls: tilequest::controls::Controls) -> Result {
    set_gpu_timestamps_requested(options.benchmark.is_some());
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
        surface: FrameSurface::new(960, 640),
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
    gal: Option<VulkanicGal>,
    renderer: Option<RenderState2D>,
    surface: FrameSurface,
    display: DisplayOptions,
    size: (u32, u32),
    frames: u64,
    stage: u8,
    started: Instant,
    samples: Vec<u64>,
    draw_stats: quasar_tilemap::render::DrawStats,
    baseline: Option<(u64, u64, u64, u64)>,
    gpu: Vec<u64>,
    last_gpu: u64,
    pause_pending: bool,
}

impl App {
    fn initialize(&mut self, context: &mut AppContext<'_>) -> Result {
        let window = context
            .window
            .ok_or("desktop runner did not provide a window")?;
        let size = window.native().inner_size();
        let mut gal = VulkanicGal::create_windowed_vulkan(
            "Tilequest",
            window.shared_native(),
            FrameSurfaceDesc {
                label: "tilequest-surface".into(),
                extent: extent(size.width, size.height),
                color_format: TextureFormat::Bgra8Unorm,
                present_mode: if self.options.benchmark.is_some() {
                    PresentMode::AutoNoVsync
                } else {
                    PresentMode::AutoVsync
                },
                max_frames_in_flight: 3,
            },
        )?;
        gal.install_gpu_profile_classifier(GpuProfileClassifier {
            classify: |kind, _| GpuProfileTag {
                timing_scope: if matches!(kind, GpuProfiledObject::RenderPass) {
                    Some(0)
                } else {
                    None
                },
                ..Default::default()
            },
            statistics_scope_name: |_| "tilequest",
        });
        self.renderer = Some(RenderState2D::new(&mut gal)?);
        self.surface = FrameSurface::new(size.width, size.height);
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
        let report = self
            .surface
            .render(gal, CompletionPolicy::Poll, |gal, target| {
                let (canvas, stats) = tilequest::scene(
                    &session.world,
                    &session.state,
                    &session.art,
                    self.display.filter,
                )
                .map_err(|e| GalError::invalid_argument(e.to_string()))?;
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
            let m = gal.backend_runtime_metrics();
            self.baseline = Some((
                gal.metrics().resource_creates,
                gal.metrics().resource_destroys,
                m.native_wait_count,
                m.native_device_wait_idle_count,
            ));
        }
        if self.options.benchmark.is_some() && self.frames > 64 {
            self.samples.push(report.cpu_frame_nanos);
            let metrics = gal.backend_runtime_metrics();
            if metrics.gpu_timestamp_status > self.last_gpu {
                self.gpu.push(metrics.gpu_frame_total_nanos);
                self.last_gpu = metrics.gpu_timestamp_status;
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
                self.surface.resize(0, 0);
                let gal = self.gal.as_mut().unwrap();
                if self.surface.acquire(gal)?.is_some() {
                    return Err("zero-sized surface must defer".into());
                }
                let size = window.native().inner_size();
                self.surface.resize(size.width, size.height);
                self.display.filter = Filter::Linear;
                self.display.scaling = Scaling::Fit;
                self.stage = 4;
            }
            _ => {}
        }
        Ok(())
    }

    fn teardown(&mut self, outputs: bool) -> Result {
        if let Some(mut gal) = self.gal.take() {
            self.surface.finish(&mut gal)?;
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
                            &mut gal,
                            &mut renderer,
                            &canvas,
                            &self.session.art.textures,
                        )?)
                    } else {
                        None
                    };
                    self.session.outputs(&self.options, pixels.as_deref())?;
                    if let Some(path) = &self.options.benchmark {
                        let base = self.baseline.ok_or("benchmark needs more than 64 frames")?;
                        let metrics = gal.backend_runtime_metrics();
                        let mut cpu = self.samples.clone();
                        cpu.sort_unstable();
                        let mut gpu = self.gpu.clone();
                        gpu.sort_unstable();
                        quasar_storage::write_json(
                            path,
                            &serde_json::json!({"frames":self.frames,"measured_frames":cpu.len(),"cpu_frame_p50_ms":cpu.get(cpu.len()/2).map(|v|*v as f64/1e6),"gpu_frame_p50_ms":gpu.get(gpu.len()/2).map(|v|*v as f64/1e6),"visited_cells":self.draw_stats.visited_cells,"emitted_tiles":self.draw_stats.emitted_tiles,"stored_cells":self.session.world.runtime.stored_cells(),"resource_creates":gal.metrics().resource_creates-base.0,"resource_destroys":gal.metrics().resource_destroys-base.1,"timeline_waits":metrics.native_wait_count-base.2,"device_idle_waits":metrics.native_device_wait_idle_count-base.3,"render_stats":{"quads":renderer.stats().quads,"batches":renderer.stats().batches,"geometry_bytes":renderer.stats().geometry_upload_bytes,"texture_bytes":renderer.stats().texture_upload_bytes},"scope":"native map traversal, draw-list construction, GPU rendering and presentation; first 64 frames excluded; one simulation tick per attempted frame"}),
                        )?;
                    }
                }
                renderer.release(&mut gal)?;
            }
            self.surface.release_targets(&mut gal)?;
            gal.retire_through(gal.latest_submission_id())?;
            gal.shutdown_frame_surface()?;
            println!(
                "resources_created={} resources_destroyed={}",
                gal.metrics().resource_creates,
                gal.metrics().resource_destroys
            );
            if gal.metrics().resource_creates != gal.metrics().resource_destroys {
                return Err("resource imbalance after native teardown".into());
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
            self.surface.resize(context.size.0, context.size.1);
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

fn extent(w: u32, h: u32) -> Extent3d {
    Extent3d {
        width: w,
        height: h,
        depth: 1,
    }
}
fn barrier(resource: Handle, before: TextureUsageState, after: TextureUsageState) -> CommandOp {
    CommandOp::Barrier(ResourceBarrier {
        resource,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    })
}
fn capture(
    gal: &mut VulkanicGal,
    state: &mut RenderState2D,
    canvas: &Canvas,
    textures: &Textures,
) -> Result<Vec<u8>> {
    let size = extent(canvas.width(), canvas.height());
    let image = gal.create_texture(TextureDesc {
        label: "quest-snapshot".into(),
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        extent: size,
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
    })?;
    let view = gal.create_texture_view(TextureViewDesc {
        label: "quest-snapshot-view".into(),
        texture: image,
        format: TextureFormat::Rgba8Unorm,
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    })?;
    let target = gal.create_render_target(RenderTargetDesc {
        label: "quest-snapshot-target".into(),
        color_views: vec![view],
        depth_stencil_view: None,
        extent: size,
    })?;
    let readback = gal.create_buffer(BufferDesc {
        label: "quest-snapshot-read".into(),
        size: u64::from(size.width) * u64::from(size.height) * 4,
        memory: MemoryDomain::Readback,
        usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
    })?;
    let result = (|| {
        let (token, _) = quasar_frame::record_and_submit(gal, |gal| {
            let prepared = state.prepare(
                gal,
                canvas,
                textures,
                target,
                DisplayOptions::default(),
                LoadOp::Clear,
            )?;
            let mut operations = vec![barrier(
                image,
                TextureUsageState::Undefined,
                TextureUsageState::ColorAttachment,
            )];
            operations.extend(prepared.commands.operations);
            operations.extend([
                barrier(
                    image,
                    TextureUsageState::ColorAttachment,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
                    buffer: readback,
                    buffer_offset: 0,
                    bytes_per_row: size.width * 4,
                    rows_per_image: size.height,
                    texture: image,
                    texture_mip: 0,
                    texture_layer: 0,
                    texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent: size,
                }),
                barrier(
                    readback,
                    TextureUsageState::TransferDst,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::HostReadBuffer {
                    buffer: readback,
                    offset: 0,
                    size: u64::from(size.width) * u64::from(size.height) * 4,
                },
            ]);
            Ok(vec![CommandList::from(CommandListDesc {
                label: "quest-snapshot".into(),
                operations,
            })])
        })?;
        gal.retire_through(token.submission)?;
        gal.completed_host_reads()
            .into_iter()
            .find(|r| r.buffer == readback && r.submission == token.submission)
            .map(|r| r.bytes)
            .ok_or_else(|| GalError::backend("missing quest snapshot"))
    })();
    state.release_targets(gal)?;
    for h in [target, view, image, readback] {
        gal.retire(h)?;
    }
    Ok(result?)
}
