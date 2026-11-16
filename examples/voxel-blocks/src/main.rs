use voxel_blocks::settings::Settings;
use voxel_blocks::{BlocksError, diagnostics};

#[cfg(feature = "desktop")]
mod desktop;

fn main() {
    if let Err(error) = run() {
        eprintln!("voxel-blocks: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), BlocksError> {
    let settings = Settings::parse(std::env::args().skip(1))?;
    let diagnostics = diagnostics(&settings);
    if settings.headless {
        return voxel_blocks::run_headless(settings, diagnostics);
    }
    #[cfg(feature = "desktop")]
    {
        desktop::run(settings, diagnostics)
    }
    #[cfg(not(feature = "desktop"))]
    {
        Err(BlocksError::invalid(
            "this build has no desktop feature; use --headless",
        ))
    }
}
