//! Native X11 metadata tests.
//!
//! These tests need a real X server. Run them under `xvfb-run`, a nested
//! `Xephyr`, or an XWayland session with `QUASAR_X11_TESTS=1`. Without that
//! variable they report a skip so ordinary headless `cargo test` stays green.
//!
//! winit permits one event loop per process, so all scenarios share one loop and
//! create/close windows sequentially, which also exercises window recreation.
#![cfg(all(target_os = "linux", feature = "x11"))]

#[cfg(feature = "diagnostics")]
use quasar_diagnostics::{Diagnostics, DiagnosticsConfig, Level, MemorySink};
use quasar_window::{
    DesktopConfig, GameWindow, WindowConfig, X11Identity,
    winit::{
        application::ApplicationHandler,
        dpi::PhysicalSize,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop},
        platform::{pump_events::EventLoopExtPumpEvents, x11::EventLoopBuilderExtX11},
        window::{Window, WindowId},
    },
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::time::{Duration, Instant};
use x11rb::{
    connection::Connection,
    protocol::xproto::{AtomEnum, ConfigureWindowAux, ConnectionExt as _, GetPropertyReply},
    rust_connection::RustConnection,
};

const POLL: Duration = Duration::from_millis(10);
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct App {
    window: Option<GameWindow>,
    pending: Option<(WindowConfig, DesktopConfig)>,
    create_error: Option<String>,
    last_resized: Option<PhysicalSize<u32>>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, _: &ActiveEventLoop) {}

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some((config, desktop)) = self.pending.take() else {
            return;
        };
        match GameWindow::create_with_desktop(event_loop, &config, &desktop) {
            Ok(window) => self.window = Some(window),
            Err(error) => self.create_error = Some(error.to_string()),
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = &mut self.window else {
            return;
        };
        if window.native().id() != id {
            return;
        }
        let update = window.process(&event);
        if let Some(size) = update.resized {
            self.last_resized = Some(size);
        }
    }
}

struct Harness {
    event_loop: EventLoop<()>,
    app: App,
    connection: RustConnection,
    window: u32,
}

impl Harness {
    fn new() -> Self {
        let mut builder = EventLoop::<()>::builder();
        builder.with_x11().with_any_thread(true);
        let event_loop = builder.build().expect("build forced-X11 event loop");
        let (connection, _) = x11rb::connect(None).expect("connect to X11");
        Self {
            event_loop,
            app: App::default(),
            connection,
            window: 0,
        }
    }

    fn pump(&mut self) {
        let _ = self.event_loop.pump_app_events(Some(POLL), &mut self.app);
    }

