use quasar_app::{AppContext, Application, Flow, HeadlessConfig, run_headless};
use quasar_config::{CommonArgs, ConfigError};
use quasar_diagnostics::{Field, Level as LogLevel};
use starfall::content::Content;
use starfall::session::{GameSession, StepResult};
use starfall::settings::{GameCli, GameSettings};
use starfall::world::{GameError, GameInput, autoplay_input};
use std::time::Duration;

#[cfg(feature = "desktop")]
mod desktop;

fn main() {
    if let Err(error) = run() {
        eprintln!("starfall: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), GameError> {
    let mut common = CommonArgs::from_env("STARFALL")
        .map_err(|error: ConfigError| GameError::invalid(error.to_string()))?;
    common
        .parse_into(std::env::args().skip(1))
        .map_err(|error| GameError::invalid(error.to_string()))?;
    if common.help {
        print!("{}", common.help("starfall", GameCli::help()));
        return Ok(());
    }
    if common.version {
        println!("starfall {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let cli = GameCli::parse(&common.remaining)?;
    let settings = GameSettings::load(&common, &cli)?;
    let diagnostics = starfall::diagnostics(&settings);
    diagnostics.event_with(
        LogLevel::Info,
        "starfall.app",
        "starting",
        [
            Field::new("headless", settings.headless),
            Field::new("seed", settings.seed),
            Field::new("level", settings.level_path.clone()),
        ],
    );
    if settings.headless {
        return run_headless_app(settings, diagnostics);
    }
    #[cfg(feature = "desktop")]
    {
        desktop::run(settings, diagnostics)
    }
    #[cfg(not(feature = "desktop"))]
    {
        Err(GameError::invalid(
            "this build has no desktop feature; use --headless or rebuild with default features",
        ))
    }
}

fn run_headless_app(
    settings: GameSettings,
    diagnostics: quasar_diagnostics::Diagnostics,
) -> Result<(), GameError> {
    let ticks = settings.ticks;
    let step = Duration::from_secs_f64(1.0 / 60.0);
    run_headless(
        HeadlessApp {
            settings,
            diagnostics,
            session: None,
            content: None,
        },
        HeadlessConfig { ticks, step },
    )
    .map_err(|error| GameError::invalid(error.to_string()))
}

struct HeadlessApp {
    settings: GameSettings,
    diagnostics: quasar_diagnostics::Diagnostics,
    session: Option<GameSession>,
    content: Option<Content>,
}
impl Application for HeadlessApp {
    type Error = GameError;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        let content = starfall::load_content_blocking(&self.settings, &self.diagnostics)?;
        let world = starfall::world_from_content(&content, &self.settings)?;
        let session = if let Some(path) = &self.settings.replay {
            GameSession::from_file(world, path, &self.settings)?
        } else {
            GameSession::live(world, &self.settings)
        };
        self.session = Some(session);
        self.content = Some(content);
        Ok(())
    }
    fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        let Some(session) = &mut self.session else {
            return Ok(Flow::Continue);
        };
        if session.is_replay() {
            if session.step(GameInput::default())? == StepResult::Exhausted {
                return Ok(Flow::Exit);
            }
        } else {
            let input = if self.settings.autoplay {
                autoplay_input(session.world())
            } else {
                GameInput::default()
            };
            session.step(input)?;
        }
        Ok(Flow::Continue)
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        if let Some(session) = &self.session {
            let summary = session.summary();
            println!("{summary}");
            if let Some(path) = &self.settings.record {
                session.write_record(path)?;
                println!("record={}", path.display());
            }
            if let (Some(content), Some(path)) = (&self.content, &self.settings.screenshot) {
                let mut settings = self.settings.clone();
                settings.screenshot = Some(path.clone());
                starfall::render_screenshot(&settings, content, session.world())?;
                println!("screenshot={}", path.display());
            }
        }
        Ok(())
    }
}
