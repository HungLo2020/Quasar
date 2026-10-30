//! Explicit, bounded shadow passes on the caller's device. No scene/object registry.
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
/// Explicit budgets for one immutable prepared shadow generation.
#[derive(Clone, Copy, Debug)]
pub struct ShadowConfig {
    /// Square map extent, 32..=4096 subject to device/memory limits.
    pub resolution: u32,
    /// Maps per preparation, 1..=16. One map per directional/spot light.
    pub max_maps: usize,
    /// Total caster draws across all maps in one preparation.
    pub max_casters: usize,
    /// Budget per prepared depth array, including the minimum two layers.
    /// Applications bound outstanding native frames to control total GPU memory.
    pub max_depth_bytes: u64,
    pub raster_bias: DepthBiasState,
}
impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            resolution: 1024,
            max_maps: 2,
            max_casters: 4096,
            max_depth_bytes: 64 * 1024 * 1024,
            raster_bias: DepthBiasState {
                constant: 1,
                slope_scale: 1.5,
                clamp: 0.,
            },
        }
    }
}
/// Game-owned geometry and transform, with alpha-test coverage for a depth pass.
pub struct ShadowCaster<'a> {
    pub mesh: &'a Mesh,
    pub model: Mat4,
    pub texture_view: TextureView,
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
/// Immutable atlas generation. Native texture references retain it across prepared frames.
#[derive(Clone, Debug)]
pub struct ShadowBinding {
    pub(crate) view: TextureView,
    pub(crate) records: Vec<[f32; 24]>,
}
impl ShadowBinding {
    pub fn texture_view(&self) -> &TextureView {
        &self.view
    }
    pub fn map_count(&self) -> usize {
        self.records.len()
    }
}
pub struct PreparedShadows {
    pub commands: CommandBuffer,
    pub stats: ShadowStats,
    binding: ShadowBinding,
}
impl PreparedShadows {
    pub fn binding(&self) -> &ShadowBinding {
        &self.binding
    }
}
/// Caller-owned native depth array. Reusing it requires producer/receiver frame batches
/// to remain ordered; `prepare` instead provides independent immutable generations.
#[derive(Clone, Debug)]
pub struct ShadowTarget {
    view: TextureView,
    layers: Vec<TextureView>,
}
impl ShadowTarget {
    pub fn new(gpu: &Gpu, config: ShadowConfig) -> GraphicsResult<Self> {
        config.validate(gpu)?;
        checked(gpu, || {
            let texture = gpu.device.create_texture(&TextureDescriptor {
                label: Some("shadow target"),
                size: Extent3d {
                    width: config.resolution,
                    height: config.resolution,
                    depth_or_array_layers: config.max_maps.max(2) as u32,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            Self::from_texture(&texture)
        })?
    }
    pub fn from_texture(texture: &Texture) -> GraphicsResult<Self> {
        if texture.dimension() != TextureDimension::D2
            || texture.width() != texture.height()
            || texture.depth_or_array_layers() < 2
            || texture.format() != TextureFormat::Depth32Float
            || texture.sample_count() != 1
            || texture.mip_level_count() != 1
            || !texture
                .usage()
                .contains(TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING)
        {
            return Err(error(
                "shadow target requires a sampled/renderable square Depth32Float array",
            ));
        }
        let view = texture.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..Default::default()
        });
        let layers = (0..texture.depth_or_array_layers())
            .map(|layer| {
                texture.create_view(&TextureViewDescriptor {
                    dimension: Some(TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        Ok(Self { view, layers })
    }
    pub fn texture_view(&self) -> &TextureView {
        &self.view
    }
    pub fn texture(&self) -> &Texture {
        self.view.texture()
    }
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct ShadowGroupKey {
    vertices: Buffer,
    image: TextureView,
    sampler: Sampler,
}
#[derive(Default)]
struct ShadowLayerCache {
    instances: Option<Buffer>,
    groups: HashMap<ShadowGroupKey, BindGroup>,
}
pub struct ShadowRenderState3D {
    config: ShadowConfig,
    layers: Vec<ShadowLayerCache>,
    batch_buffer: Buffer,
    bindings: BindGroupLayout,
    pipeline: RenderPipeline,
    sampler: Sampler,
    samplers: BTreeMap<TextureView, Sampler>,
    submission: SubmissionConfig,
    work: RenderWork,
    released: bool,
    #[cfg(feature = "diagnostics")]
    diagnostics: quasar_diagnostics::Diagnostics,
}
impl ShadowConfig {
    pub fn depth_bytes(self) -> Option<u64> {
        u64::from(self.resolution)
            .checked_mul(u64::from(self.resolution))?
            .checked_mul(self.max_maps.max(2) as u64)?
            .checked_mul(4)
    }
    fn validate(self, gpu: &Gpu) -> GraphicsResult<()> {
        if !(32..=4096).contains(&self.resolution)
            || self.resolution > gpu.device.limits().max_texture_dimension_2d
            || !(1..=16).contains(&self.max_maps)
            || self.max_maps.max(2) as u32 > gpu.device.limits().max_texture_array_layers
            || !(1..=4096).contains(&self.max_casters)
            || !self.raster_bias.slope_scale.is_finite()
            || self.raster_bias.slope_scale < 0.
            || self.raster_bias.constant < 0
            || !self.raster_bias.clamp.is_finite()
            || self.depth_bytes().is_none_or(|b| b > self.max_depth_bytes)
        {
            return Err(error(
                "invalid shadow resolution/map/caster/depth-memory budget or raster bias",
            ));
        }
        Ok(())
    }
}
impl ShadowRenderState3D {
    pub fn new(gpu: &Gpu, config: ShadowConfig) -> GraphicsResult<Self> {
        Self::with_submission(gpu, config, SubmissionConfig::default())
    }
    pub fn with_submission(
        gpu: &Gpu,
        config: ShadowConfig,
        submission: SubmissionConfig,
    ) -> GraphicsResult<Self> {
        if gpu.device.limits().max_storage_buffers_per_shader_stage < 2 {
            return Err(error("shadow rendering requires vertex storage buffers"));
        }
        config.validate(gpu)?;
        submission.validate(gpu)?;
        let entries = [
            image_entry(0, TextureViewDimension::D2, false),
            storage_entry(1, ShaderStages::VERTEX),
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            storage_entry(7, ShaderStages::VERTEX_FRAGMENT),
            sampler_entry(16, false),
        ];
        let bindings = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("shadow resources"),
                entries: &entries,
            });
        let layout = gpu
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("shadow layout"),
                bind_group_layouts: &[Some(&bindings)],
                immediate_size: 0,
            });
        let vertex = checked(gpu, || {
            gpu.device.create_shader_module(ShaderModuleDescriptor {
                label: Some("shadow vertex WGSL"),
                source: ShaderSource::Wgsl(include_str!("shadow.vert.wgsl").into()),
            })
        })?;
        let fragment = checked(gpu, || {
            gpu.device.create_shader_module(ShaderModuleDescriptor {
                label: Some("shadow alpha WGSL"),
                source: ShaderSource::Wgsl(include_str!("shadow.frag.wgsl").into()),
            })
        })?;
        let pipeline = checked(gpu, || {
            gpu.device
                .create_render_pipeline(&RenderPipelineDescriptor {
                    label: Some("shadow pipeline"),
                    layout: Some(&layout),
                    vertex: VertexState {
                        module: &vertex,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: Some(DepthStencilState {
                        format: TextureFormat::Depth32Float,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(CompareFunction::LessEqual),
                        stencil: Default::default(),
                        bias: config.raster_bias,
                    }),
                    multisample: Default::default(),
                    fragment: Some(FragmentState {
                        module: &fragment,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        targets: &[],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
        })?;
        let sampler = gpu.device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            layers: (0..config.max_maps)
                .map(|_| ShadowLayerCache::default())
                .collect(),
            batch_buffer: buffer(gpu, "shadow base", &[0; 16], BufferUsages::UNIFORM),
            config,
            bindings,
            pipeline,
            sampler,
            samplers: BTreeMap::new(),
            submission,
            work: RenderWork::default(),
            released: false,
            #[cfg(feature = "diagnostics")]
            diagnostics: quasar_diagnostics::Diagnostics::default(),
        })
    }
    pub fn resize(&mut self, gpu: &Gpu, resolution: u32) -> GraphicsResult<()> {
        let mut next = Self::with_submission(
            gpu,
            ShadowConfig {
                resolution,
                ..self.config
            },
            self.submission,
        )?;
        next.samplers = self.samplers.clone();
        #[cfg(feature = "diagnostics")]
        {
            next.diagnostics = self.diagnostics.clone();
        }
        *self = next;
        Ok(())
    }
    pub fn config(&self) -> ShadowConfig {
        self.config
    }
    pub fn work_stats(&self) -> RenderWork {
        self.work
    }
    #[cfg(feature = "diagnostics")]
    pub fn set_diagnostics(&mut self, d: quasar_diagnostics::Diagnostics) {
        self.diagnostics = d;
    }
    pub fn set_texture_sampler(
        &mut self,
        _gpu: &Gpu,
        view: &TextureView,
        sampler: &Sampler,
    ) -> GraphicsResult<()> {
        self.samplers.insert(view.clone(), sampler.clone());
        Ok(())
    }
    pub fn create_target(&self, gpu: &Gpu) -> GraphicsResult<ShadowTarget> {
        if self.released {
            return Err(error("shadow renderer released"));
        }
        ShadowTarget::new(gpu, self.config)
    }
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        views: &[ShadowView<'_>],
    ) -> GraphicsResult<PreparedShadows> {
        let target = self.create_target(gpu)?;
        self.prepare_into(gpu, views, &target)
    }
    /// Encode into an explicit reusable native target. Keep each producer and its
    /// receivers together on the queue before another frame writes that target.
    pub fn prepare_into(
        &mut self,
        gpu: &Gpu,
        views: &[ShadowView<'_>],
        target: &ShadowTarget,
    ) -> GraphicsResult<PreparedShadows> {
        checked(gpu, || {
            let total: usize = views.iter().map(|v| v.casters.len()).sum();
            if self.released
                || views.len() > self.config.max_maps
                || total > self.config.max_casters
                || total > self.submission.max_instances
            {
                return Err(error(
                    "released shadow renderer or map/caster budget exceeded",
                ));
            }
            let mut light_indices = std::collections::BTreeSet::new();
            for v in views {
                if v.light_index >= 4096
                    || !light_indices.insert(v.light_index)
                    || !v.view_projection.finite()
                    || v.view_projection.0.iter().flatten().all(|x| *x == 0.)
                    || !v.settings.bias.is_finite()
                    || !(0. ..=0.1).contains(&v.settings.bias)
                    || !v.settings.normal_bias.is_finite()
                    || v.settings.normal_bias < 0.
                    || !v.settings.strength.is_finite()
                    || !(0. ..=1.).contains(&v.settings.strength)
                    || v.casters.iter().any(|c| {
                        c.mesh.vertices.is_none()
                            || c.mesh.indices.is_none()
                            || !lighting::invertible_affine(c.model)
                            || !(0. ..=1.).contains(&c.alpha_cutoff)
                            || !(0. ..=1.).contains(&c.opacity)
                    })
                {
                    return Err(error("invalid shadow matrix/light/settings/caster"));
                }
            }
            let size = target.texture().size();
            if size.width != self.config.resolution
                || size.height != self.config.resolution
                || size.depth_or_array_layers < self.config.max_maps.max(2) as u32
            {
                return Err(error(
                    "shadow target does not match configured resolution/map capacity",
                ));
            }
            let view = target.view.clone();
            let mut encoder = gpu.encoder("shadow maps");
            let mut records = Vec::new();
            let mut stats = ShadowStats {
                maps: views.len(),
                allocated_depth_bytes: u64::from(size.width)
                    * u64::from(size.height)
                    * u64::from(size.depth_or_array_layers)
                    * 4,
                ..Default::default()
            };
            let mut work = RenderWork {
                candidates: total,
                ..Default::default()
            };
            for (layer, v) in views.iter().enumerate() {
                let visibility =
                    Visibility::new(self.submission.frustum_culling, v.view_projection);
                let mut data = Vec::new();
                let mut batches: Vec<(&Mesh, TextureView, Sampler, u32, u32)> = Vec::new();
                let mut count = 0;
                for c in v.casters {
                    if !visibility.intersects(c.mesh, c.model) {
                        work.culled += 1;
                        continue;
                    }
                    data.extend(bytes(
                        v.view_projection
                            .0
                            .iter()
                            .flatten()
                            .chain(c.model.0.iter().flatten())
                            .copied()
                            .chain([c.alpha_cutoff, c.opacity, 0., 0.]),
                    ));
                    let sampler = self
                        .samplers
                        .get(&c.texture_view)
                        .unwrap_or(&self.sampler)
                        .clone();
                    if self.submission.instancing
                        && let Some(b) = batches.last_mut()
                        && std::ptr::eq(b.0, c.mesh)
                        && b.1 == c.texture_view
                        && b.2 == sampler
                    {
                        b.4 += 1;
                    } else {
                        batches.push((c.mesh, c.texture_view.clone(), sampler, count, 1));
                    }
                    count += 1;
                }
                stats.uniform_upload_bytes += data.len();
                work.visible += count as usize;
                work.draws += batches.len();
                stats.draws += batches.len();
                if data.is_empty() {
                    data.resize(144, 0);
                }
                let cache = &mut self.layers[layer];
                if quasar_frame::grow_buffer(
                    gpu,
                    &mut cache.instances,
                    "shadow instances",
                    data.len() as u64,
                    BufferUsages::STORAGE,
                )? || cache.groups.len() > 4096
                {
                    cache.groups.clear();
                }
                let upload = quasar_frame::staging_buffer(gpu, "shadow instance upload", &data);
                encoder.copy_buffer_to_buffer(
                    &upload,
                    0,
                    cache.instances.as_ref().unwrap(),
                    0,
                    data.len() as u64,
                );
                let groups: Vec<_> = batches
                    .iter()
                    .map(|b| {
                        let key = ShadowGroupKey {
                            vertices: b.0.vertices.as_ref().unwrap().clone(),
                            image: b.1.clone(),
                            sampler: b.2.clone(),
                        };
                        if let Some(group) = cache.groups.get(&key) {
                            return group.clone();
                        }
                        let group = gpu.device.create_bind_group(&BindGroupDescriptor {
                            label: Some("shadow draw"),
                            layout: &self.bindings,
                            entries: &[
                                BindGroupEntry {
                                    binding: 0,
                                    resource: BindingResource::TextureView(&b.1),
                                },
                                BindGroupEntry {
                                    binding: 1,
                                    resource: b.0.vertices.as_ref().unwrap().as_entire_binding(),
                                },
                                BindGroupEntry {
                                    binding: 2,
                                    resource: self.batch_buffer.as_entire_binding(),
                                },
                                BindGroupEntry {
                                    binding: 7,
                                    resource: cache.instances.as_ref().unwrap().as_entire_binding(),
                                },
                                BindGroupEntry {
                                    binding: 16,
                                    resource: BindingResource::Sampler(&b.2),
                                },
                            ],
                        });
                        cache.groups.insert(key, group.clone());
                        group
                    })
                    .collect();
                let layer_view = &target.layers[layer];
                {
                    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                        label: Some("shadow depth"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                            view: layer_view,
                            depth_ops: Some(Operations {
                                load: LoadOp::Clear(1.),
                                store: StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    pass.set_pipeline(&self.pipeline);
                    for (b, group) in batches.iter().zip(&groups) {
                        pass.set_bind_group(0, group, &[]);
                        pass.set_index_buffer(
                            b.0.indices.as_ref().unwrap().slice(..),
                            IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..b.0.count, 0, b.3..b.3 + b.4);
                    }
                }
                let mut record = [0.; 24];
                record[..16].copy_from_slice(
                    &v.view_projection
                        .0
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>(),
                );
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
            work.instance_upload_bytes = stats.uniform_upload_bytes;
            self.work = work;
            #[cfg(feature = "diagnostics")]
            self.work.report(&self.diagnostics);
            Ok(PreparedShadows {
                commands: encoder.finish(),
                stats,
                binding: ShadowBinding { view, records },
            })
        })?
    }
    pub fn clear_bindings(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.samplers.clear();
        for layer in &mut self.layers {
            layer.groups.clear();
        }
        Ok(())
    }
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.released = true;
        self.samplers.clear();
        Ok(())
    }
}
