use super::*;
use quasar_core::{DisplayOptions, Filter, FixedStep, Scaling};
use quasar_frame::{CompletionPolicy, FrameSurface};
use quasar_render_2d::{Canvas, LoadOp, RenderState2D, Textures};
use quasar_window::{
    GameWindow, WindowConfig,
    winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        window::WindowId,
    },
};
use std::time::{Duration, Instant};
use vulkanic_gal::{resources::*, *};
pub fn run(options: Options, session: Session, controls: tilequest::controls::Controls) -> Result {
    set_gpu_timestamps_requested(options.benchmark.is_some());
    let mut app = App {
        options,
        session,
        controls,
        window: None,
        gal: None,
        renderer: None,
        surface: FrameSurface::new(960, 640),
        display: DisplayOptions {
            scaling: Scaling::Integer,
            ..Default::default()
        },
        clock: FixedStep::new(tilequest::HZ, 8)?,
        previous: Instant::now(),
        started: Instant::now(),
        frames: 0,
        stage: 0,
        error: None,
        samples: Vec::new(),
        draw_stats: quasar_tilemap::render::DrawStats::default(),
        baseline: None,
        gpu: Vec::new(),
        last_gpu: 0,
    };
    EventLoop::new()?.run_app(&mut app)?;
    if let Some(e) = app.error {
        return Err(e.into());
    }
    Ok(())
}
struct App {
    options: Options,
    session: Session,
    controls: tilequest::controls::Controls,
    window: Option<GameWindow>,
    gal: Option<VulkanicGal>,
    renderer: Option<RenderState2D>,
    surface: FrameSurface,
    display: DisplayOptions,
    clock: FixedStep,
    previous: Instant,
    started: Instant,
    frames: u64,
    stage: u8,
    error: Option<String>,
    samples: Vec<u64>,
    draw_stats: quasar_tilemap::render::DrawStats,
    baseline: Option<(u64, u64, u64, u64)>,
    gpu: Vec<u64>,
    last_gpu: u64,
}
impl App {
    fn initialize(&mut self, events: &ActiveEventLoop) -> Result {
        let mut window = GameWindow::create(
            events,
            &WindowConfig {
                title: "Tilequest / Quasar".into(),
                application_id: "quasar-tilequest".into(),
                size: (960, 640),
                minimum_size: (480, 320),
                ..Default::default()
            },
        )?;
        window.input.set_bindings(self.controls.bindings.clone())?;
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
        self.window = Some(window);
        self.clock.reset();
        self.previous = Instant::now();
        Ok(())
    }
    fn fail(&mut self, error: impl std::fmt::Display, events: &ActiveEventLoop) {
        self.error = Some(error.to_string());
        events.exit();
    }
    fn redraw(&mut self, events: &ActiveEventLoop) -> Result {
        if self.options.benchmark.is_some() {
            self.session.step(Input::default(), self.options.autoplay)?;
        }
        let gal = self.gal.as_mut().unwrap();
        let renderer = self.renderer.as_mut().unwrap();
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
        if let Some(report) = report {
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
                self.exercise()?;
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
                events.exit();
            }
        }
        Ok(())
    }
    fn exercise(&mut self) -> Result {
        let window = self.window.as_ref().unwrap();
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
        self.window.take();
        Ok(())
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(e) = self.initialize(events)
        {
            self.fail(e, events);
        }
    }
    fn suspended(&mut self, events: &ActiveEventLoop) {
        if let Err(e) = self.teardown(false) {
            self.fail(e, events);
        }
    }
    fn window_event(&mut self, events: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = &mut self.window else {
            return;
        };
        if window.native().id() != id {
            return;
        }
        let update = window.process(&event);
        if update.close_requested || window.input.take_pressed("quit") {
            events.exit();
            return;
        }
        if let Some(s) = update.resized {
            if let (Some(renderer), Some(gal)) = (&mut self.renderer, &mut self.gal)
                && let Err(e) = renderer.release_targets(gal)
            {
                self.fail(e, events);
                return;
            }
            self.surface.resize(s.width, s.height);
        }
        if update.focus_changed == Some(false)
            && !self.options.autoplay
            && self.session.replay.is_none()
        {
            if self.session.state.phase == tilequest::Phase::Playing {
                let _ = self.session.step(
                    Input {
                        pause: true,
                        ..Default::default()
                    },
                    false,
                );
            }
            self.clock.reset();
        }
        let window = self.window.as_mut().unwrap();
        if window.input.take_pressed("fullscreen") {
            window.toggle_fullscreen();
        }
        if window.input.take_pressed("filter") {
            self.display.filter = if self.display.filter == Filter::Nearest {
                Filter::Linear
            } else {
                Filter::Nearest
            };
        }
        if window.input.take_pressed("scale") {
            self.display.scaling = if self.display.scaling == Scaling::Integer {
                Scaling::Fit
            } else {
                Scaling::Integer
            };
        }
        if update.redraw_requested
            && let Err(e) = self.redraw(events)
        {
            self.fail(e, events);
        }
    }
    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        if self.options.frames.is_some() && self.started.elapsed() > Duration::from_secs(60) {
            self.fail("native smoke run timed out", events);
            return;
        }
        let Some(window) = &mut self.window else {
            return;
        };
        let now = Instant::now();
        if self.options.benchmark.is_none()
            && (window.focused() || self.options.autoplay || self.session.replay.is_some())
        {
            self.clock.advance(now - self.previous);
        }
        self.previous = now;
        while self.clock.take_tick() {
            let input = self.controls.input(&window.input);
            let result = self.session.step(input, self.options.autoplay);
            window.input.consume_edges();
            match result {
                Ok(true) => {}
                Ok(false) => {
                    events.exit();
                    return;
                }
                Err(e) => {
                    self.fail(e, events);
                    return;
                }
            }
            if self.session.done(&self.options) {
                events.exit();
                return;
            }
        }
        if window.drawable() {
            window.native().request_redraw();
        }
        events.set_control_flow(ControlFlow::WaitUntil(
            now + self.clock.until_next_tick().min(Duration::from_millis(16)),
        ));
    }
    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let Err(e) = self.teardown(true) {
            self.error = Some(e.to_string());
        }
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
