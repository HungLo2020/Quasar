//! A GPU framebuffer presenter built exclusively on VulkanicGAL's public API.
//! Games supply tightly packed, top-to-bottom RGBA8 pixels. Scaling happens on
//! the GPU, and the window need only implement raw-window-handle traits.
pub use quasar_core::{DisplayOptions, Filter, Scaling, Viewport};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::{path::Path, sync::Arc};
use vulkanic_gal::{resources::*, *};

const VERTEX: &str = include_str!("present.vert");
const FRAGMENT: &str = include_str!("present.frag");

/// Presentation may be deferred while a minimized or changing surface recovers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentOutcome {
    Presented,
    Deferred,
}

/// Owns its GAL, surface and GPU resources; retains the native window until all
/// GPU resources are destroyed. The caller owns scheduling and native events.
pub struct FramebufferPresenter {
    gal: VulkanicGal,
    source: Extent3d,
    size: Extent3d,
    upload: Handle,
    texture: Handle,
    uniform: Handle,
    layout: Handle,
    sets: [Handle; 2],
    vertex: Handle,
    fragment: Handle,
    pipeline: Option<(TextureFormat, Handle)>,
    initialized: bool,
    correlation: u64,
    options: DisplayOptions,
    needs_resize: bool,
}

impl FramebufferPresenter {
    pub fn new<W: HasDisplayHandle + HasWindowHandle + 'static>(
        window: Arc<W>,
        size: (u32, u32),
        source: (u32, u32),
        options: DisplayOptions,
    ) -> GalResult<Self> {
        validate_source(source)?;
        if size.0 == 0 || size.1 == 0 {
            return Err(GalError::invalid_argument(
                "initial window size must be nonzero",
            ));
        }
        options.validate().map_err(GalError::invalid_argument)?;
        let size = extent(size);
        let source = extent(source);
        let mut gal = VulkanicGal::create_windowed_vulkan(
            "Quasar framebuffer presenter",
            window,
            FrameSurfaceDesc {
                label: "quasar-surface".into(),
                extent: size,
                color_format: TextureFormat::Bgra8Unorm,
                present_mode: PresentMode::AutoVsync,
                max_frames_in_flight: 1,
            },
        )?;
        let upload = gal.create_buffer(BufferDesc {
            label: "framebuffer-upload".into(),
            size: u64::from(source.width) * u64::from(source.height) * 4,
            memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
        })?;
        let uniform = gal.create_buffer(BufferDesc {
            label: "presentation-viewport".into(),
            size: 16,
            memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::HostWrite, BufferUsage::Uniform],
        })?;
        let texture = gal.create_texture(TextureDesc {
            label: "framebuffer".into(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent: source,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::TransferDst, TextureUsage::Sampled],
        })?;
        let view = gal.create_texture_view(TextureViewDesc {
            label: "framebuffer-view".into(),
            texture,
            format: TextureFormat::Rgba8Unorm,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })?;
        let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
            label: "present-resources".into(),
            bindings: vec![
                binding_desc(0, ResourceBindingKind::CombinedTextureSampler),
                binding_desc(1, ResourceBindingKind::UniformBuffer),
            ],
        })?;
        let mut sets = Vec::new();
        for filter in [SamplerFilter::Nearest, SamplerFilter::Linear] {
            let sampler = gal.create_sampler(SamplerDesc {
                label: format!("present-{filter:?}"),
                min_filter: filter,
                mag_filter: filter,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            let combined = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                label: "framebuffer-sampler".into(),
                texture_view: view,
                sampler,
            })?;
            sets.push(gal.create_resource_set(ResourceSetDesc {
                label: "present-set".into(),
                layout: resource_layout,
                bindings: vec![
                    binding(0, combined, ResourceBindingKind::CombinedTextureSampler),
                    binding(1, uniform, ResourceBindingKind::UniformBuffer),
                ],
            })?);
        }
        let layout = gal.create_pipeline_layout(PipelineLayoutDesc {
            label: "present-layout".into(),
            resource_layouts: vec![resource_layout],
        })?;
        let vertex =
            gal.create_shader_module(shader("present-vertex", ShaderStage::Vertex, VERTEX))?;
        let fragment =
            gal.create_shader_module(shader("present-fragment", ShaderStage::Fragment, FRAGMENT))?;
        Ok(Self {
            gal,
            source,
            size,
            upload,
            texture,
            uniform,
            layout,
            sets: [sets[0], sets[1]],
            vertex,
            fragment,
            pipeline: None,
            initialized: false,
            correlation: 0,
            options,
            needs_resize: false,
        })
    }

    pub fn options(&self) -> DisplayOptions {
        self.options
    }
    pub fn set_options(&mut self, options: DisplayOptions) -> GalResult<()> {
        options.validate().map_err(GalError::invalid_argument)?;
        self.options = options;
        Ok(())
    }
    pub fn viewport(&self) -> Option<Viewport> {
        Viewport::calculate(
            (self.source.width, self.source.height),
            (self.size.width, self.size.height),
            self.options,
        )
    }
    /// Zero dimensions suspend presentation until a nonzero resize arrives.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = extent((width, height));
        self.needs_resize = true;
    }

    pub fn present(&mut self, rgba: &[u8]) -> GalResult<PresentOutcome> {
        if rgba.len() as u64 != u64::from(self.source.width) * u64::from(self.source.height) * 4 {
            return Err(GalError::invalid_argument(
                "RGBA framebuffer length does not match source dimensions",
            ));
        }
        if self.size.width == 0 || self.size.height == 0 {
            return Ok(PresentOutcome::Deferred);
        }
        self.correlation += 1;
        let correlation_id = FrameCorrelationId(self.correlation);
        if self.needs_resize {
            self.gal.resize_frame_surface(FrameResizeDesc {
                correlation_id,
                extent: self.size,
            })?;
            self.needs_resize = false;
        }
        let acquired = self.gal.acquire_frame(FrameAcquireDesc {
            correlation_id,
            expected_extent: self.size,
        })?;
        if matches!(
            acquired.status,
            FrameAcquireStatus::Minimized | FrameAcquireStatus::Resized
        ) {
            return Ok(PresentOutcome::Deferred);
        }
        let result = self.render_acquired(acquired, rgba);
        if result.is_err() {
            let _ = self.gal.cancel_frame(acquired.frame);
        }
        result
    }

    fn render_acquired(
        &mut self,
        acquired: AcquiredFrame,
        rgba: &[u8],
    ) -> GalResult<PresentOutcome> {
        if self
            .pipeline
            .is_none_or(|(format, _)| format != acquired.color_format)
        {
            if let Some((_, old)) = self.pipeline.take() {
                self.gal.destroy(old)?;
            }
            let pipeline = self.gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: "framebuffer-present".into(),
                layout: self.layout,
                vertex_shader: self.vertex,
                fragment_shader: self.fragment,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: FrontFace::CounterClockwise,
                provoking_vertex: ProvokingVertex::First,
                raster_y_direction: RasterYDirection::Down,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![acquired.color_format],
                depth_format: None,
                stencil: None,
            })?;
            self.pipeline = Some((acquired.color_format, pipeline));
        }
        let viewport = Viewport::calculate(
            (self.source.width, self.source.height),
            (acquired.extent.width, acquired.extent.height),
            self.options,
        )
        .ok_or_else(|| GalError::invalid_argument("invalid presentation viewport"))?;
        let params: Vec<u8> = [
            viewport.x as f32 / acquired.extent.width as f32,
            viewport.y as f32 / acquired.extent.height as f32,
            viewport.width as f32 / acquired.extent.width as f32,
            viewport.height as f32 / acquired.extent.height as f32,
        ]
        .into_iter()
        .flat_map(f32::to_ne_bytes)
        .collect();
        let target = self.gal.create_frame_target(FrameTargetDesc {
            label: "present-target".into(),
            frame_id: acquired.frame.0,
            render_target: acquired.render_target,
            extent: acquired.extent,
            color_format: acquired.color_format,
        })?;
        let result = (|| {
            let pass = self.gal.create_render_pass(RenderPassDesc {
                label: "present-pass".into(),
                target,
                color_formats: vec![acquired.color_format],
                depth_format: None,
            })?;
            let result = (|| {
                let previous_texture = if self.initialized {
                    TextureUsageState::ShaderRead
                } else {
                    TextureUsageState::Undefined
                };
                let set = self.sets[usize::from(self.options.filter == Filter::Linear)];
                let list = self.gal.create_command_list(CommandListDesc {
                    label: "present-frame".into(),
                    operations: vec![
                        barrier(
                            self.upload,
                            TextureUsageState::TransferSrc,
                            TextureUsageState::TransferDst,
                        ),
                        CommandOp::HostWriteBuffer {
                            buffer: self.upload,
                            offset: 0,
                            data: rgba.to_vec(),
                        },
                        barrier(
                            self.upload,
                            TextureUsageState::TransferDst,
                            TextureUsageState::TransferSrc,
                        ),
                        barrier(
                            self.uniform,
                            TextureUsageState::ShaderRead,
                            TextureUsageState::TransferDst,
                        ),
                        CommandOp::HostWriteBuffer {
                            buffer: self.uniform,
                            offset: 0,
                            data: params,
                        },
                        barrier(
                            self.uniform,
                            TextureUsageState::TransferDst,
                            TextureUsageState::ShaderRead,
                        ),
                        barrier(
                            self.texture,
                            previous_texture,
                            TextureUsageState::TransferDst,
                        ),
                        CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                            buffer: self.upload,
                            buffer_offset: 0,
                            bytes_per_row: self.source.width * 4,
                            rows_per_image: self.source.height,
                            texture: self.texture,
                            texture_mip: 0,
                            texture_layer: 0,
                            texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                            extent: self.source,
                        }),
                        barrier(
                            self.texture,
                            TextureUsageState::TransferDst,
                            TextureUsageState::ShaderRead,
                        ),
                        CommandOp::BeginPass {
                            pass,
                            target,
                            colors: vec![PassAttachment {
                                view: target,
                                load_op: AttachmentLoadOp::Clear,
                                store_op: AttachmentStoreOp::Store,
                                clear_color: Some(ClearColor {
                                    r: 0.0,
                                    g: 0.0,
                                    b: 0.0,
                                    a: 1.0,
                                }),
                            }],
                            depth_stencil: None,
                        },
                        CommandOp::BindGraphicsPipeline(self.pipeline.unwrap().1),
                        CommandOp::BindResourceSet {
                            pipeline_layout: self.layout,
                            set_index: 0,
                            set,
                            dynamic_offsets: vec![],
                        },
                        CommandOp::Draw {
                            vertices: 3,
                            instances: 1,
                        },
                        CommandOp::EndPass,
                    ],
                })?;
                let submitted = self.gal.submit(SubmissionBatch {
                    label: "present-submit".into(),
                    command_lists: vec![list],
                })?;
                self.initialized = true;
                let presented = self.gal.present_frame(PresentFrameDesc {
                    frame: acquired.frame,
                    correlation_id: acquired.correlation_id,
                    wait_for: submitted.submission,
                })?;
                self.gal.retire_through(submitted.submission)?;
                self.needs_resize |= acquired.status == FrameAcquireStatus::Suboptimal
                    || matches!(
                        presented.status,
                        FramePresentStatus::Suboptimal | FramePresentStatus::OutOfDate
                    );
                Ok(
                    if presented.status == FramePresentStatus::Presented
                        || presented.status == FramePresentStatus::Suboptimal
                    {
                        PresentOutcome::Presented
                    } else {
                        PresentOutcome::Deferred
                    },
                )
            })();
            let cleanup = self.gal.destroy(pass);
            result.and_then(|value| cleanup.map(|_| value))
        })();
        let cleanup = self.gal.destroy(target);
        result.and_then(|value| cleanup.map(|_| value))
    }
}