    fn pump_until(&mut self, mut condition: impl FnMut(&App) -> bool) -> bool {
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            self.pump();
            if condition(&self.app) {
                return true;
            }
        }
        false
    }

    fn open(&mut self, config: WindowConfig, desktop: DesktopConfig) -> Result<(), String> {
        assert!(self.app.window.is_none(), "close the previous test window");
        self.app.pending = Some((config, desktop));
        let deadline = Instant::now() + TIMEOUT;
        loop {
            self.pump();
            if let Some(error) = self.app.create_error.take() {
                return Err(error);
            }
            if let Some(window) = &self.app.window {
                self.window = x11_window_id(window.native())
                    .ok_or_else(|| "winit did not expose an Xlib/XCB window".to_string())?;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("timed out waiting for the native X11 window".into());
            }
        }
    }

    fn close(&mut self) {
        self.app.window.take();
        self.pump();
    }

    fn surface_size(&self) -> PhysicalSize<u32> {
        self.app
            .window
            .as_ref()
            .expect("window exists")
            .native()
            .inner_size()
    }

    fn atom(&self, name: &str) -> u32 {
        self.connection
            .intern_atom(false, name.as_bytes())
            .expect("intern atom")
            .reply()
            .expect("atom reply")
            .atom
    }

    fn property(&self, name: &str) -> GetPropertyReply {
        self.connection
            .get_property(
                false,
                self.window,
                self.atom(name),
                AtomEnum::ANY,
                0,
                u32::MAX,
            )
            .expect("get property")
            .reply()
            .expect("property reply")
    }

    fn property_u32(&self, name: &str) -> Option<Vec<u32>> {
        let reply = self.property(name);
        reply.value32().map(|values| values.collect())
    }

    fn wait_for_absent(&mut self, name: &str) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let reply = self.property(name);
            if reply.type_ == 0 {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{name} should not be present, found type {}",
                reply.type_
            );
            self.pump();
            std::thread::sleep(POLL);
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.connection
            .configure_window(
                self.window,
                &ConfigureWindowAux::new().width(width).height(height),
            )
            .expect("configure window");
        self.connection.flush().expect("flush resize");
        // Round-trip so the request is processed before winit is pumped.
        self.connection
            .get_geometry(self.window)
            .expect("get geometry")
            .reply()
            .expect("geometry reply");
    }

    fn assert_full_opacity(&self) {
        let opacity = self.property("_NET_WM_WINDOW_OPACITY");
        assert_eq!(
            opacity.type_,
            u32::from(AtomEnum::CARDINAL),
            "_NET_WM_WINDOW_OPACITY must be CARDINAL"
        );
        assert_eq!(opacity.format, 32);
        assert_eq!(
            self.property_u32("_NET_WM_WINDOW_OPACITY"),
            Some(vec![u32::MAX])
        );
    }

    fn assert_normal_window(&self) {
        let expected = self.atom("_NET_WM_WINDOW_TYPE_NORMAL");
        let property = self.property("_NET_WM_WINDOW_TYPE");
        assert_eq!(property.type_, u32::from(AtomEnum::ATOM));
        assert_eq!(property.format, 32);
        assert_eq!(
            self.property_u32("_NET_WM_WINDOW_TYPE"),
            Some(vec![expected])
        );
    }

    fn assert_class(&self, instance: &str, class: &str) {
        let property = self.property("WM_CLASS");
        assert_eq!(property.type_, u32::from(AtomEnum::STRING));
        assert_eq!(property.format, 8);
        assert_eq!(
            property.value,
            format!("{instance}\0{class}\0").into_bytes()
        );
    }

    fn assert_pid(&self) {
        let property = self.property("_NET_WM_PID");
        assert_eq!(property.type_, u32::from(AtomEnum::CARDINAL));
        assert_eq!(property.format, 32);
        assert_eq!(
            self.property_u32("_NET_WM_PID"),
            Some(vec![std::process::id()])
        );
    }

    fn assert_opaque_region(&self, width: u32, height: u32) {
        let property = self.property("_NET_WM_OPAQUE_REGION");
        assert_eq!(property.type_, u32::from(AtomEnum::CARDINAL));
        assert_eq!(property.format, 32);
        assert_eq!(
            self.property_u32("_NET_WM_OPAQUE_REGION"),
            Some(vec![0, 0, width, height])
        );
    }
}

fn x11_window_id(window: &Window) -> Option<u32> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok(),
        RawWindowHandle::Xcb(handle) => Some(handle.window.into()),
        _ => None,
    }
}

fn reference_config(title: &str, application_id: &str) -> WindowConfig {
    WindowConfig {
        title: title.into(),
        application_id: application_id.into(),
        size: (640, 480),
        minimum_size: (320, 200),
        fullscreen: false,
        resizable: true,
    }
}

