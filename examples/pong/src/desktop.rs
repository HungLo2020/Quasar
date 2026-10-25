use super::*;
use pong::Phase;
use quasar_app::{
    AppConfig, AppContext, Application, DesktopApplication, Flow, RenderPolicy, TimingConfig,
    run as run_app,
};
use quasar_core::{DisplayOptions, Filter, FixedStepConfig, Scaling};
use quasar_render_2d::{PresentOutcome, Renderer2D, Textures};
use quasar_storage::{GamePaths, read_json, write_json};
use quasar_window::{DesktopConfig, WindowConfig};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct AppError(Box<dyn Error>);
impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}
impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}
impl From<Box<dyn Error>> for AppError {
    fn from(error: Box<dyn Error>) -> Self {
        Self(error)
    }
}
impl From<&str> for AppError {
    fn from(error: &str) -> Self {
        Self(Box::new(std::io::Error::other(error.to_owned())))
    }
}
impl From<String> for AppError {
    fn from(error: String) -> Self {
        Self(Box::new(std::io::Error::other(error)))
    }
}
impl From<quasar_input::BindingError> for AppError {
    fn from(error: quasar_input::BindingError) -> Self {
        Self(Box::new(error))
    }
}
impl From<quasar_render_2d::Error> for AppError {
    fn from(error: quasar_render_2d::Error) -> Self {
        Self(Box::new(error))
    }
}
impl From<quasar_render_2d::GalError> for AppError {
    fn from(error: quasar_render_2d::GalError) -> Self {
        Self(Box::new(error))
    }
}
type AppResult<T> = std::result::Result<T, AppError>;

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

    let config = AppConfig {
        window: WindowConfig {
            title: "Quasar / Pong".into(),
            application_id: "quasar-pong".into(),
            size: (display.width, display.height),
            minimum_size: (320, 200),
            fullscreen: display.fullscreen,
            resizable: true,
        },
        desktop: DesktopConfig::for_application("quasar-pong"),
        timing: TimingConfig::Fixed(FixedStepConfig::for_rate(pong::TICK_HZ, 8)),
        render: RenderPolicy::EveryFrame,
        max_frame_wait: Duration::from_millis(16),
        frame_interval: None,
    };
    let app = App {
        presenter: None,
        textures: Textures::default(),
        screenshot: None,
        options,
        session,
        controls,
        save_controls,
        display,
        paths,
        save_display,
        save_records,
        records,
        size: (0, 0),
        frames: 0,
        exercise_stage: 0,
        started: Instant::now(),
        pause_pending: false,
    };
    run_app(app, config)?;
    Ok(())
}

struct App {
    presenter: Option<Renderer2D>,
    textures: Textures,
    screenshot: Option<Vec<u8>>,
    options: Options,
    session: Session,
    controls: controls::Controls,
    save_controls: bool,
    display: DisplaySettings,
    paths: GamePaths,
    save_display: bool,
    save_records: bool,
    records: Records,
    size: (u32, u32),
    frames: u64,
    exercise_stage: u8,
    started: Instant,
    pause_pending: bool,
}

impl App {
    fn automated(&self) -> bool {
        self.options.autoplay
            || self.session.replay.is_some()
            || self.options.smoke_frames.is_some()
    }

    fn initialize(&mut self, context: &mut AppContext<'_>) -> Result {
        let window = context
            .window
            .ok_or("desktop runner did not provide a window")?;
        let size = window.native().inner_size();
        self.presenter = Some(Renderer2D::for_window(
            window.shared_native(),
            (size.width.max(1), size.height.max(1)),
            self.display.options(),
        )?);
        self.size = (size.width, size.height);
        self.started = Instant::now();
        Ok(())
    }

    fn exercise(&mut self, context: &AppContext<'_>) -> Result {
        let window = context
            .window
            .ok_or("desktop runner did not provide a window")?;
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

    fn write_outputs(&mut self) -> Result {
        if let Some(presenter) = &mut self.presenter
            && self.options.screenshot.is_some()
        {
            let canvas = render::scene_with_hints(&self.session.game, &self.controls.hints());
            self.screenshot = Some(presenter.render_to_image(&canvas, &self.textures)?);
        }
        if self.options.smoke_frames.is_some()
            && let Some(renderer) = &self.presenter
        {
            println!("render_2d_stats={:?}", renderer.stats());
        }
        self.session.outputs(
            &self.options,
            Some(&self.controls.hints()),
            self.screenshot.as_deref(),
        )?;
        if self.save_controls {
            write_json(
                &self.paths.shared_config.join("controls.json"),
                &self.controls,
            )?;
        }
        if self.save_display {
            write_json(&self.paths.config.join("display.json"), &self.display)?;
        }
        if self.save_records {
            self.records.longest_rally = self.records.longest_rally.max(self.session.best_rally);
            write_json(&self.paths.save.join("records.json"), &self.records)?;
        }
        Ok(())
    }
}

impl Application for App {
    type Error = AppError;

