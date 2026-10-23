//! Standalone native windows and device input. Applications own the event loop.
//! `winit` is re-exported so callers can handle every native event and use its
//! ordinary `ApplicationHandler`, rather than entering a Quasar-specific runner.
pub mod input;
pub use quasar_input::InputState;
use std::sync::Arc;
pub use winit;
use winit::{
    dpi::{LogicalSize, PhysicalSize},
    error::ExternalError,
    event::{DeviceEvent, WindowEvent},
    event_loop::ActiveEventLoop,
    window::{CursorGrabMode, Fullscreen, Window},
};

#[derive(Clone, Debug)]
pub struct WindowConfig {
    pub title: String,
    pub application_id: String,
    /// Initial size and minimum size are logical desktop units, not framebuffer pixels.
    pub size: (u32, u32),
    pub minimum_size: (u32, u32),
    pub fullscreen: bool,
    pub resizable: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Quasar".into(),
            application_id: "quasar".into(),
            size: (960, 600),
            minimum_size: (320, 200),
            fullscreen: false,
            resizable: true,
        }
    }
}

#[derive(Default, Debug)]
pub struct WindowUpdate {
    pub close_requested: bool,
    pub redraw_requested: bool,
    pub resized: Option<PhysicalSize<u32>>,
    pub focus_changed: Option<bool>,
}

/// Owns one window and its device state; no graphics dependency.
pub struct GameWindow {
    window: Arc<Window>,
    pub input: InputState,
    focused: bool,
    occluded: bool,
}

impl GameWindow {
    pub fn create(
        event_loop: &ActiveEventLoop,
        config: &WindowConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if config.size.0 == 0
            || config.size.1 == 0
            || config.minimum_size.0 == 0
            || config.minimum_size.1 == 0
        {
            return Err("window sizes must be nonzero".into());
        }
        let attributes = Window::default_attributes()
            .with_title(&config.title)
            .with_inner_size(LogicalSize::new(config.size.0, config.size.1))
            .with_min_inner_size(LogicalSize::new(
                config.minimum_size.0,
                config.minimum_size.1,
            ))
            .with_resizable(config.resizable)
            .with_transparent(false)
            .with_fullscreen(config.fullscreen.then_some(Fullscreen::Borderless(None)));
        #[cfg(target_os = "linux")]
        let attributes = {
            use winit::platform::{
                wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11,
            };
            let attributes = WindowAttributesExtX11::with_name(
                attributes,
                &config.application_id,
                &config.application_id,
            );
            WindowAttributesExtWayland::with_name(
                attributes,
                &config.application_id,
                &config.application_id,
            )
        };
        let window = Arc::new(event_loop.create_window(attributes)?);
        let focused = window.has_focus();
        Ok(Self {
            window,
            input: InputState::default(),
            focused,
            occluded: false,
        })
    }

    pub fn native(&self) -> &Window {
        &self.window
    }
    /// Retain this Arc when creating graphics surfaces so the window outlives them.
    pub fn shared_native(&self) -> Arc<Window> {
        self.window.clone()
    }
    pub fn focused(&self) -> bool {
        self.focused
    }
    pub fn drawable(&self) -> bool {
        let size = self.window.inner_size();
        !self.occluded && size.width != 0 && size.height != 0
    }
    pub fn fullscreen(&self) -> bool {
        self.window.fullscreen().is_some()
    }
    pub fn set_fullscreen(&self, enabled: bool) {
        self.window
            .set_fullscreen(enabled.then_some(Fullscreen::Borderless(None)));
    }
    pub fn toggle_fullscreen(&self) {
        self.set_fullscreen(!self.fullscreen());
    }

    /// Attempts locked relative motion, falling back to confined cursor capture.
    pub fn capture_cursor(&self, capture: bool) -> Result<(), ExternalError> {
        if capture {
            self.window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))?;
        } else {
            self.window.set_cursor_grab(CursorGrabMode::None)?;
        }
        self.window.set_cursor_visible(!capture);
        Ok(())
    }

    pub fn process(&mut self, event: &WindowEvent) -> WindowUpdate {
        input::process_window_event(&mut self.input, event);
        let mut update = WindowUpdate::default();
        match event {
            WindowEvent::CloseRequested => update.close_requested = true,
            WindowEvent::RedrawRequested => update.redraw_requested = true,
            WindowEvent::Resized(size) => update.resized = Some(*size),
            WindowEvent::ScaleFactorChanged { .. } => {
                update.resized = Some(self.window.inner_size())
            }
            WindowEvent::Focused(focused) => {
                self.focused = *focused;
                update.focus_changed = Some(*focused);
                if !focused {
                    let _ = self.capture_cursor(false);
                }
            }
            WindowEvent::Occluded(occluded) => self.occluded = *occluded,
            _ => {}
        }
        update
    }

    pub fn process_device(&mut self, event: &DeviceEvent) {
        input::process_device_event(&mut self.input, event, self.focused);
    }
}
