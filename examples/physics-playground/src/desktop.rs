use physics_playground::settings::Settings;
use physics_playground::world::{Playground, PlaygroundInput};
use physics_playground::{PlaygroundError, Result, render};
use quasar_app::{
    AppConfig, AppContext, Application, DesktopApplication, Flow, RenderPolicy, TimingConfig,
    run as run_app,
};
use quasar_core::FixedStepConfig;
use quasar_diagnostics::{Field, Level as LogLevel};
use quasar_frame::{CompletionPolicy, FrameSurface};
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
use vulkanic_gal::{
    FrameSurfaceDesc, PresentMode, TextureFormat, VulkanicGal,
    resources::{Extent3d, GpuProfileClassifier, GpuProfileTag, GpuProfiledObject},
    set_gpu_timestamps_requested,
};

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
    gal: Option<VulkanicGal>,
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
        set_gpu_timestamps_requested(self.settings.benchmark);
        let mut gal = VulkanicGal::create_windowed_vulkan(
            "Physics Playground",
            window.shared_native(),
            FrameSurfaceDesc {
                label: "physics-playground-surface".into(),
                extent: Extent3d {
                    width: size.width.max(1),
                    height: size.height.max(1),
                    depth: 1,
                },
                color_format: TextureFormat::Bgra8Unorm,
                present_mode: if self.settings.benchmark {
                    PresentMode::AutoNoVsync
                } else {
                    PresentMode::AutoVsync
                },
                max_frames_in_flight: 3,
            },
        )
        .map_err(|error| PlaygroundError::invalid(format!("create GAL: {error}")))?;
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
                        ..GpuProfileTag::default()
                    }
                } else {
                    GpuProfileTag::default()
                }
            },
            statistics_scope_name: |index| if index == 0 { "world_3d" } else { "hud_2d" },
        });
        let graphics = render::Graphics::new(&mut gal)
            .map_err(|error| PlaygroundError::invalid(format!("create renderers: {error}")))?;
        self.surface = Some(FrameSurface::new(size.width.max(1), size.height.max(1)));
        self.graphics = Some(graphics);
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
        if let Some(mut gal) = self.gal.take() {
            if let Some(mut surface) = self.surface.take() {
                let _ = surface.finish(&mut gal);
                let _ = surface.release_targets(&mut gal);
            }
            if let Some(mut graphics) = self.graphics.take() {
                let _ = graphics.release_targets(&mut gal);
                graphics.release(&mut gal).map_err(|error| {
                    PlaygroundError::invalid(format!("release graphics: {error}"))
                })?;
            }
            let _ = gal.shutdown_frame_surface();
            let _ = gal.retire_through(gal.latest_submission_id());
            let metrics = gal.metrics();
            self.diagnostics.event_with(
                if metrics.resource_creates == metrics.resource_destroys {
                    LogLevel::Info
                } else {
                    LogLevel::Warn
                },
                "physics-playground.resources",
                "teardown",
                [
                    Field::new("created", metrics.resource_creates),
                    Field::new("destroyed", metrics.resource_destroys),
                ],
            );
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
        self.release_graphics()
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
