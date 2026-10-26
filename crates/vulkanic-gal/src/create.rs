//! Creating a GAL: the one public place a backend is chosen.
//!
//! The composition root (the bridge) picks a backend once, from what Java
//! requested, and gets back a `VulkanicGal`. Backend types stay private to
//! `vulkanic`, and after creation every caller sees capabilities only
//! (`BackendCapabilities`), never which backend is running.
//!
//! `VULKANIC_BACKEND` (`vulkan`, `wgpu` or `opengl`) overrides the requested
//! backend for every GAL the process creates through these constructors, so
//! an unmodified application can run on another compiled-in backend. The
//! override still applies once, at creation.

use super::backends::create_backend;
#[cfg(feature = "opengl")]
use super::backends::create_borrowed_opengl_backend;
#[cfg(feature = "vulkan")]
use super::backends::{self, create_native_windowed_vulkan_backend};
use super::error::{GalError, GalResult};
#[cfg(any(feature = "vulkan", feature = "wgpu"))]
use super::frame::FrameSurfaceDesc;
use super::gal::VulkanicGal;

/// The graphics API a new GAL runs on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendChoice {
    /// The Vulkan backend; requires the `vulkan` feature.
    Vulkan,
    /// The OpenGL backend; requires the `opengl` feature.
    OpenGl,
    /// The wgpu backend (Vulkan, Metal or Direct3D 12 through wgpu);
    /// requires the `wgpu` feature.
    Wgpu,
}

impl BackendChoice {
    /// The backend `VULKANIC_BACKEND` names, if it is set and not empty.
    pub fn from_env() -> GalResult<Option<Self>> {
        match std::env::var("VULKANIC_BACKEND") {
            Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
                "" => Ok(None),
                "vulkan" => Ok(Some(Self::Vulkan)),
                "opengl" | "gl" => Ok(Some(Self::OpenGl)),
                "wgpu" => Ok(Some(Self::Wgpu)),
                other => Err(GalError::invalid_argument(format!(
                    "VULKANIC_BACKEND={other} is not vulkan, wgpu or opengl"
                ))),
            },
            Err(_) => Ok(None),
        }
    }

    /// This choice, unless `VULKANIC_BACKEND` overrides it.
    fn resolved(self) -> GalResult<Self> {
        Ok(Self::from_env()?.unwrap_or(self))
    }
}

/// A native window a Vulkan GAL presents to, as handed over by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeWindow {
    /// Host windowing platform id (X11 or Wayland) from the window request.
    pub platform: u32,
    /// Non-zero id that stays stable for the lifetime of the host window.
    pub stable_window_id: u64,
    /// The host's native display handle (for example an X11 `Display*`).
    pub native_display: u64,
    /// The host's native window handle.
    pub native_window: u64,
}

impl VulkanicGal {
    /// A GAL that owns its device and has no window of its own.
    pub fn create(backend: BackendChoice, label: &str) -> GalResult<Self> {
        Ok(Self::new_with_backend(create_backend(
            backend.resolved()?,
            label,
        )?))
    }

    /// An OpenGL GAL that renders through the host's current GL context.
    #[cfg(feature = "opengl")]
    pub fn create_borrowed_opengl(label: &str, stable_window_id: u64) -> GalResult<Self> {
        Ok(Self::new_with_backend(create_borrowed_opengl_backend(
            label,
            stable_window_id,
        )?))
    }

    /// A Vulkan GAL that presents to the host's native window.
    ///
    /// # Safety
    /// The native handles must be valid for their declared platform, and the
    /// window/display must remain alive until the returned GAL is destroyed.
    /// Prefer `create_windowed_vulkan`, which retains the window owner.
    #[cfg(feature = "vulkan")]
    pub unsafe fn create_native_windowed_vulkan(
        label: &str,
        window: NativeWindow,
        surface: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        let backend = match BackendChoice::Vulkan.resolved()? {
            BackendChoice::Vulkan => create_native_windowed_vulkan_backend(
                label,
                window.platform,
                window.stable_window_id,
                window.native_display,
                window.native_window,
                surface,
            )?,
            #[cfg(feature = "wgpu")]
            BackendChoice::Wgpu => unsafe {
                backends::create_native_windowed_wgpu_backend(
                    label,
                    window.platform,
                    window.native_display,
                    window.native_window,
                    surface,
                )?
            },
            other => return Err(windowless(other)),
        };
        Ok(Self::new_with_backend(backend))
    }
}

/// Turns GPU frame timestamps on or off for every backend that records them.
pub fn set_gpu_timestamps_requested(requested: bool) {
    #[cfg(feature = "vulkan")]
    backends::set_gpu_timestamps_requested(requested);
    #[cfg(not(feature = "vulkan"))]
    let _ = requested;
}

/// The error for a backend that cannot create a window surface.
fn windowless(backend: BackendChoice) -> GalError {
    match backend {
        BackendChoice::OpenGl => GalError::unsupported_feature(
            "OpenGL GALs do not create window surfaces; use create_borrowed_opengl",
        ),
        other => GalError::unsupported_feature(format!(
            "{other:?} backend support is disabled in this build"
        )),
    }
}

#[cfg(any(feature = "vulkan", feature = "wgpu"))]
impl VulkanicGal {
    /// Creates a presenting GAL on `backend`, retaining ownership of the
    /// window until the GAL has destroyed its surface and device.
    ///
    /// Vulkan and wgpu create surfaces; OpenGL GALs borrow a host context
    /// instead (`create_borrowed_opengl`). Call on the window's owning thread.
    pub fn create_windowed<
        W: raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle + 'static,
    >(
        backend: BackendChoice,
        label: &str,
        window: std::sync::Arc<W>,
        surface: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        let backend = match backend.resolved()? {
            #[cfg(feature = "vulkan")]
            BackendChoice::Vulkan => {
                backends::create_owned_windowed_vulkan_backend(label, window, surface)?
            }
            #[cfg(feature = "wgpu")]
            BackendChoice::Wgpu => {
                super::backends::create_owned_windowed_wgpu_backend(label, window, surface)?
            }
            other => return Err(windowless(other)),
        };
        Ok(Self::new_with_backend(backend))
    }

    /// Creates a Vulkan device and surface, retaining ownership of the window.
    ///
    /// Supports the native platforms supported by `ash-window`, including X11,
    /// Wayland, Win32 and Metal (macOS requires a Vulkan portability driver).
    /// Call on the window's owning thread. The window stays alive until the GAL
    /// has destroyed its surface and device.
    #[cfg(feature = "vulkan")]
    pub fn create_windowed_vulkan<
        W: raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle + 'static,
    >(
        label: &str,
        window: std::sync::Arc<W>,
        surface: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        Self::create_windowed(BackendChoice::Vulkan, label, window, surface)
    }

    /// Creates a wgpu device and surface (Vulkan, Metal or Direct3D 12),
    /// retaining ownership of the window. See `create_windowed`.
    #[cfg(feature = "wgpu")]
    pub fn create_windowed_wgpu<
        W: raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle + 'static,
    >(
        label: &str,
        window: std::sync::Arc<W>,
        surface: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        Self::create_windowed(BackendChoice::Wgpu, label, window, surface)
    }
}
