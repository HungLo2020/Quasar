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
use voxel_blocks::render::Graphics;
use voxel_blocks::settings::Settings;
use voxel_blocks::world::{BlockWorld, Input, autoplay_input};
use voxel_blocks::{BlocksError, Result};
use vulkanic_gal::PresentMode;

const DIGITS: [Key; 9] = [
    Key::Digit1,
    Key::Digit2,
    Key::Digit3,
    Key::Digit4,
    Key::Digit5,
    Key::Digit6,
    Key::Digit7,
    Key::Digit8,
    Key::Digit9,
];

pub fn run(settings: Settings, diagnostics: quasar_diagnostics::Diagnostics) -> Result<()> {
    let config = AppConfig {
        window: WindowConfig {
            title: "Voxel Blocks - Quasar".into(),
            application_id: "voxel-blocks".into(),
            size: (settings.width, settings.height),
            minimum_size: (640, 360),
            fullscreen: false,
            resizable: true,
        },
        desktop: DesktopConfig::for_application("voxel-blocks"),
        timing: TimingConfig::Fixed(FixedStepConfig::for_rate(60.0, 8)),
        render: RenderPolicy::EveryFrame,
        max_frame_wait: Duration::from_millis(16),
        frame_interval: None,
    };
    run_app(BlocksApp::new(settings, diagnostics), config)
        .map_err(|e| BlocksError::invalid(e.to_string()))
}

struct BlocksApp {
    settings: Settings,
    diagnostics: quasar_diagnostics::Diagnostics,
    world: Option<BlockWorld>,
    graphics: Option<Graphics>,
    surface: Option<FrameSurface>,
    gal: Option<Gpu>,
    profiler: FrameProfiler,
    frames: u64,
    samples: usize,
    exit_after_render: bool,
}