impl Drop for FramebufferPresenter {
    fn drop(&mut self) {
        let last = self.gal.latest_submission_id();
        let _ = self.gal.retire_through(last);
        let _ = self.gal.shutdown_frame_surface();
    }
}

fn extent(size: (u32, u32)) -> Extent3d {
    Extent3d {
        width: size.0,
        height: size.1,
        depth: 1,
    }
}
fn validate_source(size: (u32, u32)) -> GalResult<()> {
    if size.0 == 0
        || size.1 == 0
        || size
            .0
            .checked_mul(size.1)
            .and_then(|n| n.checked_mul(4))
            .is_none()
    {
        return Err(GalError::invalid_argument(
            "framebuffer dimensions must be nonzero and fit a 32-bit byte count",
        ));
    }
    Ok(())
}
fn shader(label: &str, stage: ShaderStage, source: &str) -> ShaderModuleDesc {
    ShaderModuleDesc {
        label: label.into(),
        stage,
        code_format: ShaderCodeFormat::Glsl,
        code: source.as_bytes().to_vec(),
        entry_point: "main".into(),
    }
}
fn binding_desc(binding: u32, kind: ResourceBindingKind) -> ResourceBindingDesc {
    ResourceBindingDesc {
        binding,
        kind,
        stages: PipelineStageFlags::DRAW,
        array_count: 1,
        optional: false,
        dynamic_offset_count: 0,
    }
}
fn binding(binding: u32, resource: Handle, kind: ResourceBindingKind) -> ResourceBinding {
    ResourceBinding {
        binding,
        resource,
        kind,
        array_index: 0,
        access: AccessFlags::READ,
        dynamic_offsets: vec![],
        buffer_range: None,
    }
}
fn barrier(resource: Handle, before: TextureUsageState, after: TextureUsageState) -> CommandOp {
    CommandOp::Barrier(ResourceBarrier {
        resource,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    })
}

/// Saves a native-resolution RGBA framebuffer without a graphics device.
pub fn write_png(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    validate_source((width, height))?;
    if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
        return Err("RGBA length mismatch".into());
    }
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(path)?),
        width,
        height,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}
