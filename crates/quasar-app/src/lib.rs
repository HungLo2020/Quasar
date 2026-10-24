//! Optional application lifecycle composition for Quasar. It owns no ECS,
//! renderer, GAL or editor and is never required by lower-level crates.
//!
//! With the default `desktop` feature, [`run`] creates an ordinary winit event
//! loop, owns one [`quasar_window::GameWindow`], forwards native/device events,
//! schedules fixed or frame-driven ticks and calls application render/lifecycle
//! hooks. With `--no-default-features`, only [`run_headless`] and the common
//! application APIs are compiled.
//!
//! Applications own their VulkanicGAL device, renderer, audio and assets. Create
//! them in [`Application::setup`], update them in `tick`/`render`, and release
//! them in [`Application::suspended`]/[`Application::shutdown`] before the
//! runner drops the window.
mod app;
#[cfg(feature = "desktop")]
mod desktop;
mod headless;

#[cfg(feature = "desktop")]
pub use app::DesktopApplication;
pub use app::{AppContext, Application, Flow, RenderPolicy, TimingConfig};
#[cfg(feature = "desktop")]
pub use desktop::{AppConfig, run};
pub use headless::{HeadlessConfig, run as run_headless};
