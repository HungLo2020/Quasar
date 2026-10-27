//! The wgpu backend: VulkanicGAL on wgpu, which in turn runs on Vulkan,
//! Metal or Direct3D 12. It exists for portability; on Vulkan-capable
//! systems the native Vulkan backend avoids wgpu's second layer of
//! validation and state tracking.
//!
//! - `device`: instance, adapter, device, negotiated features, error capture.
//! - `shaders`: GLSL/SPIR-V to naga, binding renumbering and reflection.
//! - `resources`: objects behind GAL handles, bind group layout variants.
//! - `encoding`: GAL command lists to a wgpu command buffer.
//! - `utility`: internal byte-copy and mip-generation pipelines.
//! - `surface`: presentation with the GAL frame contract.
//!
//! Shader conventions equal the Vulkan backend's, so renderers that run on
//! Vulkan run here unchanged.

mod convert;
mod device;
mod encoding;
mod resources;
mod shaders;
mod surface;
mod utility;

#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::{
    graphics_backend_lock, presentation_capabilities, Backend, BackendCreateDesc,
    BackendRuntimeMetrics, BackendToken, CompletedHostRead,
};
use crate::commands::ValidatedSubmissionBatch;
use crate::create::WgpuApi;
use crate::error::{GalError, GalResult};
use crate::frame::{
    AcquiredFrame, FrameAcquireDesc, FrameId, FrameResizeDesc, FrameResizeResult, FrameSurfaceDesc,
    PresentFrameDesc, PresentedFrame,
};
use crate::handles::{Handle, HandleKind};
use crate::resources::{
    BackendCapabilities, BackendFeatureFlags, BackendLimits, GlslDialect, ShaderConventions,
};
use crate::sync::SubmissionId;

use self::device::WgpuContext;
use self::encoding::{EncodeInputs, Encoded, PendingRead};
use self::resources::WgpuObjects;
use self::surface::WgpuSurface;
use self::utility::Utility;

/// Native window platform ids of the raw-handle constructor.
const WINDOW_PLATFORM_X11: u32 = 1;
const WINDOW_PLATFORM_WAYLAND: u32 = 2;

/// Host reads retained for the caller, as on the Vulkan backend.
const MAX_COMPLETED_HOST_READ_BYTES: usize = 64 * 1024 * 1024;

struct InFlight {
    id: SubmissionId,
    index: wgpu::SubmissionIndex,
    host_reads: Vec<PendingRead>,
}

#[derive(Default)]
struct Completion {
    in_flight: VecDeque<InFlight>,
    completed: SubmissionId,
    host_reads: Vec<CompletedHostRead>,
}

#[derive(Default)]
struct Counters {
    batches: u64,
    lists: u64,
    ops: u64,
    submit_nanos: u64,
    encode_nanos: u64,
    wait_nanos: u64,
    waits: u64,
}

pub(crate) struct WgpuBackend {
    // Surface and device objects must drop before the device and window.
    surface: Option<WgpuSurface>,
    pending: Option<Encoded>,
    objects: WgpuObjects,
    utility: Utility,
    indices: encoding::IndexCache,
    belt: wgpu::util::StagingBelt,
    completion: Mutex<Completion>,
    /// Highest submission id the queue reported done.
    done: Arc<AtomicU64>,
    counters: Counters,
    context: WgpuContext,
    _global_lock: MutexGuard<'static, ()>,
    // Last field: the native window outlives every wgpu object.
    _window_owner: Option<Box<dyn std::any::Any>>,
}

fn lock_graphics() -> GalResult<MutexGuard<'static, ()>> {
    graphics_backend_lock()
        .lock()
        .map_err(|_| GalError::backend("graphics backend global lock poisoned"))
}

impl WgpuBackend {
    fn from_context(
        context: WgpuContext,
        surface: Option<WgpuSurface>,
        global_lock: MutexGuard<'static, ()>,
    ) -> Self {
        let belt = wgpu::util::StagingBelt::new(context.device.clone(), 1 << 20);
        Self {
            surface,
            pending: None,
            objects: WgpuObjects::default(),
            utility: Utility::new(),
            indices: encoding::IndexCache::default(),
            belt,
            completion: Mutex::new(Completion::default()),
            done: Arc::new(AtomicU64::new(0)),
            counters: Counters::default(),
            context,
            _global_lock: global_lock,
            _window_owner: None,
        }
    }