fn native_x11_metadata() {
    let mut harness = Harness::new();

    // MissileCommand's actual identity and resize-aware opaque region.
    let desktop = DesktopConfig {
        x11: Some(X11Identity::with_names("missile-command", "MissileCommand")),
        ..DesktopConfig::for_application("missile-command")
    };
    harness
        .open(
            reference_config("Missile Command", "missile-command"),
            desktop,
        )
        .expect("create MissileCommand-equivalent window");
    #[cfg(feature = "diagnostics")]
    let diagnostics_sink = {
        use std::sync::Arc;
        let sink = Arc::new(MemorySink::new(32));
        harness
            .app
            .window
            .as_mut()
            .expect("window")
            .set_diagnostics(Some(Diagnostics::with_config(
                sink.clone(),
                DiagnosticsConfig {
                    min_level: Level::Debug,
                    ..DiagnosticsConfig::default()
                },
            )));
        sink
    };
    harness.assert_class("missile-command", "MissileCommand");
    harness.assert_pid();
    harness.assert_normal_window();
    harness.assert_full_opacity();
    let initial = harness.surface_size();
    assert!(initial.width > 0 && initial.height > 0);
    harness.assert_opaque_region(initial.width, initial.height);

    // Resizes that become visible while graphics initialization is running must
    // be reconciled by the next process() call, without game code.
    for (width, height) in [(800, 600), (1100, 650), (440, 700)] {
        harness.resize(width, height);
        assert!(
            harness.pump_until(|app| app.window.as_ref().is_some_and(|window| {
                window.native().inner_size() == PhysicalSize::new(width, height)
            })),
            "winit did not observe the external resize to {width}x{height}"
        );
        harness.pump_until(|app| app.last_resized == Some(PhysicalSize::new(width, height)));
        let resized = harness.surface_size();
        harness.assert_opaque_region(resized.width, resized.height);
        assert_eq!(
            harness.property_u32("_NET_WM_OPAQUE_REGION"),
            Some(vec![0, 0, width, height])
        );
    }
    #[cfg(feature = "diagnostics")]
    {
        assert!(
            diagnostics_sink
                .records()
                .iter()
                .any(|record| record.name == "resized"),
            "window diagnostics did not receive resize events"
        );
    }
    harness.close();

    // BlackOps and Jackal use distinct X11 instance and class values.
    for (application_id, instance, class) in [
        ("blackops-native", "blackops-native", "BlackOpsNative"),
        ("jackal-native", "jackal-native", "JackalNative"),
    ] {
        let desktop = DesktopConfig {
            x11: Some(X11Identity::with_names(instance, class)),
            ..DesktopConfig::for_application(application_id)
        };
        harness
            .open(reference_config(class, application_id), desktop)
            .unwrap_or_else(|error| panic!("create {class} window: {error}"));
        harness.assert_class(instance, class);
        harness.assert_pid();
        harness.assert_normal_window();
        harness.assert_full_opacity();
        let size = harness.surface_size();
        harness.assert_opaque_region(size.width, size.height);
        harness.close();
    }

    // Genuinely transparent windows must not claim full opacity or opaque area.
    let transparent = DesktopConfig {
        x11: Some(X11Identity::with_names(
            "quasar-transparent",
            "QuasarTransparent",
        )),
        ..DesktopConfig::for_application("quasar-transparent")
    }
    .transparent();
    match harness.open(
        reference_config("Quasar transparent", "quasar-transparent"),
        transparent,
    ) {
        Ok(()) => {
            harness.assert_class("quasar-transparent", "QuasarTransparent");
            harness.wait_for_absent("_NET_WM_WINDOW_OPACITY");
            harness.wait_for_absent("_NET_WM_OPAQUE_REGION");
            harness.close();
        }
        Err(error) => {
            eprintln!("skipping transparency check; no ARGB visual available: {error}");
        }
    }

    // Fullscreen transitions must track the actual physical drawable size.
    harness
        .open(
            reference_config("Quasar fullscreen", "quasar-fullscreen"),
            DesktopConfig::for_application("quasar-fullscreen"),
        )
        .expect("create fullscreen test window");
    harness
        .app
        .window
        .as_ref()
        .expect("window")
        .set_fullscreen(true);
    assert!(harness.pump_until(|app| app.window.as_ref().is_some_and(GameWindow::fullscreen)));
    let fullscreen = harness.surface_size();
    harness.assert_opaque_region(fullscreen.width, fullscreen.height);
    harness
        .app
        .window
        .as_ref()
        .expect("window")
        .set_fullscreen(false);
    assert!(harness.pump_until(|app| {
        app.window
            .as_ref()
            .is_some_and(|window| !window.fullscreen())
    }));
    let windowed = harness.surface_size();
    harness.assert_opaque_region(windowed.width, windowed.height);
}

#[test]
fn x11_native_metadata() {
    if std::env::var_os("QUASAR_X11_TESTS").is_none() {
        eprintln!("skipping native X11 test; set QUASAR_X11_TESTS=1 with DISPLAY available");
        return;
    }
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("skipping native X11 test; DISPLAY is not set");
        return;
    }
    native_x11_metadata();
}
