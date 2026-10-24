//! Creates one window with explicit desktop identity and compositor policy.
//!
//! This example owns the ordinary winit event loop; it does not start a Quasar
//! application runner or initialize graphics. Use it to inspect native metadata:
//!
//! ```text
//! cargo run -p quasar-window --example desktop_identity -- \
//!   --app-id missile-command --instance missile-command --class MissileCommand
//! xprop -name "Quasar desktop identity"
//! ```
//!
//! `--transparent`, `--fullscreen`, `--no-x11` and `--hold-ms` exercise the
//! remaining presentation policies.

use quasar_window::{
    DesktopConfig, GameWindow, WindowConfig, X11Identity, desktop_capabilities,
    winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        window::WindowId,
    },
};
use std::{
    env,
    time::{Duration, Instant},
};

struct Options {
    config: WindowConfig,
    desktop: DesktopConfig,
    hold: Duration,
    force_x11: bool,
}

fn create_event_loop(force_x11: bool) -> EventLoop<()> {
    #[cfg(target_os = "linux")]
    {
        use quasar_window::winit::platform::x11::EventLoopBuilderExtX11;
        let mut builder = EventLoop::builder();
        if force_x11 {
            builder.with_x11().with_any_thread(true);
        }
        builder.build().expect("create event loop")
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = force_x11;
        EventLoop::new().expect("create event loop")
    }
}

struct App {
    options: Options,
    window: Option<GameWindow>,
    deadline: Instant,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match GameWindow::create_with_desktop(
            event_loop,
            &self.options.config,
            &self.options.desktop,
        ) {
            Ok(window) => {
                let size = window.native().inner_size();
                println!(
                    "READY id={} width={} height={} pid={}",
                    native_id(window.native()),
                    size.width,
                    size.height,
                    std::process::id()
                );
                self.window = Some(window);
            }
            Err(error) => {
                eprintln!("create window: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = &mut self.window else {
            return;
        };
        if window.native().id() != id {
            return;
        }
        let update = window.process(&event);
        if update.close_requested {
            event_loop.exit();
        }
        if let Some(size) = update.resized {
            println!("RESIZED width={} height={}", size.width, size.height);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.deadline {
            event_loop.exit();
            return;
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            self.deadline.min(now + Duration::from_millis(16)),
        ));
    }
}

fn main() {
    let options = parse_options();
    let capabilities = desktop_capabilities();
    println!(
        "CAPABILITIES x11_properties={} x11_pid={} x11_opacity={} x11_opaque_region={} wayland_application_id={} windows_class_name={}",
        capabilities.x11_properties,
        capabilities.x11_pid,
        capabilities.x11_opacity,
        capabilities.x11_opaque_region,
        capabilities.wayland_application_id,
        capabilities.windows_class_name,
    );
    let event_loop = create_event_loop(options.force_x11);
    let mut app = App {
        deadline: Instant::now() + options.hold,
        options,
        window: None,
    };
    event_loop.run_app(&mut app).expect("run event loop");
}

fn parse_options() -> Options {
    let mut title = "Quasar desktop identity".to_string();
    let mut app_id = "quasar-desktop-demo".to_string();
    let mut instance = None;
    let mut class = None;
    let mut transparent = false;
    let mut fullscreen = false;
    let mut x11 = true;
    let mut force_x11 = false;
    let mut hold_ms = 15_000;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--title" => title = next_value(&mut arguments, "--title"),
            "--app-id" => app_id = next_value(&mut arguments, "--app-id"),
            "--instance" => instance = Some(next_value(&mut arguments, "--instance")),
            "--class" => class = Some(next_value(&mut arguments, "--class")),
            "--transparent" => transparent = true,
            "--fullscreen" => fullscreen = true,
            "--no-x11" => x11 = false,
            "--force-x11" => force_x11 = true,
            "--hold-ms" => {
                hold_ms = next_value(&mut arguments, "--hold-ms")
                    .parse()
                    .expect("--hold-ms must be an integer");
            }
            "--help" | "-h" => {
                println!(
                    "usage: desktop_identity [--app-id ID] [--instance NAME] [--class NAME] \
                     [--title TITLE] [--transparent] [--fullscreen] [--no-x11] [--force-x11] \
                     [--hold-ms MS]"
                );
                std::process::exit(0);
            }
            other => panic!("unknown argument {other}"),
        }
    }
    let mut desktop = DesktopConfig::for_application(app_id.clone());
    if let Some(identity) = instance.as_deref().or(class.as_deref()) {
        desktop.x11 = Some(X11Identity::with_names(
            identity,
            class.as_deref().unwrap_or(identity),
        ));
    }
    if transparent {
        desktop = desktop.transparent();
    }
    if !x11 {
        desktop = desktop.without_x11();
    }
    Options {
        config: WindowConfig {
            title,
            application_id: app_id,
            size: (640, 480),
            minimum_size: (320, 200),
            fullscreen,
            resizable: true,
        },
        desktop,
        hold: Duration::from_millis(hold_ms),
        force_x11,
    }
}

fn next_value(arguments: &mut impl Iterator<Item = String>, option: &str) -> String {
    arguments
        .next()
        .unwrap_or_else(|| panic!("{option} needs a value"))
}

fn native_id(window: &quasar_window::winit::window::Window) -> String {
    #[cfg(target_os = "linux")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = window.window_handle() {
            match handle.as_raw() {
                RawWindowHandle::Xlib(handle) => return format!("0x{:x}", handle.window),
                RawWindowHandle::Xcb(handle) => return format!("0x{:x}", handle.window),
                _ => {}
            }
        }
    }
    let _ = window;
    "none".into()
}