    /// A headless backend on the default adapter of `api`.
    pub(crate) fn new(label: &str, api: WgpuApi) -> GalResult<Self> {
        let global_lock = lock_graphics()?;
        let context = WgpuContext::new(label, device::new_instance(api), None)?;
        Ok(Self::from_context(context, None, global_lock))
    }

    /// # Safety
    /// The handles must stay valid until the backend is dropped.
    unsafe fn new_raw_windowed(
        label: &str,
        api: WgpuApi,
        display: raw_window_handle::RawDisplayHandle,
        window: raw_window_handle::RawWindowHandle,
        desc: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        let global_lock = lock_graphics()?;
        let instance = device::new_instance(api);
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(display),
                raw_window_handle: window,
            })
        }
        .map_err(|error| GalError::backend(format!("wgpu surface creation failed: {error}")))?;
        let context = WgpuContext::new(label, instance, Some(&surface))?;
        let surface = WgpuSurface::new(&context, surface, desc)?;
        Ok(Self::from_context(context, Some(surface), global_lock))
    }

    /// A presenting backend that keeps `window` alive until it is dropped.
    pub(crate) fn new_owned_window<
        W: raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle + 'static,
    >(
        label: &str,
        api: WgpuApi,
        window: Arc<W>,
        desc: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        let display = window
            .display_handle()
            .map_err(|error| GalError::backend(error.to_string()))?
            .as_raw();
        let raw_window = window
            .window_handle()
            .map_err(|error| GalError::backend(error.to_string()))?
            .as_raw();
        // SAFETY: the backend owns `window` and drops it after the surface.
        let mut backend = unsafe { Self::new_raw_windowed(label, api, display, raw_window, desc)? };
        backend._window_owner = Some(Box::new(window));
        Ok(backend)
    }

    /// A presenting backend on an X11 (Xlib) or Wayland window given as
    /// integers, as the legacy Vulkan constructor takes them.
    ///
    /// # Safety
    /// The handles must be valid for `platform` and outlive the backend.
    pub(crate) unsafe fn new_native_windowed(
        label: &str,
        api: WgpuApi,
        platform: u32,
        native_display: u64,
        native_window: u64,
        desc: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        use raw_window_handle::{
            RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
            XlibDisplayHandle, XlibWindowHandle,
        };
        let display = std::ptr::NonNull::new(native_display as *mut std::ffi::c_void);
        let (display, window) = match platform {
            WINDOW_PLATFORM_X11 => (
                RawDisplayHandle::Xlib(XlibDisplayHandle::new(display, 0)),
                RawWindowHandle::Xlib(XlibWindowHandle::new(native_window as _)),
            ),
            WINDOW_PLATFORM_WAYLAND => {
                let display = display
                    .ok_or_else(|| GalError::invalid_argument("Wayland display handle is null"))?;
                let surface = std::ptr::NonNull::new(native_window as *mut std::ffi::c_void)
                    .ok_or_else(|| GalError::invalid_argument("Wayland surface handle is null"))?;
                (
                    RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display)),
                    RawWindowHandle::Wayland(WaylandWindowHandle::new(surface)),
                )
            }
            other => {
                return Err(GalError::unsupported_feature(format!(
                    "unsupported native window platform {other}"
                )))
            }
        };
        unsafe { Self::new_raw_windowed(label, api, display, window, desc) }
    }

    fn completion(&self) -> GalResult<std::sync::MutexGuard<'_, Completion>> {
        self.completion
            .lock()
            .map_err(|_| GalError::backend("wgpu completion state lock poisoned"))
    }

    /// Moves every submission the queue finished out of flight.
    fn collect_completed(&self, completion: &mut Completion) {
        let done = SubmissionId(self.done.load(Ordering::Acquire));
        while completion
            .in_flight
            .front()
            .is_some_and(|front| front.id <= done)
        {
            let complete = completion.in_flight.pop_front().expect("front exists");
            self.complete(completion, complete);
        }
    }

    fn complete(&self, completion: &mut Completion, complete: InFlight) {
        for read in complete.host_reads {
            let bytes = read_back(&self.context, &complete.index, &read)
                .unwrap_or_else(|error| error.to_string().into_bytes());
            completion.host_reads.push(CompletedHostRead {
                submission: complete.id,
                buffer: read.buffer,
                offset: read.offset,
                bytes,
            });
        }
        if complete.id > completion.completed {
            completion.completed = complete.id;
        }
        prune_completed_host_reads(&mut completion.host_reads, complete.id);
    }

    fn wait_through(&mut self, completed: SubmissionId) -> GalResult<()> {
        let started = std::time::Instant::now();
        let mut waits = 0;
        let mut completion = self
            .completion
            .lock()
            .map_err(|_| GalError::backend("wgpu completion state lock poisoned"))?;
        while completion
            .in_flight
            .front()
            .is_some_and(|front| front.id <= completed)
        {
            let complete = completion.in_flight.pop_front().expect("front exists");
            self.context
                .device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(complete.index.clone()),
                    timeout: None,
                })
                .map_err(|error| GalError::backend(format!("wgpu wait failed: {error}")))?;
            self.done.fetch_max(complete.id.0, Ordering::AcqRel);
            self.complete(&mut completion, complete);
            waits += 1;
        }
        drop(completion);
        self.counters.waits += waits;
        self.counters.wait_nanos = self
            .counters
            .wait_nanos
            .saturating_add(crate::metrics::elapsed_nanos_u64(started));
        Ok(())
    }

    fn latest_completed(&self) -> SubmissionId {
        let _ = self.context.device.poll(wgpu::PollType::Poll);
        match self.completion.lock() {
            Ok(mut completion) => {
                self.collect_completed(&mut completion);
                completion.completed
            }
            Err(_) => SubmissionId(0),
        }
    }
}

