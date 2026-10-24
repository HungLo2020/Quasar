//! MissileCommand-like desktop application using shared config primitives.
use quasar_config::{CommonArgs, Config};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut common = CommonArgs::from_env("DEMO")?;
    common.parse_into(std::env::args().skip(1))?;
    if common.help {
        print!(
            "{}",
            common.help(
                "config-desktop",
                "Game options:\n  --player-name NAME    Pilot name\n  --no-vsync             Disable vsync"
            )
        );
        return Ok(());
    }
    let mut player = "pilot".to_owned();
    let mut vsync = true;
    let mut args = common.remaining.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--player-name" => player = args.next().cloned().ok_or("missing player name")?,
            "--no-vsync" => vsync = false,
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    let mut config = Config::new();
    config.set_default("window.width", 1024_u32)?;
    config.set_default("window.height", 768_u32)?;
    config.set_default("window.fullscreen", false)?;
    config.set_default("game.vsync", true)?;
    if let Some(path) = &common.config {
        config.load_file(path)?;
    }
    if let Some(resolution) = common.resolution {
        config.set_cli("window.width", resolution.width)?;
        config.set_cli("window.height", resolution.height)?;
    }
    if let Some(fullscreen) = common.fullscreen {
        config.set_cli("window.fullscreen", fullscreen)?;
    }
    if !vsync {
        config.set_cli("game.vsync", false)?;
    }
    common.apply_overrides(&mut config)?;
    println!(
        "desktop player={player} {}x{} fullscreen={} vsync={} headless={}",
        config.get_u64("window.width")?,
        config.get_u64("window.height")?,
        config.get_bool("window.fullscreen")?,
        config.get_bool("game.vsync")?,
        common.is_headless(),
    );
    Ok(())
}
