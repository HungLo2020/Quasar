use physics_playground::settings::Settings;
use physics_playground::world::{Playground, PlaygroundInput};
use physics_playground::{PlaygroundError, Result, render};
use quasar_app::{
    AppConfig, AppContext, Application, DesktopApplication, Flow, RenderPolicy, TimingConfig,
    run as run_app,
};
use quasar_core::FixedStepConfig;
use quasar_diagnostics::{Field, Level as LogLevel};
use quasar_frame::{CompletionPolicy, FrameSurface, Gpu};
use quasar_input::{Key, MouseButton};
use quasar_profile::{FrameProfiler, FrameSample};
use quasar_window::{
    DesktopConfig, WindowConfig,
    winit::{
        event::{ElementState, WindowEvent},
        keyboard::{KeyCode, PhysicalKey},
    },
};
use std::time::Duration;
use vulkanic_gal::PresentMode;

pub fn run(settings: Settings, diagnostics: quasar_diagnostics::Diagnostics) -> Result<()> {
    let config = AppConfig {
        window: WindowConfig {
            title: "Physics Playground - Quasar".into(),
            application_id: "physics-playground".into(),
            size: (settings.width, settings.height),
            minimum_size: (640, 360),
            fullscreen: false,
            resizable: true,
        },
        desktop: DesktopConfig::for_application("physics-playground"),
        timing: TimingConfig::Fixed(FixedStepConfig::for_rate(60.0, 8)),
        render: RenderPolicy::EveryFrame,
        max_frame_wait: Duration::from_millis(16),
        frame_interval: None,
    };
    run_app(PlaygroundApp::new(settings, diagnostics), config)
        .map_err(|error| PlaygroundError::invalid(error.to_string()))
}

struct PlaygroundApp {
    settings: Settings,
    diagnostics: quasar_diagnostics::Diagnostics,
    world: Option<Playground>,
    graphics: Option<render::Graphics>,
    surface: Option<FrameSurface>,
    gal: Option<Gpu>,
    profiler: FrameProfiler,
    frames: u64,
    samples: usize,
    exit_after_render: bool,
}

impl PlaygroundApp {
    fn new(settings: Settings, diagnostics: quasar_diagnostics::Diagnostics) -> Self {
        let profiler = FrameProfiler::new(settings.benchmark_frames.max(16));
        Self {
            settings,
            diagnostics,
            world: None,
            graphics: None,
            surface: None,
            gal: None,
            profiler,
            frames: 0,
            samples: 0,
            exit_after_render: false,
        }
    }

    fn initialize(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        if self.world.is_none() {
            self.world = Some(Playground::new(self.settings.seed));
        }
        if self.graphics.is_some() {
            return Ok(());
        }
        let window = context
            .window
            .ok_or_else(|| PlaygroundError::invalid("desktop runner did not provide a window"))?;
        let size = window.native().inner_size();
        let (gal, mut surface) =
            FrameSurface::for_window(window.shared_native(), size.width, size.height)
                .map_err(|e| PlaygroundError::invalid(format!("create graphics: {e}")))?;
        surface.config.present_mode = if self.settings.benchmark {
            PresentMode::AutoNoVsync
        } else {
            PresentMode::AutoVsync
        };
        surface
            .reconfigure(&gal)
            .map_err(|e| PlaygroundError::invalid(e.to_string()))?;

        let graphics = render::Graphics::new(&gal)
            .map_err(|error| PlaygroundError::invalid(format!("create renderers: {error}")))?;
        self.surface = Some(surface);
        self.graphics = Some(graphics);
        gal.set_gpu_timestamps(self.settings.benchmark);
        self.gal = Some(gal);
        self.diagnostics.event_with(
            LogLevel::Info,
            "physics-playground.app",
            "initialized",
            [
                Field::new("width", u64::from(size.width)),
                Field::new("height", u64::from(size.height)),
                Field::new("seed", self.settings.seed),
            ],
        );
        Ok(())
    }

