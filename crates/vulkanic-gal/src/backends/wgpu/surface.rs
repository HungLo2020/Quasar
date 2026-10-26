//! A presentation surface with the GAL's frame contract.
//!
//! wgpu hands out one surface texture at a time and does not expose
//! swapchain image indices, so frame render-target ids cycle through one
//! slot per configured frame in flight (and change with every
//! reconfiguration). Frontends that cache frame targets by render target,
//! as `quasar-frame` does, keep a bounded set, exactly as on Vulkan.

use super::convert;
use super::device::WgpuContext;
use super::encoding::AcquiredSurface;
use crate::error::{GalError, GalResult};
use crate::frame::{
    AcquiredFrame, FrameAcquireDesc, FrameAcquireStatus, FrameId, FramePresentStatus,
    FrameRenderTargetId, FrameResizeDesc, FrameResizeResult, FrameSurfaceDesc, PresentFrameDesc,
    PresentedFrame,
};
use crate::resources::{Extent3d, TextureFormat};
use crate::sync::SubmissionId;

struct Acquired {
    frame: FrameId,
    render_target: FrameRenderTargetId,
    texture: wgpu::SurfaceTexture,
    view: wgpu::TextureView,
    suboptimal: bool,
}

pub(super) struct WgpuSurface {
    acquired: Option<Acquired>,
    surface: wgpu::Surface<'static>,
    desc: FrameSurfaceDesc,
    configured: Option<wgpu::SurfaceConfiguration>,
    color_format: TextureFormat,
    next_frame: u64,
    generation: u64,
    acquire_count: u64,
    pub(super) acquire_nanos: u64,
    pub(super) present_nanos: u64,
}

impl WgpuSurface {
    pub(super) fn new(
        context: &WgpuContext,
        surface: wgpu::Surface<'static>,
        desc: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        let mut owner = Self {
            acquired: None,
            surface,
            color_format: desc.color_format,
            desc,
            configured: None,
            next_frame: 0,
            generation: 0,
            acquire_count: 0,
            acquire_nanos: 0,
            present_nanos: 0,
        };
        owner.reconfigure(context)?;
        Ok(owner)
    }

