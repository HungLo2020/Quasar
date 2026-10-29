use crate::Result;
use quasar_config::CommonArgs;
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
    pub benchmark_file: Option<PathBuf>,
    pub benchmark: bool,
    pub fullscreen: bool,
    pub audio: bool,
    pub audio_device: Option<String>,
    pub shadows: bool,
    pub normal_maps: bool,
    pub shadow_resolution: u32,
}
impl Settings {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>> {
        let common = CommonArgs::parse_from(args)?;
        if common.help {
            println!(
                "Nightwatch - Quasar lighting FPS\n\nWASD move, mouse look, LMB fire, 1/2 weapons, R reload, SPACE jump, SHIFT sprint, F flashlight, H shadows, N normal maps, ESC pause, ENTER restart, F11 fullscreen.\n\n--headless --ticks N --autoplay --seed N --resolution WxH --frames N --screenshot PATH --benchmark --benchmark-file PATH --no-shadows --no-normal-maps --shadow-resolution N --fullscreen --no-audio --audio-device NAME --version"
            );
            return Ok(None);
        }
        if common.version {
            println!("nightwatch {}", env!("CARGO_PKG_VERSION"));
            return Ok(None);
        }
        if common.config.is_some() || !common.overrides.is_empty() {
            return Err(
                "Nightwatch has no persisted settings schema; --config/--set are unsupported"
                    .into(),
            );
        }
        let (width, height) = common
            .resolution
            .map_or((1280, 720), |r| (r.width, r.height));
        let mut s = Self {
            headless: common.is_headless(),
            autoplay: false,
            ticks: 600,
            seed: 42,
            width,
            height,
            frames: None,
            screenshot: None,
            benchmark: common.benchmark.unwrap_or(common.benchmark_file.is_some()),
            benchmark_file: common.benchmark_file,
            fullscreen: common.fullscreen.unwrap_or(false),
            audio: !common.no_audio.unwrap_or(false),
            audio_device: common.audio_device,
            shadows: true,
            normal_maps: true,
            shadow_resolution: 1024,
        };
        let mut args = common.remaining.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--ticks" => s.ticks = args.next().ok_or("missing ticks")?.parse()?,
                "--seed" => s.seed = args.next().ok_or("missing seed")?.parse()?,
                "--frames" => s.frames = Some(args.next().ok_or("missing frames")?.parse()?),
                "--autoplay" => s.autoplay = true,
                "--no-shadows" => s.shadows = false,
                "--no-normal-maps" => s.normal_maps = false,
                "--shadow-resolution" => {
                    s.shadow_resolution = args.next().ok_or("missing shadow resolution")?.parse()?
                }
                "--screenshot" => {
                    s.screenshot = Some(args.next().ok_or("missing screenshot path")?.into())
                }
                _ => return Err(format!("unknown argument {arg}").into()),
            }
        }
        if !(320..=7680).contains(&s.width)
            || !(180..=4320).contains(&s.height)
            || s.ticks == 0
            || s.ticks > 10_000_000
            || s.frames == Some(0)
            || !(32..=2048).contains(&s.shadow_resolution)
        {
            return Err("dimensions/tick/frame budget out of range".into());
        }
        if s.headless && (s.benchmark || s.benchmark_file.is_some()) {
            return Err("GPU benchmarks require native mode; headless supports simulation and optional screenshots".into());
        }
        if s.benchmark && s.frames.is_none() {
            s.frames = Some(240);
        }
        Ok(Some(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effect_options_and_shadow_memory_bounds() {
        let parse = |args: &[&str]| Settings::parse(args.iter().map(|s| s.to_string()));
        let defaults = parse(&[]).unwrap().unwrap();
        assert!(defaults.shadows && defaults.normal_maps);
        assert_eq!(defaults.shadow_resolution, 1024);
        let off = parse(&[
            "--no-shadows",
            "--no-normal-maps",
            "--shadow-resolution",
            "2048",
        ])
        .unwrap()
        .unwrap();
        assert!(!off.shadows && !off.normal_maps);
        assert_eq!(off.shadow_resolution, 2048);
        for args in [
            vec!["--shadow-resolution", "0"],
            vec!["--shadow-resolution", "4096"],
            vec!["--shadow-resolution"],
            vec!["--shadow-resolution", "nan"],
        ] {
            assert!(parse(&args).is_err());
        }
    }
}
