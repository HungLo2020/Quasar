use crate::Result;
use quasar_config::{CommonArgs, Config, ConfigValue};
use quasar_profile::BenchmarkConfig;
use std::path::PathBuf;
#[derive(Clone, Debug)]
pub struct Settings {
    pub headless: bool,
    pub autoplay: bool,
    pub ticks: u64,
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub frames: Option<u64>,
    pub screenshot: Option<PathBuf>,
    pub asset_root: Option<PathBuf>,
    pub benchmark_file: Option<PathBuf>,
    pub benchmark: bool,
    pub fullscreen: bool,
    pub audio: bool,
    pub audio_device: Option<String>,
    pub shadows: bool,
    pub normal_maps: bool,
    pub shadow_resolution: u32,
    pub instancing: bool,
    pub frustum_culling: bool,
    pub max_instances: usize,
    pub validation: Option<bool>,
    pub log_level: Option<quasar_diagnostics::Level>,
    pub benchmark_config: BenchmarkConfig,
}
impl Settings {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>> {
        Self::parse_with_env(args, |key| std::env::var(key).ok())
    }
    pub fn parse_with_env(
        args: impl IntoIterator<Item = String>,
        mut env: impl FnMut(&str) -> Option<String>,
    ) -> Result<Option<Self>> {
        let common = CommonArgs::parse_from(args)?;
        if common.help {
            println!("{}",common.help("Nightwatch - Quasar lighting FPS","WASD move, mouse look, LMB fire, 1/2 weapons, R reload, SPACE jump, SHIFT sprint, F flashlight, H shadows, N normal maps, F5 reload models, ESC pause.\n--asset-root PATH --ticks N --autoplay --seed N --frames N --screenshot PATH --no-shadows --no-normal-maps --shadow-resolution N --no-instancing --no-culling --validation --no-validation\nConfig files: TOML/JSON; renderer settings use render.*, benchmark settings use benchmark.*; --set overrides files/environment."));
            return Ok(None);
        }
        if common.version {
            println!("nightwatch {}", env!("CARGO_PKG_VERSION"));
            return Ok(None);
        }
        let mut config = Config::new();
        #[cfg(feature = "graphics")]
        quasar_render_3d::SubmissionConfig::register_defaults(&mut config, "render")?;
        for (key, value) in [
            ("common.headless", false.into()),
            ("common.fullscreen", false.into()),
            ("common.resolution", "1280x720".into()),
            ("common.no_audio", false.into()),
            ("common.audio_device", "".into()),
            ("common.benchmark", false.into()),
            ("common.benchmark_file", "".into()),
            ("common.log_level", "info".into()),
            ("game.autoplay", false.into()),
            ("game.ticks", 600u64.into()),
            ("game.seed", 42u64.into()),
            ("game.frames", 0u64.into()),
            ("game.screenshot", "".into()),
            ("assets.root", "".into()),
            ("render.shadows", true.into()),
            ("render.normal_maps", true.into()),
            ("render.shadow_resolution", 1024u64.into()),
            ("render.instancing", true.into()),
            ("render.frustum_culling", true.into()),
            ("render.max_instances", 4096u64.into()),
            ("render.validation", "auto".into()),
        ] {
            let value: ConfigValue = value;
            config.set_default(key, value)?;
        }
        BenchmarkConfig::register_defaults(&mut config, "benchmark")?;
        let known: std::collections::BTreeSet<_> = config.keys().map(str::to_owned).collect();
        if let Some(path) = common
            .config
            .clone()
            .or_else(|| env("NIGHTWATCH_CONFIG").map(PathBuf::from))
        {
            config.load_file_if_exists(path)?;
        }
        config.apply_env_with(
            &[
                ("common.headless", "NIGHTWATCH_HEADLESS"),
                ("common.fullscreen", "NIGHTWATCH_FULLSCREEN"),
                ("common.resolution", "NIGHTWATCH_RESOLUTION"),
                ("common.no_audio", "NIGHTWATCH_NO_AUDIO"),
                ("common.audio_device", "NIGHTWATCH_AUDIO_DEVICE"),
                ("common.benchmark", "NIGHTWATCH_BENCHMARK"),
                ("common.benchmark_file", "NIGHTWATCH_BENCHMARK_FILE"),
                ("common.log_level", "NIGHTWATCH_LOG_LEVEL"),
                ("render.instancing", "NIGHTWATCH_INSTANCING"),
                ("assets.root", "NIGHTWATCH_ASSET_ROOT"),
                ("render.frustum_culling", "NIGHTWATCH_FRUSTUM_CULLING"),
                ("render.validation", "NIGHTWATCH_VALIDATION"),
            ],
            &mut env,
        )?;
        common.apply_to_config(&mut config)?;
        let mut args = common.remaining.iter();
        while let Some(arg) = args.next() {
            let (key, value) = match arg.as_str() {
                "--ticks" => (
                    "game.ticks",
                    ConfigValue::parse(args.next().ok_or("missing ticks")?),
                ),
                "--seed" => (
                    "game.seed",
                    ConfigValue::parse(args.next().ok_or("missing seed")?),
                ),
                "--frames" => (
                    "game.frames",
                    ConfigValue::parse(args.next().ok_or("missing frames")?),
                ),
                "--screenshot" => (
                    "game.screenshot",
                    args.next().ok_or("missing screenshot path")?.clone().into(),
                ),
                "--asset-root" => (
                    "assets.root",
                    args.next().ok_or("missing asset root")?.clone().into(),
                ),
                "--autoplay" => ("game.autoplay", true.into()),
                "--no-shadows" => ("render.shadows", false.into()),
                "--no-normal-maps" => ("render.normal_maps", false.into()),
                "--shadow-resolution" => (
                    "render.shadow_resolution",
                    ConfigValue::parse(args.next().ok_or("missing shadow resolution")?),
                ),
                "--no-instancing" => ("render.instancing", false.into()),
                "--no-culling" => ("render.frustum_culling", false.into()),
                "--validation" => ("render.validation", true.into()),
                "--no-validation" => ("render.validation", false.into()),
                _ => return Err(format!("unknown argument {arg}").into()),
            };
            config.set_cli(key, value)?;
        }
        for (key, _) in &common.overrides {
            if !known.contains(key) {
                return Err(format!("unknown configuration override {key}").into());
            }
        }
        common.apply_overrides(&mut config)?;
        let optional = |key: &str| config.get_string(key).map(|s| (!s.is_empty()).then_some(s));
        #[cfg(feature = "graphics")]
        let _validated_submission =
            quasar_render_3d::SubmissionConfig::from_config(&config, "render")?;
        let resolution = config.get_resolution("common.resolution")?;
        let log = config.get_string("common.log_level")?;
        let mut s = Self {
            headless: config.get_bool("common.headless")?,
            fullscreen: config.get_bool("common.fullscreen")?,
            width: resolution.width,
            height: resolution.height,
            audio: !config.get_bool("common.no_audio")?,
            audio_device: optional("common.audio_device")?,
            autoplay: config.get_bool("game.autoplay")?,
            ticks: config.get_u64("game.ticks")?,
            seed: config.get_u64("game.seed")?,
            frames: match config.get_u64("game.frames")? {
                0 => None,
                n => Some(n),
            },
            screenshot: optional("game.screenshot")?.map(PathBuf::from),
            asset_root: optional("assets.root")?.map(PathBuf::from),
            benchmark_file: optional("common.benchmark_file")?.map(PathBuf::from),
            benchmark: config.get_bool("common.benchmark")?,
            shadows: config.get_bool("render.shadows")?,
            normal_maps: config.get_bool("render.normal_maps")?,
            shadow_resolution: u32::try_from(config.get_u64("render.shadow_resolution")?)?,
            instancing: config.get_bool("render.instancing")?,
            frustum_culling: config.get_bool("render.frustum_culling")?,
            max_instances: usize::try_from(config.get_u64("render.max_instances")?)?,
            validation: if matches!(config.get("render.validation"),Some(ConfigValue::String(v)) if v=="auto")
            {
                None
            } else {
                Some(config.get_bool("render.validation")?)
            },
            log_level: if log == "off" {
                None
            } else {
                Some(log.parse()?)
            },
            benchmark_config: BenchmarkConfig::from_config(&config, "benchmark")?,
        };
        if s.frames.is_none() && common.remaining.iter().any(|a| a == "--frames") {
            return Err("explicit frame budget must be positive".into());
        }
        s.benchmark |= s.benchmark_file.is_some();
        if !(320..=7680).contains(&s.width)
            || !(180..=4320).contains(&s.height)
            || s.ticks == 0
            || s.ticks > 10_000_000
            || !(32..=2048).contains(&s.shadow_resolution)
            || !(1..=4096).contains(&s.max_instances)
        {
            return Err("dimensions/tick/instance/shadow budget out of range".into());
        }
        if s.headless && s.benchmark {
            return Err("GPU benchmarks require native mode; headless supports simulation and optional screenshots".into());
        }
        if s.benchmark && s.frames.is_none() {
            s.frames = Some(240);
        }
        if s.benchmark
            && s.frames
                .is_some_and(|n| n <= s.benchmark_config.warmup_frames)
        {
            return Err("benchmark frame budget must exceed warmup".into());
        }
        Ok(Some(s))
    }
    #[cfg(feature = "graphics")]
    pub fn submission(&self) -> quasar_render_3d::SubmissionConfig {
        quasar_render_3d::SubmissionConfig {
            instancing: self.instancing,
            frustum_culling: self.frustum_culling,
            max_instances: self.max_instances,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Option<Settings>> {
        Settings::parse_with_env(args.iter().map(|s| s.to_string()), |_| None)
    }
    #[test]
    fn defaults_and_renderer_budgets_are_consistent() {
        let s = parse(&[]).unwrap().unwrap();
        assert!(s.instancing && s.frustum_culling && s.shadows && s.normal_maps);
        for args in [
            vec!["--frames", "0"],
            vec!["--shadow-resolution", "4096"],
            vec!["--shadow-resolution", "0"],
            vec!["--shadow-resolution"],
            vec!["--set", "render.max_instances=0"],
            vec!["--set", "render.instancing=garbage"],
            vec!["--set", "render.typo=true"],
            vec!["--log-level", "garbage"],
            vec!["--benchmark", "--frames", "4"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
        let s = parse(&[
            "--no-instancing",
            "--no-culling",
            "--validation",
            "--shadow-resolution",
            "2048",
        ])
        .unwrap()
        .unwrap();
        assert!(!s.instancing && !s.frustum_culling);
        assert_eq!(s.validation, Some(true));
        assert_eq!(s.shadow_resolution, 2048);
    }
    #[test]
    fn files_environment_flags_and_explicit_overrides_use_config_precedence() {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in [
            (
                "game.toml",
                "[render]\ninstancing=false\nshadow_resolution=256\n[benchmark]\nsample_capacity=32\n",
            ),
            (
                "game.json",
                "{\"render\":{\"instancing\":false,\"shadow_resolution\":256},\"benchmark\":{\"sample_capacity\":32}}",
            ),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            let original = std::fs::read(&path).unwrap();
            let s = Settings::parse_with_env(
                [
                    "--config".into(),
                    path.to_string_lossy().into(),
                    "--no-instancing".into(),
                    "--set".into(),
                    "render.instancing=true".into(),
                ],
                |k| (k == "NIGHTWATCH_INSTANCING").then(|| "true".into()),
            )
            .unwrap()
            .unwrap();
            assert!(s.instancing);
            assert_eq!(s.shadow_resolution, 256);
            assert_eq!(s.benchmark_config.sample_capacity, 32);
            assert_eq!(std::fs::read(&path).unwrap(), original);
            let s =
                Settings::parse_with_env(["--config".into(), path.to_string_lossy().into()], |k| {
                    (k == "NIGHTWATCH_INSTANCING").then(|| "true".into())
                })
                .unwrap()
                .unwrap();
            assert!(s.instancing);
            let s = parse(&["--config", path.to_str().unwrap()])
                .unwrap()
                .unwrap();
            assert!(!s.instancing);
        }
    }
}
