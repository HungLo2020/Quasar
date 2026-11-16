use voxel_drift::settings::Settings;
use voxel_drift::{DriftError, diagnostics};

#[cfg(feature = "desktop")]
mod desktop;

fn main() {
    if let Err(error) = run() {
        eprintln!("voxel-drift: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), DriftError> {
    let settings = Settings::parse(std::env::args().skip(1))?;
    let diagnostics = diagnostics(&settings);
    if settings.headless {
        return voxel_drift::run_headless(settings, diagnostics);
    }
    #[cfg(feature = "desktop")]
    {
        desktop::run(settings, diagnostics)
    }
    #[cfg(not(feature = "desktop"))]
    {
        Err(DriftError::invalid(
            "this build has no desktop feature; use --headless",
        ))
    }
}
