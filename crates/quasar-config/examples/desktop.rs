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
                "Game options:\n  --player-name NAME    Pilot name\n  --no-vsync             Disable vsync\n  --save-settings PATH   Export settings (requires storage and toml features)"
            )
        );
        return Ok(());
    }
    let mut player = "pilot".to_owned();
    let mut vsync = true;
    #[cfg(all(feature = "storage", feature = "toml"))]
    let mut save = None;
    let mut args = common.remaining.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--player-name" => player = args.next().cloned().ok_or("missing player name")?,
            "--no-vsync" => vsync = false,
            #[cfg(all(feature = "storage", feature = "toml"))]
            "--save-settings" => save = Some(args.next().cloned().ok_or("missing save path")?),
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
    // Explicit export only: no configuration file is changed during loading.
    #[cfg(all(feature = "storage", feature = "toml"))]
    if let Some(path) = save {
        #[derive(serde::Serialize)]
        struct Settings {
            width: u64,
            height: u64,
            fullscreen: bool,
            vsync: bool,
        }
        quasar_config::save_file_typed(
            path,
            &Settings {
                width: config.get_u64("window.width")?,
                height: config.get_u64("window.height")?,
                fullscreen: config.get_bool("window.fullscreen")?,
                vsync: config.get_bool("game.vsync")?,
            },
        )?;
    }
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
