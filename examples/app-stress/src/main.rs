//! Internal integration stress harness for the application runner.
//!
//! It combines a fixed simulation, variable-rate camera/UI work, frame input
//! consumption, background jobs, an asynchronous asset result, pause/deferred
//! ticks and native 2D rendering. It is intentionally not a general engine.
use quasar_app::{
    AppConfig, AppContext, Application, DesktopApplication, Flow, HeadlessConfig, TimingConfig,
    run as run_desktop, run_headless,
};
use quasar_core::FixedStepConfig;
use quasar_input::{Binding, Bindings, Key, MouseButton};
use quasar_jobs::{JobHandle, JobPool, JobPoolConfig};
use quasar_render_2d::{Canvas, Color, Rect, Renderer2D, Textures};
use quasar_window::{DesktopConfig, WindowConfig};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::time::Duration;

#[derive(Debug)]
struct AppError(String);
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AppError {}
impl From<Box<dyn std::error::Error>> for AppError {
    fn from(error: Box<dyn std::error::Error>) -> Self {
        Self(error.to_string())
    }
}
impl From<quasar_render_2d::Error> for AppError {
    fn from(error: quasar_render_2d::Error) -> Self {
        Self(error.to_string())
    }
}
impl From<quasar_render_2d::GraphicsError> for AppError {
    fn from(error: quasar_render_2d::GraphicsError) -> Self {
        Self(error.to_string())
    }
}
impl From<quasar_input::BindingError> for AppError {
    fn from(error: quasar_input::BindingError) -> Self {
        Self(error.to_string())
    }
}
type Result<T = ()> = std::result::Result<T, AppError>;

#[derive(Default)]
struct Options {
    headless: bool,
    ticks: u64,
    frames: u64,
    exercise: bool,
    fail_setup: bool,
}
impl Options {
    fn parse() -> Result<Self> {
        let mut options = Self {
            ticks: 120,
            frames: 120,
            ..Self::default()
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--headless" => options.headless = true,
                "--exercise-window" => options.exercise = true,
                "--fail-setup" => options.fail_setup = true,
                "--ticks" => {
                    options.ticks = args
                        .next()
                        .ok_or_else(|| AppError("--ticks needs a value".into()))?
                        .parse()
                        .map_err(|_| AppError("--ticks must be an integer".into()))?;
                }
                "--frames" => {
                    options.frames = args
                        .next()
                        .ok_or_else(|| AppError("--frames needs a value".into()))?
                        .parse()
                        .map_err(|_| AppError("--frames must be an integer".into()))?;
                }
                "--help" | "-h" => {
                    println!(
                        "app-stress [--headless] [--ticks N] [--frames N] [--exercise-window] [--fail-setup]"
                    );
                    std::process::exit(0);
                }
                other => return Err(AppError(format!("unknown argument {other}"))),
            }
        }
        Ok(options)
    }
}

struct StressApp {
    options: Options,
    frames_seen: u64,
    ticks_seen: u64,
    camera: f32,
    paused: bool,
    value: f32,
    hash: u64,
    setup_calls: u32,
    suspended_calls: u32,
    shutdown_calls: u32,
    job_pool: JobPool,
    job: Option<JobHandle<u64>>,
    asset_rx: Receiver<String>,
    asset_value: Option<String>,
    network: Vec<String>,
    renderer: Option<Renderer2D>,
    textures: Textures,
    size: (u32, u32),
    exercise_stage: u8,
}

impl StressApp {
    fn new(options: Options) -> Result<Self> {
        let pool = JobPool::new(JobPoolConfig::new(1, 8))
            .map_err(|error| AppError(format!("job pool: {error}")))?;
        let (asset_tx, asset_rx) = sync_channel(2);
        let job = spawn_asset_job(&pool, asset_tx)
            .map_err(|error| AppError(format!("asset job: {error}")))?;
        Ok(Self {
            options,
            frames_seen: 0,
            ticks_seen: 0,
            camera: 0.0,
            paused: false,
            value: 0.0,
            hash: 0xcbf29ce484222325,
            setup_calls: 0,
            suspended_calls: 0,
            shutdown_calls: 0,
            job_pool: pool,
            job: Some(job),
            asset_rx,
            asset_value: None,
            network: Vec::new(),
            renderer: None,
            textures: Textures::default(),
            size: (0, 0),
            exercise_stage: 0,
        })
    }

