//! Typed settings, precedence and game-specific CLI options.

use crate::world::{GameError, GameRules};
use quasar_config::{CommonArgs, Config, ConfigError, ConfigValue};
use quasar_diagnostics::Level as LogLevel;
use quasar_input::{
    Binding, Bindings, Button, GamepadAxis, GamepadButton, InputRead, Key, MouseButton,
};
use quasar_sim::SimulationConfig;
use serde::Deserialize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub const GAME_VERSION: u64 = 1;
pub const TIMELINE: &str = "starfall-main";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlKeys {
    pub forward: Key,
    pub back: Key,
    pub left: Key,
    pub right: Key,
    pub turn_left: Key,
    pub turn_right: Key,
    pub shoot: Key,
    pub restart: Key,
    pub quit: Key,
}
impl Default for ControlKeys {
    fn default() -> Self {
        Self {
            forward: Key::KeyW,
            back: Key::KeyS,
            left: Key::KeyA,
            right: Key::KeyD,
            turn_left: Key::KeyQ,
            turn_right: Key::KeyE,
            shoot: Key::Space,
            restart: Key::KeyR,
            quit: Key::Escape,
        }
    }
}
impl ControlKeys {
    pub fn from_config(config: &Config) -> Result<Self, GameError> {
        let read = |key: &str| -> Result<Key, GameError> {
            let name = config
                .get_string(key)
                .map_err(|error| GameError::invalid(error.to_string()))?;
            Key::from_name(&name).ok_or_else(|| {
                GameError::invalid(format!(
                    "{key}='{name}' is not a W3C physical key name (for example KeyW or Space)"
                ))
            })
        };
        let keys = Self {
            forward: read("controls.forward")?,
            back: read("controls.back")?,
            left: read("controls.left")?,
            right: read("controls.right")?,
            turn_left: read("controls.turn_left")?,
            turn_right: read("controls.turn_right")?,
            shoot: read("controls.shoot")?,
            restart: read("controls.restart")?,
            quit: read("controls.quit")?,
        };
        keys.validate()?;
        Ok(keys)
    }
    pub fn validate(&self) -> Result<(), GameError> {
        let keys = [
            self.forward,
            self.back,
            self.left,
            self.right,
            self.turn_left,
            self.turn_right,
            self.shoot,
            self.restart,
            self.quit,
        ];
        let unique: HashSet<Key> = keys.into_iter().collect();
        if unique.len() != keys.len() {
            return Err(GameError::invalid(
                "control bindings must not assign the same key twice",
            ));
        }
        Ok(())
    }
    pub fn bindings(self) -> Result<Bindings, GameError> {
        let mut bindings = Bindings::default();
        bindings
            .bind("forward", Binding::axis(self.back, self.forward))
            .bind("strafe", Binding::axis(self.left, self.right))
            .bind("turn", Binding::axis(self.turn_left, self.turn_right))
            .bind("shoot", Binding::key(self.shoot))
            .bind(
                "shoot",
                Binding::Button {
                    button: Button::Mouse(MouseButton::Left),
                },
            )
            .bind("restart", Binding::key(self.restart))
            .bind("quit", Binding::key(self.quit))
            // Gamepad remains optional: these actions are inert until a caller
            // feeds GamepadHub/InputState; keyboard remains fully configurable.
            .bind(
                "forward",
                Binding::gamepad_axis_scaled(GamepadAxis::LeftStickY, -1.0),
            )
            .bind("strafe", Binding::gamepad_axis(GamepadAxis::LeftStickX))
            .bind("turn", Binding::gamepad_axis(GamepadAxis::RightStickX))
            .bind("shoot", Binding::gamepad_button(GamepadButton::South))
            .bind("restart", Binding::gamepad_button(GamepadButton::Start));
        bindings
            .validate()
            .map_err(|error| GameError::invalid(error.to_string()))?;
        Ok(bindings)
    }
    pub fn input(self, input: &impl InputRead) -> crate::world::GameInput {
        crate::world::GameInput {
            forward: input.action("forward").value,
            strafe: input.action("strafe").value,
            turn: input.action("turn").value,
            look: 0.,
            shoot: input.action("shoot").held(),
            restart: input.action("restart").pressed,
        }
    }
    /// Adds captured mouse yaw to the keyboard/gamepad action state.
    pub fn input_with_mouse(
        self,
        input: &impl InputRead,
        sensitivity: f32,
    ) -> crate::world::GameInput {
        let mut result = self.input(input);
        // Screen-right mouse motion decreases the yaw used by forward=(sin,cos).
        result.look = -(input.relative_motion().0 as f32) * sensitivity;
        result
    }
    pub fn quit_requested(self, input: &impl InputRead) -> bool {
        input.action("quit").pressed
    }
}

