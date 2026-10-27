//! Test-only construction of GALs for code outside `render::vulkanic`.
//!
//! Backends stay private to the GAL; tests elsewhere build a mock, Vulkan or
//! OpenGL GAL through these functions and never name a backend type.

use super::backends;
#[cfg(feature = "opengl")]
use super::backends::opengl::OpenGlBackend;
#[cfg(feature = "vulkan")]
use super::backends::vulkan::VulkanBackend;
#[cfg(all(target_os = "linux", feature = "vulkan"))]
use super::frame::FrameSurfaceDesc;

/// The recording mock backend, for tests that inspect or steer its state
/// through `VulkanicGal::mock_backend{,_mut}`.
pub(crate) use super::backends::mock::MockBackend;
#[cfg(all(target_os = "linux", feature = "vulkan"))]
pub(crate) use super::backends::vulkan::{winit_event_loop, winit_test_window};
use super::error::GalResult;
use super::gal::VulkanicGal;
use super::resources::BackendCapabilities;

/// Whether GPU tests must find a device (`VULKANIC_REQUIRE_GPU=1`, as CI's
/// GPU job sets); otherwise a missing device is an environment gap.
pub(crate) fn require_gpu() -> bool {
    std::env::var("VULKANIC_REQUIRE_GPU").is_ok_and(|value| value == "1")
}

/// Reports that `backend` could not start: an environment gap the test
/// skips, or a failure when GPU tests are required.
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) fn environment_gap(backend: &str, error: &dyn std::fmt::Display) {
    assert!(
        !require_gpu(),
        "{backend} is required (VULKANIC_REQUIRE_GPU=1) but did not start: {error}"
    );
    eprintln!("{backend} unavailable; skipping: {error}");
}

/// A GAL over the recording mock backend with Vulkan-like capabilities.
pub(crate) fn mock_gal() -> VulkanicGal {
    VulkanicGal::new_with_backend(Box::new(MockBackend::default()))
}

/// A GAL over the recording mock backend reporting `capabilities`.
pub(crate) fn mock_gal_with_capabilities(capabilities: BackendCapabilities) -> VulkanicGal {
    VulkanicGal::new_with_backend(Box::new(MockBackend::with_capabilities(capabilities)))
}

/// A GAL over a configured mock backend.
pub(crate) fn gal_with_mock(mock: MockBackend) -> VulkanicGal {
    VulkanicGal::new_with_backend(Box::new(mock))
}

/// A presenting Vulkan GAL on a test window, or the environment gap.
#[cfg(all(target_os = "linux", feature = "vulkan"))]
pub(crate) fn windowed_vulkan_gal(
    label: &str,
    window: &super::backends::vulkan::WinitTestWindow,
    surface: FrameSurfaceDesc,
) -> GalResult<VulkanicGal> {
    Ok(VulkanicGal::new_with_backend(Box::new(
        VulkanBackend::new_windowed_for_test(label, window, surface)?,
    )))
}

/// A GAL over a real headless Vulkan device, or the environment gap.
#[cfg(feature = "vulkan")]
pub(crate) fn vulkan_gal(label: &str) -> GalResult<VulkanicGal> {
    Ok(VulkanicGal::new_with_backend(Box::new(VulkanBackend::new(
        label,
    )?)))
}

/// A GAL over a real headless wgpu device, or the environment gap.
#[cfg(feature = "wgpu")]
pub(crate) fn wgpu_gal(label: &str) -> GalResult<VulkanicGal> {
    VulkanicGal::create(super::create::BackendChoice::Wgpu, label)
}

/// A GAL over a real isolated OpenGL context, or the environment gap.
#[cfg(feature = "opengl")]
pub(crate) fn opengl_gal(label: &str) -> GalResult<VulkanicGal> {
    Ok(VulkanicGal::new_with_backend(Box::new(OpenGlBackend::new(
        label,
    )?)))
}

#[cfg(feature = "vulkan")]
pub(crate) fn vulkan_capabilities() -> BackendCapabilities {
    backends::vulkan_capabilities()
}

pub(crate) fn opengl_capabilities() -> BackendCapabilities {
    backends::opengl_capabilities()
}

pub(crate) fn presentation_capabilities(capabilities: BackendCapabilities) -> BackendCapabilities {
    backends::presentation_capabilities(capabilities)
}

/// Range bound of the GAL's buffer upload capture, for renderers whose
/// resident allocations it must cover.
pub(crate) const BUFFER_UPLOAD_CAPTURE_MAX_RANGES: usize = super::buffer_upload_capture::MAX_RANGES;
