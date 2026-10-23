use crate::Result;
use std::path::PathBuf;
#[derive(Default)]
pub struct Options {
    pub headless: bool,
    pub bindings: Option<PathBuf>,
    pub write_default_bindings: Option<PathBuf>,
    pub check_bindings: Option<PathBuf>,
    pub ticks: Option<usize>,
    pub autoplay: bool,
    pub two_player: bool,
    pub replay: Option<PathBuf>,
    pub record: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub state: Option<PathBuf>,
    pub fullscreen: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub linear: bool,
    pub integer: bool,
    pub no_save: bool,
    pub smoke_frames: Option<u64>,
    pub exercise_window: bool,
}
impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>> {
        let mut args = args.into_iter();
        let mut options = Self::default();
        while let Some(arg) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{arg} requires a value"));
            match arg.as_str() {
                "--help" | "-h" => {
                    println!("{}", include_str!("help.txt"));
                    return Ok(None);
                }
                "--headless" => options.headless = true,
                "--bindings" => options.bindings = Some(value()?.into()),
                "--write-default-bindings" => {
                    options.write_default_bindings = Some(value()?.into())
                }
                "--check-bindings" => options.check_bindings = Some(value()?.into()),
                "--autoplay" => options.autoplay = true,
                "--two-player" => options.two_player = true,
                "--fullscreen" => options.fullscreen = true,
                "--linear" => options.linear = true,
                "--integer-scale" => options.integer = true,
                "--no-save" => options.no_save = true,
                "--exercise-window" => options.exercise_window = true,
                "--ticks" => options.ticks = Some(value()?.parse()?),
                "--width" => options.width = Some(value()?.parse()?),
                "--height" => options.height = Some(value()?.parse()?),
                "--smoke-frames" => options.smoke_frames = Some(value()?.parse()?),
                "--replay" => options.replay = Some(value()?.into()),
                "--record" => options.record = Some(value()?.into()),
                "--screenshot" => options.screenshot = Some(value()?.into()),
                "--state" => options.state = Some(value()?.into()),
                _ => return Err(format!("unknown argument: {arg}; use --help").into()),
            }
        }
        if options.replay.is_some() && (options.autoplay || options.record.is_some()) {
            return Err("--replay cannot be combined with --autoplay or --record".into());
        }
        if options.ticks == Some(0) || options.smoke_frames == Some(0) {
            return Err("run limits must be greater than zero".into());
        }
        if options.width.is_some_and(|n| !(320..=16384).contains(&n))
            || options.height.is_some_and(|n| !(200..=16384).contains(&n))
        {
            return Err("window dimensions must be 320..16384 wide and 200..16384 high".into());
        }
        if options.headless && (options.smoke_frames.is_some() || options.exercise_window) {
            return Err("window smoke options require a desktop run".into());
        }
        if options.exercise_window && options.smoke_frames.is_none_or(|n| n < 20) {
            return Err("--exercise-window requires --smoke-frames of at least 20".into());
        }
        Ok(Some(options))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_or_invalid_runs() {
        for args in [
            vec!["--ticks", "0"],
            vec!["--replay", "a.json", "--autoplay"],
            vec!["--width", "0"],
            vec!["--exercise-window"],
            vec!["--unknown"],
        ] {
            assert!(Options::parse(args.into_iter().map(str::to_owned)).is_err());
        }
    }
}
