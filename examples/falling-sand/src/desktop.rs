use falling_sand::menu::{Menu, MenuAction};
use falling_sand::settings::{Settings, UiOptions};
use falling_sand::{Result, SandError, render, sand};
use quasar_app::{
    AppConfig, AppContext, Application, DesktopApplication, Flow, RenderPolicy, TimingConfig,
    run as run_app,
};
use quasar_core::FixedStepConfig;
use quasar_diagnostics::{Field, Level as LogLevel};
use quasar_frame::{CompletionPolicy, FrameSurface, Gpu};
use quasar_input::{Binding, Bindings, GamepadButton, Key, MouseButton};
use quasar_profile::{FrameProfiler, FrameSample};
use quasar_ui::{Rect, UiBindings, from_frame_input};
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
    let ui_options = UiOptions::load_default();
    let mut settings = settings;
    settings.brush_radius = ui_options.brush_radius;
    let config = AppConfig {
        window: WindowConfig {
            title: "Falling Sand - Quasar".into(),
            application_id: "falling-sand".into(),
            size: (settings.width, settings.height),
            minimum_size: (640, 360),
            fullscreen: ui_options.fullscreen,
            resizable: true,
        },
        desktop: DesktopConfig::for_application("falling-sand"),
        timing: TimingConfig::Fixed(FixedStepConfig::for_rate(60.0, 8)),
        render: RenderPolicy::EveryFrame,
        max_frame_wait: Duration::from_millis(16),
        frame_interval: None,
    };
    run_app(
        FallingSandApp::new(settings, diagnostics, ui_options),
        config,
    )
    .map_err(|error| SandError::invalid(error.to_string()))
}

struct FallingSandApp {
    settings: Settings,
    diagnostics: quasar_diagnostics::Diagnostics,
    world: Option<sand::SandWorld>,
    graphics: Option<render::Graphics>,
    surface: Option<FrameSurface>,
    gal: Option<Gpu>,
    profiler: FrameProfiler,
    frames: u64,
    samples: usize,
    exit_after_render: bool,
    tool: sand::Tool,
    cursor: Option<[f32; 2]>,
    menu: Menu,
    ui_options: UiOptions,
    playing: bool,
}

