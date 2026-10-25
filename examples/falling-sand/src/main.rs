use falling_sand::settings::Settings;
use falling_sand::{SandError, diagnostics};

#[cfg(feature = "desktop")]
mod desktop;

fn main() {
    if let Err(error) = run() {
        eprintln!("falling-sand: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), SandError> {
    let settings = Settings::parse(std::env::args().skip(1))?;
    let diagnostics = diagnostics(&settings);
    if settings.headless {
        return falling_sand::run_headless(settings, diagnostics);
    }
    #[cfg(feature = "desktop")]
    {
        desktop::run(settings, diagnostics)
    }
    #[cfg(not(feature = "desktop"))]
    {
        Err(SandError::invalid(
            "this build has no desktop feature; use --headless",
        ))
    }
}
