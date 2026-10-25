//! Optional application lifecycle composition for Quasar. It owns no ECS,
//! renderer, GAL or editor and is never required by lower-level crates.
//!
//! With the default `desktop` feature, [`run`] creates an ordinary winit event
//! loop, owns one [`quasar_window::GameWindow`], forwards native/device events,
//! and drives an explicit callback order each frame: capture input, call
//! [`Application::frame_update`] once, run zero or more fixed
//! [`Application::tick`] steps, then [`Application::render`] after the tick
//! batch. With `--no-default-features`, only [`run_headless`] and the common
//! application APIs are compiled.
//!
//! Applications own their VulkanicGAL device, renderer, audio and assets. Create
//! window-independent state in [`Application::setup`] (once) and window-bound
//! graphics in [`DesktopApplication::resumed`] (after every window creation or
//! recreation); update them in `tick`/`render`, and release graphics in
//! [`Application::suspended`] before the runner drops the window.
//!
//! # Minimal application
//!
//! ```
//! use quasar_app::{AppContext, Application, Flow};
//!
//! struct Demo {
//!     ticks: u64,
//! }
//! impl Application for Demo {
//!     type Error = std::convert::Infallible;
//!
//!     fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
//!         // Variable-rate UI/camera work; `context.input` is this frame's snapshot.
//!         let _ = context.input.cursor_physical();
//!         Ok(Flow::Continue)
//!     }
//!
//!     fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
//!         self.ticks += 1;
//!         // Fixed-step input: edges are valid once and cleared after this tick.
//!         if context.input.pressed(quasar_input::Key::Escape) {
//!             return Ok(Flow::Exit);
//!         }
//!         Ok(Flow::Continue)
//!     }
//! }
//! ```
//!
//! See `docs/app-runner.md` for the full phase, input, timing and lifecycle
//! contract.
mod app;
#[cfg(feature = "desktop")]
mod desktop;
mod headless;

#[cfg(feature = "desktop")]
pub use app::DesktopApplication;
pub use app::{AppContext, AppPhase, Application, Flow, RenderPolicy, TimingConfig, TimingInfo};
#[cfg(feature = "desktop")]
pub use desktop::{AppConfig, run};
pub use headless::{HeadlessConfig, run as run_headless};
