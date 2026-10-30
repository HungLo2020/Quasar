use quasar_render_2d::{
    Canvas, DisplayOptions, FramebufferPresenter, PresentOutcome, Rect, Renderer2D, Sprite,
    Textures, write_png,
};
use quasar_window::{
    GameWindow, WindowConfig,
    winit::{
        application::ApplicationHandler,
        dpi::LogicalSize,
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

const WIDTH: u32 = 640;
const HEIGHT: u32 = 400;

struct CountAlloc;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(size as u64, Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountAlloc = CountAlloc;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct Options {
    headless: bool,
    screenshot: Option<PathBuf>,
    frames: Option<u64>,
    exercise: bool,
    benchmark: Option<PathBuf>,
    poll: bool,
}
impl Options {
    fn parse() -> Result<Self> {
        let mut options = Self::default();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--headless" => options.headless = true,
                "--exercise-window" => options.exercise = true,
                "--poll" => options.poll = true,
                "--screenshot" => options.screenshot = Some(value()?.into()),
                "--benchmark" => options.benchmark = Some(value()?.into()),
                "--frames" => {
                    let frames: u64 = value()?.parse()?;
                    if frames == 0 {
                        return Err("frame count must be positive".into());
                    }
                    options.frames = Some(frames);
                }
                "--help" | "-h" => {
                    println!(
                        "framebuffer [--headless --screenshot FILE] [--frames N] [--exercise-window] [--poll] [--benchmark FILE]\nPresents a 640x400 RGBA8 software framebuffer through quasar-render-2d."
                    );
                    std::process::exit(0);
                }
                _ => return Err(format!("unknown argument {arg}; use --help").into()),
            }
        }
        if options.headless && (options.exercise || options.benchmark.is_some()) {
            return Err("headless runs cannot exercise a window or benchmark presentation".into());
        }
        if options.exercise && options.benchmark.is_some() {
            return Err("--exercise-window and --benchmark are mutually exclusive".into());
        }
        if options.benchmark.is_some() && options.frames.is_some_and(|n| n <= 64) {
            return Err("benchmarks need more than 64 frames".into());
        }
        if options.exercise {
            options.frames = Some(options.frames.unwrap_or(120).max(24));
        }
        Ok(options)
    }
}

fn paint(pixels: &mut [u8], frame: u64) {
    let shift = (frame % 256) as u8;
    for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = (index % WIDTH as usize) as u8;
        let y = (index / WIDTH as usize) as u8;
        pixel.copy_from_slice(&[x.wrapping_add(shift), y, (x ^ y).wrapping_add(shift), 255]);
    }
}

struct Bench {
    cpu: Vec<u64>,
    gpu: Vec<u64>,
    gpu_pending: Vec<quasar_render_2d::GpuTiming>,
    uploads: u64,
    waits: u64,
    allocations: u64,
    bytes: u64,
    started: Instant,
}
impl Bench {
    fn begin(_presenter: &FramebufferPresenter) -> Self {
        Self {
            cpu: Vec::new(),
            gpu: Vec::new(),
            gpu_pending: Vec::new(),
            uploads: 0,
            waits: 0,
            allocations: ALLOCS.load(Ordering::Relaxed),
            bytes: ALLOC_BYTES.load(Ordering::Relaxed),
            started: Instant::now(),
        }
    }
    fn sample(&mut self, presenter: &mut FramebufferPresenter, cpu_nanos: u64) {
        self.cpu.push(cpu_nanos);
        self.uploads += presenter.stats().texture_upload_bytes as u64;
        let r = presenter.renderer.frame_report();
        self.waits += r.blocking_waits;
        if let Some(t) = &r.gpu_timing {
            self.gpu_pending.push(t.clone());
        } else if let Some(n) = r.gpu_frame_nanos {
            self.gpu.push(n);
        }
    }
    fn write(&self, path: &PathBuf, presenter: &FramebufferPresenter, poll: bool) -> Result {
        presenter.renderer.gpu.wait()?;
        let gpu_samples: Vec<_> = self
            .gpu
            .iter()
            .copied()
            .chain(
                self.gpu_pending
                    .iter()
                    .filter_map(quasar_render_2d::GpuTiming::nanos),
            )
            .collect();
        let report = serde_json::json!({"build":if cfg!(debug_assertions){"debug"}else{"release"},"device":presenter.device_name(),"policy":if poll{"completion-poll"}else{"synchronous"},"frames":self.cpu.len(),"warmup_frames":64,"elapsed_ms":self.started.elapsed().as_secs_f64()*1000.,"cpu_present_ms":distribution(&self.cpu),"gpu_frame_ms":distribution(&gpu_samples),"texture_upload_bytes":self.uploads,"blocking_waits":self.waits,"rust_allocations":ALLOCS.load(Ordering::Relaxed)-self.allocations,"rust_allocated_bytes":ALLOC_BYTES.load(Ordering::Relaxed)-self.bytes});
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        println!("benchmark={}", path.display());
        Ok(())
    }
}