/// Maps a completed host read's staging buffer and copies its bytes.
fn read_back(
    context: &WgpuContext,
    index: &wgpu::SubmissionIndex,
    read: &PendingRead,
) -> GalResult<Vec<u8>> {
    let slice = read.staging.slice(..);
    let mapped = Arc::new(Mutex::new(None));
    let result = mapped.clone();
    slice.map_async(wgpu::MapMode::Read, move |outcome| {
        if let Ok(mut slot) = result.lock() {
            *slot = Some(outcome);
        }
    });
    context
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(index.clone()),
            timeout: None,
        })
        .map_err(|error| GalError::backend(format!("wgpu host read wait failed: {error}")))?;
    let outcome = mapped
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .ok_or_else(|| GalError::backend("wgpu host read was not mapped"))?;
    outcome.map_err(|error| GalError::backend(format!("wgpu host read map failed: {error}")))?;
    let start = read.staging_offset as usize;
    let bytes = slice
        .get_mapped_range()
        .map_err(|error| GalError::backend(format!("wgpu host read map failed: {error}")))?
        [start..start + read.size as usize]
        .to_vec();
    read.staging.unmap();
    Ok(bytes)
}

fn prune_completed_host_reads(reads: &mut Vec<CompletedHostRead>, newest: SubmissionId) {
    let mut bytes = 0usize;
    let mut keep_from = reads.len();
    for (index, read) in reads.iter().enumerate().rev() {
        // Keep every read of the newest submission together.
        if read.submission != newest && bytes >= MAX_COMPLETED_HOST_READ_BYTES {
            break;
        }
        bytes = bytes.saturating_add(read.bytes.len());
        keep_from = index;
    }
    reads.drain(..keep_from);
}

fn wgpu_capabilities(context: &WgpuContext, presentation: bool) -> BackendCapabilities {
    let limits = &context.limits;
    let downlevel = context.downlevel.flags;
    let indirect = downlevel.contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION);
    let compute = downlevel.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS);
    let max_2d = limits.max_texture_dimension_2d.min(16_384);
    let capabilities = BackendCapabilities {
        // Identical to the Vulkan backend: wgpu's clip space, framebuffer
        // origin and depth range match Vulkan with its flipped viewport.
        shader_conventions: ShaderConventions {
            glsl_dialect: GlslDialect::ExplicitBindings,
            zero_to_one_clip_depth: true,
            flip_fullscreen_uv_y: true,
            readback_rows_bottom_up: false,
        },
        name: "Rust wgpu",
        features: BackendFeatureFlags {
            graphics: true,
            compute,
            descriptor_arrays: false,
            optional_bindings: true,
            dynamic_buffer_offsets: true,
            uniform_buffers: true,
            storage_buffers: true,
            storage_textures: compute,
            indirect_draw: indirect,
            indirect_dispatch: indirect && compute,
            multiple_color_attachments: limits.max_color_attachments > 1,
            depth_only_pass: true,
            blended_pass: true,
            texture_subresource_copies: true,
            texture_mip_levels: true,
            texture_array_layers: true,
            host_buffer_access: true,
            presentation: false,
            renderdoc_capture: false,
            tracy_zones: false,
            texture_3d: limits.max_texture_dimension_3d > 0,
            texture_row_reversal: true,
            device_local_memory: true,
            // wgpu has no 3D depth/stencil textures.
            texture_3d_packed_formats: false,
        },
        limits: BackendLimits {
            max_buffer_size: limits.max_buffer_size.min(1 << 30),
            uniform_buffer_offset_alignment: u64::from(
                limits
                    .min_uniform_buffer_offset_alignment
                    .max(limits.min_storage_buffer_offset_alignment),
            ),
            max_texture_extent_2d: max_2d,
            max_texture_extent_3d: limits.max_texture_dimension_3d.min(2048),
            max_texture_mip_levels: 32 - max_2d.leading_zeros(),
            max_texture_array_layers: limits.max_texture_array_layers.min(256),
            max_resource_layout_bindings: 32,
            max_binding_array_count: 1,
            max_color_attachments: limits.max_color_attachments.min(8),
            max_dynamic_offsets_per_binding: 1,
            max_command_lists_per_submission: 64,
            max_commands_per_list: 16_384,
            max_draw_count: if indirect { 4096 } else { 1 },
            max_dispatch_groups_per_axis: if compute {
                limits.max_compute_workgroups_per_dimension
            } else {
                0
            },
        },
    };
    if presentation {
        presentation_capabilities(capabilities)
    } else {
        capabilities
    }
}

