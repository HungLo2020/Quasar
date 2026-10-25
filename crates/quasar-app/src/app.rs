use quasar_core::FixedStepConfig;
use quasar_input::FrameInput;
use std::{error::Error, time::Duration};

/// Result of a frame update, fixed tick, or lifecycle request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flow {
    Continue,
    Exit,
    /// The callback deliberately did not consume simulation input (for example,
    /// a paused simulation). Fixed ticks returning this preserve pending input
    /// edges for a later frame and do not advance simulation time or tick
    /// indices. The desktop runner also discards that frame's accumulated clock
    /// backlog so resuming does not replay paused wall time.
    Deferred,
}

/// How the runner turns wall time into fixed simulation ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimingConfig {
    /// A bounded fixed accumulator. `frame_update` runs every rendered frame;
    /// `tick` runs zero or more times per frame.
    Fixed(FixedStepConfig),
    /// No fixed ticks. `frame_update` runs every frame and owns any scheduling.
    Variable,
}
impl Default for TimingConfig {
    fn default() -> Self {
        Self::Fixed(FixedStepConfig::for_rate(60.0, 8))
    }
}

/// When the runner requests a native redraw after processing a frame.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RenderPolicy {
    /// Request one redraw per application frame, including frames with zero ticks.
    #[default]
    EveryFrame,
    /// Request a redraw only when the fixed scheduler produced at least one tick.
    AfterTick,
    /// The application calls `AppContext::request_redraw` itself.
    Manual,
}

/// Current callback phase. The runner never calls frame update more than once
/// per frame and never runs rendering in the middle of a catch-up tick batch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AppPhase {
    #[default]
    Frame,
    Tick,
    Render,
    Suspended,
    Shutdown,
}

/// Timing and scheduler diagnostics for one callback.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimingInfo {
    /// Wall time since the previous application frame.
    pub frame_elapsed: Duration,
    /// Exact fixed simulation step. Zero for [`TimingConfig::Variable`].
    pub fixed_step: Duration,
    /// Fraction into the next fixed step at the start of this callback.
    pub interpolation: f64,
    /// Monotonic 1-based application frame number.
    pub frame_index: u64,
    /// Monotonic 1-based simulation tick number.
    pub tick_index: u64,
    /// Number of fixed ticks scheduled for this frame. Tick callbacks see the
    /// frame's total, not the number remaining.
    pub ticks_this_frame: u32,
    /// Simulated time at the start of this callback.
    pub simulation_time: Duration,
    /// Elapsed time accepted by the scheduler.
    pub accepted_elapsed: Duration,
    /// Elapsed time removed by the configured clamp.
    pub clamped_elapsed: Duration,
    /// Backlog discarded by the catch-up policy.
    pub dropped_elapsed: Duration,
    /// Time retained for future ticks after advancement.
    pub backlog: Duration,
}

/// Read-only/mutable context passed to application callbacks.
///
/// `input` is a frame snapshot. It is the same snapshot for `frame_update`, all
/// fixed ticks in that frame, and `render`; transient edges are cleared after
/// the first tick so catch-up cannot deliver a press twice. `window` is present
/// only in desktop builds.
pub struct AppContext<'a> {
    /// The runner-owned window in desktop mode.
    #[cfg(feature = "desktop")]
    pub window: Option<&'a quasar_window::GameWindow>,
    /// Frame input snapshot. Use this instead of polling native edges.
    pub input: &'a mut FrameInput,
    /// Current inner size in physical pixels; `(0, 0)` before window creation.
    pub size: (u32, u32),
    pub scale_factor: f64,
    /// Wall time since the previous runner frame.
    pub elapsed: Duration,
    /// Monotonic 1-based frame number; 0 before the first frame.
    pub frame: u64,
    /// Monotonic 1-based simulation tick number; 0 before the first tick.
    pub tick: u64,
    /// Fraction into the next fixed step at the start of this callback.
    pub interpolation: f64,
    pub timing: TimingInfo,
    pub phase: AppPhase,
    pub focused: bool,
    /// `Some` on callbacks associated with a focus transition.
    pub focus_changed: Option<bool>,
}

impl AppContext<'_> {
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

/// Lifecycle and per-frame hooks.
///
/// `setup` runs exactly once, after the first window (desktop) or before the
/// first headless iteration. `frame_update` runs exactly once per application
/// frame, before fixed ticks.
/// `tick` runs for each fixed simulation step. `render` runs after the frame's
/// ticks only when a redraw was requested. All hooks are optional except the
/// associated error type, so a minimal application only implements the phases
/// it actually needs.
pub trait Application {
    type Error: Error + 'static;

    fn setup(&mut self, _context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Variable-rate work: UI, cameras, networking, input routing and visual
    /// state that must update even when no simulation tick is due.
    fn frame_update(&mut self, _context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        Ok(Flow::Continue)
    }

    /// One deterministic fixed step. Called zero or more times per frame.
    fn tick(&mut self, _context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        Ok(Flow::Continue)
    }

    /// Called after the frame's fixed ticks when a redraw is due.
    fn render(&mut self, _context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        Ok(Flow::Continue)
    }

    /// Called before a window/surface is destroyed or recreated. Release
    /// graphics resources here.
    fn suspended(&mut self, _context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called once at final teardown, after `suspended` if applicable.
    fn shutdown(&mut self, _context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Desktop-only hooks layered on [`Application`].
#[cfg(feature = "desktop")]
pub trait DesktopApplication: Application {
    /// Called immediately after the runner creates or recreates the window and
    /// before [`Application::setup`]. Install serializable input bindings,
    /// cursor capture, or other window/input policy here.
    fn configure_window(
        &mut self,
        _window: &mut quasar_window::GameWindow,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Raw native window event hook. Input state is already forwarded to
    /// `window.input`; use `AppContext::input` in frame/tick callbacks for the
    /// deterministic frame snapshot.
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

    /// Called after `setup` on the first window and after every recreation
    /// following a suspend. Create window/surface-bound graphics here.
    fn resumed(&mut self, _context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        Ok(())
    }
}