fn distribution(values: &[u64]) -> serde_json::Value {
    if values.is_empty() {
        return serde_json::Value::Null;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let ms = |nanos: u64| nanos as f64 / 1_000_000.;
    serde_json::json!({
        "samples": sorted.len(),
        "mean": ms(sorted.iter().copied().sum::<u64>() / sorted.len() as u64),
        "p50": ms(sorted[sorted.len() / 2]),
        "p95": ms(sorted[(sorted.len() - 1) * 95 / 100]),
        "p99": ms(sorted[(sorted.len() - 1) * 99 / 100]),
    })
}

struct App {
    options: Options,
    window: Option<GameWindow>,
    presenter: Option<FramebufferPresenter>,
    pixels: Vec<u8>,
    presented: u64,
    stage: u8,
    benchmark: Option<Bench>,
    started: Instant,
    error: Option<String>,
}
impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl std::fmt::Display) {
        self.error = Some(error.to_string());
        event_loop.exit();
    }
    fn redraw(&mut self, event_loop: &ActiveEventLoop) -> Result {
        let Some(window) = self.window.as_ref() else {
            return Ok(());
        };
        if !window.drawable() {
            return Ok(());
        }
        paint(&mut self.pixels, self.presented);
        let presenter = self.presenter.as_mut().unwrap();
        let started = Instant::now();
        let outcome = presenter.present(&self.pixels)?;
        let cpu_nanos = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        if outcome == PresentOutcome::Presented {
            self.presented += 1;
            if self.options.benchmark.is_some() {
                if self.presented == 65 {
                    self.benchmark = Some(Bench::begin(presenter));
                } else if self.presented > 65
                    && let Some(bench) = &mut self.benchmark
                {
                    bench.sample(presenter, cpu_nanos);
                }
            }
        }
        if self.options.exercise {
            self.exercise(event_loop)?;
        }
        if self
            .options
            .frames
            .is_some_and(|limit| self.presented >= limit)
        {
            event_loop.exit();
        }
        Ok(())
    }
    fn exercise(&mut self, event_loop: &ActiveEventLoop) -> Result {
        let window = self.window.as_ref().unwrap();
        let presenter = self.presenter.as_mut().unwrap();
        match self.stage {
            0 if self.presented >= 2 => {
                let _ = window
                    .native()
                    .request_inner_size(LogicalSize::new(960, 640));
                self.stage = 1;
            }
            1 if self.presented >= 6 => {
                window.set_fullscreen(true);
                self.stage = 2;
            }
            2 if self.presented >= 10 => {
                window.set_fullscreen(false);
                self.stage = 3;
            }
            3 if self.presented >= 14 => {
                presenter.resize(0, 0);
                if presenter.present(&self.pixels)? != PresentOutcome::Deferred {
                    return Err("zero-sized framebuffer must defer presentation".into());
                }
                let size = window.native().inner_size();
                presenter.resize(size.width.max(1), size.height.max(1));
                self.stage = 4;
            }
            4 if self.presented >= 18 => {
                if presenter.present(&self.pixels)? != PresentOutcome::Presented {
                    return Err("framebuffer did not recover after zero size".into());
                }
                self.presented += 1;
                self.stage = 5;
            }
            _ => {}
        }
        let _ = event_loop;
        Ok(())
    }
    fn cleanup(&mut self) {
        if let (Some(presenter), Some(path)) = (&mut self.presenter, &self.options.screenshot)
            && let Ok(pixels) = presenter.render_to_image(&self.pixels)
            && let Err(error) = write_png(path, WIDTH, HEIGHT, &pixels)
        {
            self.error = Some(error.to_string());
        }
        if let (Some(bench), Some(presenter), Some(path)) =
            (&self.benchmark, &self.presenter, &self.options.benchmark)
            && let Err(error) = bench.write(path, presenter, self.options.poll)
        {
            self.error = Some(error.to_string());
        }
        self.presenter.take();
        self.window.take();
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| -> Result {
            let window = GameWindow::create(
                event_loop,
                &WindowConfig {
                    title: "Quasar / Framebuffer".into(),
                    application_id: "quasar-framebuffer".into(),
                    ..Default::default()
                },
            )?;
            let size = window.native().inner_size();
            let mut presenter = FramebufferPresenter::new(
                window.shared_native(),
                (size.width.max(1), size.height.max(1)),
                (WIDTH, HEIGHT),
                DisplayOptions::default(),
            )?;
            presenter.set_frame_overlap(self.options.poll);
            presenter
                .renderer
                .gpu
                .set_gpu_timestamps(self.options.benchmark.is_some());
            self.presenter = Some(presenter);
            self.window = Some(window);
            self.started = Instant::now();
            Ok(())
        })();
        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = &mut self.window else {
            return;
        };
        if window.native().id() != id {
            return;
        }
        let update = window.process(&event);
        if update.close_requested {
            event_loop.exit();
            return;
        }
        if let Some(size) = update.resized {
            self.presenter
                .as_mut()
                .unwrap()
                .resize(size.width, size.height);
        }
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.state == ElementState::Pressed
            && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
        {
            event_loop.exit();
            return;
        }
        if update.redraw_requested
            && let Err(error) = self.redraw(event_loop)
        {
            self.fail(event_loop, error);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.options.frames.is_some() && self.started.elapsed() > Duration::from_secs(60) {
            self.fail(event_loop, "native framebuffer run timed out");
            return;
        }
        if self.window.as_ref().is_some_and(GameWindow::drawable) {
            self.window.as_ref().unwrap().native().request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::Poll);
    }
    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.cleanup();
    }
}

