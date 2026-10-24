//! Standalone native windows and device input. Applications own the event loop.
//! `winit` is re-exported so callers can handle every native event and use its
//! ordinary `ApplicationHandler`, rather than entering a Quasar-specific runner.
//!
//! # Desktop identity and compositor hints
//!
//! [`GameWindow::create`] preserves the existing [`WindowConfig`] API and applies
//! an opaque desktop policy derived from `application_id`. Use
//! [`GameWindow::create_with_desktop`] when a game needs distinct X11
//! instance/class names or an actually transparent surface. On X11 the crate
//! writes `WM_CLASS` through winit before the window is mapped, then applies
//! `_NET_WM_PID`, `_NET_WM_WINDOW_TYPE_NORMAL`,
//! `_NET_WM_WINDOW_OPACITY` and the physical-size `_NET_WM_OPAQUE_REGION`
//! immediately after native creation. This happens before the caller creates a
//! VulkanicGAL presenter or any other graphics API.
//!
//! Wayland uses winit's `xdg_toplevel.app_id`; Windows uses a Win32 class name;
//! macOS keeps bundle identity. X11-only properties are not fabricated on those
//! platforms, and [`desktop_capabilities`] reports what is compiled. The default
//! `x11` feature is the only path that adds `x11rb`, and its dependencies are
//! target-gated to Linux.
//!
//! Resize, DPI and fullscreen updates are reconciled by [`GameWindow::process`]
//! from the current physical drawable size, so games do not poll or repair
//! compositor metadata themselves. [`GameWindow::sync_desktop`] provides the same
//! reconciliation immediately after caller-owned graphics initialization.
//!
//! The repository document `docs/window-identity.md` contains the reference-game
//! audit, property contract, migration examples and native X11/KWin validation
//! procedure.
pub mod input;
pub use quasar_input::InputState;
use std::{error::Error, fmt, sync::Arc};
pub use winit;
use winit::{
    dpi::{LogicalSize, PhysicalSize},
    error::ExternalError,
    event::{DeviceEvent, WindowEvent},
    event_loop::ActiveEventLoop,
    window::{CursorGrabMode, Fullscreen, Window},
};

/// Desktop identity and compositor presentation policy.
///
/// The default for [`GameWindow::create`] is [`DesktopConfig::for_application`],
/// which derives X11 instance/class names from [`WindowConfig::application_id`],
/// requests a normal top-level window, and marks an opaque window's full drawable
/// area as opaque. [`GameWindow::create_with_desktop`] accepts an explicit
/// policy when a game needs different names or an actually transparent surface.
///
/// X11-specific fields are ignored on Windows, macOS and Wayland. On those
/// platforms the equivalent supported behavior is applied through winit where it
/// exists; unsupported properties are not fabricated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopConfig {
    /// Whether the native window is opaque or uses an alpha-composited surface.
    pub transparency: WindowTransparency,
    /// How the compositor should be told which part of the window is opaque.
    ///
    /// Only meaningful for opaque windows. Requesting [`OpaqueRegionPolicy::FullWindow`]
    /// for a transparent window is rejected rather than marking transparent pixels
    /// as opaque.
    pub opaque_region: OpaqueRegionPolicy,
    /// Optional X11 identity overrides. `None` leaves identity to winit/GLFW/SDL
    /// defaults and disables Quasar's X11 property management.
    pub x11: Option<X11Identity>,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            transparency: WindowTransparency::Opaque,
            opaque_region: OpaqueRegionPolicy::FullWindow,
            x11: None,
        }
    }
}

impl DesktopConfig {
    /// Builds the default desktop policy for an application identity.
    ///
    /// X11 `WM_CLASS` defaults to `(instance, class) = (application_id, application_id)`;
    /// use [`X11Identity::with_names`] or [`X11Identity::for_application`] for
    /// distinct values. The process ID and a normal window type are requested.
    pub fn for_application(application_id: impl Into<String>) -> Self {
        Self {
            transparency: WindowTransparency::Opaque,
            opaque_region: OpaqueRegionPolicy::FullWindow,
            x11: Some(X11Identity::for_application(application_id)),
        }
    }

