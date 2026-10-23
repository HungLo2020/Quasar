use crate::{Canvas, DisplayOptions, Filter, TextureId, Textures, Viewport};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::{collections::BTreeMap, sync::Arc};
use vulkanic_gal::{resources::*, *};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentOutcome {
    Presented,
    Deferred,
}
#[derive(Debug, Default, Clone, Copy)]
pub struct RenderStats {
    pub quads: usize,
    pub batches: usize,
    pub geometry_upload_bytes: usize,
    pub texture_upload_bytes: usize,
}
struct TextureGpu {
    texture: Handle,
    view: Handle,
    samplers: [Handle; 2],
    combined: [Handle; 2],
    sets: [Handle; 2],
}
struct Surface {
    size: Extent3d,
    dirty: bool,
    options: DisplayOptions,
}
struct Batch {
    texture: TextureId,
    filter: Filter,
    first: u32,
    count: u32,
}
/// A consumer of VulkanicGAL, owning its GAL and reusable rendering resources.
/// No native graphics APIs or backend implementation types are used here.
pub struct Renderer2D {
    gal: VulkanicGal,
    surface: Option<Surface>,
    layout: Handle,
    resource_layout: Handle,
    vertex: Handle,
    fragment: Handle,
    pipelines: BTreeMap<TextureFormat, Handle>,
    geometry: Handle,
    uniform: Handle,
    geometry_capacity: usize,
    uniform_capacity: usize,
    uniform_stride: usize,
    textures: BTreeMap<TextureId, TextureGpu>,
    correlation: u64,
    stats: RenderStats,
    pending_passes: Vec<Handle>,
}
impl Renderer2D {
    pub fn headless() -> GalResult<Self> {
        Self::from_gal(VulkanicGal::create(
            BackendChoice::Vulkan,
            "Quasar 2D offscreen",
        )?)
    }
    /// Accepts an existing GAL. The current shaders require Vulkan-style GLSL.
    pub fn from_gal(mut gal: VulkanicGal) -> GalResult<Self> {
        if gal.capabilities().shader_conventions.glsl_dialect != GlslDialect::ExplicitBindings {
            return Err(GalError::unsupported_feature(
                "2D shaders require explicit-binding GLSL conventions",
            ));
        }
        let stride = (gal.capabilities().limits.uniform_buffer_offset_alignment as usize)
            .max(32)
            .next_power_of_two();
        let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
            label: "2d-resources".into(),
            bindings: vec![
                desc(0, ResourceBindingKind::CombinedTextureSampler, 0),
                desc(1, ResourceBindingKind::StorageBuffer, 0),
                desc(2, ResourceBindingKind::UniformBuffer, 1),
            ],
        })?;
        let layout = gal.create_pipeline_layout(PipelineLayoutDesc {
            label: "2d-layout".into(),
            resource_layouts: vec![resource_layout],
        })?;
        let vertex = gal.create_shader_module(shader(
            "2d-vertex",
            ShaderStage::Vertex,
            include_str!("quad.vert"),
        ))?;
        let fragment = gal.create_shader_module(shader(
            "2d-fragment",
            ShaderStage::Fragment,
            include_str!("quad.frag"),
        ))?;
        let geometry = buffer(&mut gal, "2d-geometry", 80, BufferUsage::Storage)?;
        let uniform = buffer(&mut gal, "2d-frame", stride, BufferUsage::Uniform)?;
        Ok(Self {
            gal,
            surface: None,
            layout,
            resource_layout,
            vertex,
            fragment,
            pipelines: BTreeMap::new(),
            geometry,
            uniform,
            geometry_capacity: 80,
            uniform_capacity: stride,
            uniform_stride: stride,
            textures: BTreeMap::new(),
            correlation: 0,
            stats: RenderStats::default(),
            pending_passes: Vec::new(),
        })
    }
    pub fn for_window<W: HasDisplayHandle + HasWindowHandle + 'static>(
        window: Arc<W>,
        size: (u32, u32),
        options: DisplayOptions,
    ) -> GalResult<Self> {
        options.validate().map_err(GalError::invalid_argument)?;
        let gal = VulkanicGal::create_windowed_vulkan(
            "Quasar 2D",
            window,
            FrameSurfaceDesc {
                label: "2d-surface".into(),
                extent: extent(size),
                color_format: TextureFormat::Bgra8Unorm,
                present_mode: PresentMode::AutoVsync,
                max_frames_in_flight: 1,
            },
        )?;
        let mut renderer = Self::from_gal(gal)?;
        renderer.surface = Some(Surface {
            size: extent(size),
            dirty: false,
            options,
        });
        Ok(renderer)
    }
    pub fn options(&self) -> DisplayOptions {
        self.surface.as_ref().map(|s| s.options).unwrap_or_default()
    }
    pub fn set_options(&mut self, options: DisplayOptions) -> GalResult<()> {
        options.validate().map_err(GalError::invalid_argument)?;
        if let Some(surface) = &mut self.surface {
            surface.options = options;
        }
        Ok(())
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(surface) = &mut self.surface {
            surface.size = extent((width, height));
            surface.dirty = true;
        }
    }
    pub fn stats(&self) -> RenderStats {
        self.stats
    }
    pub fn viewport(&self, canvas: &Canvas) -> Option<Viewport> {
        let s = self.surface.as_ref()?;
        Viewport::calculate(
            (canvas.width, canvas.height),
            (s.size.width, s.size.height),
            s.options,
        )
    }
    pub fn present(&mut self, canvas: &Canvas, textures: &Textures) -> GalResult<PresentOutcome> {
        canvas
            .validate(textures)
            .map_err(|e| GalError::invalid_argument(e.to_string()))?;
        let surface = self
            .surface
            .as_mut()
            .ok_or_else(|| GalError::invalid_argument("renderer has no window surface"))?;
        if surface.size.width == 0 || surface.size.height == 0 {
            return Ok(PresentOutcome::Deferred);
        }
        self.correlation += 1;
        let correlation_id = FrameCorrelationId(self.correlation);
        let size = surface.size;
        let options = surface.options;
        if surface.dirty {
            self.gal.resize_frame_surface(FrameResizeDesc {
                correlation_id,
                extent: size,
            })?;
            surface.dirty = false;
        }
        let acquired = self.gal.acquire_frame(FrameAcquireDesc {
            correlation_id,
            expected_extent: size,
        })?;
        if matches!(
            acquired.status,
            FrameAcquireStatus::Minimized | FrameAcquireStatus::Resized
        ) {
            return Ok(PresentOutcome::Deferred);
        }
        let result = (|| {
            let target = self.gal.create_frame_target(FrameTargetDesc {
                label: "2d-window-target".into(),
                frame_id: acquired.frame.0,
                render_target: acquired.render_target,
                extent: acquired.extent,
                color_format: acquired.color_format,
            })?;
            let result = (|| {
                let operations = self.record(
                    canvas,
                    textures,
                    target,
                    acquired.color_format,
                    acquired.extent,
                    options,
                )?;
                let token = self.submit(operations)?;
                let presented = self.gal.present_frame(PresentFrameDesc {
                    frame: acquired.frame,
                    correlation_id,
                    wait_for: token.submission,
                })?;
                self.gal.retire_through(token.submission)?;
                if acquired.status == FrameAcquireStatus::Suboptimal
                    || matches!(
                        presented.status,
                        FramePresentStatus::Suboptimal | FramePresentStatus::OutOfDate
                    )
                {
                    self.surface.as_mut().unwrap().dirty = true;
                }
                Ok(
                    if matches!(
                        presented.status,
                        FramePresentStatus::Presented | FramePresentStatus::Suboptimal
                    ) {
                        PresentOutcome::Presented
                    } else {
                        PresentOutcome::Deferred
                    },
                )
            })();
            self.gal.destroy(target)?;
            result
        })();
        if result.is_err() {
            let _ = self.gal.cancel_frame(acquired.frame);
        }
        result
    }
    /// Real GPU rendering/readback using the same quad shaders as native frames.
    /// Returns tightly packed top-to-bottom RGBA8, without creating a window.
    pub fn render_to_image(&mut self, canvas: &Canvas, textures: &Textures) -> GalResult<Vec<u8>> {
        canvas
            .validate(textures)
            .map_err(|e| GalError::invalid_argument(e.to_string()))?;
        let size = extent((canvas.width, canvas.height));
        let texture = self.gal.create_texture(TextureDesc {
            label: "2d-offscreen".into(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent: size,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        })?;
        let view = self.gal.create_texture_view(TextureViewDesc {
            label: "2d-offscreen-view".into(),
            texture,
            format: TextureFormat::Rgba8Unorm,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })?;
        let target = self.gal.create_render_target(RenderTargetDesc {
            label: "2d-offscreen-target".into(),
            color_views: vec![view],
            depth_stencil_view: None,
            extent: size,
        })?;
        let readback = self.gal.create_buffer(BufferDesc {
            label: "2d-readback".into(),
            size: u64::from(size.width) * u64::from(size.height) * 4,
            memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        })?;
        let result = (|| {
            let mut operations = self.record(
                canvas,
                textures,
                target,
                TextureFormat::Rgba8Unorm,
                size,
                DisplayOptions::default(),
            )?;
            operations.extend([
                barrier(
                    texture,
                    TextureUsageState::ColorAttachment,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::CopyTextureToBuffer(copy(readback, texture, size)),
                barrier(
                    readback,
                    TextureUsageState::TransferDst,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::HostReadBuffer {
                    buffer: readback,
                    offset: 0,
                    size: u64::from(size.width) * u64::from(size.height) * 4,
                },
            ]);
            let token = self.submit(operations)?;
            self.gal.retire_through(token.submission)?;
            let read = self
                .gal
                .completed_host_reads()
                .into_iter()
                .find(|read| read.buffer == readback)
                .ok_or_else(|| GalError::backend("2D readback did not complete"))?;
            Ok(read.bytes)
        })();
        self.gal.destroy(readback)?;
        self.gal.destroy(target)?;
        self.gal.destroy(view)?;
        self.gal.destroy(texture)?;
        result
    }
    fn submit(&mut self, operations: Vec<CommandOp>) -> GalResult<SyncToken> {
        let result = (|| {
            let list = self.gal.create_command_list(CommandListDesc {
                label: "2d-frame".into(),
                operations,
            })?;
            self.gal.submit(SubmissionBatch {
                label: "2d-submit".into(),
                command_lists: vec![list],
            })
        })();
        let mut cleanup = Ok(());
        for pass in std::mem::take(&mut self.pending_passes) {
            if let Err(error) = self.gal.destroy(pass) {
                cleanup = Err(error);
            }
        }
        result.and_then(|token| cleanup.map(|_| token))
    }

    fn reserve(&mut self, geometry: usize, uniform: usize) -> GalResult<()> {
        if geometry <= self.geometry_capacity && uniform <= self.uniform_capacity {
            return Ok(());
        }
        // One frame in flight: all previous uses have completed before reallocation.
        let ids: Vec<_> = self.textures.keys().copied().collect();
        for id in ids {
            self.destroy_texture(id)?;
        }
        if geometry > self.geometry_capacity {
            let next = geometry.next_power_of_two();
            let new = buffer(&mut self.gal, "2d-geometry", next, BufferUsage::Storage)?;
            self.gal.destroy(self.geometry)?;
            self.geometry = new;
            self.geometry_capacity = next;
        }
        if uniform > self.uniform_capacity {
            let next = uniform.next_power_of_two();
            let new = buffer(&mut self.gal, "2d-frame", next, BufferUsage::Uniform)?;
            self.gal.destroy(self.uniform)?;
            self.uniform = new;
            self.uniform_capacity = next;
        }
        Ok(())
    }
    fn destroy_texture(&mut self, id: TextureId) -> GalResult<()> {
        if let Some(t) = self.textures.remove(&id) {
            for h in t.sets {
                self.gal.destroy(h)?;
            }
            for h in t.combined {
                self.gal.destroy(h)?;
            }
            for h in t.samplers {
                self.gal.destroy(h)?;
            }
            self.gal.destroy(t.view)?;
            self.gal.destroy(t.texture)?;
        }
        Ok(())
    }
    fn upload(&mut self, id: TextureId, assets: &Textures) -> GalResult<()> {
        if self.textures.contains_key(&id) {
            return Ok(());
        }
        let image = assets.get(id).unwrap();
        let size = extent((image.width, image.height));
        let texture = self.gal.create_texture(TextureDesc {
            label: "2d-texture".into(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent: size,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
        })?;
        let upload = buffer(
            &mut self.gal,
            "2d-texture-upload",
            image.rgba.len(),
            BufferUsage::TransferSrc,
        )?;
        let token = self.submit(vec![
            CommandOp::HostWriteBuffer {
                buffer: upload,
                offset: 0,
                data: image.rgba.to_vec(),
            },
            barrier(
                upload,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ),
            barrier(
                texture,
                TextureUsageState::Undefined,
                TextureUsageState::TransferDst,
            ),
            CommandOp::CopyBufferToTexture(copy(upload, texture, size)),
            barrier(
                texture,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ),
        ])?;
        self.gal.retire_through(token.submission)?;
        self.gal.destroy(upload)?;
        self.stats.texture_upload_bytes += image.rgba.len();
        let view = self.gal.create_texture_view(TextureViewDesc {
            label: "2d-texture-view".into(),
            texture,
            format: TextureFormat::Rgba8Unorm,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })?;
        let mut samplers = Vec::new();
        let mut combined = Vec::new();
        let mut sets = Vec::new();
        for filter in [SamplerFilter::Nearest, SamplerFilter::Linear] {
            let sampler = self.gal.create_sampler(SamplerDesc {
                label: "2d-sampler".into(),
                min_filter: filter,
                mag_filter: filter,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            let pair = self
                .gal
                .create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: "2d-image".into(),
                    texture_view: view,
                    sampler,
                })?;
            let set = self.gal.create_resource_set(ResourceSetDesc {
                label: "2d-set".into(),
                layout: self.resource_layout,
                bindings: vec![
                    binding(
                        0,
                        pair,
                        ResourceBindingKind::CombinedTextureSampler,
                        None,
                        vec![],
                    ),
                    binding(
                        1,
                        self.geometry,
                        ResourceBindingKind::StorageBuffer,
                        None,
                        vec![],
                    ),
                    binding(
                        2,
                        self.uniform,
                        ResourceBindingKind::UniformBuffer,
                        Some(32),
                        vec![0],
                    ),
                ],
            })?;
            samplers.push(sampler);
            combined.push(pair);
            sets.push(set);
        }
        self.textures.insert(
            id,
            TextureGpu {
                texture,
                view,
                samplers: [samplers[0], samplers[1]],
                combined: [combined[0], combined[1]],
                sets: [sets[0], sets[1]],
            },
        );
        Ok(())
    }
    fn pipeline(&mut self, format: TextureFormat) -> GalResult<Handle> {
        if let Some(pipeline) = self.pipelines.get(&format) {
            return Ok(*pipeline);
        }
        let pipeline = self.gal.create_graphics_pipeline(GraphicsPipelineDesc {
            label: "2d-quads".into(),
            layout: self.layout,
            vertex_shader: self.vertex,
            fragment_shader: self.fragment,
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::None,
            front_face: FrontFace::CounterClockwise,
            provoking_vertex: ProvokingVertex::First,
            raster_y_direction: RasterYDirection::Down,
            blend: BlendMode::Alpha,
            depth_compare: None,
            depth_write: false,
            depth_bias: None,
            color_formats: vec![format],
            depth_format: None,
            stencil: None,
        })?;
        self.pipelines.insert(format, pipeline);
        Ok(pipeline)
    }
    fn record(
        &mut self,
        canvas: &Canvas,
        assets: &Textures,
        target: Handle,
        format: TextureFormat,
        size: Extent3d,
        options: DisplayOptions,
    ) -> GalResult<Vec<CommandOp>> {
        self.stats = RenderStats {
            quads: canvas.quads.len(),
            ..Default::default()
        };
        let mut data = Vec::with_capacity(canvas.quads.len() * 80);
        let mut batches: Vec<Batch> = Vec::new();
        for (index, q) in canvas.quads.iter().enumerate() {
            let image = assets.get(q.texture).unwrap();
            let source = q.source.unwrap_or(crate::Rect::new(
                0.0,
                0.0,
                image.width as f32,
                image.height as f32,
            ));
            let values = [
                q.origin[0],
                q.origin[1],
                q.x[0],
                q.x[1],
                q.y[0],
                q.y[1],
                0.0,
                0.0,
                source.x / image.width as f32,
                source.y / image.height as f32,
                (source.x + source.width) / image.width as f32,
                (source.y + source.height) / image.height as f32,
                q.color.0[0] as f32 / 255.0,
                q.color.0[1] as f32 / 255.0,
                q.color.0[2] as f32 / 255.0,
                q.color.0[3] as f32 / 255.0,
                q.clip.x,
                q.clip.y,
                q.clip.x + q.clip.width,
                q.clip.y + q.clip.height,
            ];
            data.extend(values.into_iter().flat_map(f32::to_ne_bytes));
            let filter = q.filter.unwrap_or(options.filter);
            if let Some(batch) = batches.last_mut()
                && batch.texture == q.texture
                && batch.filter == filter
            {
                batch.count += 1;
            } else {
                batches.push(Batch {
                    texture: q.texture,
                    filter,
                    first: index as u32,
                    count: 1,
                });
            }
        }
        self.stats.batches = batches.len();
        self.stats.geometry_upload_bytes = data.len();
        self.reserve(
            data.len().max(80),
            batches.len().max(1) * self.uniform_stride,
        )?;
        let removed: Vec<_> = self
            .textures
            .keys()
            .filter(|id| assets.get(**id).is_none())
            .copied()
            .collect();
        for id in removed {
            self.destroy_texture(id)?;
        }
        for batch in &batches {
            self.upload(batch.texture, assets)?;
        }
        let viewport = Viewport::calculate(
            (canvas.width, canvas.height),
            (size.width, size.height),
            options,
        )
        .ok_or_else(|| GalError::invalid_argument("invalid 2D viewport"))?;
        let mut uniforms = vec![0; batches.len().max(1) * self.uniform_stride];
        for (i, batch) in batches.iter().enumerate() {
            let values = [
                canvas.width as f32,
                canvas.height as f32,
                batch.first as f32,
                0.0,
                viewport.x as f32 / size.width as f32,
                viewport.y as f32 / size.height as f32,
                viewport.width as f32 / size.width as f32,
                viewport.height as f32 / size.height as f32,
            ];
            let bytes: Vec<_> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
            uniforms[i * self.uniform_stride..i * self.uniform_stride + 32].copy_from_slice(&bytes);
        }
        let pipeline = self.pipeline(format)?;
        let pass = self.gal.create_render_pass(RenderPassDesc {
            label: "2d-pass".into(),
            target,
            color_formats: vec![format],
            depth_format: None,
        })?;
        let attachment = self.gal.pass_target_color_attachment(target)?;
        let clear = canvas.clear.0;
        let mut operations = Vec::new();
        if !data.is_empty() {
            operations.extend([
                barrier(
                    self.geometry,
                    TextureUsageState::ShaderRead,
                    TextureUsageState::TransferDst,
                ),
                CommandOp::HostWriteBuffer {
                    buffer: self.geometry,
                    offset: 0,
                    data,
                },
                barrier(
                    self.geometry,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ),
            ]);
        }
        operations.extend([
            barrier(
                self.uniform,
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferDst,
            ),
            CommandOp::HostWriteBuffer {
                buffer: self.uniform,
                offset: 0,
                data: uniforms,
            },
            barrier(
                self.uniform,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ),
            CommandOp::BeginPass {
                pass,
                target,
                colors: vec![PassAttachment {
                    view: attachment,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: Some(ClearColor {
                        r: clear[0] as f32 / 255.0,
                        g: clear[1] as f32 / 255.0,
                        b: clear[2] as f32 / 255.0,
                        a: clear[3] as f32 / 255.0,
                    }),
                }],
                depth_stencil: None,
            },
            CommandOp::BindGraphicsPipeline(pipeline),
        ]);
        for (i, batch) in batches.iter().enumerate() {
            let set =
                self.textures[&batch.texture].sets[usize::from(batch.filter == Filter::Linear)];
            operations.extend([
                CommandOp::BindResourceSet {
                    pipeline_layout: self.layout,
                    set_index: 0,
                    set,
                    dynamic_offsets: vec![(i * self.uniform_stride) as u64],
                },
                CommandOp::Draw {
                    vertices: 6,
                    instances: batch.count,
                },
            ]);
        }
        operations.push(CommandOp::EndPass);
        // submit() destroys the transient pass after GAL has retained its use.
        self.pending_passes.push(pass);
        Ok(operations)
    }
}
impl Drop for Renderer2D {
    fn drop(&mut self) {
        let _ = self.gal.retire_through(self.gal.latest_submission_id());
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
fn buffer(
    gal: &mut VulkanicGal,
    label: &str,
    size: usize,
    usage: BufferUsage,
) -> GalResult<Handle> {
    gal.create_buffer(BufferDesc {
        label: label.into(),
        size: size as u64,
        memory: MemoryDomain::Upload,
        usages: vec![usage, BufferUsage::HostWrite],
    })
}
fn desc(binding: u32, kind: ResourceBindingKind, dynamic_offset_count: u32) -> ResourceBindingDesc {
    ResourceBindingDesc {
        binding,
        kind,
        stages: PipelineStageFlags::DRAW,
        array_count: 1,
        optional: false,
        dynamic_offset_count,
    }
}
fn binding(
    binding: u32,
    resource: Handle,
    kind: ResourceBindingKind,
    buffer_range: Option<u64>,
    dynamic_offsets: Vec<u64>,
) -> ResourceBinding {
    ResourceBinding {
        binding,
        resource,
        kind,
        array_index: 0,
        access: AccessFlags::READ,
        dynamic_offsets,
        buffer_range,
    }
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
fn copy(buffer: Handle, texture: Handle, extent: Extent3d) -> BufferImageCopyRegion {
    BufferImageCopyRegion {
        buffer,
        buffer_offset: 0,
        bytes_per_row: extent.width * 4,
        rows_per_image: extent.height,
        texture,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent,
    }
}