impl Backend for WgpuBackend {
    fn selected_backend(&self) -> crate::create::GraphicsBackend {
        self.context.selected_backend()
    }

    fn device_info(&self) -> crate::create::DeviceInfo {
        self.context.device_info()
    }

    fn capabilities(&self) -> BackendCapabilities {
        wgpu_capabilities(&self.context, self.surface.is_some())
    }

    fn create(&mut self, handle: Handle, desc: BackendCreateDesc<'_>) -> GalResult<BackendToken> {
        if matches!(desc, BackendCreateDesc::FrameTarget(_)) && self.surface.is_none() {
            return Err(GalError::unsupported_feature(
                "wgpu frame targets require a presentation-capable backend",
            ));
        }
        self.objects.create(&self.context, handle, desc)
    }

    fn destroy(&mut self, handle: Handle, kind: HandleKind, _token: BackendToken) -> GalResult<()> {
        if kind == HandleKind::Buffer {
            self.indices.forget(handle);
        }
        self.objects.destroy(handle, kind)
    }

    fn encode_passes(&mut self, batch: &ValidatedSubmissionBatch) -> GalResult<()> {
        let started = std::time::Instant::now();
        let inputs = EncodeInputs {
            context: &self.context,
            objects: &mut self.objects,
            utility: &mut self.utility,
            belt: &mut self.belt,
            indices: &mut self.indices,
            surface: self
                .surface
                .as_ref()
                .and_then(WgpuSurface::acquired_surface),
        };
        let context = &self.context;
        let encoded = context.scoped("command encoding", || encoding::encode(inputs, batch));
        let encoded = match encoded {
            Ok(Ok(encoded)) => encoded,
            Ok(Err(error)) | Err(error) => {
                // Release belt chunks the abandoned encoder referenced.
                self.belt.finish();
                self.belt.recall();
                return Err(error);
            }
        };
        self.counters.batches += 1;
        self.counters.lists += batch.command_lists.len() as u64;
        self.counters.ops += encoded.ops;
        self.counters.encode_nanos = self
            .counters
            .encode_nanos
            .saturating_add(crate::metrics::elapsed_nanos_u64(started));
        self.pending = Some(encoded);
        Ok(())
    }

    fn submit(&mut self, id: SubmissionId, _batch: &ValidatedSubmissionBatch) -> GalResult<()> {
        let started = std::time::Instant::now();
        let encoded = self
            .pending
            .take()
            .ok_or_else(|| GalError::backend("wgpu submit without an encoded batch"))?;
        let Encoded {
            command_buffer,
            host_reads,
            ..
        } = encoded;
        let index = self.context.scoped("queue submission", || {
            self.context.queue.submit([command_buffer])
        })?;
        let done = self.done.clone();
        self.context.queue.on_submitted_work_done(move || {
            done.fetch_max(id.0, Ordering::AcqRel);
        });
        self.belt.recall();
        self.completion()?.in_flight.push_back(InFlight {
            id,
            index,
            host_reads,
        });
        self.counters.submit_nanos = self
            .counters
            .submit_nanos
            .saturating_add(crate::metrics::elapsed_nanos_u64(started));
        let errors = self.context.take_uncaptured_errors();
        if !errors.is_empty() {
            return Err(GalError::backend(format!(
                "wgpu reported errors: {}",
                errors.join("; ")
            )));
        }
        Ok(())
    }