impl BlocksApp {
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
            let mut world = BlockWorld::with_content(
                if self.settings.sculpted {
                    voxel_blocks::content::Content::sculpted()
                } else {
                    voxel_blocks::content::Content::new()
                },
                self.settings.seed,
                self.settings.radius,
                self.settings.workers,
            );
            if self.settings.benchmark {
                world.settle();
            }
            self.world = Some(world);
        }
        if self.graphics.is_some() {
            return Ok(());
        }
        let window = context
            .window
            .ok_or_else(|| BlocksError::invalid("desktop runner did not provide a window"))?;
        let size = window.native().inner_size();
        let (gal, mut surface) =
            FrameSurface::for_window(window.shared_native(), size.width, size.height)
                .map_err(|e| BlocksError::invalid(format!("create graphics: {e}")))?;
        surface.config.present_mode = if self.settings.benchmark {
            PresentMode::AutoNoVsync
        } else {
            PresentMode::AutoVsync
        };
        surface
            .reconfigure(&gal)
            .map_err(|e| BlocksError::invalid(e.to_string()))?;
        let mut graphics = Graphics::new(&gal)
            .map_err(|e| BlocksError::invalid(format!("create renderers: {e}")))?;
        graphics.occlusion = self.settings.occlusion;
        graphics.mist = self.settings.sculpted;
        self.surface = Some(surface);
        self.graphics = Some(graphics);
        gal.set_gpu_timestamps(self.settings.benchmark);
        self.gal = Some(gal);
        self.diagnostics.event_with(
            LogLevel::Info,
            "voxel-blocks.app",
            "initialized",
            [
                Field::new("width", u64::from(size.width)),
                Field::new("height", u64::from(size.height)),
                Field::new("seed", self.settings.seed),
            ],
        );
        Ok(())
    }

    fn input(&self, context: &AppContext<'_>) -> Input {
        let i = &context.input;
        let axis = |pos: Key, neg: Key| f32::from(i.held(pos)) - f32::from(i.held(neg));
        let (dx, dy) = i.relative_motion();
        let scroll = i.scroll_lines().1;
        Input {
            forward: axis(Key::KeyW, Key::KeyS),
            strafe: axis(Key::KeyD, Key::KeyA),
            look: [dx as f32 * 0.0025, -dy as f32 * 0.0025],
            jump: i.held(Key::Space),
            descend: i.held(Key::ShiftLeft),
            remove: i.mouse_pressed(MouseButton::Left),
            place: i.mouse_pressed(MouseButton::Right),
            slot: DIGITS.iter().position(|k| i.pressed(*k)),
            cycle: if scroll > 0.0 {
                -1
            } else if scroll < 0.0 {
                1
            } else {
                0
            },
            toggle_fly: i.pressed(Key::KeyF),
        }
    }

    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        if self.exit_after_render || context.input.pressed(Key::Escape) {
            return Ok(Flow::Exit);
        }
        let input = self.input(context);
        if let Some(world) = &mut self.world {
            let input = if self.settings.autoplay {
                autoplay_input(world)
            } else {
                input
            };
            world.update(input);
        }
        Ok(Flow::Continue)
    }

    fn render(&mut self) -> Result<()> {
        if self.exit_after_render {
            return Ok(());
        }
        let (Some(world), Some(surface), Some(graphics), Some(gal)) = (
            self.world.as_mut(),
            self.surface.as_mut(),
            self.graphics.as_mut(),
            self.gal.as_mut(),
        ) else {
            return Ok(());
        };
        let report = surface
            .render(gal, CompletionPolicy::Poll, |gal, target| {
                graphics.commands(gal, target, world, true)
            })
            .map_err(|e| BlocksError::invalid(format!("render: {e}")))?;
        graphics.submitted(gal);
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

    fn release_graphics(&mut self) -> Result<()> {
        if let Some(gpu) = self.gal.take() {
            gpu.wait()
                .map_err(|e| BlocksError::invalid(e.to_string()))?;
            if let Some(mut graphics) = self.graphics.take() {
                graphics
                    .release(&gpu)
                    .map_err(|e| BlocksError::invalid(e.to_string()))?;
            }
            drop(self.surface.take());
            gpu.wait()
                .map_err(|e| BlocksError::invalid(e.to_string()))?;
        }
        // Geometry must be re-uploaded on resume.
        if let Some(world) = &mut self.world {
            world.request_full_remesh();
        }
        Ok(())
    }
}

impl Application for BlocksApp {
    type Error = BlocksError;
    fn setup(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        self.initialize(context)
    }
    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        self.tick(context)
    }
    fn render(&mut self, _: &mut AppContext<'_>) -> Result<Flow> {
        self.render()?;
        Ok(Flow::Continue)
    }
    fn suspended(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        self.release_graphics()
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        self.release_graphics()?;
        if self.settings.benchmark
            && let Some(path) = &self.settings.benchmark_file
            && let Err(error) = self.profiler.write_json(path)
        {
            self.diagnostics.event_with(
                LogLevel::Error,
                "voxel-blocks.benchmark",
                "write_failed",
                [Field::new("error", error.to_string())],
            );
        }
        if let Some(world) = &self.world {
            let s = world.stats;
            self.diagnostics.event_with(
                LogLevel::Info,
                "voxel-blocks.app",
                "finished",
                [
                    Field::new("placed", s.placed),
                    Field::new("removed", s.removed),
                    Field::new("chunks", s.chunks as u64),
                ],
            );
        }
        Ok(())
    }
}

impl DesktopApplication for BlocksApp {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        self.initialize(context)
    }

    fn configure_window(&mut self, window: &mut quasar_window::GameWindow) -> Result<()> {
        if let Err(error) = window.capture_cursor(true) {
            self.diagnostics.event_with(
                LogLevel::Warn,
                "voxel-blocks.input",
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
            if let (Some(surface), Some(graphics), Some(gal)) =
                (&mut self.surface, &mut self.graphics, &mut self.gal)
            {
                let _ = surface.finish(gal);
                let _ = graphics.release_targets(gal);
                surface.resize(size.width, size.height);
            }
        }
        Ok(())
    }
}