fn main() -> Result {
    let options = Options::parse()?;
    if options.headless {
        return headless(&options);
    }
    let frames = options.frames.unwrap_or(if options.benchmark.is_some() {
        364
    } else {
        120
    });
    let mut app = App {
        options: Options {
            frames: Some(frames),
            ..options
        },
        window: None,
        presenter: None,
        pixels: vec![0; (WIDTH * HEIGHT * 4) as usize],
        presented: 0,
        stage: 0,
        benchmark: None,
        started: Instant::now(),
        error: None,
    };
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.error.take() {
        return Err(error.into());
    }
    Ok(())
}

fn headless(options: &Options) -> Result {
    let output = options
        .screenshot
        .clone()
        .unwrap_or_else(|| "framebuffer.png".into());
    let mut renderer = Renderer2D::headless()?;
    let mut textures = Textures::default();
    let dynamic = renderer.create_dynamic_texture(&mut textures, WIDTH, HEIGHT)?;
    let mut canvas = Canvas::new(WIDTH, HEIGHT)?;
    canvas.sprite(Sprite::new(
        dynamic.id(),
        Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
    ));
    let mut pixels = vec![0; (WIDTH * HEIGHT * 4) as usize];
    paint(&mut pixels, 0);
    renderer.update_dynamic_texture(dynamic, &pixels)?;
    let image = renderer.render_to_image(&canvas, &textures)?;
    write_png(&output, WIDTH, HEIGHT, &image)?;
    println!("wrote {}", output.display());
    Ok(())
}
