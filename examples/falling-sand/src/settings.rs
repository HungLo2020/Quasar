//! Command-line configuration for the falling-sand demo.
use crate::SandError;
use quasar_diagnostics::Level;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Settings {
    pub headless: bool,
    pub ticks: u64,
    pub seed: u64,
    pub width: u32,
    pub height: u32,
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
            seed: 0x5eed_1234,
            width: 1280,
            height: 720,
            screenshot: None,
            autoplay: false,
            benchmark: false,
            benchmark_frames: 240,
            benchmark_file: None,
            log_level: Level::Info,
        }
    }
}

impl Settings {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, SandError> {
        let mut settings = Self::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--help" | "-h" => {
                    print!("{}", Self::help());
                    std::process::exit(0);
                }
                "--version" => {
                    println!("falling-sand {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                "--headless" => settings.headless = true,
                "--autoplay" => settings.autoplay = true,
                "--benchmark" => settings.benchmark = true,
                "--ticks" => {
                    settings.ticks = value(&mut arguments, "--ticks")?.parse().map_err(|_| {
                        SandError::invalid("--ticks must be a non-negative integer")
                    })?;
                }
                "--seed" => {
                    settings.seed = value(&mut arguments, "--seed")?
                        .parse()
                        .map_err(|_| SandError::invalid("--seed must be a non-negative integer"))?;
                }
                "--width" => {
                    settings.width = value(&mut arguments, "--width")?
                        .parse()
                        .map_err(|_| SandError::invalid("--width must be in 64..=7680"))?;
                }
                "--height" => {
                    settings.height = value(&mut arguments, "--height")?
                        .parse()
                        .map_err(|_| SandError::invalid("--height must be in 64..=4320"))?;
                }
                "--screenshot" => {
                    settings.screenshot =
                        Some(PathBuf::from(value(&mut arguments, "--screenshot")?));
                }
                "--benchmark-frames" => {
                    settings.benchmark_frames = value(&mut arguments, "--benchmark-frames")?
                        .parse()
                        .map_err(|_| {
                            SandError::invalid("--benchmark-frames must be a positive integer")
                        })?;
                }
                "--benchmark-file" => {
                    settings.benchmark_file =
                        Some(PathBuf::from(value(&mut arguments, "--benchmark-file")?));
                }
                "--warn" => settings.log_level = Level::Warn,
                "--debug" => settings.log_level = Level::Debug,
                other => {
                    return Err(SandError::invalid(format!(
                        "unknown argument '{other}'; use --help"
                    )));
                }
            }
        }
        if !(64..=7680).contains(&settings.width) || !(64..=4320).contains(&settings.height) {
            return Err(SandError::invalid("window dimensions are out of range"));
        }
        if settings.ticks == 0 {
            return Err(SandError::invalid("--ticks must be at least 1"));
        }
        if settings.benchmark_frames == 0 {
            return Err(SandError::invalid("--benchmark-frames must be at least 1"));
        }
        Ok(settings)
    }

    pub fn help() -> &'static str {
        "\
Falling Sand - cellular sand with Rapier 2D obstacles and quasar-spatial queries

USAGE:
    falling-sand [OPTIONS]

OPTIONS:
    --headless                 Run deterministic ticks without a window
    --ticks N                  Headless tick budget (default 600)
    --seed N                   Initial sand/ball seed (default 0x5eed1234)
    --autoplay                 Drive the deterministic scripted brush
    --width N --height N       Window or screenshot dimensions
    --screenshot PATH          Write a PNG after the run
    --benchmark                Collect frame/profile data in native mode
    --benchmark-frames N       Frames to keep before exiting (default 240)
    --benchmark-file PATH      Write machine-readable profile JSON
    --warn | --debug           Diagnostics level
    -h, --help                 Show this help
    --version                  Show version
"
    }
}

fn value(arguments: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, SandError> {
    arguments
        .next()
        .ok_or_else(|| SandError::invalid(format!("{flag} needs a value")))
}
