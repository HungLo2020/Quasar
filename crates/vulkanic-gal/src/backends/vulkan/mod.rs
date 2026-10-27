#[cfg(test)]
pub(in crate::backends) mod conformance;
mod device;
mod lowering;
pub(crate) use lowering::set_gpu_timestamps_requested;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod renderdoc;
mod resources;
mod swapchain;
mod trace;

use std::sync::{Arc, Mutex, MutexGuard};

use super::{
    graphics_backend_lock, presentation_capabilities, vulkan_capabilities, Backend,
    BackendCreateDesc, BackendRuntimeMetrics, BackendToken, CompletedHostRead,
};
use crate::commands::ValidatedSubmissionBatch;
use crate::error::{GalError, GalResult};
use crate::frame::{
    AcquiredFrame, FrameAcquireDesc, FrameAcquireStatus, FrameResizeDesc, FrameResizeResult,
    FrameSurfaceDesc, PresentFrameDesc, PresentedFrame,
};
use crate::handles::{Handle, HandleKind};
use crate::resources::BackendCapabilities;
use crate::sync::SubmissionId;

use self::device::{ValidationMode, VulkanContext};
use self::lowering::SubmissionLowerer;
use self::resources::VulkanObjects;
pub(crate) use self::swapchain::SurfaceOwner;
use self::swapchain::VulkanSwapchain;

pub(crate) const WINDOW_PLATFORM_X11: u32 = 1;
pub(crate) const WINDOW_PLATFORM_WAYLAND: u32 = 2;

struct NativeWindowSurface {
    platform: u32,
    native_display: u64,
    native_window: u64,
    stable_window_id: u64,
}

impl SurfaceOwner for NativeWindowSurface {
    fn required_instance_extensions(&self) -> Vec<&'static std::ffi::CStr> {
        match self.platform {
            WINDOW_PLATFORM_X11 => vec![ash::khr::surface::NAME, ash::khr::xlib_surface::NAME],
            WINDOW_PLATFORM_WAYLAND => {
                vec![ash::khr::surface::NAME, ash::khr::wayland_surface::NAME]
            }
            _ => vec![ash::khr::surface::NAME],
        }
    }

    fn create_surface(
        &self,
        entry: &ash::Entry,
        instance: &ash::Instance,
    ) -> GalResult<ash::vk::SurfaceKHR> {
        use crate::error::{GalError, StatusCode};

        if self.native_display == 0 || self.native_window == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "windowed Vulkan context requires non-zero native display and window handles",
            ));
        }
        match self.platform {
            WINDOW_PLATFORM_X11 => {
                let loader = ash::khr::xlib_surface::Instance::new(entry, instance);
                let info = ash::vk::XlibSurfaceCreateInfoKHR::default()
                    .dpy((self.native_display as usize as *mut ash::vk::Display).cast())
                    // Xlib's Window is a C unsigned long: 64-bit on Linux,
                    // 32-bit on Windows (which never takes this X11 path).
                    .window(self.native_window as ash::vk::Window);
                unsafe { loader.create_xlib_surface(&info, None) }.map_err(|error| {
                    GalError::backend(format!("failed to create X11 Vulkan surface: {error:?}"))
                })
            }
            WINDOW_PLATFORM_WAYLAND => {
                let loader = ash::khr::wayland_surface::Instance::new(entry, instance);
                let info = ash::vk::WaylandSurfaceCreateInfoKHR::default()
                    .display(self.native_display as usize as *mut std::ffi::c_void)
                    .surface(self.native_window as usize as *mut std::ffi::c_void);
                unsafe { loader.create_wayland_surface(&info, None) }.map_err(|error| {
                    GalError::backend(format!(
                        "failed to create Wayland Vulkan surface: {error:?}"
                    ))
                })
            }
            other => Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unsupported window platform {other} for Rust Vulkan presentation"),
            )),
        }
    }

    fn stable_window_id(&self) -> u64 {
        self.stable_window_id
    }
}

pub(crate) struct VulkanBackend {
    context: Arc<VulkanContext>,
    objects: VulkanObjects,
    lowerer: Mutex<SubmissionLowerer>,
    swapchain: Option<VulkanSwapchain>,
    presentation_capable: bool,
    _global_lock: MutexGuard<'static, ()>,
    // Last field: surface/device resources must die before the native window.
    _window_owner: Option<Box<dyn std::any::Any>>,
}

impl VulkanBackend {
    pub(crate) fn new(label: &str) -> GalResult<Self> {
        let global_lock = graphics_backend_lock();
        let context = VulkanContext::new(label, ValidationMode::from_env())?;
        Ok(Self {
            objects: VulkanObjects::new(context.clone()),
            lowerer: Mutex::new(SubmissionLowerer::new(context.clone())),
            context,
            swapchain: None,
            presentation_capable: false,
            _global_lock: global_lock,
            _window_owner: None,
        })
    }

