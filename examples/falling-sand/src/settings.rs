//! Command-line configuration for the falling-sand demo.
use crate::SandError;
use quasar_diagnostics::Level;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiOptions {
    pub fullscreen: bool,
    pub brush_radius: f32,
    pub ui_scale: f32,
}

impl Default for UiOptions {
    fn default() -> Self {
        Self {
            fullscreen: false,
            brush_radius: 4.0,
            ui_scale: 1.0,
        }
    }
}

impl UiOptions {
    pub fn load_default() -> Self {
        let path = Self::path();
        Self::load(&path).unwrap_or_default()
    }

    pub fn path() -> PathBuf {
        default_ui_path()
    }

    pub fn load(path: &Path) -> Result<Self, SandError> {
        let mut config = quasar_config::Config::new();
        config
            .set_default("ui.fullscreen", false)
            .map_err(|error| SandError::invalid(error.to_string()))?;
        config
            .set_default("ui.brush_radius", 4.0)
            .map_err(|error| SandError::invalid(error.to_string()))?;
        config
            .set_default("ui.scale", 1.0)
            .map_err(|error| SandError::invalid(error.to_string()))?;
        if path.exists() {
            config
                .load_file(path)
                .map_err(|error| SandError::invalid(format!("UI settings: {error}")))?;
        }
        Ok(Self {
            fullscreen: config
                .get_bool("ui.fullscreen")
                .map_err(|error| SandError::invalid(error.to_string()))?,
            brush_radius: config
                .get_f32("ui.brush_radius")
                .map_err(|error| SandError::invalid(error.to_string()))?
                .clamp(1.0, 16.0),
            ui_scale: config
                .get_f32("ui.scale")
                .map_err(|error| SandError::invalid(error.to_string()))?
                .clamp(0.75, 2.0),
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), SandError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| SandError::invalid(format!("create UI settings dir: {error}")))?;
        }
        let text = format!(
            "version = 1\n\n[ui]\nfullscreen = {}\nbrush_radius = {:.2}\nscale = {:.2}\n",
            self.fullscreen, self.brush_radius, self.ui_scale
        );
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, text)
            .map_err(|error| SandError::invalid(format!("write UI settings: {error}")))?;
        std::fs::rename(&temporary, path)
            .map_err(|error| SandError::invalid(format!("replace UI settings: {error}")))?;
        Ok(())
    }
}

fn default_ui_path() -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("quasar").join("falling-sand.toml");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("quasar")
            .join("falling-sand.toml");
    }
    PathBuf::from("falling-sand-ui.toml")
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub headless: bool,
    pub ticks: u64,
    pub seed: u64,
    pub brush_radius: f32,
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
            brush_radius: 4.0,
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
                "--brush-radius" => {
                    settings.brush_radius = value(&mut arguments, "--brush-radius")?
                        .parse()
                        .map_err(|_| SandError::invalid("--brush-radius must be 1..=16"))?;
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
        if !(1.0..=16.0).contains(&settings.brush_radius) {
            return Err(SandError::invalid("--brush-radius must be 1..=16"));
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
    --brush-radius N           Brush radius in cells (default 4)
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
