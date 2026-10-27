//! Test construction of GALs, for this crate's tests and, through the
//! `test-support` feature, for renderer tests in dependent crates.
//!
//! Backends stay private to the GAL; tests build a mock, Vulkan, wgpu or
//! OpenGL GAL through these functions and never name a backend type. Enable
//! the feature only from `[dev-dependencies]`.

use super::backends;
#[cfg(feature = "opengl")]
use super::backends::opengl::OpenGlBackend;
#[cfg(feature = "vulkan")]
use super::backends::vulkan::VulkanBackend;
#[cfg(all(target_os = "linux", feature = "vulkan"))]
use super::frame::FrameSurfaceDesc;

/// The recording mock backend, for tests that inspect or steer its state
/// through `VulkanicGal::mock_backend{,_mut}`.
pub use super::backends::mock::MockBackend;
#[cfg(all(target_os = "linux", feature = "vulkan"))]
pub use super::backends::vulkan::{winit_event_loop, winit_test_window, WinitTestWindow};
/// A backend's private identifier for a live object (`MockBackend::live`).
pub use super::backends::BackendToken;
#[cfg(any(feature = "vulkan", feature = "opengl", feature = "wgpu"))]
use super::error::GalResult;
use super::gal::VulkanicGal;
use super::resources::BackendCapabilities;
#[cfg(feature = "vulkan")]
use super::resources::ShaderStage;

/// Whether GPU tests must find a device (`VULKANIC_REQUIRE_GPU=1`, for a
/// run that must not skip them); otherwise a missing device is an
/// environment gap.
pub fn require_gpu() -> bool {
    std::env::var("VULKANIC_REQUIRE_GPU").is_ok_and(|value| value == "1")
}

/// Reports that `backend` could not start: an environment gap the test
/// skips, or a failure when GPU tests are required.
pub fn environment_gap(backend: &str, error: &dyn std::fmt::Display) {
    assert!(
        !require_gpu(),
        "{backend} is required (VULKANIC_REQUIRE_GPU=1) but did not start: {error}"
    );
    eprintln!("{backend} unavailable; skipping: {error}");
}

/// A GAL over the recording mock backend with Vulkan-like capabilities.
pub fn mock_gal() -> VulkanicGal {
    VulkanicGal::new_with_backend(Box::new(MockBackend::default()))
}

/// A GAL over the recording mock backend reporting `capabilities`.
pub fn mock_gal_with_capabilities(capabilities: BackendCapabilities) -> VulkanicGal {
    VulkanicGal::new_with_backend(Box::new(MockBackend::with_capabilities(capabilities)))
}

/// A GAL over a configured mock backend.
pub fn gal_with_mock(mock: MockBackend) -> VulkanicGal {
    VulkanicGal::new_with_backend(Box::new(mock))
}

/// A presenting Vulkan GAL on a test window, or the environment gap.
#[cfg(all(target_os = "linux", feature = "vulkan"))]
pub fn windowed_vulkan_gal(
    label: &str,
    window: &WinitTestWindow,
    surface: FrameSurfaceDesc,
) -> GalResult<VulkanicGal> {
    Ok(VulkanicGal::new_with_backend(Box::new(
        VulkanBackend::new_windowed_for_test(label, window, surface)?,
    )))
}

/// A GAL over a real headless Vulkan device, or the environment gap.
#[cfg(feature = "vulkan")]
pub fn vulkan_gal(label: &str) -> GalResult<VulkanicGal> {
    Ok(VulkanicGal::new_with_backend(Box::new(VulkanBackend::new(
        label,
    )?)))
}

/// A GAL over a real headless wgpu device, or the environment gap.
#[cfg(feature = "wgpu")]
pub fn wgpu_gal(label: &str) -> GalResult<VulkanicGal> {
    VulkanicGal::create(super::create::BackendChoice::Wgpu, label)
}

/// A GAL over a real isolated OpenGL context, or the environment gap.
#[cfg(feature = "opengl")]
pub fn opengl_gal(label: &str) -> GalResult<VulkanicGal> {
    Ok(VulkanicGal::new_with_backend(Box::new(OpenGlBackend::new(
        label,
    )?)))
}

/// The capabilities a Vulkan GAL reports before device limits apply.
pub fn vulkan_capabilities() -> BackendCapabilities {
    backends::vulkan_capabilities()
}

/// The capabilities an OpenGL GAL reports before context limits apply.
pub fn opengl_capabilities() -> BackendCapabilities {
    backends::opengl_capabilities()
}

/// `capabilities` with frame presentation enabled.
pub fn presentation_capabilities(capabilities: BackendCapabilities) -> BackendCapabilities {
    backends::presentation_capabilities(capabilities)
}

/// Compiles GLSL for `stage` to the SPIR-V the Vulkan backend consumes, for
/// tests that check renderer shaders compile.
#[cfg(feature = "vulkan")]
pub fn compile_glsl_for_vulkan(
    stage: ShaderStage,
    source: &str,
    source_name: &str,
) -> Result<Vec<u8>, String> {
    backends::compile_glsl_stage_for_test(stage, source, source_name)
}

/// Range bound of the GAL's buffer upload capture, for renderers whose
/// resident allocations it must cover.
pub const BUFFER_UPLOAD_CAPTURE_MAX_RANGES: usize = super::buffer_upload_capture::MAX_RANGES;