    fn reconfigure(&mut self, context: &WgpuContext) -> GalResult<()> {
        self.acquired = None;
        self.generation += 1;
        let extent = self.desc.extent;
        if extent.width == 0 || extent.height == 0 {
            self.configured = None;
            return Ok(());
        }
        let capabilities = self.surface.get_capabilities(&context.adapter);
        let requested = convert::texture_format(self.desc.color_format);
        let format = if capabilities.formats.contains(&requested) {
            requested
        } else {
            capabilities
                .formats
                .iter()
                .copied()
                .find(|format| convert::gal_surface_format(*format).is_some())
                .ok_or_else(|| {
                    GalError::unsupported_feature(format!(
                        "the wgpu surface offers none of the GAL's frame formats ({:?})",
                        capabilities.formats
                    ))
                })?
        };
        self.color_format = convert::gal_surface_format(format).unwrap_or(self.desc.color_format);
        let alpha_mode = if capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::Opaque)
        {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            capabilities.alpha_modes[0]
        };
        // Frame-target copies need the surface as a copy endpoint.
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | (capabilities.usages
                & (wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST));
        let config = wgpu::SurfaceConfiguration {
            usage,
            format,
            color_space: Default::default(),
            width: extent.width,
            height: extent.height,
            present_mode: convert::supported_present_mode(
                self.desc.present_mode,
                &capabilities.present_modes,
            ),
            desired_maximum_frame_latency: self.desc.max_frames_in_flight.clamp(1, 3),
            alpha_mode,
            view_formats: Vec::new(),
        };
        context.scoped("surface configuration", || {
            self.surface.configure(&context.device, &config)
        })?;
        self.configured = Some(config);
        Ok(())
    }

    fn configured_extent(&self) -> Extent3d {
        self.configured
            .as_ref()
            .map_or(self.desc.extent, |config| Extent3d {
                width: config.width,
                height: config.height,
                depth: 1,
            })
    }

    pub(super) fn configure(
        &mut self,
        context: &WgpuContext,
        desc: &FrameSurfaceDesc,
    ) -> GalResult<()> {
        self.desc = desc.clone();
        self.reconfigure(context)
    }

    pub(super) fn acquire(
        &mut self,
        context: &WgpuContext,
        desc: &FrameAcquireDesc,
    ) -> GalResult<AcquiredFrame> {
        let started = std::time::Instant::now();
        let result = self.acquire_inner(context, desc);
        self.acquire_nanos = self
            .acquire_nanos
            .saturating_add(crate::metrics::elapsed_nanos_u64(started));
        result
    }

    fn skipped(
        &self,
        frame: FrameId,
        desc: &FrameAcquireDesc,
        status: FrameAcquireStatus,
    ) -> AcquiredFrame {
        AcquiredFrame {
            frame,
            correlation_id: desc.correlation_id,
            status,
            render_target: FrameRenderTargetId(0),
            extent: if status == FrameAcquireStatus::Minimized {
                desc.expected_extent
            } else {
                self.configured_extent()
            },
            color_format: self.color_format,
        }
    }

    fn acquire_inner(
        &mut self,
        context: &WgpuContext,
        desc: &FrameAcquireDesc,
    ) -> GalResult<AcquiredFrame> {
        self.next_frame += 1;
        let frame = FrameId(self.next_frame);
        if desc.expected_extent.width == 0 || desc.expected_extent.height == 0 {
            return Ok(self.skipped(frame, desc, FrameAcquireStatus::Minimized));
        }
        if self.acquired.is_some() {
            return Err(GalError::submission(
                crate::StatusCode::InFlight,
                "the wgpu surface hands out one frame at a time; present or cancel the acquired frame first",
            ));
        }
        // Never hand out an image of another size than the caller recorded
        // for: reconfigure and skip this frame, as the Vulkan swapchain does.
        if self.configured.is_none() || desc.expected_extent != self.configured_extent() {
            self.desc.extent = desc.expected_extent;
            self.reconfigure(context)?;
            return Ok(self.skipped(frame, desc, FrameAcquireStatus::Resized));
        }
        let (texture, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => (texture, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => (texture, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(self.skipped(frame, desc, FrameAcquireStatus::Minimized));
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.reconfigure(context)?;
                return Ok(self.skipped(frame, desc, FrameAcquireStatus::Resized));
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                let errors = context.take_uncaptured_errors().join("; ");
                return Err(GalError::backend(format!(
                    "wgpu surface acquire failed validation: {errors}"
                )));
            }
        };
        let slots = u64::from(self.desc.max_frames_in_flight.max(1));
        let slot = self.acquire_count % slots;
        self.acquire_count += 1;
        let render_target = FrameRenderTargetId((self.generation << 32) | (slot + 1));
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.acquired = Some(Acquired {
            frame,
            render_target,
            texture,
            view,
            suboptimal,
        });
        Ok(AcquiredFrame {
            frame,
            correlation_id: desc.correlation_id,
            status: if suboptimal {
                FrameAcquireStatus::Suboptimal
            } else {
                FrameAcquireStatus::Ready
            },
            render_target,
            extent: self.configured_extent(),
            color_format: self.color_format,
        })
    }

    pub(super) fn acquired_surface(&self) -> Option<AcquiredSurface<'_>> {
        self.acquired.as_ref().map(|acquired| AcquiredSurface {
            render_target: acquired.render_target,
            texture: &acquired.texture.texture,
            view: &acquired.view,
        })
    }

    pub(super) fn present(
        &mut self,
        context: &WgpuContext,
        desc: &PresentFrameDesc,
        completed: SubmissionId,
    ) -> GalResult<PresentedFrame> {
        let started = std::time::Instant::now();
        let acquired = match self.acquired.take() {
            Some(acquired) if acquired.frame == desc.frame => acquired,
            other => {
                self.acquired = other;
                return Err(GalError::submission(
                    crate::StatusCode::InvalidArgument,
                    "wgpu frame was not acquired before present",
                ));
            }
        };
        let render_target = acquired.render_target;
        let status = if acquired.suboptimal {
            FramePresentStatus::Suboptimal
        } else {
            FramePresentStatus::Presented
        };
        drop(acquired.view);
        context.queue.present(acquired.texture);
        self.present_nanos = self
            .present_nanos
            .saturating_add(crate::metrics::elapsed_nanos_u64(started));
        Ok(PresentedFrame {
            frame: desc.frame,
            correlation_id: desc.correlation_id,
            render_target,
            status,
            completed_submission: completed,
        })
    }

    pub(super) fn cancel(&mut self, frame: FrameId) -> GalResult<()> {
        match self.acquired.take() {
            // Dropping an unpresented surface texture discards it.
            Some(acquired) if acquired.frame == frame => Ok(()),
            other => {
                self.acquired = other;
                Err(GalError::submission(
                    crate::StatusCode::InvalidArgument,
                    "wgpu frame was not acquired before cancel",
                ))
            }
        }
    }

    pub(super) fn resize(
        &mut self,
        context: &WgpuContext,
        desc: &FrameResizeDesc,
    ) -> GalResult<FrameResizeResult> {
        self.desc.extent = desc.extent;
        if desc.extent.width == 0 || desc.extent.height == 0 {
            self.acquired = None;
            self.configured = None;
            return Ok(FrameResizeResult {
                status: FrameAcquireStatus::Minimized,
                extent: desc.extent,
            });
        }
        self.reconfigure(context)?;
        Ok(FrameResizeResult {
            status: FrameAcquireStatus::Resized,
            extent: self.configured_extent(),
        })
    }

    pub(super) fn shutdown(&mut self) {
        self.acquired = None;
        self.configured = None;
    }

    pub(super) fn frames_in_flight(&self) -> u64 {
        u64::from(self.desc.max_frames_in_flight)
    }

    pub(super) fn images_in_flight(&self) -> u64 {
        u64::from(self.acquired.is_some())
    }
}
