//! Explicit, bounded shadow passes on the caller's GAL. No scene/object registry.
use super::*;
/// Nearest depth comparisons, optionally averaged in a fixed square PCF kernel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadowFilter {
    #[default]
    Hard,
    Pcf3,
    Pcf5,
}
impl ShadowFilter {
    fn radius(self) -> f32 {
        match self {
            Self::Hard => 0.,
            Self::Pcf3 => 1.,
            Self::Pcf5 => 2.,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ShadowSettings {
    /// Receiver depth bias in normalized `[0,1]` light depth.
    pub bias: f32,
    /// Receiver offset along the geometric normal in world units.
    pub normal_bias: f32,
    /// Shadow contribution in 0..=1; ambient and emission remain unaffected.
    pub strength: f32,
    pub filter: ShadowFilter,
}
impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            bias: 0.001,
            normal_bias: 0.015,
            strength: 1.,
            filter: ShadowFilter::Pcf3,
        }
    }
}
/// Explicit worst-case budgets for at most three outstanding shadow generations.
#[derive(Clone, Copy, Debug)]
pub struct ShadowConfig {
    /// Square map extent, 32..=4096 subject to device/memory limits.
    pub resolution: u32,
    /// Maps per preparation, 1..=16. One map per directional/spot light.
    pub max_maps: usize,
    /// Total caster draws across all maps in one preparation.
    pub max_casters: usize,
    /// Budget for all three depth-array slots, including the minimum two layers.
    pub max_depth_bytes: u64,
    pub raster_bias: DepthBias,
}
impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            resolution: 1024,
            max_maps: 2,
            max_casters: 4096,
            max_depth_bytes: 64 * 1024 * 1024,
            raster_bias: DepthBias {
                constant_factor: 1.,
                slope_factor: 1.5,
            },
        }
    }
}
/// Game-owned geometry and transform, with alpha-test coverage for a depth pass.
pub struct ShadowCaster<'a> {
    pub mesh: &'a Mesh,
    pub model: Mat4,
    pub texture_view: Handle,
    pub alpha_cutoff: f32,
    pub opacity: f32,
}
/// One explicit light-space view; no scene fitting, discovery or culling.
pub struct ShadowView<'a> {
    /// Index into each receiving pass's Lighting::lights slice.
    pub light_index: usize,
    /// Right-handed light matrix with `[0,1]` clip depth.
    pub view_projection: Mat4,
    pub settings: ShadowSettings,
    pub casters: &'a [ShadowCaster<'a>],
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ShadowStats {
    pub maps: usize,
    pub draws: usize,
    pub uniform_upload_bytes: usize,
    pub allocated_depth_bytes: u64,
}
/// Owns a reservation on the atlas generation. Keep it until all receivers are prepared.
/// Cloning/holding the binding prevents overwriting maps still referenced by consumers.
#[derive(Clone, Debug)]
pub struct ShadowBinding {
    pub(crate) view: Handle,
    pub(crate) pair: Handle,
    pub(crate) records: Vec<[f32; 24]>,
    pub(crate) usage: SubmissionUsage,
}
impl ShadowBinding {
    pub fn texture_view(&self) -> Handle {
        self.view
    }
    /// Include TrackSubmission with this receipt in custom GAL consumers of the view.
    pub fn submission_usage(&self) -> SubmissionUsage {
        self.usage.clone()
    }
    pub fn map_count(&self) -> usize {
        self.records.len()
    }
}
/// Submit commands before all receivers using the accompanying binding.
pub struct PreparedShadows {
    pub commands: CommandList,
    pub stats: ShadowStats,
    binding: ShadowBinding,
}
impl PreparedShadows {
    pub fn binding(&self) -> &ShadowBinding {
        &self.binding
    }
}
struct ShadowSlot {
    texture: Handle,
    view: Handle,
    pair: Handle,
    targets: Vec<Handle>,
    passes: Vec<Handle>,
    owned: Vec<Handle>,
    uniform: Handle,
    instances: Option<Handle>,
    instance_capacity: usize,
    capacity: usize,
    sets: BTreeMap<(Handle, Handle), Handle>,
    usage: SubmissionUsage,
}
/// Depth-only map producer using the caller's device and submission ordering.
pub struct ShadowRenderState3D {
    config: ShadowConfig,
    layout: Handle,
    resource_layout: Handle,
    pipeline: Handle,
    sampler: Handle,
    owned: Vec<Handle>,
    slots: Vec<ShadowSlot>,
    pairs: BTreeMap<Handle, Handle>,
    stride: usize,
    submission: SubmissionConfig,
    work: RenderWork,
    #[cfg(feature = "diagnostics")]
    diagnostics: quasar_diagnostics::Diagnostics,
}
impl ShadowConfig {
    pub fn depth_bytes(self) -> Option<u64> {
        u64::from(self.resolution)
            .checked_mul(u64::from(self.resolution))?
            .checked_mul(self.max_maps.max(2) as u64)?
            .checked_mul(4 * 3)
    }
    fn validate(self, gal: &VulkanicGal) -> GalResult<()> {
        let c = gal.capabilities();
        if !(32..=4096).contains(&self.resolution)
            || !(1..=16).contains(&self.max_maps)
            || self.max_casters > 4096
            || self.max_casters == 0
            || !self.raster_bias.is_finite()
            || self.raster_bias.constant_factor < 0.
            || self.raster_bias.slope_factor < 0.
            || self
                .depth_bytes()
                .is_none_or(|bytes| bytes > self.max_depth_bytes)
        {
            return Err(GalError::invalid_argument(
                "invalid shadow resolution/map/caster/depth-memory budget or raster bias",
            ));
        }
        if !c.features.depth_only_pass
            || !c.features.texture_array_layers
            || !c.features.storage_buffers
            || !c.features.dynamic_buffer_offsets
            || !c.features.uniform_buffers
            || self.resolution > c.limits.max_texture_extent_2d
            || self.max_maps.max(2) as u32 > c.limits.max_texture_array_layers
            || c.limits.max_resource_layout_bindings < 3
        {
            return Err(GalError::unsupported_feature(
                "shadow maps require depth-only passes, sampled depth arrays, storage/uniform buffers and dynamic offsets",
            ));
        }
        if c.shader_conventions.glsl_dialect != GlslDialect::ExplicitBindings
            || !c.shader_conventions.zero_to_one_clip_depth
        {
            return Err(GalError::unsupported_feature(
                "shadow shaders require explicit bindings and [0,1] clip depth",
            ));
        }
        Ok(())
    }
}
impl ShadowRenderState3D {
    pub fn new(gal: &mut VulkanicGal, config: ShadowConfig) -> GalResult<Self> {
        Self::with_submission(gal, config, SubmissionConfig::default())
    }
    pub fn with_submission(
        gal: &mut VulkanicGal,
        config: ShadowConfig,
        submission: SubmissionConfig,
    ) -> GalResult<Self> {
        config.validate(gal)?;
        submission.validate(gal)?;
        let mut owned = vec![];
        let result = (|| {
            let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: "3d-shadow-resources".into(),
                bindings: {
                    let mut bindings = vec![
                        desc(0, ResourceBindingKind::CombinedTextureSampler, 0),
                        desc(1, ResourceBindingKind::StorageBuffer, 0),
                        desc(2, ResourceBindingKind::UniformBuffer, 1),
                    ];
                    if submission.instancing {
                        bindings.push(desc(7, ResourceBindingKind::StorageBuffer, 0));
                    }
                    bindings
                },
            })?;
            owned.push(resource_layout);
            let layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: "3d-shadow-layout".into(),
                resource_layouts: vec![resource_layout],
            })?;
            owned.push(layout);
            let vertex = gal.create_shader_module(shader(
                "3d-shadow-vertex",
                ShaderStage::Vertex,
                &if submission.instancing {
                    instanced_shader(include_str!("shadow.vert"), true)
                } else {
                    include_str!("shadow.vert").into()
                },
            ))?;
            owned.push(vertex);
            let fragment = gal.create_shader_module(shader(
                "3d-shadow-fragment",
                ShaderStage::Fragment,
                &if submission.instancing {
                    instanced_shader(include_str!("shadow.frag"), false)
                } else {
                    include_str!("shadow.frag").into()
                },
            ))?;
            owned.push(fragment);
            let sampler = gal.create_sampler(SamplerDesc {
                label: "3d-shadow-alpha-sampler".into(),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            owned.push(sampler);
            let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: "3d-shadow-depth".into(),
                layout,
                vertex_shader: vertex,
                fragment_shader: fragment,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: FrontFace::CounterClockwise,
                provoking_vertex: ProvokingVertex::First,
                raster_y_direction: RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: Some(CompareOp::LessOrEqual),
                depth_write: true,
                depth_bias: Some(config.raster_bias),
                color_formats: vec![],
                depth_format: Some(TextureFormat::Depth32Float),
                stencil: None,
            })?;
            owned.push(pipeline);
            Ok((resource_layout, layout, pipeline, sampler))
        })();
        match result {
            Ok((resource_layout, layout, pipeline, sampler)) => Ok(Self {
                config,
                layout,
                resource_layout,
                pipeline,
                sampler,
                owned,
                slots: vec![],
                pairs: BTreeMap::new(),
                stride: (gal.capabilities().limits.uniform_buffer_offset_alignment as usize)
                    .max(if submission.instancing { 16 } else { 144 })
                    .next_power_of_two(),
                submission,
                work: RenderWork::default(),
                #[cfg(feature = "diagnostics")]
                diagnostics: quasar_diagnostics::Diagnostics::disabled(),
            }),
            Err(e) => {
                for h in owned.into_iter().rev() {
                    let _ = gal.retire(h);
                }
                Err(e)
            }
        }
    }
    pub fn work_stats(&self) -> RenderWork {
        self.work
    }
    #[cfg(feature = "diagnostics")]
    pub fn set_diagnostics(&mut self, d: quasar_diagnostics::Diagnostics) {
        self.diagnostics = d;
    }
    fn slot(
        &self,
        gal: &mut VulkanicGal,
        bytes: usize,
        instance_bytes: usize,
    ) -> GalResult<ShadowSlot> {
        let mut owned = vec![];
        let result = (|| {
            let texture = gal.create_texture(TextureDesc {
                label: "3d-shadow-array".into(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent: Extent3d {
                    width: self.config.resolution,
                    height: self.config.resolution,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: self.config.max_maps.max(2) as u32,
                usages: vec![TextureUsage::DepthStencilAttachment, TextureUsage::Sampled],
            })?;
            owned.push(texture);
            let view = gal.create_texture_view(TextureViewDesc {
                label: "3d-shadow-sampled-array".into(),
                texture,
                format: TextureFormat::Depth32Float,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: self.config.max_maps.max(2) as u32,
            })?;
            owned.push(view);
            let sampler = gal.create_sampler(SamplerDesc {
                label: "3d-shadow-comparison".into(),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: Some(CompareOp::LessOrEqual),
            })?;
            owned.push(sampler);
            let pair = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                label: "3d-shadow-pair".into(),
                texture_view: view,
                sampler,
            })?;
            owned.push(pair);
            let mut targets = vec![];
            let mut passes = vec![];
            for layer in 0..self.config.max_maps {
                let layer_view = gal.create_texture_view(TextureViewDesc {
                    label: format!("3d-shadow-layer-{layer}"),
                    texture,
                    format: TextureFormat::Depth32Float,
                    base_mip: 0,
                    mip_count: 1,
                    base_layer: layer as u32,
                    layer_count: 1,
                })?;
                owned.push(layer_view);
                let target = gal.create_render_target(RenderTargetDesc {
                    label: "3d-shadow-target".into(),
                    color_views: vec![],
                    depth_stencil_view: Some(layer_view),
                    extent: Extent3d {
                        width: self.config.resolution,
                        height: self.config.resolution,
                        depth: 1,
                    },
                })?;
                owned.push(target);
                targets.push(target);
                let pass = gal.create_render_pass(RenderPassDesc {
                    label: "3d-shadow-pass".into(),
                    target,
                    color_formats: vec![],
                    depth_format: Some(TextureFormat::Depth32Float),
                })?;
                owned.push(pass);
                passes.push(pass);
            }
            let uniform = buffer(
                gal,
                "3d-shadow-uniforms",
                bytes.next_power_of_two(),
                BufferUsage::Uniform,
            )?;
            owned.push(uniform);
            let instances = if self.submission.instancing {
                let h = buffer(
                    gal,
                    "3d-shadow-instances",
                    instance_bytes.next_power_of_two(),
                    BufferUsage::Storage,
                )?;
                owned.push(h);
                Some(h)
            } else {
                None
            };
            Ok((texture, view, pair, targets, passes, uniform, instances))
        })();
        match result {
            Ok((texture, view, pair, targets, passes, uniform, instances)) => {
                if instances.is_some() {
                    owned.pop();
                }
                owned.pop();
                Ok(ShadowSlot {
                    texture,
                    view,
                    pair,
                    targets,
                    passes,
                    uniform,
                    instances,
                    instance_capacity: instance_bytes.next_power_of_two(),
                    capacity: bytes.next_power_of_two(),
                    owned,
                    sets: BTreeMap::new(),
                    usage: SubmissionUsage::default(),
                })
            }
            Err(e) => {
                for h in owned.into_iter().rev() {
                    let _ = gal.retire(h);
                }
                Err(e)
            }
        }
    }
    /// Record passes without submitting. Each outstanding generation reserves a slot.
    pub fn prepare(
        &mut self,
        gal: &mut VulkanicGal,
        views: &[ShadowView<'_>],
    ) -> GalResult<PreparedShadows> {
        #[cfg(feature = "diagnostics")]
        let _span = self.diagnostics.span("quasar.render3d", "shadows");
        let count: usize = views.iter().map(|v| v.casters.len()).sum();
        let mut indexes = std::collections::BTreeSet::new();
        if self.layout.is_null()
            || views.len() > self.config.max_maps
            || count > self.config.max_casters
            || count > self.submission.max_instances
        {
            return Err(GalError::invalid_argument(
                "released shadow renderer or shadow map/caster budget exceeded",
            ));
        }
        for v in views {
            if v.light_index >= 4096
                || !indexes.insert(v.light_index)
                || !v.view_projection.finite()
                || v.view_projection.0.iter().flatten().all(|x| *x == 0.)
                || !v.settings.bias.is_finite()
                || !(0. ..=0.1).contains(&v.settings.bias)
                || !v.settings.normal_bias.is_finite()
                || v.settings.normal_bias < 0.
                || !v.settings.strength.is_finite()
                || !(0. ..=1.).contains(&v.settings.strength)
                || v.casters.iter().any(|c| {
                    c.mesh.vertices.is_null()
                        || !c.model.finite()
                        || !c.alpha_cutoff.is_finite()
                        || !(0. ..=1.).contains(&c.alpha_cutoff)
                        || !c.opacity.is_finite()
                        || !(0. ..=1.).contains(&c.opacity)
                })
            {
                return Err(GalError::invalid_argument(
                    "invalid shadow matrix/light index/settings/caster",
                ));
            }
            for c in v.casters {
                let info = gal.texture_view_info(c.texture_view)?;
                if info.dimension != TextureDimension::D2
                    || info.range.layer_count != 1
                    || info.format == TextureFormat::R8Uint
                    || !info.usages.contains(&TextureUsage::Sampled)
                {
                    return Err(GalError::invalid_argument(
                        "shadow alpha texture must be sampled normalized/float 2D",
                    ));
                }
            }
        }
        let bytes = count.max(1) * self.stride;
        let completed = gal.poll_completed();
        let mut selected = self.slots.iter().position(|s| {
            !s.usage.has_pending_commands() && s.usage.last_submission() <= completed
        });
        if selected.is_none() && self.slots.len() < 3 {
            let slot = self.slot(gal, bytes, count.max(1) * 144)?;
            self.slots.push(slot);
            selected = Some(self.slots.len() - 1);
        }
        if selected.is_none() {
            let (i, id) = self
                .slots
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.usage.has_pending_commands())
                .map(|(i, s)| (i, s.usage.last_submission()))
                .min_by_key(|(_, id)| *id)
                .ok_or_else(|| {
                    GalError::submission(
                        StatusCode::InFlight,
                        "all shadow slots have outstanding producer/receiver bindings",
                    )
                })?;
            gal.retire_through(id)?;
            selected = Some(i);
        }
        let index = selected.unwrap();
        if self.slots[index].capacity < bytes {
            let new = buffer(
                gal,
                "3d-shadow-uniforms",
                bytes.next_power_of_two(),
                BufferUsage::Uniform,
            )?;
            let slot = &mut self.slots[index];
            for h in std::mem::take(&mut slot.sets).into_values() {
                gal.retire(h)?;
            }
            gal.retire(slot.uniform)?;
            slot.uniform = new;
            slot.capacity = bytes.next_power_of_two();
        }
        let instance_bytes = count.max(1) * 144;
        if self.submission.instancing && self.slots[index].instance_capacity < instance_bytes {
            let new = buffer(
                gal,
                "3d-shadow-instances",
                instance_bytes.next_power_of_two(),
                BufferUsage::Storage,
            )?;
            let slot = &mut self.slots[index];
            for h in std::mem::take(&mut slot.sets).into_values() {
                gal.retire(h)?;
            }
            gal.retire(slot.instances.unwrap())?;
            slot.instances = Some(new);
            slot.instance_capacity = instance_bytes.next_power_of_two();
        }
        gal.begin_command_recording()?;
        let result = self.record(gal, index, views, count, bytes);
        let finish = gal.finish_command_recording();
        result.and_then(|r| finish.map(|_| r))
    }
    fn record(
        &mut self,
        gal: &mut VulkanicGal,
        index: usize,
        views: &[ShadowView<'_>],
        count: usize,
        bytes: usize,
    ) -> GalResult<PreparedShadows> {
        let mut data = vec![0; bytes];
        let mut draws = vec![];
        let mut records = vec![];
        let mut draw_index = 0usize;
        let mut visible = 0usize;
        let mut instance_data = Vec::new();
        for (layer, v) in views.iter().enumerate() {
            let mut work = vec![];
            let mut previous = None;
            let visibility = Visibility::new(self.submission.frustum_culling, v.view_projection);
            for c in v.casters {
                if !visibility.intersects(c.mesh, c.model) {
                    continue;
                }
                let pair = if let Some(p) = self.pairs.get(&c.texture_view) {
                    *p
                } else {
                    let p = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                        label: "3d-shadow-alpha".into(),
                        texture_view: c.texture_view,
                        sampler: self.sampler,
                    })?;
                    self.pairs.insert(c.texture_view, p);
                    p
                };
                let slot = &mut self.slots[index];
                let key = (c.mesh.vertices, c.texture_view);
                if let std::collections::btree_map::Entry::Vacant(entry) = slot.sets.entry(key) {
                    let set = gal.create_resource_set(ResourceSetDesc {
                        label: "3d-shadow-set".into(),
                        layout: self.resource_layout,
                        bindings: {
                            let mut bindings = vec![
                                binding(
                                    0,
                                    pair,
                                    ResourceBindingKind::CombinedTextureSampler,
                                    None,
                                    vec![],
                                ),
                                binding(
                                    1,
                                    c.mesh.vertices,
                                    ResourceBindingKind::StorageBuffer,
                                    None,
                                    vec![],
                                ),
                                binding(
                                    2,
                                    slot.uniform,
                                    ResourceBindingKind::UniformBuffer,
                                    Some(if self.submission.instancing { 16 } else { 144 }),
                                    vec![0],
                                ),
                            ];
                            if let Some(instances) = slot.instances {
                                bindings.push(binding(
                                    7,
                                    instances,
                                    ResourceBindingKind::StorageBuffer,
                                    None,
                                    vec![],
                                ));
                            }
                            bindings
                        },
                    })?;
                    entry.insert(set);
                }
                let values = v
                    .view_projection
                    .0
                    .iter()
                    .flatten()
                    .chain(c.model.0.iter().flatten())
                    .copied()
                    .chain([c.alpha_cutoff, c.opacity, 0., 0.]);
                let encoded: Vec<u8> = values.flat_map(f32::to_ne_bytes).collect();
                let set = slot.sets[&key];
                if self.submission.instancing {
                    instance_data.extend_from_slice(&encoded);
                    if previous == Some((set, c.mesh.indices)) {
                        if let Some(CommandOp::DrawIndexed { instances, .. }) = work.last_mut() {
                            *instances += 1;
                        }
                        visible += 1;
                        continue;
                    }
                    data[draw_index * self.stride..draw_index * self.stride + 4]
                        .copy_from_slice(&(visible as u32).to_ne_bytes());
                } else {
                    data[draw_index * self.stride..draw_index * self.stride + 144]
                        .copy_from_slice(&encoded);
                }
                work.extend([
                    CommandOp::TrackSubmission(c.mesh.usage.clone()),
                    CommandOp::BindResourceSet {
                        pipeline_layout: self.layout,
                        set_index: 0,
                        set,
                        dynamic_offsets: [(draw_index * self.stride) as u64].into(),
                    },
                    CommandOp::SetIndexBuffer {
                        buffer: c.mesh.indices,
                        offset: 0,
                        index_type: IndexType::U32,
                    },
                    CommandOp::DrawIndexed {
                        indices: c.mesh.count,
                        instances: 1,
                    },
                ]);
                previous = Some((set, c.mesh.indices));
                draw_index += 1;
                visible += 1;
            }

            let slot = &self.slots[index];
            let (_, depth_view) = gal
                .pass_target_depth_attachment(slot.targets[layer])?
                .unwrap();
            draws.push(CommandOp::BeginPass {
                pass: slot.passes[layer],
                target: slot.targets[layer],
                colors: vec![],
                depth_stencil: Some(PassAttachment {
                    view: depth_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: None,
                }),
            });
            draws.push(CommandOp::BindGraphicsPipeline(self.pipeline));
            draws.extend(work);
            draws.push(CommandOp::EndPass);
            let mut record = [0.; 24];
            for (out, value) in record.iter_mut().zip(v.view_projection.0.iter().flatten()) {
                *out = *value;
            }
            record[16..].copy_from_slice(&[
                v.light_index as f32,
                layer as f32,
                v.settings.bias,
                v.settings.normal_bias,
                1. / self.config.resolution as f32,
                v.settings.filter.radius(),
                v.settings.strength,
                0.,
            ]);
            records.push(record);
        }
        let bytes = draw_index.max(1) * self.stride;
        data.truncate(bytes);
        self.work = RenderWork {
            candidates: count,
            visible,
            culled: count - visible,
            draws: draw_index,
            instance_upload_bytes: instance_data.len(),
        };
        #[cfg(feature = "diagnostics")]
        self.work.report(&self.diagnostics);
        let slot = &self.slots[index];
        let mut operations = vec![
            CommandOp::TrackSubmission(slot.usage.clone()),
            barrier(
                slot.uniform,
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferDst,
            ),
            CommandOp::HostWriteBuffer {
                buffer: slot.uniform,
                offset: 0,
                data,
            },
            barrier(
                slot.uniform,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ),
            barrier(
                slot.texture,
                TextureUsageState::Undefined,
                TextureUsageState::DepthStencilAttachment,
            ),
        ];
        if let Some(instances) = slot.instances
            && !instance_data.is_empty()
        {
            operations.extend([
                barrier(
                    instances,
                    TextureUsageState::ShaderRead,
                    TextureUsageState::TransferDst,
                ),
                CommandOp::HostWriteBuffer {
                    buffer: instances,
                    offset: 0,
                    data: instance_data,
                },
                barrier(
                    instances,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ),
            ]);
        }

        operations.extend(draws);
        operations.push(barrier(
            slot.texture,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::ShaderRead,
        ));
        let commands = gal.create_command_list(CommandListDesc {
            label: "3d-shadow-prepared".into(),
            operations,
        })?;
        Ok(PreparedShadows {
            commands,
            binding: ShadowBinding {
                view: slot.view,
                pair: slot.pair,
                records,
                usage: slot.usage.clone(),
            },
            stats: ShadowStats {
                maps: views.len(),
                draws: draw_index,
                uniform_upload_bytes: bytes,
                allocated_depth_bytes: self.config.depth_bytes().unwrap() / 3
                    * self.slots.len() as u64,
            },
        })
    }
    /// Clear surface receiver descriptor caches before resizing/releasing map generations.
    pub fn resize(&mut self, gal: &mut VulkanicGal, resolution: u32) -> GalResult<()> {
        let config = ShadowConfig {
            resolution,
            ..self.config
        };
        config.validate(gal)?;
        self.clear_slots(gal)?;
        self.config = config;
        Ok(())
    }
    fn clear_slots(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self.slots.iter().any(|s| s.usage.has_pending_commands()) {
            return Err(GalError::submission(
                StatusCode::InFlight,
                "shadow producer/receiver/binding still retained",
            ));
        }
        for slot in self.slots.drain(..) {
            for h in slot.sets.into_values() {
                gal.retire(h)?;
            }
            gal.retire(slot.uniform)?;
            if let Some(instances) = slot.instances {
                gal.retire(instances)?;
            }
            for h in slot.owned.into_iter().rev() {
                gal.retire(h)?;
            }
        }
        Ok(())
    }
    pub fn clear_bindings(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self.slots.iter().any(|s| s.usage.has_pending_commands()) {
            return Err(GalError::submission(
                StatusCode::InFlight,
                "shadow commands/bindings retained",
            ));
        }
        for slot in &mut self.slots {
            for h in std::mem::take(&mut slot.sets).into_values() {
                gal.retire(h)?;
            }
        }
        for h in std::mem::take(&mut self.pairs).into_values() {
            gal.retire(h)?;
        }
        Ok(())
    }
    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.clear_slots(gal)?;
        for h in std::mem::take(&mut self.pairs).into_values() {
            gal.retire(h)?;
        }
        for h in self.owned.drain(..).rev() {
            gal.retire(h)?;
        }
        self.layout = Handle::NULL;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn depth_budget_accounts_for_three_slots_and_minimum_array_layers() {
        assert_eq!(
            ShadowConfig::default().depth_bytes(),
            Some(24 * 1024 * 1024)
        );
        assert_eq!(
            ShadowConfig {
                max_maps: 1,
                ..ShadowConfig::default()
            }
            .depth_bytes(),
            Some(24 * 1024 * 1024)
        );
        assert_eq!(
            ShadowConfig {
                resolution: 2048,
                ..ShadowConfig::default()
            }
            .depth_bytes(),
            Some(96 * 1024 * 1024)
        );
        assert!(
            ShadowConfig {
                resolution: u32::MAX,
                max_maps: 16,
                ..ShadowConfig::default()
            }
            .depth_bytes()
            .is_none()
        );
    }
}