    pub(in crate::backends) fn new_windowed(
        label: &str,
        surface_owner: &dyn SurfaceOwner,
        surface_desc: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        let global_lock = graphics_backend_lock();
        let context = VulkanContext::new_with_surface(
            label,
            ValidationMode::from_env(),
            Some(surface_owner),
        )?;
        let stable_window_id = surface_owner.stable_window_id();
        let swapchain = VulkanSwapchain::new(context.clone(), surface_desc, stable_window_id)?;
        let _ = swapchain.stable_window_id();
        Ok(Self {
            objects: VulkanObjects::new(context.clone()),
            lowerer: Mutex::new(SubmissionLowerer::new(context.clone())),
            context,
            swapchain: Some(swapchain),
            presentation_capable: true,
            _global_lock: global_lock,
            _window_owner: None,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn new_windowed_for_test(
        label: &str,
        surface_owner: &dyn SurfaceOwner,
        surface_desc: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        Self::new_windowed(label, surface_owner, surface_desc)
    }

    pub(crate) fn new_native_windowed(
        label: &str,
        platform: u32,
        stable_window_id: u64,
        native_display: u64,
        native_window: u64,
        surface_desc: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        let surface = NativeWindowSurface {
            platform,
            native_display,
            native_window,
            stable_window_id,
        };
        Self::new_windowed(label, &surface, surface_desc)
    }

    pub(crate) fn new_owned_window<
        W: raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle + 'static,
    >(
        label: &str,
        window: Arc<W>,
        desc: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        let display = window
            .display_handle()
            .map_err(|e| GalError::backend(e.to_string()))?
            .as_raw();
        let raw_window = window
            .window_handle()
            .map_err(|e| GalError::backend(e.to_string()))?
            .as_raw();
        let extensions = ash_window::enumerate_required_extensions(display)
            .map_err(|e| GalError::backend(format!("unsupported native display: {e:?}")))?;
        struct Surface {
            display: raw_window_handle::RawDisplayHandle,
            window: raw_window_handle::RawWindowHandle,
            extensions: &'static [*const std::ffi::c_char],
        }
        impl SurfaceOwner for Surface {
            fn required_instance_extensions(&self) -> Vec<&'static std::ffi::CStr> {
                self.extensions
                    .iter()
                    .map(|p| unsafe { std::ffi::CStr::from_ptr(*p) })
                    .collect()
            }
            fn create_surface(
                &self,
                entry: &ash::Entry,
                instance: &ash::Instance,
            ) -> GalResult<ash::vk::SurfaceKHR> {
                // The caller retains the Arc<W> until all Vulkan resources are destroyed.
                unsafe {
                    ash_window::create_surface(entry, instance, self.display, self.window, None)
                }
                .map_err(|e| GalError::backend(format!("native surface creation failed: {e:?}")))
            }
            fn stable_window_id(&self) -> u64 {
                1
            }
        }
        let surface = Surface {
            display,
            window: raw_window,
            extensions,
        };
        let mut backend = Self::new_windowed(label, &surface, desc)?;
        backend._window_owner = Some(Box::new(window));
        Ok(backend)
    }

    fn host_reads_since(&self, after: SubmissionId) -> Vec<CompletedHostRead> {
        self.lowerer
            .lock()
            .map(|lowerer| lowerer.completed_host_reads_since(after).cloned().collect())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn staging_chunks_created_for_test(&self) -> u64 {
        self.lowerer
            .lock()
            .map(|lowerer| lowerer.metrics().staging_chunks_created)
            .unwrap_or_default()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn clear_acquired_frame_for_test(
        &mut self,
        frame: crate::frame::FrameId,
        color: [f32; 4],
    ) -> GalResult<()> {
        let Some(swapchain) = &mut self.swapchain else {
            return Err(crate::error::GalError::backend(
                "Vulkan backend has no test swapchain",
            ));
        };
        swapchain.clear_acquired_for_test(frame, color)
    }
}

impl Backend for VulkanBackend {
    fn selected_backend(&self) -> crate::create::GraphicsBackend {
        crate::create::GraphicsBackend::Vulkan
    }

    fn device_info(&self) -> crate::create::DeviceInfo {
        self.context.device_info()
    }

    fn set_gpu_profile_tag(&mut self, handle: Handle, tag: crate::resources::GpuProfileTag) {
        self.objects.set_profile_tag(handle, tag);
    }

    fn set_gpu_profile_scope_names(&mut self, names: fn(u8) -> &'static str) {
        if let Ok(mut lowerer) = self.lowerer.lock() {
            lowerer.set_statistics_scope_names(names);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn begin_debug_capture(&self) -> Option<Box<dyn std::any::Any>> {
        renderdoc::RenderDocFrame::start_if_requested()
            .map(|frame| Box::new(frame) as Box<dyn std::any::Any>)
    }

    fn capabilities(&self) -> BackendCapabilities {
        let mut capabilities = vulkan_capabilities();
        let properties = unsafe {
            self.context
                .instance
                .get_physical_device_properties(self.context.physical_device)
        };
        capabilities.limits.uniform_buffer_offset_alignment =
            properties.limits.min_uniform_buffer_offset_alignment.max(1);
        capabilities.limits.max_texture_extent_2d = capabilities
            .limits
            .max_texture_extent_2d
            .min(properties.limits.max_image_dimension2_d);
        capabilities.limits.max_texture_extent_3d = capabilities
            .limits
            .max_texture_extent_3d
            .min(properties.limits.max_image_dimension3_d);
        capabilities.features.texture_3d &= properties.limits.max_image_dimension3_d != 0;
        if self.presentation_capable {
            presentation_capabilities(capabilities)
        } else {
            capabilities
        }
    }

    fn create(&mut self, handle: Handle, desc: BackendCreateDesc<'_>) -> GalResult<BackendToken> {
        let _zone = trace::Zone::new("vulkan.backend.resource.create");
        if let BackendCreateDesc::FrameTarget(frame_target) = desc {
            let Some(swapchain) = &self.swapchain else {
                return Err(crate::error::GalError::unsupported_feature(
                    "Vulkan frame targets require a presentation-capable backend",
                ));
            };
            return self.objects.create_frame_target_from_swapchain(
                handle,
                frame_target,
                |token| swapchain.frame_target_object(frame_target, token),
            );
        }
        self.objects.create(handle, desc)
    }

    fn destroy(&mut self, handle: Handle, kind: HandleKind, token: BackendToken) -> GalResult<()> {
        let _zone = trace::Zone::new("vulkan.backend.resource.destroy");
        self.objects.destroy(handle, kind, token)
    }

    fn encode_passes(&mut self, batch: &ValidatedSubmissionBatch) -> GalResult<()> {
        let _zone = trace::Zone::new("vulkan.backend.lowering.encode");
        if let Some(swapchain) = &self.swapchain {
            let mut frame_targets = Vec::new();
            for list in &batch.command_lists {
                for op in &list.operations {
                    if let crate::commands::CommandOp::BeginPass { target, .. } = op {
                        if target.kind() == Some(crate::handles::HandleKind::FrameTarget)
                            && !frame_targets.contains(target)
                        {
                            frame_targets.push(*target);
                        }
                    }
                }
            }
            for target in frame_targets {
                self.objects.refresh_frame_target_from_swapchain(
                    target,
                    |token, render_target, extent, color_format| {
                        swapchain.frame_target_object_for_render_target(
                            render_target,
                            extent,
                            color_format,
                            token,
                        )
                    },
                )?;
            }
        }
        self.lowerer
            .lock()
            .map_err(|_| crate::error::GalError::backend("Vulkan lowerer lock poisoned"))?
            .encode(&self.objects, batch)
    }

    fn submit(&mut self, id: SubmissionId, _batch: &ValidatedSubmissionBatch) -> GalResult<()> {
        let _zone = trace::Zone::new("vulkan.backend.submit");
        trace::message(format_args!("gal.submission backend=vulkan id={}", id.0));
        let mut lowerer = self
            .lowerer
            .lock()
            .map_err(|_| crate::error::GalError::backend("Vulkan lowerer lock poisoned"))?;
        // The first submission that touches an acquired image waits on its
        // acquire semaphore, whatever the op (a capture copy reads the image
        // too). Later submissions are ordered after it on the queue.
        let images: smallvec::SmallVec<[u32; 1]> =
            lowerer.pending_acquired_images().iter().copied().collect();
        let acquire_waits: smallvec::SmallVec<[ash::vk::Semaphore; 1]> = images
            .iter()
            .filter_map(|&image_index| {
                self.swapchain
                    .as_ref()?
                    .acquire_semaphore_for_image(image_index)
            })
            .collect();
        lowerer.submit(id, acquire_waits)?;
        if let Some(swapchain) = self.swapchain.as_mut() {
            for image_index in images {
                swapchain.take_acquire_semaphore_for_image(image_index);
            }
        }
        Ok(())
    }

    fn completed_submission(&self) -> SubmissionId {
        self.lowerer
            .lock()
            .map(|mut lowerer| lowerer.completed_submission())
            .unwrap_or(SubmissionId(0))
    }

    fn retire(&mut self, completed: SubmissionId) -> GalResult<()> {
        let _zone = trace::Zone::new("vulkan.backend.retire");
        let mut lowerer = self
            .lowerer
            .lock()
            .map_err(|_| crate::error::GalError::backend("Vulkan lowerer lock poisoned"))?;
        let polled = lowerer.completed_submission();
        lowerer.retire(std::cmp::max(completed, polled))
    }

    fn completed_host_reads(&self) -> Vec<CompletedHostRead> {
        self.host_reads_since(SubmissionId(0))
    }

    fn completed_host_reads_since(&self, after: SubmissionId) -> Vec<CompletedHostRead> {
        self.host_reads_since(after)
    }

    fn runtime_metrics(&self) -> BackendRuntimeMetrics {
        let lowering = self
            .lowerer
            .lock()
            .map(|lowerer| lowerer.metrics())
            .unwrap_or_default();
        let swapchain = self
            .swapchain
            .as_ref()
            .map(|swapchain| swapchain.metrics())
            .unwrap_or_default();
        BackendRuntimeMetrics {
            native_command_buffer_alloc_nanos: lowering.command_buffer_alloc_nanos,
            native_command_buffer_begin_nanos: lowering.command_buffer_begin_nanos,
            native_command_recording_nanos: lowering.command_recording_nanos,
            native_command_buffer_end_nanos: lowering.command_buffer_end_nanos,
            native_queue_submit_nanos: lowering.queue_submit_nanos,
            native_timeline_poll_nanos: lowering.timeline_poll_nanos,
            native_timeline_wait_nanos: lowering.timeline_wait_nanos,
            native_device_wait_idle_nanos: lowering.device_wait_idle_nanos,
            native_acquire_nanos: swapchain.acquire_nanos,
            native_present_nanos: swapchain.present_nanos,
            native_present_wait_nanos: swapchain.present_wait_nanos,
            native_command_buffers_allocated: lowering.command_buffers_allocated,
            native_command_buffers_freed: lowering.command_buffers_freed,
            native_wait_count: lowering.wait_count,
            native_device_wait_idle_count: lowering.device_wait_idle_count,
            native_present_mode: swapchain.present_mode,
            native_requested_present_mode: swapchain.requested_present_mode,
            native_supported_present_modes: swapchain.supported_present_modes,
            native_present_mode_fallback_reason: swapchain.present_mode_fallback_reason,
            native_acquired_image_index: swapchain.acquired_image_index,
            native_swapchain_generation: swapchain.swapchain_generation,
            native_swapchain_image_count: swapchain.swapchain_image_count,
            native_surface_min_image_count: swapchain.surface_min_image_count,
            native_surface_max_image_count: swapchain.surface_max_image_count,
            native_configured_frames_in_flight: swapchain.configured_frames_in_flight,
            native_images_in_flight: swapchain.images_in_flight,
            native_available_frame_slots: swapchain.available_frame_slots,
            gpu_timestamp_status: lowering.gpu_timestamp_status,
            gpu_scope_nanos: lowering.gpu_scope_nanos,
            gpu_frame_total_nanos: lowering.gpu_frame_total_nanos,
            ..BackendRuntimeMetrics::default()
        }
    }

    fn configure_frame_surface(&mut self, desc: &FrameSurfaceDesc) -> GalResult<()> {
        self.retire_before_surface_mutation()?;
        let Some(swapchain) = &mut self.swapchain else {
            return Err(crate::error::GalError::unsupported_feature(
                "Vulkan backend was not created with a presentation surface",
            ));
        };
        swapchain.configure(desc)
    }

    fn acquire_frame(&mut self, desc: &FrameAcquireDesc) -> GalResult<AcquiredFrame> {
        if self.swapchain.is_none() {
            return Err(crate::error::GalError::unsupported_feature(
                "Vulkan backend was not created with a presentation surface",
            ));
        }
        let acquire_semaphore = {
            let mut lowerer = self
                .lowerer
                .lock()
                .map_err(|_| crate::error::GalError::backend("Vulkan lowerer lock poisoned"))?;
            lowerer.checkout_acquire_semaphore()?
        };
        let result = self
            .swapchain
            .as_mut()
            .expect("swapchain presence checked")
            .acquire(desc, acquire_semaphore);
        let release_unsubmitted = match &result {
            Ok(frame) => matches!(
                frame.status,
                FrameAcquireStatus::Minimized | FrameAcquireStatus::Resized
            ),
            Err(_) => true,
        };
        if release_unsubmitted {
            self.lowerer
                .lock()
                .map_err(|_| crate::error::GalError::backend("Vulkan lowerer lock poisoned"))?
                .release_unsubmitted_acquire_semaphore(acquire_semaphore);
        }
        result
    }

    fn resize_frame_surface(&mut self, desc: &FrameResizeDesc) -> GalResult<FrameResizeResult> {
        self.retire_before_surface_mutation()?;
        let Some(swapchain) = &mut self.swapchain else {
            return Err(crate::error::GalError::unsupported_feature(
                "Vulkan backend was not created with a presentation surface",
            ));
        };
        swapchain.resize(desc)
    }

    fn present_frame(&mut self, desc: &PresentFrameDesc) -> GalResult<PresentedFrame> {
        let Some(swapchain) = &mut self.swapchain else {
            return Err(crate::error::GalError::unsupported_feature(
                "Vulkan backend was not created with a presentation surface",
            ));
        };
        let image_index = swapchain.acquired_image_index(desc.frame)?;
        let mut lowerer = self
            .lowerer
            .lock()
            .map_err(|_| crate::error::GalError::backend("Vulkan lowerer lock poisoned"))?;
        let present_semaphore = lowerer.present_semaphore_for(desc.wait_for, image_index);
        let used_gpu_wait = present_semaphore.is_some();
        let mut presented = swapchain.present(desc, present_semaphore)?;
        if used_gpu_wait {
            // queuePresent now waits on the render-finished binary semaphore;
            // polling here retires only work that is already complete and does
            // not turn presentation back into a CPU timeline wait.
            lowerer.complete_present(desc.wait_for);
            presented.completed_submission = lowerer.completed_submission();
        } else {
            // Preserve the explicit synchronous contract for test-only or
            // non-frame submissions that have no render-finished semaphore.
            lowerer.retire(desc.wait_for)?;
            presented.completed_submission = desc.wait_for;
        }
        Ok(presented)
    }

    fn cancel_frame(&mut self, frame: crate::frame::FrameId) -> GalResult<()> {
        let Some(swapchain) = &mut self.swapchain else {
            return Err(crate::error::GalError::unsupported_feature(
                "Vulkan backend was not created with a presentation surface",
            ));
        };
        let acquire_semaphore = swapchain.acquire_semaphore_for_frame(frame);
        swapchain.cancel(frame)?;
        if let Some(semaphore) = acquire_semaphore {
            self.lowerer
                .lock()
                .map_err(|_| crate::error::GalError::backend("Vulkan lowerer lock poisoned"))?
                .discard_quiescent_acquire_semaphore(semaphore);
        }
        Ok(())
    }

    fn shutdown_frame_surface(&mut self) -> GalResult<()> {
        self.retire_before_surface_mutation()?;
        if let Some(swapchain) = &mut self.swapchain {
            swapchain.shutdown();
        }
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl VulkanBackend {
    fn retire_before_surface_mutation(&self) -> GalResult<()> {
        let mut lowerer = self.lowerer.lock().map_err(|_| {
            crate::error::GalError::backend("Vulkan lowerer lock poisoned before surface mutation")
        })?;
        lowerer.retire_all_in_flight()
    }
}

impl Drop for VulkanBackend {
    fn drop(&mut self) {
        let _ = self.context.wait_idle();
        if let Some(swapchain) = &mut self.swapchain {
            swapchain.shutdown();
        }
        self.objects.destroy_all();
    }
}

/// A native winit window that a test Vulkan GAL presents to.
#[cfg(all(any(test, feature = "test-support"), target_os = "linux"))]
pub struct WinitTestWindow {
    _window: winit::window::Window,
    raw_display: raw_window_handle::RawDisplayHandle,
    raw_window: raw_window_handle::RawWindowHandle,
    stable_window_id: u64,
}

#[cfg(all(any(test, feature = "test-support"), target_os = "linux"))]
impl SurfaceOwner for WinitTestWindow {
    fn required_instance_extensions(&self) -> Vec<&'static std::ffi::CStr> {
        match self.raw_display {
            raw_window_handle::RawDisplayHandle::Xlib(_) => {
                vec![ash::khr::surface::NAME, ash::khr::xlib_surface::NAME]
            }
            raw_window_handle::RawDisplayHandle::Xcb(_) => {
                vec![ash::khr::surface::NAME, ash::khr::xcb_surface::NAME]
            }
            raw_window_handle::RawDisplayHandle::Wayland(_) => {
                vec![ash::khr::surface::NAME, ash::khr::wayland_surface::NAME]
            }
            _ => vec![ash::khr::surface::NAME],
        }
    }

    fn create_surface(
        &self,
        entry: &ash::Entry,
        instance: &ash::Instance,
    ) -> GalResult<ash::vk::SurfaceKHR> {
        match (self.raw_display, self.raw_window) {
            (
                raw_window_handle::RawDisplayHandle::Xlib(display),
                raw_window_handle::RawWindowHandle::Xlib(window),
            ) => {
                let display = display.display.ok_or_else(|| {
                    crate::error::GalError::backend("winit Xlib display handle is empty")
                })?;
                let loader = ash::khr::xlib_surface::Instance::new(entry, instance);
                let info = ash::vk::XlibSurfaceCreateInfoKHR::default()
                    .dpy(display.as_ptr().cast::<ash::vk::Display>())
                    .window(window.window);
                unsafe { loader.create_xlib_surface(&info, None) }.map_err(|error| {
                    crate::error::GalError::backend(format!(
                        "failed to create winit Xlib Vulkan surface: {error:?}"
                    ))
                })
            }
            (
                raw_window_handle::RawDisplayHandle::Xcb(display),
                raw_window_handle::RawWindowHandle::Xcb(window),
            ) => {
                let connection = display.connection.ok_or_else(|| {
                    crate::error::GalError::backend("winit XCB connection handle is empty")
                })?;
                let loader = ash::khr::xcb_surface::Instance::new(entry, instance);
                let info = ash::vk::XcbSurfaceCreateInfoKHR::default()
                    .connection(connection.as_ptr().cast::<ash::vk::xcb_connection_t>())
                    .window(window.window.get());
                unsafe { loader.create_xcb_surface(&info, None) }.map_err(|error| {
                    crate::error::GalError::backend(format!(
                        "failed to create winit XCB Vulkan surface: {error:?}"
                    ))
                })
            }
            (
                raw_window_handle::RawDisplayHandle::Wayland(display),
                raw_window_handle::RawWindowHandle::Wayland(window),
            ) => {
                let loader = ash::khr::wayland_surface::Instance::new(entry, instance);
                let info = ash::vk::WaylandSurfaceCreateInfoKHR::default()
                    .display(display.display.as_ptr())
                    .surface(window.surface.as_ptr());
                unsafe { loader.create_wayland_surface(&info, None) }.map_err(|error| {
                    crate::error::GalError::backend(format!(
                        "failed to create winit Wayland Vulkan surface: {error:?}"
                    ))
                })
            }
            _ => Err(crate::error::GalError::backend(
                "winit produced unsupported Linux window/display handle pair for Vulkan",
            )),
        }
    }

    fn stable_window_id(&self) -> u64 {
        self.stable_window_id
    }
}

/// An event loop for test windows, usable from any test thread.
#[cfg(all(any(test, feature = "test-support"), target_os = "linux"))]
pub fn winit_event_loop() -> GalResult<winit::event_loop::EventLoop<()>> {
    use winit::event_loop::EventLoop;

    let mut builder = EventLoop::builder();
    winit::platform::x11::EventLoopBuilderExtX11::with_any_thread(&mut builder, true);
    winit::platform::wayland::EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
    builder.build().map_err(|error| {
        crate::error::GalError::backend(format!(
            "failed to create winit event loop for windowed conformance: {error}"
        ))
    })
}

/// A visible `width`×`height` test window; `stable_window_id` names its
/// surface in diagnostics.
#[cfg(all(any(test, feature = "test-support"), target_os = "linux"))]
pub fn winit_test_window(
    event_loop: &winit::event_loop::EventLoop<()>,
    width: u32,
    height: u32,
    stable_window_id: u64,
) -> GalResult<WinitTestWindow> {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use winit::dpi::PhysicalSize;
    use winit::window::Window;

    #[allow(deprecated)]
    let window = event_loop
        .create_window(
            Window::default_attributes()
                .with_title("MattMC VulkanicGAL windowed conformance")
                .with_inner_size(PhysicalSize::new(width, height))
                .with_visible(true),
        )
        .map_err(|error| {
            crate::error::GalError::backend(format!(
                "failed to create winit window for conformance: {error}"
            ))
        })?;
    let raw_display = window
        .display_handle()
        .map_err(|error| {
            crate::error::GalError::backend(format!("failed to read winit display handle: {error}"))
        })?
        .as_raw();
    let raw_window = window
        .window_handle()
        .map_err(|error| {
            crate::error::GalError::backend(format!("failed to read winit window handle: {error}"))
        })?
        .as_raw();
    Ok(WinitTestWindow {
        _window: window,
        raw_display,
        raw_window,
        stable_window_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandListDesc, CommandOp, SubmissionBatch};
    use crate::frame::{
        FrameAcquireDesc, FrameAcquireStatus, FrameCorrelationId, FrameResizeDesc,
        FrameSurfaceDesc, PresentFrameDesc, PresentMode,
    };
    use crate::gal::VulkanicGal;
    use crate::resources::{
        AccessFlags, BufferDesc, BufferUsage, Extent3d, MemoryDomain, PipelineLayoutDesc,
        PipelineStageFlags, ResourceBinding, ResourceBindingDesc, ResourceBindingKind,
        ResourceLayoutDesc, ResourceSetDesc, TextureFormat,
    };

    #[test]
    fn vulkan_backend_can_bootstrap_or_reports_environment_gap() {
        match VulkanBackend::new("VulkanicGAL backend bootstrap test") {
            Ok(mut backend) => {
                let capabilities = backend.capabilities();
                let properties = unsafe {
                    backend
                        .context
                        .instance
                        .get_physical_device_properties(backend.context.physical_device)
                };
                assert!(
                    capabilities.limits.max_texture_extent_2d
                        <= properties.limits.max_image_dimension2_d
                );
                assert!(
                    capabilities.limits.max_texture_extent_3d
                        <= properties.limits.max_image_dimension3_d
                        || !capabilities.features.texture_3d
                );
                backend
                    .retire(SubmissionId(0))
                    .expect("retirement should be no-op");
            }
            Err(error) => {
                crate::test_support::environment_gap("Vulkan", &error);
                let message = error.to_string();
                assert!(
                    message.contains("Vulkan")
                        || message.contains("vulkan")
                        || message.contains("physical device"),
                    "unexpected bootstrap failure: {message}"
                );
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn vulkan_windowed_conformance_acquires_presents_resizes_or_reports_environment_gap() {
        if crate::env::var("VULKANIC_RUN_WINDOWED_CONFORMANCE")
            .map(|value| value != "1" && !value.eq_ignore_ascii_case("true"))
            .unwrap_or(true)
        {
            return;
        }
        let event_loop = match winit_event_loop() {
            Ok(event_loop) => event_loop,
            Err(error) => {
                assert!(
                    error.to_string().contains("window")
                        || error.to_string().contains("display")
                        || error.to_string().contains("event loop"),
                    "unexpected event-loop creation failure: {error}"
                );
                return;
            }
        };
        let window = match winit_test_window(&event_loop, 64, 48, 1) {
            Ok(window) => window,
            Err(error) => {
                assert!(
                    error.to_string().contains("window")
                        || error.to_string().contains("display")
                        || error.to_string().contains("surface"),
                    "unexpected window creation failure: {error}"
                );
                return;
            }
        };
        let surface = FrameSurfaceDesc {
            label: "test-window".to_string(),
            extent: Extent3d {
                width: 64,
                height: 48,
                depth: 1,
            },
            color_format: TextureFormat::Bgra8Unorm,
            present_mode: PresentMode::Fifo,
            max_frames_in_flight: 2,
        };
        let backend = match VulkanBackend::new_windowed(
            "MattMC windowed VulkanicGAL conformance",
            &window,
            surface,
        ) {
            Ok(backend) => backend,
            Err(error) => {
                crate::test_support::environment_gap("Vulkan", &error);
                let message = error.to_string();
                assert!(
                    message.contains("Vulkan")
                        || message.contains("vulkan")
                        || message.contains("surface")
                        || message.contains("present"),
                    "unexpected windowed Vulkan bootstrap failure: {message}"
                );
                return;
            }
        };
        let mut gal = VulkanicGal::new_with_backend(Box::new(backend));
        for frame_index in 0..2 {
            let acquired = gal
                .acquire_frame(FrameAcquireDesc {
                    correlation_id: FrameCorrelationId(frame_index + 1),
                    expected_extent: Extent3d {
                        width: 64,
                        height: 48,
                        depth: 1,
                    },
                })
                .expect("windowed Vulkan acquire should succeed");
            assert!(matches!(
                acquired.status,
                FrameAcquireStatus::Ready | FrameAcquireStatus::Suboptimal
            ));
            gal.vulkan_backend_mut()
                .unwrap()
                .clear_acquired_frame_for_test(acquired.frame, [0.1, 0.2, 0.8, 1.0])
                .expect("test clear should prepare the swapchain image for present");
            let presented = gal
                .present_frame(PresentFrameDesc {
                    frame: acquired.frame,
                    correlation_id: acquired.correlation_id,
                    wait_for: SubmissionId(0),
                })
                .expect("windowed Vulkan present should succeed");
            assert!(matches!(
                presented.status,
                crate::frame::FramePresentStatus::Presented
                    | crate::frame::FramePresentStatus::Suboptimal
            ));
        }
        let resized = gal
            .resize_frame_surface(FrameResizeDesc {
                correlation_id: FrameCorrelationId(10),
                extent: Extent3d {
                    width: 32,
                    height: 32,
                    depth: 1,
                },
            })
            .expect("windowed Vulkan resize should recreate swapchain");
        assert_eq!(resized.status, FrameAcquireStatus::Resized);
        let minimized = gal
            .resize_frame_surface(FrameResizeDesc {
                correlation_id: FrameCorrelationId(11),
                extent: Extent3d {
                    width: 0,
                    height: 0,
                    depth: 1,
                },
            })
            .expect("zero-size Vulkan resize should be modeled as minimized");
        assert_eq!(minimized.status, FrameAcquireStatus::Minimized);
        gal.shutdown_frame_surface()
            .expect("windowed Vulkan shutdown should be clean");
        drop(gal);
        drop(window);

        for cycle in 0..2 {
            let window = match winit_test_window(&event_loop, 32, 32, 10 + cycle) {
                Ok(window) => window,
                Err(_) => return,
            };
            let backend = match VulkanBackend::new_windowed(
                "MattMC repeated windowed VulkanicGAL conformance",
                &window,
                FrameSurfaceDesc {
                    label: format!("repeated-window-{cycle}"),
                    extent: Extent3d {
                        width: 32,
                        height: 32,
                        depth: 1,
                    },
                    color_format: TextureFormat::Bgra8Unorm,
                    present_mode: PresentMode::Fifo,
                    max_frames_in_flight: 2,
                },
            ) {
                Ok(backend) => backend,
                Err(_) => return,
            };
            let mut gal = VulkanicGal::new_with_backend(Box::new(backend));
            let acquired = gal
                .acquire_frame(FrameAcquireDesc {
                    correlation_id: FrameCorrelationId(20 + cycle),
                    expected_extent: Extent3d {
                        width: 32,
                        height: 32,
                        depth: 1,
                    },
                })
                .expect("repeated window acquire should succeed");
            if matches!(
                acquired.status,
                FrameAcquireStatus::Ready | FrameAcquireStatus::Suboptimal
            ) {
                gal.vulkan_backend_mut()
                    .unwrap()
                    .clear_acquired_frame_for_test(acquired.frame, [0.0, 0.3, 0.2, 1.0])
                    .expect("test clear should prepare repeated window frame");
                gal.present_frame(PresentFrameDesc {
                    frame: acquired.frame,
                    correlation_id: acquired.correlation_id,
                    wait_for: SubmissionId(0),
                })
                .expect("repeated window present should succeed");
            }
            gal.shutdown_frame_surface()
                .expect("repeated window shutdown should be clean");
        }
    }

    #[test]
    fn vulkan_backend_consumes_owned_validated_submission_batch() {
        let backend = match VulkanBackend::new("VulkanicGAL validated submission test") {
            Ok(backend) => backend,
            Err(error) => {
                crate::test_support::environment_gap("Vulkan", &error);
                assert!(
                    error.to_string().contains("Vulkan")
                        || error.to_string().contains("physical device"),
                    "unexpected Vulkan bootstrap failure: {error}"
                );
                return;
            }
        };
        let mut gal = VulkanicGal::new_with_backend(Box::new(backend));
        let src = gal
            .create_buffer(BufferDesc {
                label: "copy-src".to_string(),
                size: 16,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::TransferSrc, BufferUsage::HostWrite],
            })
            .expect("source buffer should be created");
        let dst = gal
            .create_buffer(BufferDesc {
                label: "copy-dst".to_string(),
                size: 16,
                memory: MemoryDomain::Readback,
                usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
            })
            .expect("destination buffer should be created");
        let list = gal
            .create_command_list(CommandListDesc {
                label: "copy-list".to_string(),
                operations: vec![CommandOp::CopyBuffer { src, dst, size: 16 }],
            })
            .expect("copy command should validate");
        let token = gal
            .submit(SubmissionBatch {
                label: "copy-submit".to_string(),
                command_lists: vec![list],
            })
            .expect("validated submission should be consumed by Vulkan backend");
        assert_eq!(1, token.submission.0);
        let _ = gal
            .retire_completed()
            .expect("retirement polling should not fail");
    }

    #[test]
    fn vulkan_backend_supports_dynamic_descriptor_offsets_and_optional_sets() {
        let backend = match VulkanBackend::new("VulkanicGAL descriptor hardening test") {
            Ok(backend) => backend,
            Err(error) => {
                crate::test_support::environment_gap("Vulkan", &error);
                assert!(
                    error.to_string().contains("Vulkan")
                        || error.to_string().contains("physical device"),
                    "unexpected Vulkan bootstrap failure: {error}"
                );
                return;
            }
        };
        let mut gal = VulkanicGal::new_with_backend(Box::new(backend));
        assert!(gal
            .capabilities()
            .limits
            .uniform_buffer_offset_alignment
            .is_power_of_two());
        let uniform = gal
            .create_buffer(BufferDesc {
                label: "dynamic-uniform".to_string(),
                size: 256,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::Uniform],
            })
            .expect("dynamic uniform buffer should be created");
        let layout = gal
            .create_resource_layout(ResourceLayoutDesc {
                label: "dynamic-layout".to_string(),
                bindings: vec![
                    ResourceBindingDesc {
                        binding: 0,
                        kind: ResourceBindingKind::UniformBuffer,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 1,
                    },
                    ResourceBindingDesc {
                        binding: 1,
                        kind: ResourceBindingKind::Sampler,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: true,
                        dynamic_offset_count: 0,
                    },
                ],
            })
            .expect("dynamic resource layout should be created");
        let set = gal
            .create_resource_set(ResourceSetDesc {
                label: "dynamic-set".to_string(),
                layout,
                bindings: vec![ResourceBinding {
                    binding: 0,
                    array_index: 0,
                    resource: uniform,
                    kind: ResourceBindingKind::UniformBuffer,
                    access: AccessFlags::READ,
                    dynamic_offsets: vec![64],
                    buffer_range: None,
                }],
            })
            .expect("dynamic resource set should allow optional sampler omission");
        let pipeline_layout = gal
            .create_pipeline_layout(PipelineLayoutDesc {
                label: "dynamic-pipeline-layout".to_string(),
                resource_layouts: vec![layout],
            })
            .expect("pipeline layout should accept dynamic descriptor layout");
        assert_ne!(0, set.raw());
        assert_ne!(0, pipeline_layout.raw());
    }

    #[test]
    fn vulkan_backend_stresses_repeated_submissions_and_deferred_destroy() {
        let backend = match VulkanBackend::new("VulkanicGAL lifetime stress test") {
            Ok(backend) => backend,
            Err(error) => {
                crate::test_support::environment_gap("Vulkan", &error);
                assert!(
                    error.to_string().contains("Vulkan")
                        || error.to_string().contains("physical device"),
                    "unexpected Vulkan bootstrap failure: {error}"
                );
                return;
            }
        };
        let mut gal = VulkanicGal::new_with_backend(Box::new(backend));
        let mut tokens = Vec::new();
        for batch_index in 0..12 {
            let src = gal
                .create_buffer(BufferDesc {
                    label: format!("stress-src-{batch_index}"),
                    size: 256,
                    memory: MemoryDomain::Upload,
                    usages: vec![BufferUsage::TransferSrc, BufferUsage::HostWrite],
                })
                .expect("stress src buffer should be created");
            let dst = gal
                .create_buffer(BufferDesc {
                    label: format!("stress-dst-{batch_index}"),
                    size: 256,
                    memory: MemoryDomain::Readback,
                    usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
                })
                .expect("stress dst buffer should be created");
            let list = gal
                .create_command_list(CommandListDesc {
                    label: format!("stress-list-{batch_index}"),
                    operations: vec![CommandOp::CopyBuffer {
                        src,
                        dst,
                        size: 256,
                    }],
                })
                .expect("stress copy should validate");
            let token = gal
                .submit(SubmissionBatch {
                    label: format!("stress-submit-{batch_index}"),
                    command_lists: vec![list],
                })
                .expect("stress submit should reach Vulkan");
            gal.destroy(src)
                .expect("in-flight source destroy should defer");
            gal.destroy(dst)
                .expect("in-flight destination destroy should defer");
            tokens.push(token.submission);
        }
        let last = *tokens.last().expect("stress submitted at least one batch");
        let retired = gal
            .retire_through_for_test(last)
            .expect("stress resources should retire cleanly");
        assert_eq!(24, retired.len());
    }

    #[test]
    fn vulkan_large_host_writes_reuse_retired_staging_chunks() {
        use crate::commands::{ResourceBarrier, TextureUsageState};
        use crate::resources::QueueClass;
        let backend = match VulkanBackend::new("VulkanicGAL staging reuse test") {
            Ok(backend) => backend,
            Err(error) => {
                crate::test_support::environment_gap("Vulkan", &error);
                assert!(
                    error.to_string().contains("Vulkan")
                        || error.to_string().contains("physical device"),
                    "unexpected Vulkan bootstrap failure: {error}"
                );
                return;
            }
        };
        let mut gal = VulkanicGal::new_with_backend(Box::new(backend));
        // Above the 64 KiB inline-update limit, so the payload is staged.
        const SIZE: u64 = 300 * 1024;
        let device = gal
            .create_buffer(BufferDesc {
                label: "staging-reuse.device".to_string(),
                size: SIZE,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::TransferDst,
                    BufferUsage::TransferSrc,
                    BufferUsage::HostWrite,
                ],
            })
            .expect("device buffer");
        let readback = gal
            .create_buffer(BufferDesc {
                label: "staging-reuse.readback".to_string(),
                size: SIZE,
                memory: MemoryDomain::Readback,
                usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
            })
            .expect("readback buffer");
        let barrier = |resource, before, after| {
            CommandOp::Barrier(ResourceBarrier {
                resource,
                subresources: None,
                before,
                after,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            })
        };
        let mut created_after_first = 0;
        for round in 0..6_u8 {
            // An aligned large write and an unaligned tail share one chunk.
            let body: Vec<u8> = (0..SIZE as usize - 3)
                .map(|index| (index as u8).wrapping_mul(31).wrapping_add(round))
                .collect();
            let tail = vec![round ^ 0xa5; 3];
            let list = gal
                .create_command_list(CommandListDesc {
                    label: format!("staging-reuse.{round}"),
                    operations: vec![
                        CommandOp::HostWriteBuffer {
                            buffer: device,
                            offset: 0,
                            data: body.clone(),
                        },
                        CommandOp::HostWriteBuffer {
                            buffer: device,
                            offset: SIZE - 3,
                            data: tail.clone(),
                        },
                        barrier(
                            device,
                            TextureUsageState::TransferDst,
                            TextureUsageState::TransferSrc,
                        ),
                        CommandOp::CopyBuffer {
                            src: device,
                            dst: readback,
                            size: SIZE,
                        },
                        barrier(
                            device,
                            TextureUsageState::TransferSrc,
                            TextureUsageState::TransferDst,
                        ),
                        barrier(
                            readback,
                            TextureUsageState::TransferDst,
                            TextureUsageState::ShaderRead,
                        ),
                        CommandOp::HostReadBuffer {
                            buffer: readback,
                            offset: 0,
                            size: SIZE,
                        },
                    ],
                })
                .expect("staged upload list validates");
            let token = gal
                .submit(SubmissionBatch {
                    label: format!("staging-reuse.submit.{round}"),
                    command_lists: vec![list],
                })
                .expect("staged upload submits");
            gal.retire_through_for_test(token.submission)
                .expect("staged upload retires");
            let read = gal
                .completed_host_reads()
                .into_iter()
                .rev()
                .find(|read| read.submission == token.submission && read.buffer == readback)
                .expect("readback completed");
            let mut expected = body;
            expected.extend_from_slice(&tail);
            assert!(read.bytes == expected, "round {round} staged bytes differ");
            let created = gal
                .vulkan_backend()
                .expect("vulkan GAL")
                .staging_chunks_created_for_test();
            if round == 0 {
                created_after_first = created;
                assert!(created >= 1, "large writes must use staging");
            } else {
                assert_eq!(
                    created_after_first, created,
                    "retired staging must be reused"
                );
            }
        }
    }
}
