use physics_playground::settings::Settings;
use physics_playground::{PlaygroundError, diagnostics};

#[cfg(feature = "desktop")]
mod desktop;

fn main() {
    if let Err(error) = run() {
        eprintln!("physics-playground: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), PlaygroundError> {
    let settings = Settings::parse(std::env::args().skip(1))?;
    let diagnostics = diagnostics(&settings);
    if settings.headless {
        return physics_playground::run_headless(settings, diagnostics);
    }
    #[cfg(feature = "desktop")]
    {
        desktop::run(settings, diagnostics)
    }
    #[cfg(not(feature = "desktop"))]
    {
        Err(PlaygroundError::invalid(
            "this build has no desktop feature; use --headless",
        ))
    }
}
