//! Command-line configuration.
use crate::BlocksError;
use quasar_diagnostics::Level;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Settings {
    pub headless: bool,
    pub ticks: u64,
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub radius: i32,
    pub workers: usize,
    pub screenshot: Option<PathBuf>,
    pub autoplay: bool,
    pub occlusion: bool,
    /// Bevelled stone and valley mist instead of the classic look.
    pub sculpted: bool,
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
            seed: 0xb10c,
            width: 1280,
            height: 720,
            radius: 8,
            workers: 0,
            screenshot: None,
            autoplay: false,
            occlusion: true,
            sculpted: false,
            benchmark: false,
            benchmark_frames: 240,
            benchmark_file: None,
            log_level: Level::Info,
        }
    }
}

fn value(arguments: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, BlocksError> {
    arguments
        .next()
        .ok_or_else(|| BlocksError::invalid(format!("{flag} needs a value")))
}

fn number<T: std::str::FromStr>(
    arguments: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<T, BlocksError> {
    value(arguments, flag)?
        .parse()
        .map_err(|_| BlocksError::invalid(format!("{flag} needs a number")))
}

impl Settings {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, BlocksError> {
        let mut settings = Self::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--help" | "-h" => {
                    print!("{}", Self::help());
                    std::process::exit(0);
                }
                "--version" => {
                    println!("voxel-blocks {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                "--headless" => settings.headless = true,
                "--autoplay" => settings.autoplay = true,
                "--no-occlusion" => settings.occlusion = false,
                "--sculpted" => settings.sculpted = true,
                "--benchmark" => settings.benchmark = true,
                "--ticks" => settings.ticks = number(&mut arguments, "--ticks")?,
                "--seed" => settings.seed = number(&mut arguments, "--seed")?,
                "--width" => settings.width = number(&mut arguments, "--width")?,
                "--height" => settings.height = number(&mut arguments, "--height")?,
                "--radius" => settings.radius = number(&mut arguments, "--radius")?,
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
                    return Err(BlocksError::invalid(format!(
                        "unknown argument '{other}'; use --help"
                    )));
                }
            }
        }
        if !(64..=7680).contains(&settings.width) || !(64..=4320).contains(&settings.height) {
            return Err(BlocksError::invalid("window dimensions are out of range"));
        }
        if !(1..=48).contains(&settings.radius) {
            return Err(BlocksError::invalid("--radius must be in 1..=48"));
        }
        if settings.ticks == 0 || settings.benchmark_frames == 0 {
            return Err(BlocksError::invalid(
                "--ticks and --benchmark-frames must be at least 1",
            ));
        }
        Ok(settings)
    }

    pub fn help() -> &'static str {
        "\
Voxel Blocks - a block world on Quasar's voxel stack (MattMC-style defaults)

USAGE:
    voxel-blocks [OPTIONS]

OPTIONS:
    --headless                 Run deterministic ticks without a window
    --ticks N                  Headless tick budget (default 600)
    --seed N                   World seed
    --radius N                 View radius in chunks (default 8)
    --workers N                Generation/meshing threads (default: all cores)
    --autoplay                 Drive the scripted player (walk, dig, build, fly)
    --no-occlusion             Draw every chunk (disable connectivity culling)
    --sculpted                 Bevelled, chipped stone and valley mist
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
