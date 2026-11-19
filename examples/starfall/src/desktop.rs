//! Native window, GAL and audio lifecycle.

use quasar_app::{
    AppConfig, AppContext, Application, DesktopApplication, Flow, RenderPolicy, TimingConfig,
    run as run_app,
};
use quasar_audio::{AudioEngine, EngineConfig, PlaybackConfig};
use quasar_core::FixedStepConfig;
use quasar_diagnostics::{Diagnostics, Field, Level as LogLevel};
use quasar_frame::{CompletionPolicy, FrameSurface, Gpu};
use quasar_input::{Binding, Bindings, GamepadButton, Key};
use quasar_profile::{FrameProfiler, FrameSample};
use quasar_text::{FontData, FontStack};
use quasar_ui::{
    Edges, Layer, LayoutItem, PanelLayout, Rect, ShapedTextMeasure, StackDirection, TextRenderer,
    TextRendererOptions, UiBindings, UiTextStyles, Widget, from_frame_input,
};
use quasar_window::{DesktopConfig, WindowConfig, winit::event::WindowEvent};
use starfall::{
    content::SoundBank,
    load_content_blocking,
    menu::{Menu, MenuAction, Options},
    render::Graphics,
    session::{GameSession, StepResult},
    settings::GameSettings,
    world::{GameError, GameEvent, GameInput},
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use vulkanic_gal::PresentMode;

/// Game actions plus UI navigation actions installed on one input state.
fn desktop_bindings(game: &Bindings) -> Result<Bindings, GameError> {
    let mut bindings = game.clone();
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
    Ok(bindings)
}

fn build_text(settings: &GameSettings) -> Result<(ShapedTextMeasure, TextRenderer), GameError> {
    let fonts = settings.assets_root.join("fonts");
    let mut stack = FontStack::new();
    for (name, license) in [
        ("DejaVuSans.ttf", "Bitstream Vera / public-domain changes"),
        (
            "DejaVuSans-Bold.ttf",
            "Bitstream Vera / public-domain changes",
        ),
        (
            "NotoSansDevanagari-Regular.ttf",
            "SIL Open Font License 1.1",
        ),
    ] {
        let path = fonts.join(name);
        let bytes = std::fs::read(&path)
            .map_err(|error| GameError::invalid(format!("{}: {error}", path.display())))?;
        stack.add_font(FontData::new(name, bytes).with_license(license));
    }
    stack.with_default_family("DejaVu Sans");
    let engine = stack
        .build("en-US")
        .map_err(|error| GameError::invalid(format!("font system: {error}")))?;
    let engine = Rc::new(RefCell::new(engine));
    let styles = UiTextStyles::new("DejaVu Sans");
    let renderer = TextRenderer::new(engine, styles, TextRendererOptions::default())
        .map_err(|error| GameError::invalid(format!("text renderer: {error}")))?;
    let measure = renderer.measure();
    Ok((measure, renderer))
}

pub fn run(settings: GameSettings, diagnostics: Diagnostics) -> Result<(), GameError> {
    let mut settings = settings;
    let saved = Options::load_default();
    if let Some(saved) = &saved {
        settings.fullscreen = saved.fullscreen;
        settings.audio_volume = saved.volume;
        settings.mouse_sensitivity = saved.sensitivity;
    }
    let config = AppConfig {
        window: WindowConfig {
            title: settings.window_title.clone(),
            application_id: "starfall".into(),
            size: (settings.window_width, settings.window_height),
            minimum_size: (640, 360),
            fullscreen: settings.fullscreen,
            resizable: true,
        },
        desktop: DesktopConfig::for_application("starfall"),
        timing: TimingConfig::Fixed(FixedStepConfig::for_rate(60.0, 8)),
        render: RenderPolicy::EveryFrame,
        max_frame_wait: Duration::from_millis(16),
        frame_interval: None,
    };
    let initial = saved.unwrap_or(Options {
        fullscreen: settings.fullscreen,
        volume: settings.audio_volume,
        sensitivity: settings.mouse_sensitivity,
        player_name: "Pilot".into(),
        language: "en".into(),
    });
    let (text_measure, text_renderer) = build_text(&settings)?;
    run_app(
        StarfallApp::new(settings, diagnostics, initial, text_measure, text_renderer),
        config,
    )
    .map_err(|error| GameError::invalid(error.to_string()))
}

struct StarfallApp {
    settings: GameSettings,
    diagnostics: Diagnostics,
    session: Option<GameSession>,
    graphics: Option<Graphics>,
    surface: Option<FrameSurface>,
    gal: Option<Gpu>,
    audio: Option<AudioEngine>,
    sounds: Option<SoundBank>,
    texture: Option<starfall::content::TextureData>,
    profiler: FrameProfiler,
    frames: u64,
    samples: u64,
    exit_after_render: bool,
    recorded_written: bool,
    menu: Menu,
    interactive: bool,
    playing: bool,
    cursor_captured: bool,
    text_measure: ShapedTextMeasure,
    text_renderer: Option<TextRenderer>,
    player_name: String,
    /// Game actions as rebound on the controls screen.
    game_bindings: Bindings,
    pending_bindings: Option<Bindings>,
    /// Hit-stop, camera shake and render interpolation.
    feel: starfall::feel::Feel,
    interpolation: f32,
    /// Developer console (backquote), with its own per-frame UI.
    console: Option<quasar_ui::console::Console>,
    console_ui: Option<quasar_ui::Ui>,
    console_commands: quasar_ui::console::CommandSet<starfall::console::ConsoleState>,
}
impl StarfallApp {
    fn new(
        settings: GameSettings,
        diagnostics: Diagnostics,
        initial_options: Options,
        text_measure: ShapedTextMeasure,
        text_renderer: TextRenderer,
    ) -> Self {
        let profiler = FrameProfiler::new(settings.profile_capacity);
        let interactive = !settings.headless
            && !settings.benchmark
            && settings.replay.is_none()
            && !settings.autoplay;
        let player_name = initial_options.player_name.clone();
        let mut menu = Menu::new(initial_options);
        let defaults = settings.controls.bindings().unwrap_or_default();
        let game_bindings =
            match quasar_ui::load_bindings(starfall::menu::controls_path(), &defaults) {
                Ok(bindings) => bindings,
                Err(error) => {
                    diagnostics.event_with(
                        LogLevel::Warn,
                        "starfall.settings",
                        "controls_load_failed",
                        [Field::new("error", error.to_string())],
                    );
                    defaults.clone()
                }
            };
        menu.set_bindings(game_bindings.clone(), defaults);
        if interactive {
            menu.open_main();
        }
        Self {
            settings,
            diagnostics,
            session: None,
            graphics: None,
            surface: None,
            gal: None,
            audio: None,
            sounds: None,
            texture: None,
            profiler,
            frames: 0,
            samples: 0,
            exit_after_render: false,
            recorded_written: false,
            menu,
            interactive,
            playing: !interactive,
            cursor_captured: false,
            text_measure,
            text_renderer: Some(text_renderer),
            player_name,
            game_bindings,
            pending_bindings: None,
            feel: starfall::feel::Feel::new(),
            interpolation: 0.0,
            console: None,
            console_ui: None,
            console_commands: starfall::console::commands(),
        }
    }
    fn initialize(&mut self, context: &mut AppContext<'_>) -> Result<(), GameError> {
        if self.session.is_none() {
            let content = load_content_blocking(&self.settings, &self.diagnostics)?;
            let world = starfall::world_from_content(&content, &self.settings)?;
            let session = if let Some(path) = &self.settings.replay {
                GameSession::from_file(world, path, &self.settings)?
            } else {
                GameSession::live(world, &self.settings)
            };
            self.session = Some(session);
            self.texture = Some(content.texture.clone());
            self.sounds = Some(content.sounds.clone());
        }
        let expected_bindings = desktop_bindings(&self.game_bindings)?;
        if let Some(window) = context.window
            && window.input.bindings() != &expected_bindings
        {
            return Err(GameError::invalid(
                "configure_window did not install the configured controls",
            ));
        }
        if self.graphics.is_some() {
            return Ok(());
        }
        let window = context
            .window
            .ok_or_else(|| GameError::invalid("desktop runner did not provide a window"))?;
        let size = window.native().inner_size();
        let (gal, mut surface) =
            FrameSurface::for_window(window.shared_native(), size.width, size.height)
                .map_err(|e| GameError::invalid(format!("create graphics: {e}")))?;
        surface.config.present_mode = if self.settings.benchmark {
            PresentMode::AutoNoVsync
        } else {
            PresentMode::AutoVsync
        };
        surface
            .reconfigure(&gal)
            .map_err(|e| GameError::invalid(e.to_string()))?;

        let texture = self
            .texture
            .as_ref()
            .ok_or_else(|| GameError::invalid("content texture missing"))?;
        let mut graphics = Graphics::new(&gal, texture)
            .map_err(|error| GameError::invalid(format!("create renderers: {error}")))?;
        if let Some(text) = self.text_renderer.take() {
            graphics.enable_text(text);
        }
        self.surface = Some(surface);
        self.graphics = Some(graphics);
        gal.set_gpu_timestamps(self.settings.benchmark);
        self.gal = Some(gal);
        if self.settings.audio_enabled && self.audio.is_none() {
            match AudioEngine::open(
                self.settings.audio_device.as_deref(),
                EngineConfig::default(),
            ) {
                Ok(mut engine) => {
                    engine.set_diagnostics(Some(self.diagnostics.clone()));
                    let _ = engine.set_master_volume(self.settings.audio_volume);
                    self.audio = Some(engine);
                    self.diagnostics.event_with(
                        LogLevel::Info,
                        "starfall.audio",
                        "ready",
                        [Field::new("volume", f64::from(self.settings.audio_volume))],
                    );
                }
                Err(error) => {
                    self.diagnostics.event_with(
                        LogLevel::Warn,
                        "starfall.audio",
                        "unavailable",
                        [Field::new("error", error.to_string())],
                    );
                }
            }
        }
        self.diagnostics.event_with(
            LogLevel::Info,
            "starfall.app",
            "initialized",
            [
                Field::new("width", u64::from(size.width)),
                Field::new("height", u64::from(size.height)),
                Field::new(
                    "replay",
                    self.session.as_ref().is_some_and(GameSession::is_replay),
                ),
            ],
        );
        Ok(())
    }
    fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow, GameError> {
        if self.exit_after_render {
            return Ok(Flow::Exit);
        }
        // `elapsed` is the wall time since the previous frame.
        let dt = context.elapsed.as_secs_f32();
        if !self.menu.is_open() {
            self.feel.frame(dt.min(0.1));
        }
        if context.focus_changed == Some(false)
            && self.interactive
            && self.playing
            && !self.menu.is_open()
        {
            self.playing = false;
            self.menu.open_pause();
            if let Some(window) = context.window {
                self.set_cursor_capture(window, false);
            }
            context.input.consume_all();
            return Ok(Flow::Continue);
        }
        if self.menu.is_open() {
            let viewport = Rect::new(0.0, 0.0, context.size.0 as f32, context.size.1 as f32);
            let scale = context.scale_factor.max(0.5) as f32;
            let action = if self.menu.is_capturing() {
                // A controls row waits for a key (or, for analogue rows, a
                // stick): offer raw presses and axes instead of navigation.
                let axes = quasar_input::GamepadAxis::ALL
                    .map(|axis| (axis, context.input.gamepad_axis(axis)));
                match self.menu.capture(&context.input.pressed_buttons())? {
                    MenuAction::None => self.menu.capture_axes(&axes)?,
                    action => action,
                }
            } else {
                let input = from_frame_input(context.input, &UiBindings::default());
                self.menu.update(
                    &input,
                    viewport,
                    scale,
                    &self.text_measure,
                    context.elapsed.as_secs_f32(),
                )?
            };
            context.input.consume_all();
            if let Some(window) = context.window
                && let Some(flow) = self.apply_menu_action(action, window)?
            {
                return Ok(flow);
            }
            return Ok(Flow::Continue);
        }
        if self.interactive && self.playing && context.input.pressed(Key::Backquote) {
            self.console = match self.console.take() {
                Some(_) => None,
                None => {
                    let mut console = quasar_ui::console::Console::new();
                    console.print(
                        "Developer console. Tab completes; /help lists commands.",
                        quasar_ui::console::LogKind::Info,
                    );
                    Some(console)
                }
            };
            context.input.consume_all();
            return Ok(Flow::Continue);
        }
        if let Some(console) = &mut self.console {
            let input = from_frame_input(context.input, &UiBindings::default());
            let state = starfall::console::ConsoleState {
                volume: self.settings.audio_volume,
                shake: 1.0,
                hit_stop: self.feel.hit_stop.enabled(),
            };
            let outcome =
                console.handle(&self.console_commands, &state, &input, &self.text_measure);
            context.input.consume_all();
            match outcome {
                quasar_ui::console::ConsoleOutcome::Close => self.console = None,
                quasar_ui::console::ConsoleOutcome::Command(matched) => {
                    use starfall::console::ConsoleCommand;
                    match starfall::console::interpret(&matched) {
                        ConsoleCommand::Help => {
                            for line in self.console_commands.help() {
                                console.print(line, quasar_ui::console::LogKind::Info);
                            }
                        }
                        ConsoleCommand::Restart => {
                            console.print("Restarting.", quasar_ui::console::LogKind::Info);
                            self.restart_session()?;
                        }
                        ConsoleCommand::Volume(volume) => {
                            self.settings.audio_volume = volume;
                            if let Some(audio) = &self.audio {
                                let _ = audio.set_master_volume(volume);
                            }
                        }
                        ConsoleCommand::Shake(scale) => self.feel.shake.set_scale(scale),
                        ConsoleCommand::HitStop(on) => self.feel.hit_stop.set_enabled(on),
                    }
                }
                quasar_ui::console::ConsoleOutcome::None => {}
            }
            // Rebuild the overlay for this frame (the console owns its state).
            if let Some(console) = &self.console {
                let mut ui = quasar_ui::Ui::new(quasar_ui::Theme::modern());
                let root = ui.add_root(
                    Widget::Panel {
                        layout: PanelLayout::Stack {
                            direction: StackDirection::Vertical,
                            gap: 0.0,
                        },
                        padding: Edges::all(12.0),
                        clip: false,
                        scrollable: false,
                    },
                    LayoutItem::fill(),
                    Layer::Overlay,
                    false,
                );
                ui.set_style(root, Some(quasar_ui::NodeStyle::transparent()))
                    .map_err(|e| GameError::invalid(e.to_string()))?;
                console
                    .build(&mut ui, root, &self.text_measure)
                    .map_err(|e| GameError::invalid(e.to_string()))?;
                let viewport = Rect::new(0.0, 0.0, context.size.0 as f32, context.size.1 as f32);
                let scale = context.scale_factor.max(0.5) as f32;
                ui.layout(viewport, scale, Edges::ZERO, &self.text_measure);
                self.console_ui = Some(ui);
            } else {
                self.console_ui = None;
            }
            return Ok(Flow::Continue);
        }
        self.console_ui = None;
        if self.interactive && context.input.pressed(Key::Escape) {
            self.playing = false;
            self.menu.open_pause();
            if let Some(window) = context.window {
                self.set_cursor_capture(window, false);
            }
            context.input.consume_all();
            return Ok(Flow::Continue);
        }
        Ok(Flow::Continue)
    }

    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, GameError> {
        if self.exit_after_render {
            return Ok(Flow::Exit);
        }
        if self.menu.is_open() || self.console.is_some() {
            return Ok(Flow::Deferred);
        }
        // Hit-stop: hold the simulation for a beat (interactive play only,
        // so headless runs and recordings are unchanged).
        if self.interactive && self.feel.frozen() {
            return Ok(Flow::Deferred);
        }
        let Some(session) = &mut self.session else {
            return Ok(Flow::Continue);
        };
        self.feel.record(session.world());
        if session.is_replay() {
            if session.step(GameInput::default())? == StepResult::Exhausted {
                self.diagnostics
                    .event(LogLevel::Info, "starfall.replay", "complete");
                return Ok(Flow::Exit);
            }
        } else if self.playing {
            if self.settings.controls.quit_requested(context.input) {
                return Ok(Flow::Exit);
            }
            let input = if self.settings.autoplay {
                starfall::world::autoplay_input(session.world())
            } else {
                self.settings.input(context.input)
            };
            session.step(input)?;
        }
        let events = session.world().events.clone();
        if self.interactive {
            self.feel.on_events(&events);
        }
        self.play_events(&events);
        Ok(Flow::Continue)
    }

    fn apply_menu_action(
        &mut self,
        action: MenuAction,
        window: &quasar_window::GameWindow,
    ) -> Result<Option<Flow>, GameError> {
        match action {
            MenuAction::None => {}
            MenuAction::Start | MenuAction::Resume => {
                self.playing = true;
                self.menu.close();
                self.set_cursor_capture(window, true);
            }
            MenuAction::Restart => {
                self.playing = true;
                self.menu.close();
                self.set_cursor_capture(window, true);
                self.restart_session()?;
            }
            MenuAction::QuitToMenu => {
                self.playing = false;
                self.restart_session()?;
                self.menu.open_main();
                self.set_cursor_capture(window, false);
            }
            MenuAction::Quit => return Ok(Some(Flow::Exit)),
            MenuAction::ApplyBindings(bindings) => {
                self.game_bindings = bindings;
                // Installed by the runner through `take_bindings`.
                self.pending_bindings = Some(desktop_bindings(&self.game_bindings)?);
                if let Err(error) =
                    quasar_ui::save_bindings(starfall::menu::controls_path(), &self.game_bindings)
                {
                    self.diagnostics.event_with(
                        LogLevel::Error,
                        "starfall.settings",
                        "controls_save_failed",
                        [Field::new("error", error.to_string())],
                    );
                }
            }
            MenuAction::Apply(options) => {
                self.settings.fullscreen = options.fullscreen;
                self.settings.audio_volume = options.volume;
                self.settings.mouse_sensitivity = options.sensitivity;
                self.player_name.clone_from(&options.player_name);
                window.set_fullscreen(options.fullscreen);
                if let Some(audio) = &self.audio
                    && let Err(error) = audio.set_master_volume(options.volume)
                {
                    self.diagnostics.event_with(
                        LogLevel::Warn,
                        "starfall.audio",
                        "volume_apply_failed",
                        [Field::new("error", error.to_string())],
                    );
                }
                if let Err(error) = options.save(&Options::path()) {
                    self.diagnostics.event_with(
                        LogLevel::Error,
                        "starfall.settings",
                        "save_failed",
                        [Field::new("error", error.to_string())],
                    );
                }
            }
        }
        Ok(None)
    }

    fn restart_session(&mut self) -> Result<(), GameError> {
        if let Some(session) = &mut self.session
            && !session.is_replay()
        {
            session.step(GameInput {
                restart: true,
                ..GameInput::default()
            })?;
        }
        Ok(())
    }

    fn set_cursor_capture(&mut self, window: &quasar_window::GameWindow, capture: bool) {
        if self.cursor_captured == capture {
            return;
        }
        match window.capture_cursor(capture) {
            Ok(()) => self.cursor_captured = capture,
            Err(error) => {
                self.diagnostics.event_with(
                    LogLevel::Warn,
                    "starfall.input",
                    "mouse_capture_unavailable",
                    [Field::new("error", error.to_string())],
                );
            }
        }
    }
    fn play_events(&mut self, events: &[GameEvent]) {
        for event in events {
            if matches!(event, GameEvent::Won | GameEvent::Lost) {
                self.diagnostics
                    .event(LogLevel::Info, "starfall.game", "run_finished");
            }
            let (Some(audio), Some(sounds)) = (&self.audio, &self.sounds) else {
                continue;
            };
            if let Some(clip) = sounds.for_event(*event) {
                let _ = audio.play(
                    clip.clone(),
                    PlaybackConfig {
                        volume: 0.65,
                        ..PlaybackConfig::default()
                    },
                );
            }
        }
    }
    fn render(&mut self, context: &mut AppContext<'_>) -> Result<(), GameError> {
        if self.exit_after_render {
            return Ok(());
        }
        self.interpolation = context.interpolation as f32;
        let Some(session) = &self.session else {
            return Ok(());
        };
        let (Some(surface), Some(graphics), Some(gal)) = (
            self.surface.as_mut(),
            self.graphics.as_mut(),
            self.gal.as_mut(),
        ) else {
            return Ok(());
        };
        let menu = self.menu.ui().or(self.console_ui.as_ref());
        // Draw between ticks, with the camera shaken by recent hits.
        let shown = self.feel.shown(session.world(), self.interpolation);
        graphics.camera_shake = self.feel.camera_shake();
        let report = surface
            .render(gal, CompletionPolicy::Poll, |gal, target| {
                graphics.commands(gal, target, &shown, menu)
            })
            .map_err(|error| GameError::invalid(format!("render: {error}")))?;
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
    fn release_graphics(&mut self) -> Result<(), GameError> {
        if let Some(gpu) = self.gal.take() {
            gpu.wait().map_err(|e| GameError::invalid(e.to_string()))?;
            drop(self.graphics.take());
            drop(self.surface.take());
            gpu.wait().map_err(|e| GameError::invalid(e.to_string()))?;
            if let Some(report) = gpu.instance.generate_report() {
                let buffers = report.hub.buffers.num_kept_from_user;
                let textures = report.hub.textures.num_kept_from_user;
                let views = report.hub.texture_views.num_kept_from_user;
                self.diagnostics.event_with(
                    LogLevel::Info,
                    "starfall.resources",
                    "teardown",
                    [
                        Field::new("live_buffers", buffers as u64),
                        Field::new("live_textures", textures as u64),
                        Field::new("live_views", views as u64),
                    ],
                );
                if buffers + textures + views != 0 {
                    return Err(GameError::invalid("GPU resources remain owned at teardown"));
                }
            }
        }
        self.audio.take();
        Ok(())
    }
    fn write_outputs(&mut self) {
        if let Some(session) = &self.session
            && let Some(path) = &self.settings.record
        {
            if let Err(error) = session.write_record(path) {
                self.diagnostics.event_with(
                    LogLevel::Error,
                    "starfall.record",
                    "write_failed",
                    [Field::new("error", error.to_string())],
                );
            } else if !self.recorded_written {
                self.recorded_written = true;
                self.diagnostics.event_with(
                    LogLevel::Info,
                    "starfall.record",
                    "written",
                    [Field::new("path", path.display().to_string())],
                );
            }
        }
        if self.settings.benchmark
            && let Some(path) = &self.settings.benchmark_file
            && let Err(error) = self.profiler.write_json(path)
        {
            self.diagnostics.event_with(
                LogLevel::Error,
                "starfall.benchmark",
                "write_failed",
                [Field::new("error", error.to_string())],
            );
        }
    }
}
impl Application for StarfallApp {
    type Error = GameError;
    fn setup(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.initialize(context)
    }
    fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.frame_update(context)
    }
    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.tick(context)
    }
    fn render(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.render(context)?;
        Ok(Flow::Continue)
    }
    fn suspended(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.release_graphics()
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.release_graphics()?;
        self.write_outputs();
        Ok(())
    }
}
impl DesktopApplication for StarfallApp {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        self.initialize(context)
    }

    fn take_bindings(&mut self) -> Option<Bindings> {
        self.pending_bindings.take()
    }

    fn configure_window(
        &mut self,
        window: &mut quasar_window::GameWindow,
    ) -> Result<(), Self::Error> {
        let bindings = desktop_bindings(&self.game_bindings)?;
        window.native().set_ime_allowed(true);
        window
            .input
            .set_bindings(bindings)
            .map_err(|error| GameError::invalid(format!("install controls: {error}")))?;
        let capture = !self.menu.is_open();
        if let Err(error) = window.capture_cursor(capture) {
            self.diagnostics.event_with(
                LogLevel::Warn,
                "starfall.input",
                "mouse_capture_unavailable",
                [Field::new("error", error.to_string())],
            );
        } else {
            self.cursor_captured = capture;
        }
        Ok(())
    }
    fn window_event(
        &mut self,
        context: &mut AppContext<'_>,
        event: &WindowEvent,
    ) -> Result<(), Self::Error> {
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state == quasar_window::winit::event::ElementState::Pressed
            && !event.repeat
            && matches!(
                event.physical_key,
                quasar_window::winit::keyboard::PhysicalKey::Code(
                    quasar_window::winit::keyboard::KeyCode::F11
                )
            )
        {
            self.settings.fullscreen = !self.settings.fullscreen;
            if let Some(window) = context.window {
                window.set_fullscreen(self.settings.fullscreen);
            }
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