    /// Selects an alpha-composited window and disables the opaque-region hint.
    pub fn transparent(mut self) -> Self {
        self.transparency = WindowTransparency::Transparent;
        self.opaque_region = OpaqueRegionPolicy::Disabled;
        self
    }

    /// Disables Quasar's X11-native property management for this window.
    ///
    /// Winit still receives the normal application/name attributes.
    pub fn without_x11(mut self) -> Self {
        self.x11 = None;
        self
    }

    /// Validates identity strings and presentation policy before window creation.
    pub fn validate(&self) -> Result<(), DesktopError> {
        if self.transparency == WindowTransparency::Transparent
            && self.opaque_region == OpaqueRegionPolicy::FullWindow
        {
            return Err(DesktopError::new(
                "transparent windows cannot request a full-window opaque region",
            ));
        }
        if let Some(x11) = &self.x11 {
            x11.validate()?;
        }
        Ok(())
    }
}

/// Native window transparency requested at creation time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowTransparency {
    /// A normal, fully opaque window (the default).
    Opaque,
    /// An alpha-composited window. The exact visual result remains compositor-
    /// and platform-dependent.
    Transparent,
}

/// How much of the X11 drawable is advertised as opaque.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OpaqueRegionPolicy {
    /// Do not write `_NET_WM_OPAQUE_REGION` (and remove a stale value).
    Disabled,
    /// Track the full physical drawable rectangle as opaque.
    #[default]
    FullWindow,
}

/// Desktop capabilities actually compiled and available on this target.
///
/// Games can report unavailable optional behavior instead of assuming that every
/// platform implements the same native properties. For example, an X11-only
/// opacity hint is not fabricated on Wayland, Windows or macOS.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesktopCapabilities {
    /// The X11-native property backend is compiled and available.
    pub x11_properties: bool,
    /// `_NET_WM_PID` is supported by [`X11Identity::set_pid`].
    pub x11_pid: bool,
    /// `_NET_WM_WINDOW_OPACITY` is supported.
    pub x11_opacity: bool,
    /// `_NET_WM_OPAQUE_REGION` is supported.
    pub x11_opaque_region: bool,
    /// Application identity is supplied to Wayland through `xdg_toplevel.app_id`.
    pub wayland_application_id: bool,
    /// A Win32 window class name is supplied at window creation.
    pub windows_class_name: bool,
}

/// Returns the compiled desktop capabilities for this target.
pub const fn desktop_capabilities() -> DesktopCapabilities {
    DesktopCapabilities {
        x11_properties: cfg!(all(target_os = "linux", feature = "x11")),
        x11_pid: cfg!(all(target_os = "linux", feature = "x11")),
        x11_opacity: cfg!(all(target_os = "linux", feature = "x11")),
        x11_opaque_region: cfg!(all(target_os = "linux", feature = "x11")),
        wayland_application_id: cfg!(target_os = "linux"),
        windows_class_name: cfg!(target_os = "windows"),
    }
}

/// X11 identity and compositor properties.
///
/// These settings are consulted only for an Xlib or XCB native window on Linux.
/// Wayland application identity comes from [`WindowConfig::application_id`] via
/// winit and does not use X11 properties.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct X11Identity {
    /// `WM_CLASS` instance name. `None` uses the application ID.
    pub instance: Option<String>,
    /// `WM_CLASS` class name. `None` uses the application ID.
    pub class: Option<String>,
    /// Write `_NET_WM_PID` with the current process ID.
    pub set_pid: bool,
    /// Explicitly request `_NET_WM_WINDOW_TYPE_NORMAL` through winit's X11
    /// attributes. `false` leaves winit's default, which is currently `Normal`.
    pub normal_window_type: bool,
}

impl Default for X11Identity {
    fn default() -> Self {
        Self {
            instance: None,
            class: None,
            set_pid: true,
            normal_window_type: true,
        }
    }
}

