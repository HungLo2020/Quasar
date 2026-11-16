//! Command-line configuration.
use crate::DriftError;
use quasar_diagnostics::Level;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Settings {
    pub headless: bool,
    pub ticks: u64,
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub workers: usize,
    pub screenshot: Option<PathBuf>,
    pub autoplay: bool,
    pub benchmark: bool,
    pub benchmark_frames: usize,
    pub benchmark_file: Option<PathBuf>,
    pub log_level: Level,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            headless: false,
            ticks: 600,
            seed: 0xd51f7,
            width: 1280,
            height: 720,
            workers: 0,
            screenshot: None,
            autoplay: false,
            benchmark: false,
            benchmark_frames: 240,
            benchmark_file: None,
            log_level: Level::Info,
        }
    }
}

fn value(arguments: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, DriftError> {
    arguments
        .next()
        .ok_or_else(|| DriftError::invalid(format!("{flag} needs a value")))
}

fn number<T: std::str::FromStr>(
    arguments: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<T, DriftError> {
    value(arguments, flag)?
        .parse()
        .map_err(|_| DriftError::invalid(format!("{flag} needs a number")))
}

impl Settings {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, DriftError> {
        let mut settings = Self::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--help" | "-h" => {
                    print!("{}", Self::help());
                    std::process::exit(0);
                }
                "--version" => {
                    println!("voxel-drift {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                "--headless" => settings.headless = true,
                "--autoplay" => settings.autoplay = true,
                "--benchmark" => settings.benchmark = true,
                "--ticks" => settings.ticks = number(&mut arguments, "--ticks")?,
                "--seed" => settings.seed = number(&mut arguments, "--seed")?,
                "--width" => settings.width = number(&mut arguments, "--width")?,
                "--height" => settings.height = number(&mut arguments, "--height")?,
                "--workers" => settings.workers = number(&mut arguments, "--workers")?,
                "--screenshot" => {
                    settings.screenshot =
                        Some(PathBuf::from(value(&mut arguments, "--screenshot")?))
                }
                "--benchmark-frames" => {
                    settings.benchmark_frames = number(&mut arguments, "--benchmark-frames")?
                }
                "--benchmark-file" => {
                    settings.benchmark_file =
                        Some(PathBuf::from(value(&mut arguments, "--benchmark-file")?))
                }
                "--warn" => settings.log_level = Level::Warn,
                "--debug" => settings.log_level = Level::Debug,
                other => {
                    return Err(DriftError::invalid(format!(
                        "unknown argument '{other}'; use --help"
                    )));
                }
            }
        }
        if !(64..=7680).contains(&settings.width) || !(64..=4320).contains(&settings.height) {
            return Err(DriftError::invalid("window dimensions are out of range"));
        }
        if settings.ticks == 0 || settings.benchmark_frames == 0 {
            return Err(DriftError::invalid(
                "--ticks and --benchmark-frames must be at least 1",
            ));
        }
        Ok(settings)
    }

    pub fn help() -> &'static str {
        "\
Voxel Drift - a Space Engineers-style scene on Quasar's voxel stack

USAGE:
    voxel-drift [OPTIONS]

OPTIONS:
    --headless                 Run deterministic ticks without a window
    --ticks N                  Headless tick budget (default 600)
    --seed N                   Asteroid seed
    --workers N                Meshing/generation threads (default: all cores)
    --autoplay                 Drive the scripted pilot (dig, fill, build, cut the ship)
    --width N --height N       Window or screenshot dimensions
    --screenshot PATH          Write a PNG after the run
    --benchmark                Native: collect frame data, then exit
    --benchmark-frames N       Frames to keep before exiting (default 240)
    --benchmark-file PATH      Write machine-readable JSON (native and headless)
    --warn | --debug           Diagnostics level
    -h, --help                 Show this help
    --version                  Show version
"
    }
}
