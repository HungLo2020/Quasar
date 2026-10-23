use quasar_composition::{Demo, Offscreen, extent, submit_explicit};
use quasar_frame::{CompletionPolicy, FrameReport, FrameSurface};
use quasar_window::{
    GameWindow, WindowConfig,
    winit::{
        application::ApplicationHandler,
        event::{ElementState, WindowEvent},
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        keyboard::{KeyCode, PhysicalKey},
        window::WindowId,
    },
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use vulkanic_gal::{resources::*, *};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
struct CountAlloc;
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(n as u64, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static ALLOCATOR: CountAlloc = CountAlloc;
#[derive(Default)]
struct Options {
    headless: bool,
    screenshot: Option<PathBuf>,
    frames: Option<u64>,
    sync: bool,
    explicit: bool,
    exercise: bool,
    benchmark: Option<PathBuf>,
}
fn options() -> Result<Options> {
    let mut o = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--headless" => o.headless = true,
            "--sync" => o.sync = true,
            "--explicit" => o.explicit = true,
            "--exercise-window" => o.exercise = true,
            "--screenshot" => {
                o.screenshot = Some(args.next().ok_or("missing screenshot path")?.into())
            }
            "--frames" => {
                let n = args.next().ok_or("missing frame count")?.parse()?;
                if n == 0 {
                    return Err("frame count must be positive".into());
                }
                o.frames = Some(n);
            }
            "--benchmark" => {
                o.benchmark = Some(args.next().ok_or("missing benchmark path")?.into())
            }
            "--help" => {
                println!(
                    "cargo run -p quasar-composition -- [--headless] [--screenshot FILE] [--frames N] [--sync] [--explicit] [--exercise-window] [--benchmark FILE.json]\nEscape quits. Left/right arrows rotate the cube. Benchmark uses 64 warmup frames and disables vsync."
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {a}").into()),
        }
    }
    if o.benchmark.is_some() && o.frames.is_none() {
        o.frames = Some(364);
    }
    if o.benchmark.is_some() && (o.headless || o.exercise) {
        return Err("benchmark requires native frames without window exercises".into());
    }
    Ok(o)
}
fn main() -> Result {
    let o = options()?;
    set_gpu_timestamps_requested(true);
    if o.headless {
        let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "Quasar composition snapshot")?;
        let mut demo = Demo::new(&mut gal)?;
        let mut target = Offscreen::new(&mut gal, 800, 600)?;
        let image = target.image(&mut gal, &mut demo, 0.7, true)?;
        write_png(
            o.screenshot.unwrap_or_else(|| "composition.png".into()),
            800,
            600,
            &image,
        )?;
        demo.release(&mut gal)?;
        target.release(&mut gal)?;
        gal.retire_through(gal.latest_submission_id())?;
        return Ok(());
    }
    let mut app = App {
        options: o,
        window: None,
        gal: None,
        demo: None,
        surface: FrameSurface::new(800, 600),
        frames: 0,
        device_creations: 0,
        stage: 0,
        angle: 0.7,
        error: None,
        started: Instant::now(),
        bench: Bench::default(),
    };
    EventLoop::new()?.run_app(&mut app)?;
    if let Some(e) = app.error {
        return Err(e.into());
    }
    Ok(())
}
fn write_png(path: PathBuf, w: u32, h: u32, rgba: &[u8]) -> Result {
    let mut e = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path)?), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(rgba)?;
    Ok(())
}
struct App {
    options: Options,
    window: Option<GameWindow>,
    gal: Option<VulkanicGal>,
    demo: Option<Demo>,
    surface: FrameSurface,
    frames: u64,
    device_creations: u64,
    stage: u8,
    angle: f32,
    error: Option<String>,
    started: Instant,
    bench: Bench,
}
impl App {
    fn fail(&mut self, e: impl std::fmt::Display, events: &ActiveEventLoop) {
        self.error = Some(e.to_string());
        events.exit();
    }
    fn initialize(&mut self, events: &ActiveEventLoop) -> Result {
        let window = GameWindow::create(
            events,
            &WindowConfig {
                title: "Quasar / one device / 3D + 2D".into(),
                application_id: "quasar-composition".into(),
                size: (800, 600),
                minimum_size: (320, 240),
                ..Default::default()
            },
        )?;
        let size = window.native().inner_size();
        let mut gal = VulkanicGal::create_windowed_vulkan(
            "Quasar composition",
            window.shared_native(),
            FrameSurfaceDesc {
                label: "composition-surface".into(),
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
        self.device_creations += 1;
        gal.install_gpu_profile_classifier(GpuProfileClassifier {
            classify: |kind, label| {
                if matches!(kind, GpuProfiledObject::RenderPass) {
                    GpuProfileTag {
                        timing_scope: if label.starts_with("3d") {
                            Some(0)
                        } else if label.starts_with("2d") {
                            Some(1)
                        } else {
                            None
                        },
                        ..Default::default()
                    }
                } else {
                    GpuProfileTag::default()
                }
            },
            statistics_scope_name: |i| if i == 0 { "3d" } else { "2d" },
        });
        let demo = Demo::new(&mut gal)?;
        self.surface = FrameSurface::new(size.width, size.height);
        self.window = Some(window);
        self.demo = Some(demo);
        self.gal = Some(gal);
        Ok(())
    }
    fn redraw(&mut self, events: &ActiveEventLoop) -> Result {
        let Some(gal) = &mut self.gal else {
            return Ok(());
        };
        let demo = self.demo.as_mut().unwrap();
        let policy = if self.options.sync {
            CompletionPolicy::Wait
        } else {
            CompletionPolicy::Poll
        };
        let report = if self.options.explicit {
            let start = Instant::now();
            if let Some(frame) = self.surface.acquire(gal)? {
                let acquire_nanos = start.elapsed().as_nanos() as u64;
                let result = submit_explicit(gal, |gal| {
                    demo.commands(gal, frame.target, self.angle, true)
                });
                match result {
                    Ok((token, profile)) => {
                        let p = Instant::now();
                        if let Err(e) = self.surface.present(gal, &frame, token, policy) {
                            let _ = self.surface.cancel(gal, &frame);
                            return Err(e.into());
                        }
                        Some(FrameReport {
                            submission: token.submission,
                            profile,
                            acquire_nanos,
                            present_nanos: p.elapsed().as_nanos() as u64,
                            cpu_frame_nanos: start.elapsed().as_nanos() as u64,
                        })
                    }
                    Err(e) => {
                        let _ = self.surface.cancel(gal, &frame);
                        return Err(e.into());
                    }
                }
            } else {
                None
            }
        } else {
            self.surface.render(gal, policy, |gal, target| {
                demo.commands(gal, target, self.angle, true)
            })?
        };
        if let Some(report) = report {
            self.frames += 1;
            self.angle += 0.012;
            if self.options.benchmark.is_some() {
                if self.frames == 64 {
                    self.bench.begin(gal);
                    self.surface.take_completion_latencies();
                    self.surface.take_frame_completion_latencies();
                } else if self.frames > 64 {
                    self.bench.sample(gal, &report, &mut self.surface);
                }
            }
            if self.options.exercise {
                self.exercise()?;
            }
            if self.options.frames.is_some_and(|n| self.frames >= n) {
                if self.options.exercise && self.stage != 6 {
                    return Err(format!("incomplete window exercises: stage {}", self.stage).into());
                }
                println!(
                    "presented_frames={} window_exercise_stages={} device_creations={}",
                    self.frames, self.stage, self.device_creations
                );
                events.exit();
            }
        }
        Ok(())
    }
    fn exercise(&mut self) -> Result {
        let gal = self.gal.as_mut().unwrap();
        let demo = self.demo.as_mut().unwrap();
        let window = self.window.as_ref().unwrap();
        match self.stage {
            0 if self.frames >= 2 => {
                let _ = window
                    .native()
                    .request_inner_size(quasar_window::winit::dpi::LogicalSize::new(960, 640));
                self.stage = 1;
            }
            1 if self.frames >= 6 => {
                window.set_fullscreen(true);
                self.stage = 2;
            }
            2 if self.frames >= 10 => {
                window.set_fullscreen(false);
                self.stage = 3;
            }
            3 if self.frames >= 14 => {
                self.surface.resize(0, 0);
                if self
                    .surface
                    .render(gal, CompletionPolicy::Poll, |gal, target| {
                        demo.commands(gal, target, self.angle, true)
                    })?
                    .is_some()
                {
                    return Err("zero size must defer".into());
                }
                let s = window.native().inner_size();
                self.surface.resize(s.width, s.height);
                self.stage = 4;
            }
            4 if self.frames >= 18 => {
                let failure = self
                    .surface
                    .render(gal, CompletionPolicy::Poll, |gal, target| {
                        let mut commands = demo.commands(gal, target, self.angle, true)?;
                        commands.push(quasar_composition::list(
                            "intentional failure",
                            vec![CommandOp::Draw {
                                vertices: 3,
                                instances: 1,
                            }],
                        ));
                        Ok(commands)
                    });
                if failure.is_ok() {
                    return Err("invalid submission was accepted".into());
                }
                self.stage = 5;
            }
            5 if self.frames >= 24 => {
                self.stage = 6;
            }
            _ => {}
        }
        Ok(())
    }
    fn cleanup(&mut self) -> Result {
        if let Some(mut gal) = self.gal.take() {
            self.surface.finish(&mut gal)?;
            if let Some(path) = &self.options.benchmark {
                self.bench.sample_gpu(&gal);
                self.bench.latencies.extend(
                    self.surface
                        .take_completion_latencies()
                        .into_iter()
                        .filter(|(id, _)| id.0 >= self.bench.first_submission)
                        .map(|(_, n)| n),
                );
                self.bench.frame_latencies.extend(
                    self.surface
                        .take_frame_completion_latencies()
                        .into_iter()
                        .filter(|(id, _)| id.0 >= self.bench.first_submission)
                        .map(|(_, n)| n),
                );
                self.bench.write(
                    path,
                    &gal,
                    self.options.sync,
                    self.options.explicit,
                    self.device_creations,
                )?;
            }
            if let Some(mut demo) = self.demo.take() {
                if let Some(path) = self.options.screenshot.clone() {
                    let mut target = Offscreen::new(&mut gal, 800, 600)?;
                    let image = target.image(&mut gal, &mut demo, self.angle, true)?;
                    write_png(path, 800, 600, &image)?;
                    target.release(&mut gal)?;
                }
                demo.release(&mut gal)?;
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
                return Err("resource imbalance after teardown".into());
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
        if let Err(e) = self.cleanup() {
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
        if update.close_requested {
            events.exit();
            return;
        }
        if let Some(s) = update.resized {
            if let (Some(demo), Some(gal)) = (&mut self.demo, &mut self.gal)
                && let Err(e) = demo.release_targets(gal)
            {
                self.fail(e, events);
                return;
            }
            self.surface.resize(s.width, s.height);
        }
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.state == ElementState::Pressed
        {
            match event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => events.exit(),
                PhysicalKey::Code(KeyCode::ArrowLeft) => self.angle -= 0.15,
                PhysicalKey::Code(KeyCode::ArrowRight) => self.angle += 0.15,
                _ => {}
            }
        }
        if update.redraw_requested
            && let Err(e) = self.redraw(events)
        {
            self.fail(e, events);
        }
    }
    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        if self.options.frames.is_some() && self.started.elapsed() > Duration::from_secs(60) {
            self.fail("native run timed out", events);
            return;
        }
        if let Some(window) = &self.window
            && window.drawable()
        {
            window.native().request_redraw();
        }
        events.set_control_flow(ControlFlow::Poll);
    }
    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let Err(e) = self.cleanup() {
            self.error = Some(e.to_string());
        }
    }
}
#[derive(Default)]
struct Bench {
    cpu: Vec<u64>,
    acquire: Vec<u64>,
    present: Vec<u64>,
    submit: Vec<u64>,
    gpu: Vec<u64>,
    latencies: Vec<u64>,
    frame_latencies: Vec<u64>,
    last_gpu: u64,
    first_submission: u64,
    waits: u64,
    idles: u64,
    creates: u64,
    destroys: u64,
    allocs: u64,
    bytes: u64,
    uploads: u64,
    start: Option<Instant>,
}
impl Bench {
    fn begin(&mut self, gal: &VulkanicGal) {
        let m = gal.backend_runtime_metrics();
        self.waits = m.native_wait_count;
        self.idles = m.native_device_wait_idle_count;
        self.creates = gal.metrics().resource_creates;
        self.destroys = gal.metrics().resource_destroys;
        self.allocs = ALLOCS.load(Ordering::Relaxed);
        self.bytes = ALLOC_BYTES.load(Ordering::Relaxed);
        self.last_gpu = m.gpu_timestamp_status;
        self.first_submission = gal.latest_submission_id().0 + 1;
        self.start = Some(Instant::now());
    }
    fn sample(&mut self, gal: &VulkanicGal, r: &FrameReport, surface: &mut FrameSurface) {
        self.cpu.push(r.cpu_frame_nanos);
        self.acquire.push(r.acquire_nanos);
        self.present.push(r.present_nanos);
        self.submit.push(r.profile.gal_submit_total_nanos);
        self.uploads += r.profile.host_write_bytes;
        self.sample_gpu(gal);
        self.frame_latencies.extend(
            surface
                .take_frame_completion_latencies()
                .into_iter()
                .filter(|(id, _)| id.0 >= self.first_submission)
                .map(|(_, n)| n),
        );
        self.latencies.extend(
            surface
                .take_completion_latencies()
                .into_iter()
                .filter(|(id, _)| id.0 >= self.first_submission)
                .map(|(_, n)| n),
        );
    }
    fn sample_gpu(&mut self, gal: &VulkanicGal) {
        let m = gal.backend_runtime_metrics();
        if m.gpu_timestamp_status > self.last_gpu && m.gpu_timestamp_status >= self.first_submission
        {
            self.gpu.push(m.gpu_frame_total_nanos);
            self.last_gpu = m.gpu_timestamp_status;
        }
    }
    fn write(
        &self,
        path: &PathBuf,
        gal: &VulkanicGal,
        sync: bool,
        explicit: bool,
        device_creations: u64,
    ) -> Result {
        if self.cpu.is_empty() {
            return Err("benchmark needs more than 64 frames".into());
        }
        let m = gal.backend_runtime_metrics();
        let rust_allocations = ALLOCS.load(Ordering::Relaxed) - self.allocs;
        let rust_allocated_bytes = ALLOC_BYTES.load(Ordering::Relaxed) - self.bytes;
        let elapsed_ms = self.start.unwrap().elapsed().as_secs_f64() * 1000.;
        let report = serde_json::json!({
            "policy":if sync{"synchronous"}else{"completion-poll"},
            "explicit_submission":explicit,
            "build":if cfg!(debug_assertions){"debug"}else{"release"},
            "device":gal.capabilities().name,
            "device_creations":device_creations,
            "frames":self.cpu.len(),
            "elapsed_ms":elapsed_ms,
            "cpu_frame_ms":distribution(&self.cpu),
            "acquire_ms":distribution(&self.acquire),
            "present_and_retire_ms":distribution(&self.present),
            "cpu_submit_ms":distribution(&self.submit),
            "gpu_frame_ms":distribution(&self.gpu),
            "submission_to_observed_completion_ms":distribution(&self.latencies),
            "engine_frame_to_observed_completion_ms":distribution(&self.frame_latencies),
            "latency_note":"CPU observation of GPU completion; excludes display/compositor/input-to-photon latency; poll cadence affects observations",
            "blocking_timeline_waits":m.native_wait_count-self.waits,
            "device_idle_waits":m.native_device_wait_idle_count-self.idles,
            "resource_creates":gal.metrics().resource_creates-self.creates,
            "resource_destroys":gal.metrics().resource_destroys-self.destroys,
            "rust_allocations":rust_allocations,
            "rust_allocated_bytes":rust_allocated_bytes,
            "host_upload_bytes":self.uploads,
            "swapchain_images":m.native_swapchain_image_count,
            "configured_frames":m.native_configured_frames_in_flight
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        println!("benchmark={}", path.display());
        Ok(())
    }
}
fn distribution(values: &[u64]) -> serde_json::Value {
    if values.is_empty() {
        return serde_json::Value::Null;
    }
    let mut v = values.to_vec();
    v.sort_unstable();
    serde_json::json!({"samples":v.len(),"mean":v.iter().map(|n|*n as f64).sum::<f64>()/v.len() as f64/1e6,"p50":v[v.len()/2] as f64/1e6,"p95":v[(v.len()-1)*95/100] as f64/1e6,"p99":v[(v.len()-1)*99/100] as f64/1e6})
}