impl X11Identity {
    /// Uses the same instance and class name, matching the existing
    /// `WindowConfig.application_id` behavior.
    pub fn for_application(application_id: impl Into<String>) -> Self {
        let name = application_id.into();
        Self {
            instance: Some(name.clone()),
            class: Some(name),
            ..Self::default()
        }
    }

    /// Sets distinct X11 instance and class names.
    ///
    /// The resulting `WM_CLASS` is `"instance", "class"`.
    pub fn with_names(instance: impl Into<String>, class: impl Into<String>) -> Self {
        Self {
            instance: Some(instance.into()),
            class: Some(class.into()),
            ..Self::default()
        }
    }

    fn validate(&self) -> Result<(), DesktopError> {
        for (label, value) in [
            ("X11 instance", self.instance.as_deref()),
            ("X11 class", self.class.as_deref()),
        ] {
            if let Some(value) = value {
                validate_text(label, value)?;
            }
        }
        Ok(())
    }

    fn instance_or<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.instance.as_deref().unwrap_or(fallback)
    }

    fn class_or<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.class.as_deref().unwrap_or(fallback)
    }
}

/// A desktop identity/property error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopError {
    message: String,
}

impl DesktopError {
    /// Creates an error from a displayable message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Human-readable error detail.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DesktopError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for DesktopError {}

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

struct DesktopManager {
    #[cfg(all(target_os = "linux", feature = "x11"))]
    x11: Option<X11Desktop>,
}

impl DesktopManager {
    fn new(window: &Window, desktop: &DesktopConfig) -> Result<Self, DesktopError> {
        #[cfg(all(target_os = "linux", feature = "x11"))]
        {
            Ok(Self {
                x11: X11Desktop::attach(window, desktop)?,
            })
        }
        #[cfg(not(all(target_os = "linux", feature = "x11")))]
        {
            let _ = (window, desktop);
            Ok(Self {})
        }
    }

    fn update_size(&mut self, width: u32, height: u32) -> Result<(), DesktopError> {
        #[cfg(all(target_os = "linux", feature = "x11"))]
        if let Some(x11) = &self.x11 {
            x11.update_size(width, height)?;
        }
        let _ = (width, height);
        Ok(())
    }
}

/// Extracts a 32-bit X11 window id from either raw handle flavor used on Linux.
#[cfg(all(target_os = "linux", feature = "x11"))]
fn x11_window_id(handle: raw_window_handle::RawWindowHandle) -> Result<Option<u32>, DesktopError> {
    use raw_window_handle::RawWindowHandle;
    match handle {
        RawWindowHandle::Xlib(handle) => u32::try_from(handle.window)
            .map(Some)
            .map_err(|_| DesktopError::new("Xlib window ID does not fit in 32 bits")),
        RawWindowHandle::Xcb(handle) => Ok(Some(handle.window.get())),
        _ => Ok(None),
    }
}

#[cfg(all(target_os = "linux", feature = "x11"))]
struct X11Desktop {
    connection: x11rb::rust_connection::RustConnection,
    window: u32,
    opaque_region: u32,
    transparency: WindowTransparency,
    opaque_policy: OpaqueRegionPolicy,
}

#[cfg(all(target_os = "linux", feature = "x11"))]
impl X11Desktop {
    fn attach(window: &Window, desktop: &DesktopConfig) -> Result<Option<Self>, DesktopError> {
        use raw_window_handle::HasWindowHandle;
        use x11rb::{
            connection::Connection,
            protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode},
            wrapper::ConnectionExt as _,
        };