#[derive(Debug, Deserialize)]
struct FileSchema {
    version: u64,
    window: WindowSchema,
    game: GameSchema,
    audio: AudioSchema,
    logging: LoggingSchema,
    profile: ProfileSchema,
    assets: AssetsSchema,
}
#[derive(Debug, Deserialize)]
struct WindowSchema {
    title: String,
    width: u64,
    height: u64,
    fullscreen: bool,
}
#[derive(Debug, Deserialize)]
struct GameSchema {
    seed: u64,
    time_limit: f32,
    player_speed: f32,
    turn_speed: f32,
    camera_distance: f32,
    camera_height: f32,
    mouse_sensitivity: f32,
}
#[derive(Debug, Deserialize)]
struct AudioSchema {
    enabled: bool,
    volume: f32,
    device: String,
}
#[derive(Debug, Deserialize)]
struct LoggingSchema {
    level: String,
}
#[derive(Debug, Deserialize)]
struct ProfileSchema {
    capacity: u64,
}
#[derive(Debug, Deserialize)]
struct AssetsSchema {
    level: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameSettings {
    pub window_title: String,
    pub window_width: u32,
    pub window_height: u32,
    pub fullscreen: bool,
    pub headless: bool,
    pub seed: u64,
    pub time_limit: f32,
    pub player_speed: f32,
    pub turn_speed: f32,
    pub camera_distance: f32,
    pub camera_height: f32,
    pub mouse_sensitivity: f32,
    pub audio_enabled: bool,
    pub audio_volume: f32,
    pub audio_device: Option<String>,
    pub log_level: LogLevel,
    pub profile_capacity: usize,
    pub benchmark: bool,
    pub benchmark_file: Option<PathBuf>,
    pub benchmark_frames: u64,
    pub ticks: u64,
    pub autoplay: bool,
    pub record: Option<PathBuf>,
    pub replay: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub level_path: String,
    pub assets_root: PathBuf,
    pub controls: ControlKeys,
    pub config_path: Option<PathBuf>,
}
impl GameSettings {
    pub fn input(&self, input: &impl InputRead) -> crate::world::GameInput {
        self.controls
            .input_with_mouse(input, self.mouse_sensitivity)
    }
    pub fn rules(&self) -> GameRules {
        GameRules {
            hz: 60.0,
            time_limit: self.time_limit,
            player_speed: self.player_speed,
            turn_speed: self.turn_speed,
            camera_distance: self.camera_distance,
            camera_height: self.camera_height,
        }
    }
    pub fn simulation_config(&self) -> SimulationConfig {
        SimulationConfig::new(GAME_VERSION, self.seed, TIMELINE)
    }
    /// Defaults, file, environment, common CLI, game CLI, explicit `--set`.
    pub fn load(common: &CommonArgs, cli: &GameCli) -> Result<Self, GameError> {
        if cli.record.is_some() && cli.replay.is_some() {
            return Err(GameError::invalid(
                "--record and --replay are mutually exclusive",
            ));
        }
        let mut config = Config::new();
        set_defaults(&mut config).map_err(config_error)?;
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let default_toml = manifest.join("config.toml");
        let default_json = manifest.join("config.json");
        let explicit_config = common.config.clone();
        let config_path = explicit_config.clone().unwrap_or_else(|| {
            if default_toml.exists() {
                default_toml
            } else {
                default_json
            }
        });
        if explicit_config.is_some() && !config_path.exists() {
            return Err(GameError::invalid(format!(
                "configuration file does not exist: {}",
                config_path.display()
            )));
        }
        if config_path.exists() {
            config
                .load_file(&config_path)
                .map_err(|error| GameError::invalid(format!("configuration file: {error}")))?;
        }
        let version = config.schema_version().map_err(config_error)?;
        if version > GAME_VERSION {
            return Err(GameError::invalid(format!(
                "configuration version {version} is newer than supported {GAME_VERSION}"
            )));
        }
        config
            .apply_env_with(ENV_MAPPINGS, |variable| std::env::var(variable).ok())
            .map_err(config_error)?;
        if let Some(resolution) = common.resolution {
            config
                .set_cli("window.width", resolution.width)
                .map_err(config_error)?;
            config
                .set_cli("window.height", resolution.height)
                .map_err(config_error)?;
        }
        if let Some(fullscreen) = common.fullscreen {
            config
                .set_cli("window.fullscreen", fullscreen)
                .map_err(config_error)?;
        }
        if common.no_audio == Some(true) {
            config
                .set_cli("audio.enabled", false)
                .map_err(config_error)?;
        }
        if let Some(device) = &common.audio_device {
            config
                .set_cli("audio.device", device.clone())
                .map_err(config_error)?;
        }
        if let Some(level) = &common.log_level {
            config
                .set_cli("logging.level", level.clone())
                .map_err(config_error)?;
        }
        if let Some(seed) = cli.seed {
            config.set_cli("game.seed", seed).map_err(config_error)?;
        }
        if let Some(level) = &cli.level {
            config
                .set_cli("assets.level", level.clone())
                .map_err(config_error)?;
        }
        common.apply_overrides(&mut config).map_err(config_error)?;

        let settings = Self::from_config(&config, common, cli, config_path.clone())?;
        if settings.benchmark && settings.headless {
            return Err(GameError::invalid(
                "graphics benchmarks require the native window; omit --headless",
            ));
        }
        Ok(settings)
    }
    fn from_config(
        config: &Config,
        common: &CommonArgs,
        cli: &GameCli,
        config_path: PathBuf,
    ) -> Result<Self, GameError> {
        // Config owns precedence/provenance; Serde turns the resolved scalar map
        // into a typed file schema.
        let file: FileSchema = config.deserialize().map_err(config_error)?;
        let _ = file.version;
        if !(320..=16_384).contains(&file.window.width)
            || !(240..=16_384).contains(&file.window.height)
        {
            return Err(GameError::invalid(
                "window dimensions must be 320..=16384 by 240..=16384",
            ));
        }
        if !(10.0..=600.0).contains(&file.game.time_limit) {
            return Err(GameError::invalid("game.time_limit must be 10..=600"));
        }
        if !(0.5..=20.0).contains(&file.game.player_speed) {
            return Err(GameError::invalid("game.player_speed must be 0.5..=20"));
        }
        if !(0.1..=10.0).contains(&file.game.turn_speed) {
            return Err(GameError::invalid("game.turn_speed must be 0.1..=10"));
        }
        if !(2.0..=20.0).contains(&file.game.camera_distance) {
            return Err(GameError::invalid("game.camera_distance must be 2..=20"));
        }
        if !(0.5..=20.0).contains(&file.game.camera_height) {
            return Err(GameError::invalid("game.camera_height must be 0.5..=20"));
        }
        if !(0.0001..=0.05).contains(&file.game.mouse_sensitivity) {
            return Err(GameError::invalid(
                "game.mouse_sensitivity must be 0.0001..=0.05",
            ));
        }
        if !(0.0..=4.0).contains(&file.audio.volume) {
            return Err(GameError::invalid("audio.volume must be 0..=4"));
        }
        if !(1..=100_000).contains(&file.profile.capacity) {
            return Err(GameError::invalid("profile.capacity must be 1..=100000"));
        }
        if file.window.title.trim().is_empty() {
            return Err(GameError::invalid("window.title must not be empty"));
        }
        if file.logging.level.trim().is_empty() {
            return Err(GameError::invalid("logging.level must not be empty"));
        }
        if file.assets.level.trim().is_empty() {
            return Err(GameError::invalid("assets.level must not be empty"));
        }
        let log_level = parse_log_level(&file.logging.level)?;
        let controls = ControlKeys::from_config(config)?;
        let audio_device = file.audio.device.trim().to_owned();
        let benchmark = common.benchmark.unwrap_or(false);
        let benchmark_file = common
            .benchmark_file
            .clone()
            .or_else(|| benchmark.then(|| PathBuf::from("starfall-profile.json")));
        let autoplay = cli.autoplay || common.headless.unwrap_or(false);
        Ok(Self {
            window_title: file.window.title,
            window_width: file.window.width as u32,
            window_height: file.window.height as u32,
            fullscreen: file.window.fullscreen,
            headless: common.headless.unwrap_or(false),
            seed: file.game.seed,
            time_limit: file.game.time_limit,
            player_speed: file.game.player_speed,
            turn_speed: file.game.turn_speed,
            camera_distance: file.game.camera_distance,
            camera_height: file.game.camera_height,
            mouse_sensitivity: file.game.mouse_sensitivity,
            audio_enabled: file.audio.enabled,
            audio_volume: file.audio.volume,
            audio_device: (!audio_device.is_empty()).then_some(audio_device),
            log_level,
            profile_capacity: file.profile.capacity as usize,
            benchmark,
            benchmark_file,
            benchmark_frames: cli.benchmark_frames,
            ticks: cli.ticks,
            autoplay,
            record: cli.record.clone(),
            replay: cli.replay.clone(),
            screenshot: cli.screenshot.clone(),
            level_path: file.assets.level,
            assets_root: Path::new(env!("CARGO_MANIFEST_DIR")).join("assets"),
            controls,
            config_path: Some(config_path),
        })
    }
}

pub fn set_defaults(config: &mut Config) -> Result<(), ConfigError> {
    let controls = ControlKeys::default();
    let defaults = [
        ("version", ConfigValue::from(GAME_VERSION)),
        ("window.title", ConfigValue::from("Starfall")),
        ("window.width", ConfigValue::from(1280_u32)),
        ("window.height", ConfigValue::from(720_u32)),
        ("window.fullscreen", ConfigValue::from(false)),
        ("game.seed", ConfigValue::from(0x5EED_u64)),
        ("game.time_limit", ConfigValue::from(90.0_f64)),
        ("game.player_speed", ConfigValue::from(5.0_f64)),
        ("game.turn_speed", ConfigValue::from(1.8_f64)),
        ("game.camera_distance", ConfigValue::from(9.0_f64)),
        ("game.camera_height", ConfigValue::from(5.2_f64)),
        ("game.mouse_sensitivity", ConfigValue::from(0.0026_f64)),
        ("audio.enabled", ConfigValue::from(true)),
        ("audio.volume", ConfigValue::from(0.6_f64)),
        ("audio.device", ConfigValue::from("")),
        ("logging.level", ConfigValue::from("info")),
        ("profile.capacity", ConfigValue::from(240_u32)),
        ("assets.level", ConfigValue::from("level.json")),
        (
            "controls.forward",
            ConfigValue::from(controls.forward.name()),
        ),
        ("controls.back", ConfigValue::from(controls.back.name())),
        ("controls.left", ConfigValue::from(controls.left.name())),
        ("controls.right", ConfigValue::from(controls.right.name())),
        (
            "controls.turn_left",
            ConfigValue::from(controls.turn_left.name()),
        ),
        (
            "controls.turn_right",
            ConfigValue::from(controls.turn_right.name()),
        ),
        ("controls.shoot", ConfigValue::from(controls.shoot.name())),
        (
            "controls.restart",
            ConfigValue::from(controls.restart.name()),
        ),
        ("controls.quit", ConfigValue::from(controls.quit.name())),
    ];
    for (key, value) in defaults {
        config.set_default(key, value)?;
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameCli {
    pub seed: Option<u64>,
    pub ticks: u64,
    pub autoplay: bool,
    pub level: Option<String>,
    pub record: Option<PathBuf>,
    pub replay: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub benchmark_frames: u64,
}
impl GameCli {
    pub fn parse(args: &[String]) -> Result<Self, GameError> {
        let mut cli = Self {
            ticks: 600,
            benchmark_frames: 240,
            ..Self::default()
        };
        let mut args = args.iter();
        while let Some(argument) = args.next() {
            let mut value = |name: &str| {
                args.next()
                    .cloned()
                    .ok_or_else(|| GameError::invalid(format!("{name} requires a value")))
            };
            match argument.as_str() {
                "--seed" => {
                    cli.seed = Some(
                        value("--seed")?
                            .parse()
                            .map_err(|_| GameError::invalid("--seed must be an integer"))?,
                    )
                }
                "--ticks" => {
                    let parsed: u64 = value("--ticks")?
                        .parse()
                        .map_err(|_| GameError::invalid("--ticks must be a positive integer"))?;
                    if parsed == 0 {
                        return Err(GameError::invalid("--ticks must be positive"));
                    }
                    cli.ticks = parsed;
                }
                "--autoplay" => cli.autoplay = true,
                "--level" => cli.level = Some(value("--level")?),
                "--record" => cli.record = Some(value("--record")?.into()),
                "--replay" => cli.replay = Some(value("--replay")?.into()),
                "--screenshot" => cli.screenshot = Some(value("--screenshot")?.into()),
                "--benchmark-frames" => {
                    let parsed: u64 = value("--benchmark-frames")?
                        .parse()
                        .map_err(|_| GameError::invalid("--benchmark-frames must be positive"))?;
                    if parsed == 0 {
                        return Err(GameError::invalid("--benchmark-frames must be positive"));
                    }
                    cli.benchmark_frames = parsed;
                }
                _ => {
                    return Err(GameError::invalid(format!(
                        "unknown game argument: {argument}"
                    )));
                }
            }
        }
        Ok(cli)
    }
    pub fn help() -> &'static str {
        "Game options:\n  --seed N              Deterministic simulation seed\n  --ticks N             Headless tick count\n  --autoplay            Use the deterministic bot\n  --level PATH          Virtual level asset path (default level.json)\n  --record FILE         Write a deterministic JSON journal\n  --replay FILE         Replay a journal and verify every hash\n  --screenshot FILE     Render a headless frame after simulation\n  --benchmark-frames N  Native benchmark sample count (default 240)"
    }
}

fn parse_log_level(value: &str) -> Result<LogLevel, GameError> {
    match value.to_ascii_lowercase().as_str() {
        "trace" => Ok(LogLevel::Trace),
        "debug" => Ok(LogLevel::Debug),
        "info" => Ok(LogLevel::Info),
        "warn" | "warning" => Ok(LogLevel::Warn),
        "error" => Ok(LogLevel::Error),
        _ => Err(GameError::invalid(
            "logging.level must be trace, debug, info, warn, or error",
        )),
    }
}
fn config_error(error: ConfigError) -> GameError {
    GameError::invalid(error.to_string())
}
pub const ENV_MAPPINGS: &[(&str, &str)] = &[
    ("window.title", "STARFALL_TITLE"),
    ("window.width", "STARFALL_WIDTH"),
    ("window.height", "STARFALL_HEIGHT"),
    ("window.fullscreen", "STARFALL_FULLSCREEN"),
    ("game.seed", "STARFALL_SEED"),
    ("game.time_limit", "STARFALL_TIME_LIMIT"),
    ("game.player_speed", "STARFALL_PLAYER_SPEED"),
    ("game.turn_speed", "STARFALL_TURN_SPEED"),
    ("game.camera_distance", "STARFALL_CAMERA_DISTANCE"),
    ("game.camera_height", "STARFALL_CAMERA_HEIGHT"),
    ("game.mouse_sensitivity", "STARFALL_MOUSE_SENSITIVITY"),
    ("audio.enabled", "STARFALL_AUDIO"),
    ("audio.volume", "STARFALL_VOLUME"),
    ("audio.device", "STARFALL_AUDIO_DEVICE"),
    ("logging.level", "STARFALL_LOG_LEVEL"),
    ("profile.capacity", "STARFALL_PROFILE_CAPACITY"),
    ("assets.level", "STARFALL_LEVEL"),
    ("controls.forward", "STARFALL_FORWARD"),
    ("controls.back", "STARFALL_BACK"),
    ("controls.left", "STARFALL_LEFT"),
    ("controls.right", "STARFALL_RIGHT"),
    ("controls.turn_left", "STARFALL_TURN_LEFT"),
    ("controls.turn_right", "STARFALL_TURN_RIGHT"),
    ("controls.shoot", "STARFALL_SHOOT"),
    ("controls.restart", "STARFALL_RESTART"),
    ("controls.quit", "STARFALL_QUIT"),
];
