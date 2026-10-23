mod cli;
mod controls;
#[cfg(feature = "desktop")]
mod desktop;
use cli::Options;
use pong::{Game, Input, Mode, Replay, render};
use std::{error::Error, path::Path};
type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

struct Session {
    game: Game,
    replay: Option<Replay>,
    recording: Option<Replay>,
    ticks: usize,
    best_rally: u32,
}
impl Session {
    fn new(options: &Options) -> Result<Self> {
        let replay: Option<Replay> = options
            .replay
            .as_ref()
            .map(|path| -> Result<Replay> {
                let replay: Replay = serde_json::from_slice(&std::fs::read(path)?)?;
                replay.validate()?;
                Ok(replay)
            })
            .transpose()?;
        let mode = replay
            .as_ref()
            .map(|r| r.mode)
            .unwrap_or(if options.two_player {
                Mode::Local
            } else {
                Mode::Solo
            });
        if options.two_player && mode != Mode::Local {
            return Err("--two-player conflicts with replay mode".into());
        }
        Ok(Self {
            game: Game::new(mode),
            replay,
            recording: options.record.as_ref().map(|_| Replay::new(mode)),
            ticks: 0,
            best_rally: 0,
        })
    }
    fn tick(&mut self, live: Input, autoplay: bool) -> bool {
        let input = if let Some(replay) = &self.replay {
            let Some(input) = replay.inputs.get(self.ticks) else {
                return false;
            };
            *input
        } else if autoplay {
            self.game.autoplay()
        } else {
            live
        };
        self.game.tick(input);
        self.best_rally = self.best_rally.max(self.game.longest_rally);
        self.ticks += 1;
        if let Some(recording) = &mut self.recording {
            recording.inputs.push(input);
        }
        true
    }
    #[cfg(feature = "desktop")]
    fn exhausted(&self) -> bool {
        self.replay
            .as_ref()
            .is_some_and(|r| self.ticks >= r.inputs.len())
    }
    fn outputs(&self, options: &Options, hints: Option<&pong::render::ControlHints>) -> Result {
        if let Some(path) = &options.record {
            quasar_storage::write_json(path, self.recording.as_ref().unwrap())?;
        }
        if let Some(path) = &options.state {
            quasar_storage::write_json(path, &self.game)?;
        }
        if let Some(path) = &options.screenshot {
            let pixels = hints.map_or_else(
                || render::frame(&self.game),
                |hints| render::frame_with_hints(&self.game, hints),
            );
            write_png(path, &pixels)?;
        }
        println!(
            "ticks={} score={}:{} phase={:?} longest_rally={}",
            self.ticks,
            self.game.score[0],
            self.game.score[1],
            self.game.phase,
            self.game.longest_rally
        );
        Ok(())
    }
}
fn write_png(path: &Path, rgba: &[u8]) -> Result {
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(path)?),
        pong::WIDTH,
        pong::HEIGHT,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}
fn main() -> Result {
    let Some(options) = Options::parse(std::env::args().skip(1))? else {
        return Ok(());
    };
    if let Some(path) = &options.write_default_bindings {
        quasar_storage::write_json(path, &controls::Controls::default())?;
        println!("Wrote default Pong controls to {}", path.display());
        return Ok(());
    }
    if let Some(path) = &options.check_bindings {
        controls::Controls::load(path)?;
        println!("Pong controls are valid");
        return Ok(());
    }
    let controls = options
        .bindings
        .as_ref()
        .map(|path| controls::Controls::load(path))
        .transpose()?;
    let mut session = Session::new(&options)?;
    if options.headless {
        let ticks = options
            .ticks
            .unwrap_or_else(|| session.replay.as_ref().map_or(3600, |r| r.inputs.len()));
        for _ in 0..ticks {
            if !session.tick(Input::default(), options.autoplay) {
                break;
            }
        }
        return session.outputs(&options, None);
    }
    #[cfg(feature = "desktop")]
    {
        desktop::run(options, session, controls)
    }
    #[cfg(not(feature = "desktop"))]
    {
        let _ = controls;
        Err("desktop support was disabled at build time; use --headless or rebuild with default features".into())
    }
}