        let Some(identity) = desktop.x11.as_ref() else {
            return Ok(None);
        };
        let handle = window
            .window_handle()
            .map_err(|error| DesktopError::new(format!("read native window handle: {error}")))?;
        let Some(window_id) = x11_window_id(handle.as_raw())? else {
            return Ok(None);
        };
        let (connection, _) = x11rb::connect(None)
            .map_err(|error| DesktopError::new(format!("connect to X11: {error}")))?;
        let atom = |name: &[u8]| -> Result<u32, DesktopError> {
            connection
                .intern_atom(false, name)
                .map_err(|error| DesktopError::new(format!("intern X11 atom: {error}")))?
                .reply()
                .map_err(|error| DesktopError::new(format!("read X11 atom: {error}")))
                .map(|reply| reply.atom)
        };
        let opacity = atom(b"_NET_WM_WINDOW_OPACITY")?;
        let opacity_region = atom(b"_NET_WM_OPAQUE_REGION")?;
        if identity.set_pid {
            connection
                .change_property32(
                    PropMode::REPLACE,
                    window_id,
                    atom(b"_NET_WM_PID")?,
                    AtomEnum::CARDINAL,
                    &[std::process::id()],
                )
                .map_err(|error| DesktopError::new(format!("set _NET_WM_PID: {error}")))?
                .check()
                .map_err(|error| DesktopError::new(format!("commit _NET_WM_PID: {error}")))?;
        }
        if desktop.transparency == WindowTransparency::Opaque {
            connection
                .change_property32(
                    PropMode::REPLACE,
                    window_id,
                    opacity,
                    AtomEnum::CARDINAL,
                    &[u32::MAX],
                )
                .map_err(|error| DesktopError::new(format!("set _NET_WM_WINDOW_OPACITY: {error}")))?
                .check()
                .map_err(|error| {
                    DesktopError::new(format!("commit _NET_WM_WINDOW_OPACITY: {error}"))
                })?;
        } else {
            connection
                .delete_property(window_id, opacity)
                .map_err(|error| {
                    DesktopError::new(format!("clear _NET_WM_WINDOW_OPACITY: {error}"))
                })?
                .check()
                .map_err(|error| {
                    DesktopError::new(format!("commit _NET_WM_WINDOW_OPACITY: {error}"))
                })?;
        }
        if desktop.opaque_region == OpaqueRegionPolicy::FullWindow
            && desktop.transparency == WindowTransparency::Opaque
        {
            // The initial rectangle is applied by update_size after the window
            // has reported its physical drawable extent.
        } else {
            connection
                .delete_property(window_id, opacity_region)
                .map_err(|error| {
                    DesktopError::new(format!("clear _NET_WM_OPAQUE_REGION: {error}"))
                })?
                .check()
                .map_err(|error| {
                    DesktopError::new(format!("commit _NET_WM_OPAQUE_REGION: {error}"))
                })?;
        }
        connection
            .flush()
            .map_err(|error| DesktopError::new(format!("flush X11 properties: {error}")))?;
        Ok(Some(Self {
            connection,
            window: window_id,
            opaque_region: opacity_region,
            transparency: desktop.transparency,
            opaque_policy: desktop.opaque_region,
        }))
    }

    fn update_size(&self, width: u32, height: u32) -> Result<(), DesktopError> {
        use x11rb::{
            connection::Connection,
            protocol::xproto::{AtomEnum, PropMode},
            wrapper::ConnectionExt as _,
        };
        if self.transparency == WindowTransparency::Transparent
            || self.opaque_policy == OpaqueRegionPolicy::Disabled
        {
            return Ok(());
        }
        let Some(rect) = opaque_region_rect(width, height) else {
            return Ok(());
        };
        self.connection
            .change_property32(
                PropMode::REPLACE,
                self.window,
                self.opaque_region,
                AtomEnum::CARDINAL,
                &rect,
            )
            .map_err(|error| DesktopError::new(format!("set _NET_WM_OPAQUE_REGION: {error}")))?
            .check()
            .map_err(|error| DesktopError::new(format!("commit _NET_WM_OPAQUE_REGION: {error}")))?;
        self.connection
            .flush()
            .map_err(|error| DesktopError::new(format!("flush X11 properties: {error}")))
    }
}

/// Owns one window and its device state; no graphics dependency.
pub struct GameWindow {
    window: Arc<Window>,
    pub input: InputState,
    focused: bool,
    occluded: bool,
    desktop: DesktopManager,
    desktop_size: Option<(u32, u32)>,
    desktop_error: Option<String>,
}

