//! Test-only access to backends, descriptors and failure injection.
//!
//! The hooks that the `test-support` feature exports serve renderer tests
//! outside this crate; those returning backend types stay crate-private.

use super::*;

impl VulkanicGal {
    #[cfg(test)]
    pub(crate) fn force_buffer_generation_for_test(&mut self, handle: Handle, generation: u32) {
        self.buffers.force_generation(handle, generation);
    }

    /// The recording mock backend, when this GAL was built over one.
    #[cfg(any(test, feature = "test-support"))]
    pub fn mock_backend(&self) -> Option<&crate::backends::mock::MockBackend> {
        self.backend.as_any().downcast_ref()
    }

    /// The recording mock backend, when this GAL was built over one.
    #[cfg(any(test, feature = "test-support"))]
    pub fn mock_backend_mut(&mut self) -> Option<&mut crate::backends::mock::MockBackend> {
        self.backend.as_any_mut().downcast_mut()
    }

    /// The descriptor a live sampler was created with.
    #[cfg(any(test, feature = "test-support"))]
    pub fn sampler_descriptor_for_test(&self, handle: Handle) -> GalResult<&SamplerDesc> {
        self.sampler_descriptor_for_capture(handle)
    }

    /// The descriptor a live resource set was created with.
    #[cfg(any(test, feature = "test-support"))]
    pub fn resource_set_descriptor_for_test(&self, handle: Handle) -> GalResult<&ResourceSetDesc> {
        self.resource_set_descriptor_for_capture(handle)
    }

    /// The descriptor a live graphics pipeline was created with.
    #[cfg(any(test, feature = "test-support"))]
    pub fn graphics_pipeline_descriptor_for_test(
        &self,
        handle: Handle,
    ) -> GalResult<&GraphicsPipelineDesc> {
        self.graphics_pipeline_descriptor_for_capture(handle)
    }

    /// Makes the mock backend's next submission fail (mock GALs only).
    #[cfg(any(test, feature = "test-support"))]
    pub fn fail_next_submit_for_test(&mut self) {
        self.mock_backend_mut()
            .expect("GAL built by test_support::mock_gal")
            .fail_next_submit();
    }

    #[cfg(all(test, feature = "vulkan"))]
    pub(crate) fn vulkan_backend(&self) -> Option<&crate::backends::vulkan::VulkanBackend> {
        self.backend.as_any().downcast_ref()
    }

    /// Clears an acquired presentation frame on the Vulkan backend (Vulkan
    /// test GALs only).
    #[cfg(all(any(test, feature = "test-support"), feature = "vulkan"))]
    pub fn clear_acquired_frame_for_test(
        &mut self,
        frame: crate::frame::FrameId,
        color: [f32; 4],
    ) -> GalResult<()> {
        self.vulkan_backend_mut()
            .ok_or_else(|| GalError::backend("acquired-frame clear requires a Vulkan test GAL"))?
            .clear_acquired_frame_for_test(frame, color)
    }

    #[cfg(all(any(test, feature = "test-support"), feature = "vulkan"))]
    pub(crate) fn vulkan_backend_mut(
        &mut self,
    ) -> Option<&mut crate::backends::vulkan::VulkanBackend> {
        self.backend.as_any_mut().downcast_mut()
    }

    #[cfg(all(test, feature = "wgpu"))]
    pub(crate) fn wgpu_backend(&self) -> Option<&crate::backends::wgpu::WgpuBackend> {
        self.backend.as_any().downcast_ref()
    }

    #[cfg(all(test, feature = "opengl"))]
    pub(crate) fn opengl_backend(&self) -> Option<&crate::backends::opengl::OpenGlBackend> {
        self.backend.as_any().downcast_ref()
    }

    /// Waits for and retires every submission through `id`, returning the
    /// handles whose deferred destruction completed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn retire_through_for_test(&mut self, id: SubmissionId) -> GalResult<Vec<Handle>> {
        self.retire_through(id)
    }
}
