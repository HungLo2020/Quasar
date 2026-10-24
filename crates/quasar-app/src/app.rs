use quasar_core::FixedStepConfig;
#[cfg(not(feature = "desktop"))]
use std::marker::PhantomData;
use std::{error::Error, time::Duration};

/// Result of one simulation tick or desktop event hook.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flow {
    Continue,
    Exit,
}

/// How the runner turns elapsed time into application ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimingConfig {
    /// A fixed accumulator. The application receives one call per tick.
    Fixed(FixedStepConfig),
    /// One application tick per native frame; the application owns any finer
    /// scheduling it needs. `AppContext::elapsed` still exposes frame time.
    EveryFrame,
}
impl Default for TimingConfig {
    fn default() -> Self {
        Self::Fixed(FixedStepConfig::for_rate(60.0, 8))
    }
}

/// When the runner requests a native redraw after processing ticks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RenderPolicy {
    /// Request a redraw every frame, even when no simulation tick ran.
    #[default]
    EveryFrame,
    /// Request only after at least one tick.
    AfterTick,
    /// The application calls `AppContext::request_redraw` itself.
    Manual,
}

/// Read-only frame context passed to application hooks.
///
/// `window` is present only with the `desktop` feature. Applications that need
/// a caller-owned GAL, renderer or other GPU resource create and own it in
/// `setup`; the runner never owns graphics state.
pub struct AppContext<'a> {
    /// The runner-owned window in desktop mode. Applications can call
    /// [`quasar_window::GameWindow::shared_native`] in `setup` to create their
    /// own VulkanicGAL presenter/device without transferring ownership.
    #[cfg(feature = "desktop")]
    pub window: Option<&'a quasar_window::GameWindow>,
    /// Current inner size in physical pixels; `(0, 0)` before window creation.
    pub size: (u32, u32),
    pub scale_factor: f64,
    /// Wall time since the previous runner frame.
    pub elapsed: Duration,
    /// Monotonic 1-based frame number; 0 before the first frame.
    pub frame: u64,
    /// Monotonic 1-based simulation tick number; 0 before the first tick.
    pub tick: u64,
    /// Fraction into the next fixed step; 0 for `TimingConfig::EveryFrame`.
    pub interpolation: f64,
    pub focused: bool,
    #[cfg(not(feature = "desktop"))]
    pub(crate) marker: PhantomData<&'a ()>,
}
impl<'a> AppContext<'a> {
    #[cfg(feature = "desktop")]
    pub fn request_redraw(&self) {
        if let Some(window) = self.window {
            window.native().request_redraw();
        }
    }
    pub fn drawable(&self) -> bool {
        #[cfg(feature = "desktop")]
        if let Some(window) = self.window {
            return window.drawable();
        }
        true
    }
    pub fn focused(&self) -> bool {
        self.focused
    }
}

/// Application hooks. `tick` is the only required method; all others are opt-in.
pub trait Application {
    type Error: Error + 'static;

    /// Called after a native window is created, or once for a headless run.
    /// Recreate caller-owned render resources here after suspension.
    fn setup(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error>;
    /// One simulation step. The runner consumes `GameWindow` input edges after
    /// this callback, so read `context.window.input` here rather than polling
    /// native redraw events. Desktop `window_event` hooks run before the tick
    /// and may also inspect pending edges.
    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error>;
    /// Called from `RedrawRequested` in desktop mode. Headless runs do not call it.
    fn render(&mut self, _context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        Ok(())
    }
    /// Called before the native window is dropped. Release GPU resources here.
    fn suspended(&mut self, _context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        Ok(())
    }
    /// Called once before final teardown.
    fn shutdown(&mut self, _context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Desktop-only hooks layered on [`Application`].
#[cfg(feature = "desktop")]
pub trait DesktopApplication: Application {
    /// Called immediately after the runner creates or recreates the window and
    /// before [`Application::setup`]. Install serializable input bindings,
    /// cursor capture, or other window/input policy here. This is the supported
    /// mutable-window hook; later contexts expose a read-only window.
    fn configure_window(
        &mut self,
        _window: &mut quasar_window::GameWindow,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn window_event(
        &mut self,
        _context: &mut AppContext<'_>,
        _event: &quasar_window::winit::event::WindowEvent,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn device_event(
        &mut self,
        _context: &mut AppContext<'_>,
        _event: &quasar_window::winit::event::DeviceEvent,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    /// Called after a window is recreated and before the first tick.
    fn resumed(&mut self, _context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        Ok(())
    }
}