impl FallingSandApp {
    fn new(
        settings: Settings,
        diagnostics: quasar_diagnostics::Diagnostics,
        ui_options: UiOptions,
    ) -> Self {
        let profiler = FrameProfiler::new(settings.benchmark_frames.max(16));
        let interactive = !settings.benchmark;
        let mut menu = Menu::new(ui_options);
        if interactive {
            menu.open_main();
        }
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
            tool: sand::Tool::Sand,
            cursor: None,
            menu,
            ui_options,
            playing: !interactive,
        }
    }

    fn initialize(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        if self.world.is_none() {
            self.world = Some(sand::SandWorld::new(self.settings.seed));
        }
        if self.graphics.is_some() {
            return Ok(());
        }
        let window = context
            .window
            .ok_or_else(|| SandError::invalid("desktop runner did not provide a window"))?;
        let size = window.native().inner_size();
        let (gal, mut surface) =
            FrameSurface::for_window(window.shared_native(), size.width, size.height)
                .map_err(|e| SandError::invalid(format!("create graphics: {e}")))?;
        surface.config.present_mode = if self.settings.benchmark {
            PresentMode::AutoNoVsync
        } else {
            PresentMode::AutoVsync
        };
        surface
            .reconfigure(&gal)
            .map_err(|e| SandError::invalid(e.to_string()))?;

        let graphics = render::Graphics::new(&gal)
            .map_err(|error| SandError::invalid(format!("create renderer: {error}")))?;
        self.surface = Some(surface);
        self.graphics = Some(graphics);
        gal.set_gpu_timestamps(self.settings.benchmark);
        self.gal = Some(gal);
        self.diagnostics.event_with(
            LogLevel::Info,
            "falling-sand.app",
            "initialized",
            [
                Field::new("width", u64::from(size.width)),
                Field::new("height", u64::from(size.height)),
                Field::new("seed", self.settings.seed),
            ],
        );
        Ok(())
    }

    fn input(&self, context: &AppContext<'_>) -> sand::SandInput {
        let size = context.size;
        let pointer = context.input.cursor_physical().map(|(x, y)| {
            [
                (x / f64::from(size.0.max(1)) * f64::from(sand::GRID_WIDTH as u32)) as f32,
                (y / f64::from(size.1.max(1)) * f64::from(sand::GRID_HEIGHT as u32)) as f32,
            ]
        });
        sand::SandInput {
            pointer: pointer.unwrap_or([0.0, 0.0]),
            pointer_valid: pointer.is_some(),
            primary_held: context.input.mouse_held(MouseButton::Left),
            primary_pressed: context.input.mouse_pressed(MouseButton::Left),
            secondary_held: context.input.mouse_held(MouseButton::Right),
            reset: context.input.pressed(Key::KeyR),
            tool: self.tool,
            brush_radius: self.settings.brush_radius,
        }
    }

    fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        if self.exit_after_render {
            return Ok(Flow::Exit);
        }
        let viewport = Rect::new(0.0, 0.0, context.size.0 as f32, context.size.1 as f32);
        let scale = (context.scale_factor as f32 * self.ui_options.ui_scale).max(0.5);
        if self.menu.is_open() {
            let input = from_frame_input(context.input, &UiBindings::default());
            let action =
                self.menu
                    .update(&input, viewport, scale, context.elapsed.as_secs_f32())?;
            context.input.consume_all();
            match action {
                MenuAction::Start | MenuAction::Resume => {
                    self.playing = true;
                    self.menu.close();
                }
                MenuAction::Restart => {
                    self.world = Some(sand::SandWorld::new(self.settings.seed));
                    self.playing = true;
                    self.menu.close();
                }
                MenuAction::QuitToMenu => {
                    self.world = Some(sand::SandWorld::new(self.settings.seed));
                    self.playing = false;
                    self.menu.open_main();
                }
                MenuAction::Quit => return Ok(Flow::Exit),
                MenuAction::Apply(options) => {
                    self.ui_options = options;
                    self.settings.brush_radius = options.brush_radius;
                    self.menu.set_options(options);
                    if let Some(window) = context.window {
                        window.set_fullscreen(options.fullscreen);
                    }
                    let path = UiOptions::path();
                    if let Err(error) = options.save(&path) {
                        self.diagnostics.event_with(
                            LogLevel::Error,
                            "falling-sand.settings",
                            "save_failed",
                            [Field::new("error", error.to_string())],
                        );
                    }
                }
                MenuAction::None => {}
            }
            return Ok(Flow::Continue);
        }
        if context.input.pressed(Key::Escape) {
            self.playing = false;
            self.menu.open_pause();
            context.input.consume_all();
            return Ok(Flow::Continue);
        }
        Ok(Flow::Continue)
    }

    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        if self.exit_after_render {
            return Ok(Flow::Continue);
        }
        if self.menu.is_open() {
            return Ok(Flow::Deferred);
        }
        if context.input.pressed(Key::Digit1) {
            self.tool = sand::Tool::Sand;
        } else if context.input.pressed(Key::Digit2) {
            self.tool = sand::Tool::Rock;
        } else if context.input.pressed(Key::Digit3) {
            self.tool = sand::Tool::Erase;
        }
        let input = self.input(context);
        self.cursor = input.pointer_valid.then_some(input.pointer);
        if !self.playing {
            return Ok(Flow::Deferred);
        }
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
        let cursor = self.cursor;
        let tool = self.tool;
        let menu = self.menu.ui();
        let report = surface
            .render(gal, CompletionPolicy::Poll, |gal, target| {
                graphics.commands(gal, target, world, cursor, tool, menu)
            })
            .map_err(|error| SandError::invalid(format!("render: {error}")))?;
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
                "falling-sand.benchmark",
                "write_failed",
                [Field::new("error", error.to_string())],
            );
        }
    }

    fn release_graphics(&mut self) -> Result<()> {
        if let Some(gpu) = self.gal.take() {
            gpu.wait().map_err(|e| SandError::invalid(e.to_string()))?;
            drop(self.graphics.take());
            drop(self.surface.take());
            gpu.wait().map_err(|e| SandError::invalid(e.to_string()))?;
            if let Some(report) = gpu.instance.generate_report() {
                let buffers = report.hub.buffers.num_kept_from_user;
                let textures = report.hub.textures.num_kept_from_user;
                let views = report.hub.texture_views.num_kept_from_user;
                self.diagnostics.event_with(
                    LogLevel::Info,
                    "falling-sand.resources",
                    "teardown",
                    [
                        Field::new("live_buffers", buffers as u64),
                        Field::new("live_textures", textures as u64),
                        Field::new("live_views", views as u64),
                    ],
                );
                if buffers + textures + views != 0 {
                    return Err(SandError::invalid("GPU resources remain owned at teardown"));
                }
            }
        }
        Ok(())
    }
}

impl Application for FallingSandApp {
    type Error = SandError;
    fn setup(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        self.initialize(context)
    }
    fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        self.frame_update(context)
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
                "falling-sand.app",
                "finished",
                [
                    Field::new("sand", stats.sand as u64),
                    Field::new("obstacles", stats.obstacles as u64),
                    Field::new("steps", stats.physics_steps),
                ],
            );
        }
        Ok(())
    }
}

impl DesktopApplication for FallingSandApp {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        self.initialize(context)
    }

    fn configure_window(&mut self, window: &mut quasar_window::GameWindow) -> Result<()> {
        let mut bindings = Bindings::default();
        bindings
            .bind("ui_up", Binding::key(Key::ArrowUp))
            .bind("ui_down", Binding::key(Key::ArrowDown))
            .bind("ui_left", Binding::key(Key::ArrowLeft))
            .bind("ui_right", Binding::key(Key::ArrowRight))
            .bind("ui_next", Binding::key(Key::Tab))
            .bind("ui_previous", Binding::key(Key::ArrowUp))
            .bind("ui_activate", Binding::key(Key::Enter))
            .bind("ui_cancel", Binding::key(Key::Escape))
            .bind("ui_up", Binding::gamepad_button(GamepadButton::DPadUp))
            .bind("ui_down", Binding::gamepad_button(GamepadButton::DPadDown))
            .bind("ui_left", Binding::gamepad_button(GamepadButton::DPadLeft))
            .bind(
                "ui_right",
                Binding::gamepad_button(GamepadButton::DPadRight),
            )
            .bind("ui_activate", Binding::gamepad_button(GamepadButton::South))
            .bind("ui_cancel", Binding::gamepad_button(GamepadButton::East));
        window
            .input
            .set_bindings(bindings)
            .map_err(|error| SandError::invalid(format!("install UI bindings: {error}")))?;
        if let Err(error) = window.capture_cursor(false) {
            self.diagnostics.event_with(
                LogLevel::Warn,
                "falling-sand.input",
                "cursor_visibility_unavailable",
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
