use quasar_composition::{Demo, Offscreen};
use quasar_frame::{CompletionPolicy, FrameReport, FrameSurface, Gpu};
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
use vulkanic_gal::*;
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
    if o.headless {
        let gal = Gpu::headless()?;
        let mut demo = Demo::new(&gal)?;
        let mut target = Offscreen::new(&gal, 800, 600)?;
        let image = target.image(&gal, &mut demo, 0.7, true)?;
        write_png(
            o.screenshot.unwrap_or_else(|| "composition.png".into()),
            800,
            600,
            &image,
        )?;
        demo.release(&gal)?;
        target.release(&gal)?;
        gal.wait()?;
        return Ok(());
    }
    let mut app = App {
        options: o,
        window: None,
        gal: None,
        demo: None,
        surface: None,
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
    gal: Option<Gpu>,
    demo: Option<Demo>,
    surface: Option<FrameSurface>,
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
        let (gal, mut surface) =
            FrameSurface::for_window(window.shared_native(), size.width, size.height)?;
        if self.options.benchmark.is_some() {
            surface.config.present_mode = PresentMode::AutoNoVsync;
            surface.reconfigure(&gal)?;
        }
        self.device_creations += 1;
        let demo = Demo::new(&gal)?;
        self.surface = Some(surface);
        self.window = Some(window);
        self.demo = Some(demo);
        gal.set_gpu_timestamps(self.options.benchmark.is_some());
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
        let surface = self.surface.as_mut().unwrap();
        let report = if self.options.explicit {
            if let Some(frame) = surface.acquire(gal)? {
                let started = Instant::now();
                let commands = demo.commands(gal, &frame.target, self.angle, true)?;
                let prepare_nanos = quasar_frame::nanos(started.elapsed());
                // The application records native command buffers; presentation submits them unchanged.
                let mut report = surface.present(gal, frame, commands, policy)?;
                report.prepare_nanos = prepare_nanos;
                Some(report)
            } else {
                None
            }
        } else {
            surface.render(gal, policy, |gal, target| {
                demo.commands(gal, target, self.angle, true)
            })?
        };
        if let Some(report) = report {
            self.frames += 1;
            self.angle += 0.012;
            if self.options.benchmark.is_some() {
                if self.frames == 64 {
                    self.bench.begin(gal);
                } else if self.frames > 64 {
                    self.bench.sample(&report);
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
                self.surface.as_mut().unwrap().resize(0, 0);
                if self
                    .surface
                    .as_mut()
                    .unwrap()
                    .render(gal, CompletionPolicy::Poll, |gal, target| {
                        demo.commands(gal, target, self.angle, true)
                    })?
                    .is_some()
                {
                    return Err("zero size must defer".into());
                }
                let s = window.native().inner_size();
                self.surface.as_mut().unwrap().resize(s.width, s.height);
                self.stage = 4;
            }
            4 if self.frames >= 18 => {
                let failure =
                    self.surface
                        .as_mut()
                        .unwrap()
                        .render(gal, CompletionPolicy::Poll, |_, _| {
                            Err(quasar_frame::error("intentional preparation failure"))
                        });
                if failure.is_ok() {
                    return Err("preparation failure was ignored".into());
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
        if let Some(gal) = self.gal.take() {
            self.surface.as_mut().unwrap().finish(&gal)?;
            if let Some(path) = &self.options.benchmark {
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
                    let mut target = Offscreen::new(&gal, 800, 600)?;
                    let image = target.image(&gal, &mut demo, self.angle, true)?;
                    write_png(path, 800, 600, &image)?;
                    target.release(&gal)?;
                }
                demo.release(&gal)?;
            }
            self.surface.take();
            gal.wait()?;
            if let Some(report) = gal.instance.generate_report() {
                let h = &report.hub;
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
                    return Err("resource ownership leaked after teardown".into());
                }
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
            self.surface.as_mut().unwrap().resize(s.width, s.height);
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
    gpu_pending: Vec<quasar_frame::GpuTiming>,
    waits: u64,
    allocs: u64,
    bytes: u64,
    start: Option<Instant>,
}
impl Bench {
    fn begin(&mut self, _gpu: &Gpu) {
        self.allocs = ALLOCS.load(Ordering::Relaxed);
        self.bytes = ALLOC_BYTES.load(Ordering::Relaxed);
        self.start = Some(Instant::now());
    }
    fn sample(&mut self, r: &FrameReport) {
        self.cpu.push(r.cpu_frame_nanos);
        self.acquire.push(r.acquire_nanos);
        self.present.push(r.present_nanos);
        self.submit.push(r.submit_nanos);
        self.waits += r.blocking_waits;
        if let Some(t) = &r.gpu_timing {
            self.gpu_pending.push(t.clone());
        } else if let Some(n) = r.gpu_frame_nanos {
            self.gpu.push(n);
        }
    }
    fn write(
        &self,
        path: &PathBuf,
        gpu: &Gpu,
        sync: bool,
        explicit: bool,
        device_creations: u64,
    ) -> Result {
        if self.cpu.is_empty() {
            return Err("benchmark needs more than 64 frames".into());
        }
        let gpu_samples: Vec<_> = self
            .gpu
            .iter()
            .copied()
            .chain(
                self.gpu_pending
                    .iter()
                    .filter_map(quasar_frame::GpuTiming::nanos),
            )
            .collect();
        let memory=gpu.device.generate_allocator_report().map(|r|serde_json::json!({"allocated_bytes":r.total_allocated_bytes,"reserved_bytes":r.total_reserved_bytes}));
        let report = serde_json::json!({"policy":if sync{"synchronous"}else{"completion-poll"},"explicit_submission":explicit,"build":if cfg!(debug_assertions){"debug"}else{"release"},"device":gpu.adapter.get_info().name,"device_creations":device_creations,"frames":self.cpu.len(),"elapsed_ms":self.start.unwrap().elapsed().as_secs_f64()*1000.,"cpu_frame_ms":distribution(&self.cpu),"acquire_ms":distribution(&self.acquire),"present_ms":distribution(&self.present),"cpu_submit_ms":distribution(&self.submit),"gpu_frame_ms":distribution(&gpu_samples),"blocking_waits":self.waits,"rust_allocations":ALLOCS.load(Ordering::Relaxed)-self.allocs,"rust_allocated_bytes":ALLOC_BYTES.load(Ordering::Relaxed)-self.bytes,"gpu_allocator":memory,"configured_frames":3,"timing_note":"Native timestamps when available; no synthetic resource or completion counters"});
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