    fn poll_background(&mut self) {
        if let Some(job) = &self.job
            && let Some(result) = job.try_take()
        {
            self.job = None;
            match result {
                Ok(value) => self.value = value as f32,
                Err(error) => self.network.push(format!("job error: {error}")),
            }
        }
        loop {
            match self.asset_rx.try_recv() {
                Ok(asset) => self.asset_value = Some(asset),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        if self.frames_seen.is_multiple_of(11) {
            self.network.push(format!("snapshot:{}", self.ticks_seen));
            if self.network.len() > 16 {
                self.network.remove(0);
            }
        }
    }

    fn update_frame(
        &mut self,
        context: &mut AppContext<'_>,
        input: FrameInputView,
    ) -> Result<Flow> {
        self.frames_seen += 1;
        self.camera = (self.frames_seen as f32 * 0.02).sin();
        if self.size != context.size {
            if let Some(renderer) = &mut self.renderer {
                renderer.resize(context.size.0, context.size.1);
            }
            self.size = context.size;
        }
        self.poll_background();
        if input.ui_toggle {
            self.paused = !self.paused;
        }
        if input.pause {
            self.paused = true;
        }
        if input.resume {
            self.paused = false;
        }
        if input.fail_now {
            return Err(AppError("requested frame failure".into()));
        }
        if let Some(window) = context.window {
            if input.fullscreen {
                window.toggle_fullscreen();
            }
            if self.options.exercise {
                match self.exercise_stage {
                    0 if self.frames_seen >= 2 => {
                        let _ = window.native().request_inner_size(
                            quasar_window::winit::dpi::LogicalSize::new(900, 600),
                        );
                        self.exercise_stage = 1;
                    }
                    1 if self.frames_seen >= 5 => {
                        window.set_fullscreen(true);
                        self.exercise_stage = 2;
                    }
                    2 if self.frames_seen >= 8 => {
                        window.set_fullscreen(false);
                        self.exercise_stage = 3;
                    }
                    _ => {}
                }
            }
        }
        Ok(Flow::Continue)
    }

    fn update_tick(&mut self) -> Flow {
        if self.paused {
            // Deferring preserves unconsumed edges for the frame that resumes.
            return Flow::Deferred;
        }
        self.ticks_seen += 1;
        self.hash = (self.hash ^ self.ticks_seen)
            .wrapping_mul(0x100000001b3)
            .rotate_left(7);
        if self.ticks_seen.is_multiple_of(17) {
            self.network.push(format!("tick:{}", self.ticks_seen));
        }
        Flow::Continue
    }

    fn render_frame(&mut self, context: &AppContext<'_>) -> Result<Flow> {
        let Some(renderer) = &mut self.renderer else {
            return Ok(Flow::Continue);
        };
        if !context.drawable() {
            return Ok(Flow::Continue);
        }
        let size = context.size;
        let mut canvas = Canvas::new(size.0.max(1), size.1.max(1))?;
        canvas.clear = Color([10, 14, 24, 255]);
        let phase = (self.camera + 1.0) * 0.5;
        canvas.rect(
            Rect::new(40.0, 40.0, 200.0 + phase * 200.0, 32.0),
            Color([60, 150, 220, 255]),
        );
        canvas.rect(
            Rect::new(40.0, 90.0, (self.value / 8.0) * 200.0, 16.0),
            Color([220, 180, 70, 255]),
        );
        canvas.text(
            [40.0, 130.0],
            if self.paused { "PAUSED" } else { "RUNNING" },
            2.0,
            [240, 240, 240, 255],
        );
        canvas.text(
            [40.0, 154.0],
            &format!(
                "ticks={} frame={} jobs={} network={}",
                self.ticks_seen,
                self.frames_seen,
                self.value,
                self.network.len()
            ),
            1.0,
            [170, 190, 210, 255],
        );
        if let Some(asset) = &self.asset_value {
            canvas.text([40.0, 172.0], asset, 1.0, [130, 240, 170, 255]);
        }
        renderer.present(&canvas, &self.textures)?;
        if self.options.exercise && self.exercise_stage == 3 && self.frames_seen >= 12 {
            if let Some(window) = context.window {
                let _ = window
                    .native()
                    .request_inner_size(quasar_window::winit::dpi::LogicalSize::new(640, 400));
            }
            self.exercise_stage = 4;
        }
        if self.frames_seen >= self.options.frames {
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }
}

#[derive(Clone, Copy, Default)]
struct FrameInputView {
    ui_toggle: bool,
    pause: bool,
    resume: bool,
    fullscreen: bool,
    fail_now: bool,
}

impl Application for StressApp {
    type Error = AppError;

    fn setup(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        self.setup_calls += 1;
        if self.options.fail_setup {
            return Err(AppError("requested setup failure".into()));
        }
        if let Some(window) = context.window {
            self.renderer = Some(Renderer2D::for_window(
                window.shared_native(),
                (context.size.0.max(1), context.size.1.max(1)),
                quasar_render_2d::DisplayOptions::default(),
            )?);
        }
        self.size = context.size;
        Ok(())
    }

    fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        let input = if context.window.is_some() {
            FrameInputView {
                ui_toggle: context.input.action("ui_toggle").pressed,
                pause: context.input.action("pause").pressed,
                resume: context.input.action("resume").pressed,
                fullscreen: context.input.action("fullscreen").pressed,
                fail_now: context.input.action("fail").pressed,
            }
        } else {
            FrameInputView {
                ui_toggle: self.frames_seen > 0 && self.frames_seen.is_multiple_of(23),
                pause: self.frames_seen == 7,
                resume: self.frames_seen == 11,
                ..FrameInputView::default()
            }
        };
        if context.window.is_some() {
            context.input.consume_action("ui_toggle");
            context.input.consume_action("pause");
            context.input.consume_action("resume");
            context.input.consume_action("fullscreen");
            context.input.consume_action("fail");
        }
        self.update_frame(context, input)
    }

    fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow> {
        Ok(self.update_tick())
    }

    fn render(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        self.render_frame(context)
    }

    fn suspended(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        self.suspended_calls += 1;
        self.renderer.take();
        Ok(())
    }

    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        self.shutdown_calls += 1;
        let _ = self.job_pool.shutdown();
        println!(
            "app-stress setup={} suspended={} shutdown={} frames={} ticks={} hash={:016x}",
            self.setup_calls,
            self.suspended_calls,
            self.shutdown_calls,
            self.frames_seen,
            self.ticks_seen,
            self.hash
        );
        Ok(())
    }
}

impl DesktopApplication for StressApp {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        if self.renderer.is_none()
            && let Some(window) = context.window
        {
            self.renderer = Some(Renderer2D::for_window(
                window.shared_native(),
                (context.size.0.max(1), context.size.1.max(1)),
                quasar_render_2d::DisplayOptions::default(),
            )?);
            self.size = context.size;
        }
        Ok(())
    }

