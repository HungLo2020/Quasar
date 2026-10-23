use super::*;
use pong::Phase;
use quasar_core::FixedStep;
use quasar_render_2d::{DisplayOptions, Filter, PresentOutcome, Renderer2D, Scaling, Textures};
use quasar_storage::{GamePaths, read_json, write_json};
use quasar_window::{
    GameWindow, WindowConfig,
    winit::{
        application::ApplicationHandler,
        event::{DeviceEvent, DeviceId, WindowEvent},
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        window::WindowId,
    },
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize)]
struct DisplaySettings {
    version: u32,
    width: u32,
    height: u32,
    fullscreen: bool,
    linear: bool,
    integer: bool,
}
impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            version: 1,
            width: 960,
            height: 600,
            fullscreen: false,
            linear: false,
            integer: false,
        }
    }
}
impl DisplaySettings {
    fn valid(&self) -> bool {
        self.version == 1
            && (320..=16384).contains(&self.width)
            && (200..=16384).contains(&self.height)
    }
    fn options(&self) -> DisplayOptions {
        DisplayOptions {
            filter: if self.linear {
                Filter::Linear
            } else {
                Filter::Nearest
            },
            scaling: if self.integer {
                Scaling::Integer
            } else {
                Scaling::Fit
            },
            aspect: None,
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Records {
    version: u32,
    longest_rally: u32,
}

pub fn run(
    options: Options,
    session: Session,
    explicit_controls: Option<controls::Controls>,
) -> Result {
    let paths = GamePaths::discover(Some(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .as_path(),
    ))?;
    let controls_path = paths.shared_config.join("controls.json");
    let (controls, save_controls) = if let Some(controls) = explicit_controls {
        (controls, false)
    } else if options.no_save {
        (controls::Controls::default(), false)
    } else {
        match read_json::<controls::Controls>(&controls_path) {
            Ok(Some(controls)) => match controls.validate() {
                Ok(()) => (controls, false),
                Err(error) => {
                    eprintln!("Invalid controls ({error}); preserving file and using defaults");
                    (controls::Controls::default(), false)
                }
            },
            Ok(None) => (controls::Controls::default(), true),
            Err(error) => {
                eprintln!("Cannot load controls ({error}); preserving file and using defaults");
                (controls::Controls::default(), false)
            }
        }
    };
    let (mut display, save_display) = if options.no_save {
        (DisplaySettings::default(), false)
    } else {
        match read_json::<DisplaySettings>(&paths.config.join("display.json")) {
            Ok(Some(display)) if display.valid() => (display, true),
            Ok(None) => (DisplaySettings::default(), true),
            Ok(Some(_)) => {
                eprintln!("Unsupported display settings; preserving file and using defaults");
                (DisplaySettings::default(), false)
            }
            Err(error) => {
                eprintln!(
                    "Cannot load display settings ({error}); preserving file and using defaults"
                );
                (DisplaySettings::default(), false)
            }
        }
    };
    let (records, save_records) = if options.no_save {
        (
            Records {
                version: 1,
                longest_rally: 0,
            },
            false,
        )
    } else {
        match read_json::<Records>(&paths.save.join("records.json")) {
            Ok(Some(records)) if records.version == 1 => (records, true),
            Ok(None) => (
                Records {
                    version: 1,
                    longest_rally: 0,
                },
                true,
            ),
            _ => {
                eprintln!("Cannot load records; preserving file");
                (
                    Records {
                        version: 1,
                        longest_rally: 0,
                    },
                    false,
                )
            }
        }
    };
    display.width = options.width.unwrap_or(display.width);
    display.height = options.height.unwrap_or(display.height);
    display.fullscreen |= options.fullscreen;
    display.linear |= options.linear;
    display.integer |= options.integer;
    let mut app = App {
        presenter: None,
        textures: Textures::default(),
        screenshot: None,
        window: None,
        options,
        session,
        controls,
        save_controls,
        display,
        paths,
        save_display,
        save_records,
        records,
        clock: FixedStep::new(pong::TICK_HZ, 8)?,
        previous: Instant::now(),
        started: Instant::now(),
        frames: 0,
        exercise_stage: 0,
        error: None,
    };
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.error.take() {
        return Err(error.into());
    }
    app.session.outputs(
        &app.options,
        Some(&app.controls.hints()),
        app.screenshot.as_deref(),
    )?;
    if app.save_controls {
        write_json(
            &app.paths.shared_config.join("controls.json"),
            &app.controls,
        )?;
    }
    if app.save_display {
        write_json(&app.paths.config.join("display.json"), &app.display)?;
    }
    if app.save_records {
        app.records.longest_rally = app.records.longest_rally.max(app.session.best_rally);
        write_json(&app.paths.save.join("records.json"), &app.records)?;
    }
    Ok(())
}
struct App {
    // Graphics is explicitly destroyed before the window on suspension/exit.
    presenter: Option<Renderer2D>,
    textures: Textures,
    screenshot: Option<Vec<u8>>,
    window: Option<GameWindow>,
    options: Options,
    session: Session,
    controls: controls::Controls,
    save_controls: bool,
    display: DisplaySettings,
    paths: GamePaths,
    save_display: bool,
    save_records: bool,
    records: Records,
    clock: FixedStep,
    previous: Instant,
    started: Instant,
    frames: u64,
    exercise_stage: u8,
    error: Option<String>,
}
impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl std::fmt::Display) {
        self.error = Some(error.to_string());
        event_loop.exit();
    }
    fn automated(&self) -> bool {
        self.options.autoplay
            || self.session.replay.is_some()
            || self.options.smoke_frames.is_some()
    }
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result {
        let mut window = GameWindow::create(
            event_loop,
            &WindowConfig {
                title: "Quasar / Pong".into(),
                application_id: "quasar-pong".into(),
                size: (self.display.width, self.display.height),
                minimum_size: (320, 200),
                fullscreen: self.display.fullscreen,
                resizable: true,
            },
        )?;
        window.input.set_bindings(self.controls.bindings.clone())?;
        let size = window.native().inner_size();
        self.presenter = Some(Renderer2D::for_window(
            window.shared_native(),
            (size.width.max(1), size.height.max(1)),
            self.display.options(),
        )?);
        self.window = Some(window);
        self.previous = Instant::now();
        self.clock.reset();
        Ok(())
    }
    fn redraw(&mut self, event_loop: &ActiveEventLoop) -> Result {
        let window = self.window.as_ref().unwrap();
        if !window.drawable() {
            return Ok(());
        }
        let outcome = self.presenter.as_mut().unwrap().present(
            &render::scene_with_hints(&self.session.game, &self.controls.hints()),
            &self.textures,
        )?;
        if outcome == PresentOutcome::Presented {
            self.frames += 1;
            if self.options.exercise_window {
                self.exercise()?;
            }
            if self.options.smoke_frames.is_some_and(|n| self.frames >= n) {
                println!(
                    "presented_frames={} window_exercise_stages={}",
                    self.frames, self.exercise_stage
                );
                event_loop.exit();
            }
        }
        Ok(())
    }
    fn exercise(&mut self) -> Result {
        let window = self.window.as_ref().unwrap();
        match self.exercise_stage {
            0 if self.frames >= 2 => {
                let _ = window
                    .native()
                    .request_inner_size(quasar_window::winit::dpi::LogicalSize::new(800, 600));
                self.exercise_stage = 1;
            }
            1 if self.frames >= 5 => {
                window.set_fullscreen(true);
                self.exercise_stage = 2;
            }
            2 if self.frames >= 10 => {
                window.set_fullscreen(false);
                self.exercise_stage = 3;
            }
            3 if self.frames >= 13 => {
                let presenter = self.presenter.as_mut().unwrap();
                presenter.set_options(DisplayOptions {
                    filter: Filter::Linear,
                    scaling: Scaling::Integer,
                    aspect: None,
                })?;
                presenter.resize(0, 0);
                if presenter.present(
                    &render::scene_with_hints(&self.session.game, &self.controls.hints()),
                    &self.textures,
                )? != PresentOutcome::Deferred
                {
                    return Err("zero-size surface was not deferred".into());
                }
                let size = window.native().inner_size();
                presenter.resize(size.width, size.height);
                self.exercise_stage = 4;
            }
            _ => {}
        }
        Ok(())
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(error) = self.initialize(event_loop)
        {
            self.fail(event_loop, error);
        }
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.presenter.take();
        self.window.take();
        self.clock.reset();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let automated = self.automated();
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
            if !window.fullscreen() && size.width > 0 && size.height > 0 {
                let logical = size.to_logical::<u32>(window.native().scale_factor());
                self.display.width = logical.width.clamp(320, 16384);
                self.display.height = logical.height.clamp(200, 16384);
            }
        }
        if update.focus_changed.is_some() {
            self.clock.reset();
            self.previous = Instant::now();
            if update.focus_changed == Some(false)
                && !automated
                && self.session.game.phase == Phase::Playing
            {
                self.session.tick(
                    Input {
                        pause: true,
                        ..Default::default()
                    },
                    false,
                );
            }
        }
        let input = &mut window.input;
        if input.take_pressed(controls::QUIT) {
            event_loop.exit();
            return;
        }
        if input.take_pressed(controls::FULLSCREEN) {
            window.toggle_fullscreen();
            self.display.fullscreen = window.fullscreen();
        }
        let input = &mut window.input;
        let presenter = self.presenter.as_mut().unwrap();
        let mut options = presenter.options();
        if input.take_pressed(controls::FILTER) {
            self.display.linear = !self.display.linear;
            options.filter = if self.display.linear {
                Filter::Linear
            } else {
                Filter::Nearest
            };
        }
        if input.take_pressed(controls::SCALE) {
            self.display.integer = !self.display.integer;
            options.scaling = if self.display.integer {
                Scaling::Integer
            } else {
                Scaling::Fit
            };
        }
        if let Err(error) = presenter.set_options(options) {
            self.fail(event_loop, error);
            return;
        }
        if update.redraw_requested
            && let Err(error) = self.redraw(event_loop)
        {
            self.fail(event_loop, error);
        }
    }
    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let Some(window) = &mut self.window {
            window.process_device(&event);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.options.smoke_frames.is_some() && self.started.elapsed() > Duration::from_secs(30) {
            self.fail(
                event_loop,
                "native smoke run timed out before presenting requested frames",
            );
            return;
        }
        let automated = self.automated();
        let Some(window) = &mut self.window else {
            return;
        };
        let now = Instant::now();
        if window.focused() || automated {
            self.clock.advance(now - self.previous);
        } else {
            self.clock.reset();
        }
        self.previous = now;
        while self.clock.take_tick() {
            let input = controls::game_input(&window.input);
            let advanced = self.session.tick(input, self.options.autoplay);
            window.input.consume_edges();
            if !advanced
                || self.session.exhausted()
                || self.options.ticks.is_some_and(|n| self.session.ticks >= n)
            {
                event_loop.exit();
                break;
            }
        }
        if window.drawable() {
            window.native().request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            now + self.clock.until_next_tick().min(Duration::from_millis(16)),
        ));
    }
    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let Some(renderer) = &mut self.presenter {
            if self.options.screenshot.is_some() {
                let canvas = render::scene_with_hints(&self.session.game, &self.controls.hints());
                match renderer.render_to_image(&canvas, &self.textures) {
                    Ok(pixels) => self.screenshot = Some(pixels),
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            if self.options.smoke_frames.is_some() {
                println!("render_2d_stats={:?}", renderer.stats());
            }
        }
        self.presenter.take();
        self.window.take();
    }
}
