#[cfg(feature = "desktop")]
mod desktop;
use std::path::{Path, PathBuf};
use tilequest::{Art, Autoplay, Input, Replay, Result, State, World};
#[derive(Default)]
struct Options {
    headless: bool,
    autoplay: bool,
    ticks: Option<usize>,
    frames: Option<u64>,
    exercise: bool,
    map: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    state: Option<PathBuf>,
    record: Option<PathBuf>,
    replay: Option<PathBuf>,
    bindings: Option<PathBuf>,
    write_bindings: Option<PathBuf>,
    benchmark: Option<PathBuf>,
}
impl Options {
    fn parse() -> Result<Self> {
        let mut o = Self::default();
        let mut a = std::env::args().skip(1);
        let mut seen = std::collections::BTreeSet::new();
        while let Some(flag) = a.next() {
            if !seen.insert(flag.clone()) {
                return Err(format!("duplicate {flag}").into());
            }
            let next =
                |a: &mut std::iter::Skip<std::env::Args>| a.next().ok_or("missing option argument");
            match flag.as_str() {
                "--headless" => o.headless = true,
                "--autoplay" => o.autoplay = true,
                "--exercise-window" => o.exercise = true,
                "--ticks" => o.ticks = Some(next(&mut a)?.parse()?),
                "--smoke-frames" => o.frames = Some(next(&mut a)?.parse()?),
                "--map" => o.map = Some(next(&mut a)?.into()),
                "--screenshot" => o.screenshot = Some(next(&mut a)?.into()),
                "--state" => o.state = Some(next(&mut a)?.into()),
                "--record" => o.record = Some(next(&mut a)?.into()),
                "--replay" => o.replay = Some(next(&mut a)?.into()),
                "--bindings" => o.bindings = Some(next(&mut a)?.into()),
                "--write-default-bindings" => o.write_bindings = Some(next(&mut a)?.into()),
                "--benchmark" => o.benchmark = Some(next(&mut a)?.into()),
                "--help" => {
                    println!(
                        "Tilequest: --headless --autoplay --ticks N --state FILE --screenshot PNG --record FILE --replay FILE --map MAP --smoke-frames N --exercise-window --benchmark FILE --bindings FILE --write-default-bindings FILE"
                    );
                    std::process::exit(0);
                }
                _ => return Err(format!("unknown option {flag}").into()),
            }
        }
        if o.benchmark.is_some() && o.frames.is_none() {
            o.frames = Some(364);
        }
        if o.benchmark.is_some()
            && (o.frames.is_some_and(|n| n <= 64) || o.exercise || o.screenshot.is_some())
        {
            return Err(
                "benchmark needs >64 frames and excludes window exercises/screenshots".into(),
            );
        }
        if o.ticks == Some(0)
            || o.ticks.is_some_and(|n| n > 1_000_000)
            || o.frames == Some(0)
            || o.autoplay && o.replay.is_some()
            || o.record.is_some() && o.replay.is_some()
            || o.headless && (o.frames.is_some() || o.exercise || o.benchmark.is_some())
            || o.exercise && o.frames.is_none()
        {
            return Err("invalid or conflicting run options".into());
        }
        Ok(o)
    }
}
struct Session {
    world: World,
    state: State,
    art: Art,
    auto: Autoplay,
    replay: Option<Replay>,
    record: Vec<Input>,
    recording: bool,
    ticks: usize,
}
impl Session {
    fn new(world: World, replay: Option<Replay>, recording: bool) -> Result<Self> {
        if let Some(replay) = &replay {
            replay.validate(&world)?;
        }
        let state = world.initial();
        let art = Art::load(&world)?;
        Ok(Self {
            world,
            state,
            art,
            auto: Autoplay::default(),
            replay,
            record: Vec::new(),
            recording,
            ticks: 0,
        })
    }
    fn step(&mut self, input: Input, autoplay: bool) -> Result<bool> {
        let input = if let Some(replay) = &self.replay {
            let Some(input) = replay.inputs.get(self.ticks) else {
                return Ok(false);
            };
            *input
        } else if autoplay {
            self.auto.input(&self.world, &self.state)?
        } else {
            input
        };
        self.world.tick(&mut self.state, input)?;
        if self.recording {
            if self.record.len() >= 1_000_000 {
                return Err("recording exceeds one million ticks".into());
            }
            self.record.push(input);
        }
        self.ticks += 1;
        Ok(true)
    }
    #[cfg(feature = "desktop")]
    fn done(&self, o: &Options) -> bool {
        o.ticks.is_some_and(|n| self.ticks >= n)
            || self
                .replay
                .as_ref()
                .is_some_and(|r| self.ticks >= r.inputs.len())
    }
    fn outputs(&self, o: &Options, gpu: Option<&[u8]>) -> Result {
        if let Some(path) = &o.state {
            quasar_storage::write_json(path, &self.state)?;
        }
        if let Some(path) = &o.record {
            quasar_storage::write_json(
                path,
                &Replay {
                    version: 1,
                    map_hash: self.world.hash,
                    inputs: self.record.clone(),
                },
            )?;
        }
        if let Some(path) = &o.screenshot {
            let rgba = if let Some(bytes) = gpu {
                bytes.to_vec()
            } else {
                let (canvas, _) = tilequest::scene(
                    &self.world,
                    &self.state,
                    &self.art,
                    quasar_render_2d::Filter::Nearest,
                )?;
                quasar_render_2d::reference::rasterize(&canvas, &self.art.textures)?
            };
            quasar_tilemap::render::write_png(path, tilequest::WIDTH, tilequest::HEIGHT, &rgba)?;
        }
        println!(
            "ticks={} beacons={}/{} phase={:?} stored_cells={}",
            self.ticks,
            self.state.collected.len(),
            self.world.beacons.len(),
            self.state.phase,
            self.world.runtime.stored_cells()
        );
        Ok(())
    }
}
fn default_map() -> Result<PathBuf> {
    let paths = quasar_storage::GamePaths::discover(Some(Path::new(env!("CARGO_MANIFEST_DIR"))))?;
    let packaged = paths.app.join("assets/island.tmj");
    if packaged.is_file() {
        return Ok(packaged);
    }
    let beside = std::env::current_exe()?
        .parent()
        .ok_or("missing executable parent")?
        .join("assets/island.tmj");
    if beside.is_file() {
        return Ok(beside);
    }
    Ok(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/island.tmj"))
}
fn main() -> Result {
    let o = Options::parse()?;
    if let Some(path) = &o.write_bindings {
        quasar_storage::write_json(path, &tilequest::controls::Controls::default())?;
        return Ok(());
    }
    let controls = if let Some(path) = &o.bindings {
        let c: tilequest::controls::Controls =
            quasar_storage::read_json(path)?.ok_or("missing bindings")?;
        c.validate()?;
        c
    } else {
        tilequest::controls::Controls::default()
    };
    let world = World::load(o.map.clone().map_or_else(default_map, Ok)?)?;
    let replay = if let Some(path) = &o.replay {
        Some(quasar_storage::read_json(path)?.ok_or("missing replay")?)
    } else {
        None
    };
    let mut session = Session::new(world, replay, o.record.is_some())?;
    if o.headless {
        let limit = o
            .ticks
            .unwrap_or_else(|| session.replay.as_ref().map_or(6000, |r| r.inputs.len()));
        for _ in 0..limit {
            if !session.step(Input::default(), o.autoplay)? {
                break;
            }
        }
        return session.outputs(&o, None);
    }
    #[cfg(feature = "desktop")]
    {
        desktop::run(o, session, controls)
    }
    #[cfg(not(feature = "desktop"))]
    {
        let _ = controls;
        Err("this build needs --headless; enable desktop to open a window".into())
    }
}