    fn configure_window(&mut self, window: &mut quasar_window::GameWindow) -> Result<()> {
        let mut bindings = Bindings::default();
        bindings
            .bind("ui_toggle", Binding::key(Key::Tab))
            .bind("pause", Binding::key(Key::KeyP))
            .bind("resume", Binding::key(Key::KeyO))
            .bind("fullscreen", Binding::key(Key::F11))
            .bind("fail", Binding::key(Key::KeyX))
            .bind("look", Binding::axis(Key::KeyA, Key::KeyD))
            .bind(
                "click",
                Binding::Button {
                    button: quasar_input::Button::Mouse(MouseButton::Left),
                },
            );
        window
            .input
            .set_bindings(bindings)
            .map_err(|error: quasar_input::BindingError| AppError(error.to_string()))?;
        Ok(())
    }
}

fn spawn_asset_job(
    pool: &JobPool,
    tx: SyncSender<String>,
) -> std::result::Result<JobHandle<u64>, quasar_jobs::JobError> {
    pool.spawn(move |_| {
        tx.send("async asset loaded".into()).ok();
        Ok::<_, std::convert::Infallible>(7)
    })
}

fn main() -> Result<()> {
    let options = Options::parse()?;
    if options.headless {
        let ticks = options.ticks;
        let app = StressApp::new(Options {
            headless: true,
            ..options
        })?;
        run_headless(
            app,
            HeadlessConfig {
                ticks,
                step: Duration::from_secs_f64(1.0 / 60.0),
            },
        )?;
        return Ok(());
    }
    let app = StressApp::new(options)?;
    run_desktop(
        app,
        AppConfig {
            window: WindowConfig {
                title: "Quasar App Stress".into(),
                application_id: "quasar-app-stress".into(),
                size: (800, 500),
                minimum_size: (400, 300),
                ..Default::default()
            },
            desktop: DesktopConfig::for_application("quasar-app-stress"),
            timing: TimingConfig::Fixed(FixedStepConfig::for_rate(60.0, 8)),
            render: quasar_app::RenderPolicy::EveryFrame,
            max_frame_wait: Duration::from_millis(16),
            frame_interval: None,
        },
    )?;
    Ok(())
}
