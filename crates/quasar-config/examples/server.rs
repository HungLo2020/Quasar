//! MattMC-like headless server using the same common arguments and Config.
use quasar_config::{CommonArgs, Config};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut common = CommonArgs::from_env("DEMO_SERVER")?;
    common.parse_into(std::env::args().skip(1))?;
    if common.headless.is_none() {
        common.headless = Some(true);
    }
    if common.help {
        print!(
            "{}",
            common.help(
                "config-server",
                "Server options:\n  --bind ADDR          Listen address\n  --max-players N      Player limit\n  --tick-rate N        Simulation rate"
            )
        );
        return Ok(());
    }
    let mut bind = "127.0.0.1:25565".to_owned();
    let mut max_players = 8_u32;
    let mut tick_rate = 20_u32;
    let mut args = common.remaining.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--bind" => bind = args.next().cloned().ok_or("missing bind address")?,
            "--max-players" => {
                max_players = args.next().ok_or("missing max players")?.parse()?;
            }
            "--tick-rate" => {
                tick_rate = args.next().ok_or("missing tick rate")?.parse()?;
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    let mut config = Config::new();
    config.set_default("server.bind", bind)?;
    config.set_default("server.max_players", max_players)?;
    config.set_default("server.tick_rate", tick_rate)?;
    config.set_default("server.headless", true)?;
    if let Some(path) = &common.config {
        config.load_file(path)?;
    }
    if let Some(log_level) = &common.log_level {
        config.set_cli("server.log_level", log_level.clone())?;
    }
    common.apply_overrides(&mut config)?;
    println!(
        "server bind={} max_players={} tick_rate={} headless={} log_level={}",
        config.get_string("server.bind")?,
        config.get_u64("server.max_players")?,
        config.get_u64("server.tick_rate")?,
        common.is_headless(),
        config.get_or("server.log_level", "info".to_owned())?,
    );
    Ok(())
}