    fn completed_submission(&self) -> SubmissionId {
        self.latest_completed()
    }

    fn retire(&mut self, completed: SubmissionId) -> GalResult<()> {
        let polled = self.latest_completed();
        self.wait_through(std::cmp::max(completed, polled))
    }

    fn completed_host_reads(&self) -> Vec<CompletedHostRead> {
        self.completion
            .lock()
            .map(|completion| completion.host_reads.clone())
            .unwrap_or_default()
    }

    fn runtime_metrics(&self) -> BackendRuntimeMetrics {
        let surface = self.surface.as_ref();
        BackendRuntimeMetrics {
            command_batches: self.counters.batches,
            command_lists: self.counters.lists,
            command_ops: self.counters.ops,
            native_command_recording_nanos: self.counters.encode_nanos,
            native_queue_submit_nanos: self.counters.submit_nanos,
            native_timeline_wait_nanos: self.counters.wait_nanos,
            native_wait_count: self.counters.waits,
            native_acquire_nanos: surface.map_or(0, |surface| surface.acquire_nanos),
            native_present_nanos: surface.map_or(0, |surface| surface.present_nanos),
            native_configured_frames_in_flight: surface.map_or(0, WgpuSurface::frames_in_flight),
            native_images_in_flight: surface.map_or(0, WgpuSurface::images_in_flight),
            ..BackendRuntimeMetrics::default()
        }
    }

    fn configure_frame_surface(&mut self, desc: &FrameSurfaceDesc) -> GalResult<()> {
        let completed = self.latest_completed();
        self.wait_through(std::cmp::max(completed, self.latest_in_flight()))?;
        let surface = self.surface.as_mut().ok_or_else(|| {
            GalError::unsupported_feature(
                "wgpu backend was not created with a presentation surface",
            )
        })?;
        surface.configure(&self.context, desc)
    }

    fn acquire_frame(&mut self, desc: &FrameAcquireDesc) -> GalResult<AcquiredFrame> {
        let surface = self.surface.as_mut().ok_or_else(|| {
            GalError::unsupported_feature(
                "wgpu backend was not created with a presentation surface",
            )
        })?;
        surface.acquire(&self.context, desc)
    }

    fn resize_frame_surface(&mut self, desc: &FrameResizeDesc) -> GalResult<FrameResizeResult> {
        let completed = self.latest_completed();
        self.wait_through(std::cmp::max(completed, self.latest_in_flight()))?;
        let surface = self.surface.as_mut().ok_or_else(|| {
            GalError::unsupported_feature(
                "wgpu backend was not created with a presentation surface",
            )
        })?;
        surface.resize(&self.context, desc)
    }

    fn present_frame(&mut self, desc: &PresentFrameDesc) -> GalResult<PresentedFrame> {
        let completed = self.latest_completed();
        let surface = self.surface.as_mut().ok_or_else(|| {
            GalError::unsupported_feature(
                "wgpu backend was not created with a presentation surface",
            )
        })?;
        surface.present(&self.context, desc, completed)
    }

    fn cancel_frame(&mut self, frame: FrameId) -> GalResult<()> {
        let surface = self.surface.as_mut().ok_or_else(|| {
            GalError::unsupported_feature(
                "wgpu backend was not created with a presentation surface",
            )
        })?;
        surface.cancel(frame)
    }

    fn shutdown_frame_surface(&mut self) -> GalResult<()> {
        let completed = self.latest_completed();
        self.wait_through(std::cmp::max(completed, self.latest_in_flight()))?;
        if let Some(surface) = &mut self.surface {
            surface.shutdown();
        }
        Ok(())
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl WgpuBackend {
    fn latest_in_flight(&self) -> SubmissionId {
        self.completion
            .lock()
            .ok()
            .and_then(|completion| completion.in_flight.back().map(|entry| entry.id))
            .unwrap_or(SubmissionId(0))
    }

    #[cfg(test)]
    pub(crate) fn adapter_name_for_test(&self) -> String {
        self.context.adapter_name()
    }
}

impl Drop for WgpuBackend {
    fn drop(&mut self) {
        self.surface = None;
        let _ = self
            .context
            .device
            .poll(wgpu::PollType::wait_indefinitely());
    }
}
