use crate::{Canvas, DisplayOptions, Filter, TextureId, Textures, Viewport};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::{collections::BTreeMap, sync::Arc};
use vulkanic_gal::{commands::SubmissionUsage, resources::*, *};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentOutcome {
    Presented,
    Deferred,
}
/// Whether a 2D pass clears its target or preserves the existing image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOp {
    Clear,
    Load,
}
#[derive(Debug, Default, Clone, Copy)]
pub struct RenderStats {
    pub quads: usize,
    pub batches: usize,
    pub geometry_upload_bytes: usize,
    pub texture_upload_bytes: usize,
}
/// Owned GAL commands. Keeping this value (or a command-list clone) reserves
/// its buffers. Dropping unsubmitted commands abandons the reservation.
#[derive(Debug, Clone)]
pub struct Prepared2D {
    pub commands: CommandList,
    pub stats: RenderStats,
}
struct TextureGpu {
    texture: Option<Handle>,
    view: Handle,
    samplers: [Handle; 2],
    combined: [Handle; 2],
}
struct Slot {
    geometry: Handle,
    uniform: Handle,
    geometry_capacity: usize,
    uniform_capacity: usize,
    sets: BTreeMap<TextureId, [Handle; 2]>,
    passes: BTreeMap<Handle, Handle>,
    usage: SubmissionUsage,
}
struct Surface {
    size: Extent3d,
    dirty: bool,
    options: DisplayOptions,
    targets: BTreeMap<FrameRenderTargetId, Handle>,
}
struct Batch {
    texture: TextureId,
    filter: Filter,
    first: u32,
    count: u32,
}
/// Standalone convenience owner. Existing games keep their original API.
pub struct Renderer2D {
    gal: VulkanicGal,
    state: RenderState2D,
    surface: Option<Surface>,
    correlation: u64,
    frame_overlap: bool,
}
impl Renderer2D {
    pub fn headless() -> GalResult<Self> {
        Self::from_gal(VulkanicGal::create(
            BackendChoice::Vulkan,
            "Quasar 2D offscreen",
        )?)
    }
    pub fn from_gal(mut gal: VulkanicGal) -> GalResult<Self> {
        let state = RenderState2D::new(&mut gal)?;
        Ok(Self {
            gal,
            state,
            surface: None,
            correlation: 0,
            frame_overlap: false,
        })
    }
    /// Enables completion polling and bounded multi-frame resource reuse.
    /// The default retains synchronous native retirement for existing clients.
    /// Readback always waits regardless of this presentation policy.
    pub fn set_frame_overlap(&mut self, enabled: bool) {
        self.frame_overlap = enabled;
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
                max_frames_in_flight: 3,
            },
        )?;
        let mut renderer = Self::from_gal(gal)?;
        renderer.surface = Some(Surface {
            size: extent(size),
            dirty: false,
            options,
            targets: BTreeMap::new(),
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
        self.state.stats
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
            self.state.release_targets(&mut self.gal)?;
            for h in std::mem::take(&mut surface.targets).into_values() {
                self.gal.retire(h)?;
            }
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
            let target = if let Some(target) = self
                .surface
                .as_ref()
                .unwrap()
                .targets
                .get(&acquired.render_target)
            {
                *target
            } else {
                let target = self.gal.create_frame_target(FrameTargetDesc {
                    label: "2d-window-target".into(),
                    frame_id: acquired.frame.0,
                    render_target: acquired.render_target,
                    extent: acquired.extent,
                    color_format: acquired.color_format,
                })?;
                self.surface
                    .as_mut()
                    .unwrap()
                    .targets
                    .insert(acquired.render_target, target);
                target
            };
            (|| {
                let prepared = self.state.prepare(
                    &mut self.gal,
                    canvas,
                    textures,
                    target,
                    options,
                    LoadOp::Clear,
                )?;
                let token = self.gal.submit(SubmissionBatch {
                    label: "2d-frame".into(),
                    command_lists: vec![prepared.commands],
                })?;
                let presented = self.gal.present_frame(PresentFrameDesc {
                    frame: acquired.frame,
                    correlation_id,
                    wait_for: token.submission,
                })?;
                if self.frame_overlap {
                    self.gal.retire_completed()?;
                } else {
                    self.gal.retire_through(token.submission)?;
                }
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
            })()
        })();
        if result.is_err() {
            let _ = self.gal.cancel_frame(acquired.frame);
            let _ = self.state.release_targets(&mut self.gal);
            for h in std::mem::take(&mut self.surface.as_mut().unwrap().targets).into_values() {
                let _ = self.gal.retire(h);
            }
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
            let prepared = self.state.prepare(
                &mut self.gal,
                canvas,
                textures,
                target,
                DisplayOptions::default(),
                LoadOp::Clear,
            )?;
            let mut operations = prepared.commands.operations;
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
            let token = self.gal.submit(SubmissionBatch {
                label: "2d-readback".into(),
                command_lists: vec![CommandList::from(CommandListDesc {
                    label: "2d-readback".into(),
                    operations,
                })],
            })?;
            self.gal.retire_through(token.submission)?;
            let read = self
                .gal
                .completed_host_reads()
                .into_iter()
                .find(|read| read.buffer == readback)
                .ok_or_else(|| GalError::backend("2D readback did not complete"))?;
            Ok(read.bytes)
        })();
        self.state.release_targets(&mut self.gal)?;
        self.gal.destroy(readback)?;
        self.gal.destroy(target)?;
        self.gal.destroy(view)?;
        self.gal.destroy(texture)?;
        result
    }
}
impl Drop for Renderer2D {
    fn drop(&mut self) {
        let _ = self.gal.retire_through(self.gal.latest_submission_id());
        let _ = self.state.release(&mut self.gal);
        if let Some(surface) = &mut self.surface {
            for h in std::mem::take(&mut surface.targets).into_values() {
                let _ = self.gal.retire(h);
            }
        }
        let _ = self.gal.shutdown_frame_surface();
    }
}
/// Reusable rendering state on a caller-owned GAL. No window, submission or
/// presentation ownership. All methods must use the same GAL as `new`.
/// Call `release` before dropping to return cached resources to GAL.
pub struct RenderState2D {
    layout: Handle,
    resource_layout: Handle,
    vertex: Handle,
    fragment: Handle,
    pipelines: BTreeMap<(TextureFormat, Option<TextureFormat>), Handle>,
    slots: Vec<Slot>,
    textures: BTreeMap<TextureId, TextureGpu>,
    uniform_stride: usize,
    stats: RenderStats,
}
impl RenderState2D {
    pub fn new(gal: &mut VulkanicGal) -> GalResult<Self> {
        let mut created = Vec::new();
        let result = (|| {
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
            created.push(resource_layout);
            let layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: "2d-layout".into(),
                resource_layouts: vec![resource_layout],
            })?;
            created.push(layout);
            let vertex = gal.create_shader_module(shader(
                "2d-vertex",
                ShaderStage::Vertex,
                include_str!("quad.vert"),
            ))?;
            created.push(vertex);
            let fragment = gal.create_shader_module(shader(
                "2d-fragment",
                ShaderStage::Fragment,
                include_str!("quad.frag"),
            ))?;
            created.push(fragment);
            Ok(Self {
                layout,
                resource_layout,
                vertex,
                fragment,
                pipelines: BTreeMap::new(),
                slots: Vec::new(),
                textures: BTreeMap::new(),
                uniform_stride: stride,
                stats: RenderStats::default(),
            })
        })();
        if result.is_err() {
            for h in created.into_iter().rev() {
                let _ = gal.retire(h);
            }
        }
        result
    }

    pub fn stats(&self) -> RenderStats {
        self.stats
    }
    /// Prepares a pass without acquiring, presenting, or waiting for this pass.
    /// First-use CPU images are uploaded on this GAL queue before their draws.
    /// Up to three independent command lists may reserve transient slots.
    pub fn prepare(
        &mut self,
        gal: &mut VulkanicGal,
        canvas: &Canvas,
        assets: &Textures,
        target: Handle,
        options: DisplayOptions,
        load: LoadOp,
    ) -> GalResult<Prepared2D> {
        if self.layout.is_null() {
            return Err(GalError::invalid_argument("2D state is released"));
        }
        canvas
            .validate(assets)
            .map_err(|e| GalError::invalid_argument(e.to_string()))?;
        options.validate().map_err(GalError::invalid_argument)?;
        let format = gal.pass_target_color_format(target)?;
        let size = gal.pass_target_extent(target)?;
        let depth = None;
        gal.begin_command_recording()?;
        let result = self.record(
            gal, canvas, assets, target, format, size, depth, options, load,
        );
        let finish = gal.finish_command_recording();
        result.and_then(|prepared| finish.map(|()| prepared))
    }
    /// Registers a private view of another renderer's sampled image. The source
    /// owner keeps the texture; this renderer owns only its view and bindings.
    /// The caller orders writes -> ShaderRead before recording consumers.
    pub fn register_texture(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &mut Textures,
        source_view: Handle,
    ) -> GalResult<TextureId> {
        if self.layout.is_null() {
            return Err(GalError::invalid_argument("2D state is released"));
        }
        let info = gal.texture_view_info(source_view)?;
        if info.dimension != TextureDimension::D2
            || info.extent.width > 16384
            || info.extent.height > 16384
            || info.range.base_mip != 0
            || info.range.mip_count != 1
            || info.range.layer_count != 1
            || info.extent.depth != 1
            || !info.usages.contains(&TextureUsage::Sampled)
            || !matches!(
                info.format,
                TextureFormat::Rgba8Unorm | TextureFormat::Bgra8Unorm
            )
        {
            return Err(GalError::invalid_argument(
                "shared image must be a sampled 2D RGBA8/BGRA8 single-mip view",
            ));
        }
        let view = gal.create_texture_view(TextureViewDesc {
            label: "2d-shared-view".into(),
            texture: info.texture,
            format: info.format,
            base_mip: 0,
            mip_count: 1,
            base_layer: info.range.base_layer,
            layer_count: 1,
        })?;
        let id = assets
            .declare_external(info.extent.width, info.extent.height)
            .map_err(|e| GalError::invalid_argument(e.to_string()))?;
        if let Err(error) =
            self.install_texture(gal, id, None, view, info.extent.width, info.extent.height)
        {
            assets.remove(id);
            let _ = gal.retire(view);
            return Err(error);
        }
        Ok(id)
    }
    fn install_texture(
        &mut self,
        gal: &mut VulkanicGal,
        id: TextureId,
        texture: Option<Handle>,
        view: Handle,
        _width: u32,
        _height: u32,
    ) -> GalResult<()> {
        let mut samplers = Vec::new();
        let mut combined = Vec::new();
        let result = (|| {
            for filter in [SamplerFilter::Nearest, SamplerFilter::Linear] {
                let sampler = gal.create_sampler(SamplerDesc {
                    label: "2d-sampler".into(),
                    min_filter: filter,
                    mag_filter: filter,
                    mip_filter: SamplerFilter::Nearest,
                    address_u: SamplerAddressMode::ClampToEdge,
                    address_v: SamplerAddressMode::ClampToEdge,
                    address_w: SamplerAddressMode::ClampToEdge,
                    comparison: None,
                })?;
                samplers.push(sampler);
                combined.push(
                    gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                        label: "2d-image".into(),
                        texture_view: view,
                        sampler,
                    })?,
                );
            }
            Ok(())
        })();
        if let Err(error) = result {
            for h in combined.into_iter().rev() {
                let _ = gal.retire(h);
            }
            for h in samplers.into_iter().rev() {
                let _ = gal.retire(h);
            }
            return Err(error);
        }
        self.textures.insert(
            id,
            TextureGpu {
                texture,
                view,
                samplers: [samplers[0], samplers[1]],
                combined: [combined[0], combined[1]],
            },
        );
        Ok(())
    }
    fn collect_removed(&mut self, gal: &mut VulkanicGal, assets: &Textures) -> GalResult<()> {
        for slot in &mut self.slots {
            if slot.usage.has_pending_commands() {
                continue;
            }
            let removed: Vec<_> = slot
                .sets
                .keys()
                .filter(|id| assets.dimensions(**id).is_none())
                .copied()
                .collect();
            for id in removed {
                for h in slot.sets.remove(&id).unwrap() {
                    gal.retire(h)?;
                }
            }
        }
        let removed: Vec<_> = self
            .textures
            .keys()
            .filter(|id| assets.dimensions(**id).is_none())
            .copied()
            .collect();
        for id in removed {
            let t = self.textures.remove(&id).unwrap();
            for h in t.combined {
                gal.retire(h)?;
            }
            for h in t.samplers {
                gal.retire(h)?;
            }
            gal.retire(t.view)?;
            if let Some(h) = t.texture {
                gal.retire(h)?;
            }
        }
        Ok(())
    }
    fn ensure_set(&mut self, gal: &mut VulkanicGal, index: usize, id: TextureId) -> GalResult<()> {
        let slot = &mut self.slots[index];
        if slot.sets.contains_key(&id) {
            return Ok(());
        }
        let texture = &self.textures[&id];
        let mut sets = Vec::new();
        let result = (|| {
            for pair in texture.combined {
                sets.push(gal.create_resource_set(ResourceSetDesc {
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
                            slot.geometry,
                            ResourceBindingKind::StorageBuffer,
                            None,
                            vec![],
                        ),
                        binding(
                            2,
                            slot.uniform,
                            ResourceBindingKind::UniformBuffer,
                            Some(32),
                            vec![0],
                        ),
                    ],
                })?);
            }
            Ok(())
        })();
        if let Err(error) = result {
            for h in sets {
                let _ = gal.retire(h);
            }
            return Err(error);
        }
        slot.sets.insert(id, [sets[0], sets[1]]);
        Ok(())
    }
    fn acquire_slot(
        &mut self,
        gal: &mut VulkanicGal,
        geometry: usize,
        uniform: usize,
    ) -> GalResult<usize> {
        let completed = gal.poll_completed();
        let mut index = self.slots.iter().position(|s| {
            !s.usage.has_pending_commands() && s.usage.last_submission() <= completed
        });
        if index.is_none() && self.slots.len() < 3 {
            let g = geometry.next_power_of_two();
            let u = uniform.next_power_of_two();
            self.slots.push(Slot {
                geometry: buffer(gal, "2d-geometry", g, BufferUsage::Storage)?,
                uniform: buffer(gal, "2d-frame", u, BufferUsage::Uniform)?,
                geometry_capacity: g,
                uniform_capacity: u,
                sets: BTreeMap::new(),
                passes: BTreeMap::new(),
                usage: SubmissionUsage::default(),
            });
            index = Some(self.slots.len() - 1);
        }
        if index.is_none() {
            let (i, submission) = self
                .slots
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.usage.has_pending_commands())
                .map(|(i, s)| (i, s.usage.last_submission()))
                .min_by_key(|(_, id)| *id)
                .ok_or_else(|| {
                    GalError::submission(
                        StatusCode::InFlight,
                        "all 2D slots are reserved by unsubmitted commands",
                    )
                })?;
            gal.retire_through(submission)?;
            index = Some(i);
        }
        let i = index.unwrap();
        let s = &mut self.slots[i];
        if geometry > s.geometry_capacity || uniform > s.uniform_capacity {
            for sets in std::mem::take(&mut s.sets).into_values() {
                for h in sets {
                    gal.retire(h)?;
                }
            }
            if geometry > s.geometry_capacity {
                gal.retire(s.geometry)?;
                s.geometry_capacity = geometry.next_power_of_two();
                s.geometry = buffer(
                    gal,
                    "2d-geometry",
                    s.geometry_capacity,
                    BufferUsage::Storage,
                )?;
            }
            if uniform > s.uniform_capacity {
                gal.retire(s.uniform)?;
                s.uniform_capacity = uniform.next_power_of_two();
                s.uniform = buffer(gal, "2d-frame", s.uniform_capacity, BufferUsage::Uniform)?;
            }
        }
        Ok(i)
    }
    pub fn release_targets(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.require_no_pending()?;
        for slot in &mut self.slots {
            for h in std::mem::take(&mut slot.passes).into_values() {
                gal.retire(h)?;
            }
        }
        Ok(())
    }
    fn require_no_pending(&self) -> GalResult<()> {
        if self.slots.iter().any(|s| s.usage.has_pending_commands()) {
            return Err(GalError::submission(
                StatusCode::InFlight,
                "2D commands still retain resources",
            ));
        }
        Ok(())
    }
    /// Explicit cleanup; rejected without mutation while prepared commands live.
    /// Submitted work uses GAL's deferred retirement, without a device idle wait.
    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self.layout.is_null() {
            return Ok(());
        }
        self.require_no_pending()?;
        self.release_targets(gal)?;
        for slot in self.slots.drain(..) {
            for sets in slot.sets.into_values() {
                for h in sets {
                    gal.retire(h)?;
                }
            }
            gal.retire(slot.geometry)?;
            gal.retire(slot.uniform)?;
        }
        for t in std::mem::take(&mut self.textures).into_values() {
            for h in t.combined {
                gal.retire(h)?;
            }
            for h in t.samplers {
                gal.retire(h)?;
            }
            gal.retire(t.view)?;
            if let Some(h) = t.texture {
                gal.retire(h)?;
            }
        }
        for h in std::mem::take(&mut self.pipelines).into_values() {
            gal.retire(h)?;
        }
        for h in [
            self.layout,
            self.vertex,
            self.fragment,
            self.resource_layout,
        ] {
            gal.retire(h)?;
        }
        self.layout = Handle::NULL;
        Ok(())
    }
    fn upload(&mut self, gal: &mut VulkanicGal, id: TextureId, assets: &Textures) -> GalResult<()> {
        if self.textures.contains_key(&id) {
            return Ok(());
        }
        let image = assets.get(id).ok_or_else(|| {
            GalError::invalid_argument("GPU-only image is not registered on this renderer")
        })?;
        let size = extent((image.width, image.height));
        let mut created = Vec::new();
        let result = (|| {
            let texture = gal.create_texture(TextureDesc {
                label: "2d-texture".into(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: size,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
            })?;
            created.push(texture);
            let upload = buffer(
                gal,
                "2d-texture-upload",
                image.rgba.len(),
                BufferUsage::TransferSrc,
            )?;
            created.push(upload);
            let operations = vec![
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
            ];
            gal.submit(SubmissionBatch {
                label: "2d-initial-upload".into(),
                command_lists: vec![CommandList::from(CommandListDesc {
                    label: "2d-initial-upload".into(),
                    operations,
                })],
            })?;
            gal.destroy(upload)?;
            self.stats.texture_upload_bytes += image.rgba.len();
            let view = gal.create_texture_view(TextureViewDesc {
                label: "2d-texture-view".into(),
                texture,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(view);
            self.install_texture(gal, id, Some(texture), view, image.width, image.height)?;
            Ok(())
        })();
        if result.is_err() {
            for h in created.into_iter().rev() {
                let _ = gal.retire(h);
            }
        }
        result
    }
    fn pipeline(
        &mut self,
        gal: &mut VulkanicGal,
        format: TextureFormat,
        depth: Option<TextureFormat>,
    ) -> GalResult<Handle> {
        if let Some(pipeline) = self.pipelines.get(&(format, depth)) {
            return Ok(*pipeline);
        }
        let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
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
            depth_format: depth,
            stencil: None,
        })?;
        self.pipelines.insert((format, depth), pipeline);
        Ok(pipeline)
    }
    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        gal: &mut VulkanicGal,
        canvas: &Canvas,
        assets: &Textures,
        target: Handle,
        format: TextureFormat,
        size: Extent3d,
        depth: Option<(Handle, Handle, TextureFormat)>,
        options: DisplayOptions,
        load: LoadOp,
    ) -> GalResult<Prepared2D> {
        self.collect_removed(gal, assets)?;
        self.stats = RenderStats {
            quads: canvas.quads.len(),
            ..Default::default()
        };
        let mut data = Vec::with_capacity(canvas.quads.len() * 80);
        let mut batches: Vec<Batch> = Vec::new();
        for (index, q) in canvas.quads.iter().enumerate() {
            let image = assets.dimensions(q.texture).unwrap();
            let source =
                q.source
                    .unwrap_or(crate::Rect::new(0.0, 0.0, image.0 as f32, image.1 as f32));
            let values = [
                q.origin[0],
                q.origin[1],
                q.x[0],
                q.x[1],
                q.y[0],
                q.y[1],
                0.0,
                0.0,
                source.x / image.0 as f32,
                source.y / image.1 as f32,
                (source.x + source.width) / image.0 as f32,
                (source.y + source.height) / image.1 as f32,
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
        let index = self.acquire_slot(
            gal,
            data.len().max(80),
            batches.len().max(1) * self.uniform_stride,
        )?;
        for batch in &batches {
            self.upload(gal, batch.texture, assets)?;
            self.ensure_set(gal, index, batch.texture)?;
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
        let pipeline = self.pipeline(gal, format, depth.map(|(_, _, format)| format))?;
        let slot = &mut self.slots[index];
        if !slot.passes.contains_key(&target) {
            if slot.passes.len() >= 8 {
                for h in std::mem::take(&mut slot.passes).into_values() {
                    gal.retire(h)?;
                }
            }
            let pass = gal.create_render_pass(RenderPassDesc {
                label: "2d-pass".into(),
                target,
                color_formats: vec![format],
                depth_format: None,
            })?;
            slot.passes.insert(target, pass);
        }
        let pass = slot.passes[&target];
        let attachment = gal.pass_target_color_attachment(target)?;
        let clear = canvas.clear.0;
        let mut operations = Vec::new();
        if !data.is_empty() {
            operations.extend([
                barrier(
                    slot.geometry,
                    TextureUsageState::ShaderRead,
                    TextureUsageState::TransferDst,
                ),
                CommandOp::HostWriteBuffer {
                    buffer: slot.geometry,
                    offset: 0,
                    data,
                },
                barrier(
                    slot.geometry,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ),
            ]);
        }
        operations.extend([
            barrier(
                slot.uniform,
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferDst,
            ),
            CommandOp::HostWriteBuffer {
                buffer: slot.uniform,
                offset: 0,
                data: uniforms,
            },
            barrier(
                slot.uniform,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ),
            CommandOp::BeginPass {
                pass,
                target,
                colors: vec![PassAttachment {
                    view: attachment,
                    load_op: if load == LoadOp::Clear {
                        AttachmentLoadOp::Clear
                    } else {
                        AttachmentLoadOp::Load
                    },
                    store_op: AttachmentStoreOp::Store,
                    clear_color: (load == LoadOp::Clear).then_some(ClearColor {
                        r: clear[0] as f32 / 255.0,
                        g: clear[1] as f32 / 255.0,
                        b: clear[2] as f32 / 255.0,
                        a: clear[3] as f32 / 255.0,
                    }),
                }],
                depth_stencil: depth.map(|(_, view, _)| PassAttachment {
                    view,
                    load_op: AttachmentLoadOp::Load,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: None,
                }),
            },
            CommandOp::BindGraphicsPipeline(pipeline),
        ]);
        for (i, batch) in batches.iter().enumerate() {
            let set = slot.sets[&batch.texture][usize::from(batch.filter == Filter::Linear)];
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
        operations.push(CommandOp::TrackSubmission(slot.usage.clone()));
        let commands = gal.create_command_list(CommandListDesc {
            label: "2d-prepared".into(),
            operations,
        })?;
        Ok(Prepared2D {
            commands,
            stats: self.stats,
        })
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