impl GameWindow {
    /// Creates a window with the default desktop policy derived from
    /// `application_id` and applies identity/compositor metadata before returning.
    pub fn create(
        event_loop: &ActiveEventLoop,
        config: &WindowConfig,
    ) -> Result<Self, Box<dyn Error>> {
        let desktop = DesktopConfig::for_application(config.application_id.clone());
        Self::create_with_desktop(event_loop, config, &desktop)
    }

    /// Creates a window with an explicit desktop identity and presentation policy.
    ///
    /// Metadata is applied immediately after native creation. Callers can create
    /// their graphics API after this returns; a resize that happens during that
    /// initialization is reconciled on the next [`Self::process`] call.
    pub fn create_with_desktop(
        event_loop: &ActiveEventLoop,
        config: &WindowConfig,
        desktop: &DesktopConfig,
    ) -> Result<Self, Box<dyn Error>> {
        validate_config(config)?;
        desktop.validate()?;
        let attributes = Window::default_attributes()
            .with_title(&config.title)
            .with_inner_size(LogicalSize::new(config.size.0, config.size.1))
            .with_min_inner_size(LogicalSize::new(
                config.minimum_size.0,
                config.minimum_size.1,
            ))
            .with_resizable(config.resizable)
            .with_transparent(desktop.transparency == WindowTransparency::Transparent)
            .with_fullscreen(config.fullscreen.then_some(Fullscreen::Borderless(None)));
        #[cfg(target_os = "linux")]
        let attributes = {
            use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
            use winit::platform::{
                wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11, x11::WindowType,
            };
            // X11 and Wayland share one `ApplicationName` slot in winit, so the
            // identity that matches the actual display backend must be applied
            // last. Applying both would silently replace the X11 class names with
            // the Wayland application ID.
            match event_loop.display_handle().map(|handle| handle.as_raw()) {
                Ok(RawDisplayHandle::Wayland(_)) => WindowAttributesExtWayland::with_name(
                    attributes,
                    &config.application_id,
                    &config.application_id,
                ),
                Ok(RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_)) => {
                    let (instance, class) = desktop
                        .x11
                        .as_ref()
                        .map(|identity| {
                            (
                                identity.instance_or(&config.application_id),
                                identity.class_or(&config.application_id),
                            )
                        })
                        .unwrap_or((&config.application_id, &config.application_id));
                    let attributes = WindowAttributesExtX11::with_name(attributes, class, instance);
                    if desktop
                        .x11
                        .as_ref()
                        .is_some_and(|identity| identity.normal_window_type)
                    {
                        WindowAttributesExtX11::with_x11_window_type(
                            attributes,
                            vec![WindowType::Normal],
                        )
                    } else {
                        attributes
                    }
                }
                _ => attributes,
            }
        };
        #[cfg(target_os = "windows")]
        let attributes = {
            use winit::platform::windows::WindowAttributesExtWindows;
            WindowAttributesExtWindows::with_class_name(attributes, config.application_id.clone())
        };
        let window = Arc::new(event_loop.create_window(attributes)?);
        let mut desktop_manager = DesktopManager::new(&window, desktop)?;
        let size = window.inner_size();
        let mut desktop_size = None;
        if size.width > 0 && size.height > 0 {
            desktop_manager.update_size(size.width, size.height)?;
            desktop_size = Some((size.width, size.height));
        }
        let focused = window.has_focus();
        Ok(Self {
            window,
            input: InputState::default(),
            focused,
            occluded: false,
            desktop: desktop_manager,
            desktop_size,
            desktop_error: None,
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

    /// Last error from an automatic opaque-region update, if any.
    ///
    /// `process` records this instead of making the event loop fail. A later
    /// successful resize clears it.
    pub fn desktop_error(&self) -> Option<&str> {
        self.desktop_error.as_deref()
    }

    /// Re-applies desktop metadata from the current physical drawable size.
    ///
    /// `process` calls this automatically. It is also useful immediately after
    /// graphics initialization or after a caller-owned resize request.
    pub fn sync_desktop(&mut self) {
        self.reconcile_desktop();
    }

    fn reconcile_desktop(&mut self) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            // Minimized/zero-sized surfaces keep their last valid opaque region;
            // the restore resize will replace it.
            return;
        }
        if self.desktop_size == Some((size.width, size.height)) {
            return;
        }
        match self.desktop.update_size(size.width, size.height) {
            Ok(()) => {
                self.desktop_size = Some((size.width, size.height));
                self.desktop_error = None;
            }
            Err(error) => self.desktop_error = Some(error.to_string()),
        }
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
        self.reconcile_desktop();
        update
    }