    fn input(&self, context: &AppContext<'_>) -> PlaygroundInput {
        let forward =
            f32::from(context.input.held(Key::KeyW)) - f32::from(context.input.held(Key::KeyS));
        let strafe =
            f32::from(context.input.held(Key::KeyD)) - f32::from(context.input.held(Key::KeyA));
        PlaygroundInput {
            forward,
            strafe,
            look: [
                context.input.relative_motion().0 as f32,
                context.input.relative_motion().1 as f32,
            ],
            jump: context.input.held(Key::Space),
            kick: context.input.mouse_pressed(MouseButton::Left),
            spawn: context.input.pressed(Key::KeyF),
            reset: context.input.pressed(Key::KeyR),
            toggle_debug: context.input.pressed(Key::KeyN),
        }
    }

    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        if self.exit_after_render {
            return Ok(Flow::Exit);
        }
        if context.input.pressed(Key::Escape) {
            return Ok(Flow::Exit);
        }
        let input = self.input(context);
        let Some(world) = &mut self.world else {
            return Ok(Flow::Continue);
        };
        world.update(input);
        Ok(Flow::Continue)
    }

    fn render(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        if self.exit_after_render {
            return Ok(());
        }
        let Some(world) = &self.world else {
            return Ok(());
        };
        let (Some(surface), Some(graphics), Some(gal)) = (
            self.surface.as_mut(),
            self.graphics.as_mut(),
            self.gal.as_mut(),
        ) else {
            return Ok(());
        };
        let report = surface
            .render(gal, CompletionPolicy::Poll, |gal, target| {
                graphics.commands(gal, target, world)
            })
            .map_err(|error| PlaygroundError::invalid(format!("render: {error}")))?;
        if let Some(report) = report {
            self.frames += 1;
            if self.settings.benchmark && self.frames > 8 {
                let mut sample = FrameSample::from_frame_report(self.frames, &report);
                sample.with_renderer_stats(graphics.last_stats());
                self.profiler.record(sample);
                self.samples += 1;
                if self.samples >= self.settings.benchmark_frames {
                    self.exit_after_render = true;
                }
            }
        }
        Ok(())
    }

    fn write_benchmark(&self) {
        if self.settings.benchmark
            && let Some(path) = &self.settings.benchmark_file
            && let Err(error) = self.profiler.write_json(path)
        {
            self.diagnostics.event_with(
                LogLevel::Error,
                "physics-playground.benchmark",
                "write_failed",
                [Field::new("error", error.to_string())],
            );
        }
    }

    fn release_graphics(&mut self) -> Result<()> {
        if let Some(gpu) = self.gal.take() {
            gpu.wait()
                .map_err(|e| PlaygroundError::invalid(e.to_string()))?;
            drop(self.graphics.take());
            drop(self.surface.take());
            gpu.wait()
                .map_err(|e| PlaygroundError::invalid(e.to_string()))?;
            if let Some(report) = gpu.instance.generate_report() {
                let buffers = report.hub.buffers.num_kept_from_user;
                let textures = report.hub.textures.num_kept_from_user;
                let views = report.hub.texture_views.num_kept_from_user;
                self.diagnostics.event_with(
                    LogLevel::Info,
                    "physics-playground.resources",
                    "teardown",
                    [
                        Field::new("live_buffers", buffers as u64),
                        Field::new("live_textures", textures as u64),
                        Field::new("live_views", views as u64),
                    ],
                );
                if buffers + textures + views != 0 {
                    return Err(PlaygroundError::invalid(
                        "GPU resources remain owned at teardown",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Application for PlaygroundApp {
    type Error = PlaygroundError;
    fn setup(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        self.initialize(context)
    }
    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        self.tick(context)
    }
    fn render(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        self.render(context)?;
        Ok(Flow::Continue)
    }
    fn suspended(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        self.release_graphics()
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        self.release_graphics()?;
        self.write_benchmark();
        if let Some(world) = &self.world {
            let stats = world.stats();
            self.diagnostics.event_with(
                LogLevel::Info,
                "physics-playground.app",
                "finished",
                [
                    Field::new("props", stats.props as u64),
                    Field::new("events", stats.collision_events),
                ],
            );
        }
        Ok(())
    }
}

impl DesktopApplication for PlaygroundApp {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        self.initialize(context)
    }

    fn configure_window(&mut self, window: &mut quasar_window::GameWindow) -> Result<()> {
        if let Err(error) = window.capture_cursor(true) {
            self.diagnostics.event_with(
                LogLevel::Warn,
                "physics-playground.input",
                "mouse_capture_unavailable",
                [Field::new("error", error.to_string())],
            );
        }
        Ok(())
    }

    fn window_event(&mut self, context: &mut AppContext<'_>, event: &WindowEvent) -> Result<()> {
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state == ElementState::Pressed
            && !event.repeat
            && matches!(event.physical_key, PhysicalKey::Code(KeyCode::F11))
            && let Some(window) = context.window
        {
            window.toggle_fullscreen();
        }
        if let WindowEvent::Focused(true) = event
            && let Some(window) = context.window
        {
            let _ = window.capture_cursor(true);
        }
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            let size = context
                .window
                .map(|window| window.native().inner_size())
                .unwrap_or(quasar_window::winit::dpi::PhysicalSize::new(1, 1));
            if let Some(surface) = &mut self.surface
                && let Some(graphics) = &mut self.graphics
                && let Some(gal) = &mut self.gal
            {
                let _ = surface.finish(gal);
                let _ = graphics.release_targets(gal);
                surface.resize(size.width, size.height);
            }
        }
        Ok(())
    }
}