    fn setup(&mut self, context: &mut AppContext<'_>) -> AppResult<()> {
        self.initialize(context).map_err(AppError::from)
    }

    fn frame_update(&mut self, context: &mut AppContext<'_>) -> AppResult<Flow> {
        if self.options.smoke_frames.is_some() && self.started.elapsed() > Duration::from_secs(30) {
            return Err("native smoke run timed out before presenting requested frames".into());
        }
        let size = context.size;
        if size != self.size {
            if let Some(presenter) = &mut self.presenter {
                presenter.resize(size.0, size.1);
            }
            self.size = size;
            if !self.display.fullscreen
                && size.0 > 0
                && size.1 > 0
                && let Some(window) = context.window
            {
                let logical = quasar_window::winit::dpi::PhysicalSize::new(size.0, size.1)
                    .to_logical::<u32>(window.native().scale_factor());
                self.display.width = logical.width.clamp(320, 16384);
                self.display.height = logical.height.clamp(200, 16384);
            }
        }
        if context.focus_changed == Some(false)
            && !self.automated()
            && self.session.game.phase == Phase::Playing
        {
            self.pause_pending = true;
        }

        let mut options = self.presenter.as_ref().map(|presenter| presenter.options());
        let mut changed = false;
        if context.input.action(controls::QUIT).pressed {
            context.input.consume_action(controls::QUIT);
            return Ok(Flow::Exit);
        }
        if context.input.action(controls::FULLSCREEN).pressed {
            context.input.consume_action(controls::FULLSCREEN);
            if let Some(window) = context.window {
                window.toggle_fullscreen();
                self.display.fullscreen = window.fullscreen();
            }
        }
        if context.input.action(controls::FILTER).pressed {
            context.input.consume_action(controls::FILTER);
            self.display.linear = !self.display.linear;
            if let Some(options) = &mut options {
                options.filter = if self.display.linear {
                    Filter::Linear
                } else {
                    Filter::Nearest
                };
                changed = true;
            }
        }
        if context.input.action(controls::SCALE).pressed {
            context.input.consume_action(controls::SCALE);
            self.display.integer = !self.display.integer;
            if let Some(options) = &mut options {
                options.scaling = if self.display.integer {
                    Scaling::Integer
                } else {
                    Scaling::Fit
                };
                changed = true;
            }
        }
        if changed && let (Some(options), Some(presenter)) = (options, &mut self.presenter) {
            presenter.set_options(options)?;
        }
        Ok(Flow::Continue)
    }

    fn tick(&mut self, context: &mut AppContext<'_>) -> AppResult<Flow> {
        let mut input = controls::game_input(context.input);
        if self.pause_pending {
            input.pause = true;
            self.pause_pending = false;
        }
        let advanced = self.session.tick(input, self.options.autoplay);
        if !advanced
            || self.session.exhausted()
            || self.options.ticks.is_some_and(|n| self.session.ticks >= n)
        {
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    fn render(&mut self, context: &mut AppContext<'_>) -> AppResult<Flow> {
        if !context.drawable() {
            return Ok(Flow::Continue);
        }
        let Some(presenter) = &mut self.presenter else {
            return Ok(Flow::Continue);
        };
        let outcome = presenter.present(
            &render::scene_with_hints(&self.session.game, &self.controls.hints()),
            &self.textures,
        )?;
        if outcome == PresentOutcome::Presented {
            self.frames += 1;
            if self.options.exercise_window {
                self.exercise(context)?;
            }
            if self.options.smoke_frames.is_some_and(|n| self.frames >= n) {
                println!(
                    "presented_frames={} window_exercise_stages={}",
                    self.frames, self.exercise_stage
                );
                return Ok(Flow::Exit);
            }
        }
        Ok(Flow::Continue)
    }

    fn suspended(&mut self, _: &mut AppContext<'_>) -> AppResult<()> {
        self.presenter.take();
        Ok(())
    }

    fn shutdown(&mut self, _: &mut AppContext<'_>) -> AppResult<()> {
        self.write_outputs().map_err(AppError::from)
    }
}

impl DesktopApplication for App {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> AppResult<()> {
        if self.presenter.is_none() {
            self.initialize(context).map_err(AppError::from)?;
        }
        Ok(())
    }

    fn configure_window(&mut self, window: &mut quasar_window::GameWindow) -> AppResult<()> {
        window.input.set_bindings(self.controls.bindings.clone())?;
        Ok(())
    }
}