    pub fn process_device(&mut self, event: &DeviceEvent) {
        input::process_device_event(&mut self.input, event, self.focused);
    }
}

/// Physical rectangle advertised as opaque, or `None` when nothing is drawable.
///
/// `_NET_WM_OPAQUE_REGION` is in physical pixels. Zero-sized/minimized surfaces
/// deliberately keep the last valid rectangle; the restore resize replaces it.
#[cfg(any(all(target_os = "linux", feature = "x11"), test))]
fn opaque_region_rect(width: u32, height: u32) -> Option<[u32; 4]> {
    (width > 0 && height > 0).then_some([0, 0, width, height])
}

fn validate_config(config: &WindowConfig) -> Result<(), DesktopError> {
    validate_text("application ID", &config.application_id)?;
    // An empty title is valid; only reject the NUL that would truncate winit's
    // native string conversion.
    if config.title.contains('\0') {
        return Err(DesktopError::new("window title must not contain NUL"));
    }
    Ok(())
}

fn validate_text(label: &str, value: &str) -> Result<(), DesktopError> {
    if value.is_empty() {
        return Err(DesktopError::new(format!("{label} must not be empty")));
    }
    if value.contains('\0') {
        return Err(DesktopError::new(format!("{label} must not contain NUL")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_defaults_are_opaque_and_use_distinct_identity_slots() {
        let desktop = DesktopConfig::for_application("missile-command");
        assert_eq!(desktop.transparency, WindowTransparency::Opaque);
        assert_eq!(desktop.opaque_region, OpaqueRegionPolicy::FullWindow);
        let identity = desktop.x11.as_ref().unwrap();
        assert_eq!(identity.instance.as_deref(), Some("missile-command"));
        assert_eq!(identity.class.as_deref(), Some("missile-command"));
        assert!(identity.set_pid);
        assert!(identity.normal_window_type);
    }

    #[test]
    fn distinct_x11_names_and_transparent_policy_validate() {
        let desktop = DesktopConfig {
            transparency: WindowTransparency::Transparent,
            opaque_region: OpaqueRegionPolicy::Disabled,
            x11: Some(X11Identity::with_names("missile-command", "MissileCommand")),
        };
        desktop.validate().unwrap();
        assert_eq!(
            desktop.x11.as_ref().unwrap().instance.as_deref(),
            Some("missile-command")
        );
        assert_eq!(
            desktop.x11.as_ref().unwrap().class.as_deref(),
            Some("MissileCommand")
        );
    }

    #[test]
    fn transparent_window_cannot_claim_a_full_opaque_region() {
        let desktop = DesktopConfig {
            transparency: WindowTransparency::Transparent,
            ..DesktopConfig::for_application("test")
        };
        assert!(desktop.validate().is_err());
        assert!(desktop.transparent().validate().is_ok());
    }

    #[test]
    fn empty_and_nul_suffixed_identities_are_rejected() {
        let mut desktop = DesktopConfig::for_application("");
        assert!(desktop.validate().is_err());
        desktop = DesktopConfig::for_application("app\0class");
        assert!(desktop.validate().is_err());
        assert!(validate_text("application ID", "ok").is_ok());
        assert!(validate_text("application ID", "").is_err());
        assert!(validate_text("application ID", "a\0b").is_err());
    }

    #[test]
    fn opaque_rect_uses_physical_size_and_ignores_zero_sized_surfaces() {
        assert_eq!(opaque_region_rect(0, 720), None);
        assert_eq!(opaque_region_rect(1280, 0), None);
        assert_eq!(opaque_region_rect(0, 0), None);
        assert_eq!(opaque_region_rect(1280, 720), Some([0, 0, 1280, 720]));
        assert_eq!(opaque_region_rect(2560, 1440), Some([0, 0, 2560, 1440]));
    }

    #[test]
    fn missing_x11_names_fall_back_to_the_application_id() {
        let identity = X11Identity::default();
        assert_eq!(identity.instance_or("pong"), "pong");
        assert_eq!(identity.class_or("pong"), "pong");
        let identity = X11Identity::with_names("jackal-native", "JackalNative");
        assert_eq!(identity.instance_or("ignored"), "jackal-native");
        assert_eq!(identity.class_or("ignored"), "JackalNative");
    }

    #[test]
    fn disabling_x11_management_preserves_winit_identity_only() {
        let desktop = DesktopConfig::for_application("snake").without_x11();
        assert!(desktop.x11.is_none());
        assert!(desktop.validate().is_ok());
    }

    #[test]
    fn empty_x11_names_are_rejected() {
        let desktop = DesktopConfig {
            x11: Some(X11Identity::with_names("", "Class")),
            ..DesktopConfig::default()
        };
        assert!(desktop.validate().is_err());
        let desktop = DesktopConfig {
            x11: Some(X11Identity::with_names("instance", "")),
            ..DesktopConfig::default()
        };
        assert!(desktop.validate().is_err());
    }

    #[test]
    fn capabilities_match_the_target_compilation() {
        let capabilities = desktop_capabilities();
        let x11 = cfg!(all(target_os = "linux", feature = "x11"));
        assert_eq!(capabilities.x11_properties, x11);
        assert_eq!(capabilities.x11_pid, x11);
        assert_eq!(capabilities.x11_opacity, x11);
        assert_eq!(capabilities.x11_opaque_region, x11);
        assert_eq!(
            capabilities.wayland_application_id,
            cfg!(target_os = "linux")
        );
        assert_eq!(capabilities.windows_class_name, cfg!(target_os = "windows"));
    }

    #[cfg(all(target_os = "linux", feature = "x11"))]
    #[test]
    fn xlib_and_xcb_window_handles_map_to_x11_ids() {
        use raw_window_handle::{RawWindowHandle, XcbWindowHandle, XlibWindowHandle};
        use std::num::NonZeroU32;

        let xlib = RawWindowHandle::Xlib(XlibWindowHandle::new(0x1234));
        assert_eq!(x11_window_id(xlib).unwrap(), Some(0x1234));
        let xcb = RawWindowHandle::Xcb(XcbWindowHandle::new(NonZeroU32::new(0x5678).unwrap()));
        assert_eq!(x11_window_id(xcb).unwrap(), Some(0x5678));
        #[cfg(target_pointer_width = "64")]
        assert!(
            x11_window_id(RawWindowHandle::Xlib(XlibWindowHandle::new(0x1_0000_0000))).is_err()
        );
    }

    #[test]
    fn empty_titles_remain_valid_but_nul_is_rejected() {
        let config = WindowConfig {
            title: String::new(),
            ..WindowConfig::default()
        };
        assert!(validate_config(&config).is_ok());
        let config = WindowConfig {
            title: "a\0b".into(),
            ..WindowConfig::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn existing_window_config_literal_remains_source_compatible() {
        let config = WindowConfig {
            title: "Pong".into(),
            application_id: "pong".into(),
            size: (640, 400),
            minimum_size: (320, 200),
            fullscreen: false,
            resizable: true,
        };
        assert_eq!(config.application_id, "pong");
        assert_eq!(
            DesktopConfig::for_application("pong")
                .x11
                .unwrap()
                .class
                .as_deref(),
            Some("pong")
        );
    }
}
