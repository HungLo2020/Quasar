use nightwatch::{
    Result,
    settings::Settings,
    world::{Input, World, autoplay},
};
#[cfg(feature = "desktop")]
mod desktop;
#[cfg(feature = "desktop")]
mod sound;
fn main() {
    if let Err(error) = run() {
        eprintln!("nightwatch: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let Some(settings) = Settings::parse(std::env::args().skip(1))? else {
        return Ok(());
    };
    if settings.headless {
        let mut world = World::new(settings.seed);
        for _ in 0..settings.ticks {
            let input = if settings.autoplay {
                autoplay(&world)
            } else {
                Input::default()
            };
            world.update(input);
        }
        println!(
            "nightwatch ticks={} wave={} kills={} shots={} health={} hash={:016x}",
            world.tick,
            world.wave,
            world.kills,
            world.shots_fired,
            world.health,
            world.state_hash()
        );
        if let Some(path) = settings.screenshot {
            #[cfg(feature = "graphics")]
            nightwatch::render::screenshot(&world, settings.width, settings.height, &path)?;
            #[cfg(not(feature = "graphics"))]
            {
                let _ = path;
                return Err("screenshots require --features graphics".into());
            }
        }
        return Ok(());
    }
    #[cfg(feature = "desktop")]
    {
        desktop::run(settings)
    }
    #[cfg(not(feature = "desktop"))]
    {
        Err("desktop disabled; use --headless".into())
    }
}
