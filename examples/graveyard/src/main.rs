//! `graveyard`: walk a foggy graveyard at night.
//!
//! ```sh
//! cargo run -p graveyard
//! cargo run -p graveyard -- --screenshot /tmp/graveyard.png
//! ```
#[cfg(feature = "desktop")]
mod desktop;

const HELP: &str = "graveyard: a walkable foggy graveyard

USAGE:
    graveyard [OPTIONS]

OPTIONS:
    --screenshot PATH   Render the scripted walk headlessly to a PNG and exit
    --ticks N           Ticks of the scripted walk before the screenshot (default 150)
    --no-flashlight     Screenshot with the flashlight off
    --width N           Window or screenshot width (default 1280)
    --height N          Window or screenshot height (default 720)
    -h, --help          Show this help

CONTROLS:
    WASD move, mouse look, Shift run, F flashlight, F11 fullscreen, Esc quit
";

fn main() -> std::process::ExitCode {
    let mut screenshot = None;
    let (mut width, mut height, mut ticks) = (1280u32, 720u32, 150u32);
    let mut flashlight = true;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut number = |name: &str| -> u32 {
            args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| {
                eprintln!("{name} needs a number");
                std::process::exit(2)
            })
        };
        match arg.as_str() {
            "--screenshot" => screenshot = args.next().map(std::path::PathBuf::from),
            "--ticks" => ticks = number("--ticks"),
            "--no-flashlight" => flashlight = false,
            "--width" => width = number("--width"),
            "--height" => height = number("--height"),
            "-h" | "--help" => {
                print!("{HELP}");
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument '{other}'; use --help");
                return std::process::ExitCode::from(2);
            }
        }
    }
    let result = match screenshot {
        Some(path) => graveyard::screenshot(&path, width, height, ticks, flashlight).map(|()| {
            println!("screenshot={}", path.display());
        }),
        #[cfg(feature = "desktop")]
        None => desktop::run(width, height),
        #[cfg(not(feature = "desktop"))]
        None => Err(graveyard::GraveError::new(
            "built without the desktop feature",
        )),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("graveyard: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
