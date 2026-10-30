//! Caller-owned mesh rendering using upstream wgpu types through VulkanicGAL.
mod lighting;
mod math;
#[cfg(feature = "model-assets")]
mod model_assets;
mod shadows;
mod submission;
mod surface;
pub use lighting::{
    Attenuation, ColorSpace, DEFAULT_LIT_FRAGMENT_SHADER, Light, Lighting, LitCamera3D, LitDraw3D,
    LitMaterial, LitPassOptions, LitRenderState3D, ShadingModel,
};
pub use math::Mat4;
#[cfg(feature = "model-assets")]
pub use model_assets::{MaterialConversion, ModelUploadOptions, UploadedModel, UploadedTexture};
use quasar_frame::{Gpu, GraphicsResult, RenderTarget, error, texture_2d};
pub use shadows::{
    PreparedShadows, ShadowBinding, ShadowCaster, ShadowConfig, ShadowFilter, ShadowRenderState3D,
    ShadowSettings, ShadowStats, ShadowTarget, ShadowView,
};
use std::collections::{BTreeMap, HashMap};
use submission::Visibility;
pub use submission::{RenderWork, SubmissionConfig};
pub use surface::{
    DEFAULT_SURFACE_FRAGMENT_SHADER, NormalMap, SurfaceDraw3D, SurfaceMaterial,
    SurfaceRenderState3D,
};
use vulkanic_gal::util::DeviceExt;
use vulkanic_gal::*;
#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshFacePolicy {
    #[default]
    Legacy,
    SingleSided,
    DoubleSided,
}
pub struct Mesh {
    vertices: Option<Buffer>,
    indices: Option<Buffer>,
    count: u32,
    face_policy: MeshFacePolicy,
    opaque_alpha: bool,
    bounds: ([f32; 3], [f32; 3]),
}
impl Mesh {
    pub fn upload(gpu: &Gpu, vertices: &[Vertex], indices: &[u32]) -> GraphicsResult<Self> {
        if vertices.is_empty()
            || indices.is_empty()
            || !indices.len().is_multiple_of(3)
            || indices.len() > u32::MAX as usize
            || indices.iter().any(|i| *i as usize >= vertices.len())
            || vertices.iter().any(|v| {
                v.position
                    .iter()
                    .chain(&v.normal)
                    .chain(&v.uv)
                    .any(|v| !v.is_finite())
            })
        {
            return Err(error("invalid indexed triangle mesh"));
        }
        if (vertices.len() as u64).saturating_mul(48)
            > gpu.device.limits().max_storage_buffer_binding_size
            || (vertices.len() as u64).saturating_mul(48) > gpu.device.limits().max_buffer_size
            || (indices.len() as u64).saturating_mul(4) > gpu.device.limits().max_buffer_size
        {
            return Err(error("mesh exceeds storage buffer binding limit"));
        }
        let encoded: Vec<_> = vertices
            .iter()
            .flat_map(|v| {
                [
                    v.position[0],
                    v.position[1],
                    v.position[2],
                    1.,
                    v.normal[0],
                    v.normal[1],
                    v.normal[2],
                    0.,
                    v.uv[0],
                    v.uv[1],
                    0.,
                    0.,
                ]
            })
            .flat_map(f32::to_ne_bytes)
            .collect();
        let index_data: Vec<_> = indices.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let vertex = gpu.device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("mesh vertices"),
            contents: &encoded,
            usage: BufferUsages::STORAGE,
        });
        let index = gpu.device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("mesh indices"),
            contents: &index_data,
            usage: BufferUsages::INDEX,
        });
        Ok(Self {
            vertices: Some(vertex),
            indices: Some(index),
            count: indices.len() as u32,
            face_policy: MeshFacePolicy::Legacy,
            opaque_alpha: false,
            bounds: (
                std::array::from_fn(|a| {
                    vertices
                        .iter()
                        .map(|v| v.position[a])
                        .fold(f32::INFINITY, f32::min)
                }),
                std::array::from_fn(|a| {
                    vertices
                        .iter()
                        .map(|v| v.position[a])
                        .fold(f32::NEG_INFINITY, f32::max)
                }),
            ),
        })
    }
    pub fn set_opaque_alpha(&mut self, value: bool) {
        self.opaque_alpha = value;
    }
    pub fn set_face_policy(&mut self, policy: MeshFacePolicy) {
        self.face_policy = policy;
    }
    pub fn local_bounds(&self) -> ([f32; 3], [f32; 3]) {
        self.bounds
    }
    /// Drops this owner's references. Encoded/submitted native commands retain their own.
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.vertices = None;
        self.indices = None;
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct Material {
    pub texture_view: TextureView,
    pub tint: [f32; 4],
    pub transparent: bool,
    pub depth_write: bool,
}
impl Material {
    pub fn textured(texture_view: &TextureView) -> Self {
        Self {
            texture_view: texture_view.clone(),
            tint: [1.; 4],
            transparent: false,
            depth_write: true,
        }
    }
}
pub struct Draw3D<'a> {
    pub mesh: &'a Mesh,
    pub model: Mat4,
    pub material: Material,
}
#[derive(Clone, Copy, Debug)]
pub struct Camera3D {
    pub view_projection: Mat4,
    pub light_direction: [f32; 3],
}
#[derive(Clone, Copy, Debug)]
pub struct PassOptions {
    pub clear_color: Option<[f32; 4]>,
    pub clear_depth: bool,
}
impl Default for PassOptions {
    fn default() -> Self {
        Self {
            clear_color: Some([0.025, 0.04, 0.07, 1.]),
            clear_depth: true,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub draws: usize,
    pub uniform_upload_bytes: usize,
}
pub struct Prepared3D {
    pub commands: CommandBuffer,
    pub stats: RenderStats,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PipelineKey {
    format: TextureFormat,
    transparent: bool,
    depth_write: bool,
    cull: bool,
    mirrored: bool,
}
struct Input<'a> {
    mesh: &'a Mesh,
    model: Mat4,
    base: &'a Material,
    lit: Option<&'a LitMaterial>,
    normal: Option<&'a NormalMap>,
    receives_shadows: bool,
}
struct Batch<'a> {
    mesh: &'a Mesh,
    image: TextureView,
    normal: TextureView,
    sampler: Sampler,
    normal_sampler: Sampler,
    key: PipelineKey,
    first: u32,
    count: u32,
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct BindingKey {
    vertices: Buffer,
    image: TextureView,
    normal: TextureView,
    sampler: Sampler,
    normal_sampler: Sampler,
}
struct Core3D {
    lit: bool,
    instances: Option<Buffer>,
    light_buffer: Option<Buffer>,
    shadow_buffer: Option<Buffer>,
    batch_buffer: Buffer,
    groups: HashMap<BindingKey, BindGroup>,
    last_shadow: Option<TextureView>,
    layout: PipelineLayout,
    bindings: BindGroupLayout,
    vertex: ShaderModule,
    fragment: ShaderModule,
    pipelines: HashMap<PipelineKey, RenderPipeline>,
    sampler: Sampler,
    normal_sampler: Sampler,
    comparison: Sampler,
    normal: TextureView,
    empty_shadow: TextureView,
    samplers: BTreeMap<TextureView, Sampler>,
    normal_samplers: BTreeMap<TextureView, Sampler>,
    depths: HashMap<TextureView, Texture>,
    submission: SubmissionConfig,
    work: RenderWork,
    released: bool,
    #[cfg(feature = "diagnostics")]
    diagnostics: quasar_diagnostics::Diagnostics,
}
use quasar_frame::validation_scope as checked;
fn buffer(gpu: &Gpu, label: &str, data: &[u8], usage: BufferUsages) -> Buffer {
    gpu.device.create_buffer_init(&util::BufferInitDescriptor {
        label: Some(label),
        contents: data,
        usage,
    })
}
fn bytes(values: impl IntoIterator<Item = f32>) -> Vec<u8> {
    values.into_iter().flat_map(f32::to_ne_bytes).collect()
}
fn storage_entry(binding: u32, visibility: ShaderStages) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn image_entry(binding: u32, dimension: TextureViewDimension, depth: bool) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Texture {
            sample_type: if depth {
                TextureSampleType::Depth
            } else {
                TextureSampleType::Float { filterable: true }
            },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}
fn sampler_entry(binding: u32, comparison: bool) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Sampler(if comparison {
            SamplerBindingType::Comparison
        } else {
            SamplerBindingType::Filtering
        }),
        count: None,
    }
}
impl Core3D {
    fn new(
        gpu: &Gpu,
        lit: bool,
        submission: SubmissionConfig,
        fragment: Option<&str>,
    ) -> GraphicsResult<Self> {
        submission.validate(gpu)?;
        if gpu.device.limits().max_storage_buffers_per_shader_stage < if lit { 3 } else { 2 } {
            return Err(error("renderer requires vertex/fragment storage buffers"));
        }
        let mut entries = vec![
            image_entry(0, TextureViewDimension::D2, false),
            storage_entry(1, ShaderStages::VERTEX),
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::VERTEX,
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
        if lit {
            entries.extend([
                storage_entry(3, ShaderStages::FRAGMENT),
                image_entry(4, TextureViewDimension::D2, false),
                image_entry(5, TextureViewDimension::D2Array, true),
                storage_entry(6, ShaderStages::FRAGMENT),
                sampler_entry(20, false),
                sampler_entry(21, true),
            ]);
        }
        let bindings = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("3D native resources"),
                entries: &entries,
            });
        let layout = gpu
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("3D native layout"),
                bind_group_layouts: &[Some(&bindings)],
                immediate_size: 0,
            });
        let vertex = checked(gpu, || {
            gpu.device.create_shader_module(ShaderModuleDescriptor {
                label: Some("3D vertex WGSL"),
                source: ShaderSource::Wgsl(
                    if lit {
                        include_str!("surface.vert.wgsl")
                    } else {
                        include_str!("mesh.vert.wgsl")
                    }
                    .into(),
                ),
            })
        })?;
        let fragment = checked(gpu, || {
            gpu.device.create_shader_module(ShaderModuleDescriptor {
                label: Some("3D fragment WGSL"),
                source: ShaderSource::Wgsl(
                    fragment
                        .unwrap_or(if lit {
                            DEFAULT_SURFACE_FRAGMENT_SHADER
                        } else {
                            include_str!("mesh.frag.wgsl")
                        })
                        .into(),
                ),
            })
        })?;
        let sampler = gpu.device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        let normal_sampler = gpu.device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            ..Default::default()
        });
        let comparison = gpu.device.create_sampler(&SamplerDescriptor {
            compare: Some(CompareFunction::LessEqual),
            ..Default::default()
        });
        let normal = texture_2d(
            &gpu.device,
            "flat normal",
            1,
            1,
            TextureFormat::Rgba8Unorm,
            TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
        )?;
        gpu.queue.write_texture(
            normal.as_image_copy(),
            &[128, 128, 255, 255],
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            normal.size(),
        );
        let empty_shadow = texture_2d(
            &gpu.device,
            "unused shadow",
            1,
            1,
            TextureFormat::Depth32Float,
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        )?;
        Ok(Self {
            instances: None,
            light_buffer: None,
            shadow_buffer: None,
            batch_buffer: buffer(gpu, "3D batch base", &[0; 16], BufferUsages::UNIFORM),
            groups: HashMap::new(),
            last_shadow: None,
            lit,
            layout,
            bindings,
            vertex,
            fragment,
            pipelines: HashMap::new(),
            sampler,
            normal_sampler,
            comparison,
            normal: normal.create_view(&Default::default()),
            empty_shadow: empty_shadow.create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..Default::default()
            }),
            samplers: BTreeMap::new(),
            normal_samplers: BTreeMap::new(),
            depths: HashMap::new(),
            submission,
            work: RenderWork::default(),
            released: false,
            #[cfg(feature = "diagnostics")]
            diagnostics: quasar_diagnostics::Diagnostics::default(),
        })
    }
    fn pipeline(&mut self, gpu: &Gpu, key: PipelineKey) -> GraphicsResult<()> {
        if self.pipelines.contains_key(&key) {
            return Ok(());
        }
        let pipeline = checked(gpu, || {
            gpu.device
                .create_render_pipeline(&RenderPipelineDescriptor {
                    label: Some("3D pipeline"),
                    layout: Some(&self.layout),
                    vertex: VertexState {
                        module: &self.vertex,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: PrimitiveState {
                        cull_mode: key.cull.then_some(Face::Back),
                        front_face: if key.mirrored {
                            FrontFace::Cw
                        } else {
                            FrontFace::Ccw
                        },
                        ..Default::default()
                    },
                    depth_stencil: Some(DepthStencilState {
                        format: TextureFormat::Depth32Float,
                        depth_write_enabled: Some(key.depth_write),
                        depth_compare: Some(CompareFunction::LessEqual),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(FragmentState {
                        module: &self.fragment,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        targets: &[Some(ColorTargetState {
                            format: key.format,
                            blend: key.transparent.then_some(BlendState::ALPHA_BLENDING),
                            write_mask: ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
        })?;
        self.pipelines.insert(key, pipeline);
        Ok(())
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Explicit caller-owned pass inputs preserve independent composition."
    )]
    fn prepare(
        &mut self,
        gpu: &Gpu,
        target: &RenderTarget,
        camera: Camera3D,
        draws: &[Input<'_>],
        options: PassOptions,
        lights: Option<&[u8]>,
        shadow: Option<&ShadowBinding>,
    ) -> GraphicsResult<Prepared3D> {
        checked(gpu, || {
            if self.released
                || !camera.view_projection.finite()
                || camera.view_projection.0.iter().flatten().all(|v| *v == 0.)
                || draws.len() > self.submission.max_instances
                || options
                    .clear_color
                    .is_some_and(|c| c.iter().any(|v| !v.is_finite()))
            {
                return Err(error(
                    "invalid/released 3D pass or instance budget exceeded",
                ));
            }
            if !self.lit
                && (camera.light_direction.iter().any(|v| !v.is_finite())
                    || camera.light_direction.iter().map(|v| v * v).sum::<f32>() < 1e-20)
            {
                return Err(error("invalid legacy light direction"));
            }
            for d in draws {
                if d.mesh.vertices.is_none()
                    || d.mesh.indices.is_none()
                    || !lighting::invertible_affine(d.model)
                    || d.base.tint.iter().any(|v| !v.is_finite())
                    || !(0. ..=1.).contains(&d.base.tint[3])
                {
                    return Err(error("invalid mesh/material/model"));
                }
            }
            let visibility =
                Visibility::new(self.submission.frustum_culling, camera.view_projection);
            let mut data = Vec::new();
            let mut batches: Vec<Batch<'_>> = Vec::new();
            let mut visible = 0;
            for d in draws {
                if !visibility.intersects(d.mesh, d.model) {
                    continue;
                }
                let key = PipelineKey {
                    format: target.format,
                    transparent: d.base.transparent,
                    depth_write: d.base.depth_write,
                    cull: d.mesh.face_policy == MeshFacePolicy::SingleSided,
                    mirrored: d.mesh.face_policy == MeshFacePolicy::SingleSided
                        && mirrored(d.model),
                };
                self.pipeline(gpu, key)?;
                let mut values: Vec<_> = camera
                    .view_projection
                    .0
                    .iter()
                    .flatten()
                    .chain(d.model.0.iter().flatten())
                    .copied()
                    .chain(d.base.tint)
                    .collect();
                if self.lit {
                    let lit = d.lit.ok_or_else(|| error("missing lit material"))?;
                    values.extend(lit.parameters());
                    values[46] = f32::from(d.receives_shadows)
                        + if d.mesh.face_policy == MeshFacePolicy::DoubleSided {
                            2.
                        } else {
                            0.
                        }
                        + if d.mesh.opaque_alpha { 4. } else { 0. };
                    values[47] = f32::from(d.normal.is_some());
                    values.extend(d.normal.map_or([1., 1., 0., 1.], |n| {
                        [
                            n.uv_scale[0],
                            n.uv_scale[1],
                            n.strength,
                            if n.flip_y { -1. } else { 1. },
                        ]
                    }));
                } else {
                    values.extend([
                        camera.light_direction[0],
                        camera.light_direction[1],
                        camera.light_direction[2],
                        0.,
                    ]);
                }
                data.extend(bytes(values));
                let normal = d.normal.map_or(&self.normal, |n| &n.texture_view).clone();
                let sampler = self
                    .samplers
                    .get(&d.base.texture_view)
                    .unwrap_or(&self.sampler)
                    .clone();
                let normal_sampler = self
                    .normal_samplers
                    .get(&normal)
                    .unwrap_or(&self.normal_sampler)
                    .clone();
                if self.submission.instancing
                    && !d.base.transparent
                    && let Some(b) = batches.last_mut()
                    && std::ptr::eq(b.mesh, d.mesh)
                    && b.image == d.base.texture_view
                    && b.normal == normal
                    && b.sampler == sampler
                    && b.normal_sampler == normal_sampler
                    && b.key == key
                {
                    b.count += 1;
                } else {
                    batches.push(Batch {
                        mesh: d.mesh,
                        image: d.base.texture_view.clone(),
                        normal,
                        sampler,
                        normal_sampler,
                        key,
                        first: visible,
                        count: 1,
                    });
                }
                visible += 1;
            }
            self.work = RenderWork {
                candidates: draws.len(),
                visible: visible as usize,
                culled: draws.len() - visible as usize,
                draws: batches.len(),
                instance_upload_bytes: data.len(),
            };
            #[cfg(feature = "diagnostics")]
            self.work.report(&self.diagnostics);
            let stats = RenderStats {
                draws: batches.len(),
                uniform_upload_bytes: data.len(),
            };
            if data.is_empty() {
                data.resize(if self.lit { 208 } else { 160 }, 0);
            }
            let instance_size = data.len() as u64;
            let mut changed = quasar_frame::grow_buffer(
                gpu,
                &mut self.instances,
                "3D instances",
                instance_size,
                BufferUsages::STORAGE,
            )?;
            let lights_offset = data.len() as u64;
            let light_data = lights.unwrap_or(&[0; 112]);
            if self.lit {
                data.extend_from_slice(light_data);
                changed |= quasar_frame::grow_buffer(
                    gpu,
                    &mut self.light_buffer,
                    "3D lights",
                    light_data.len() as u64,
                    BufferUsages::STORAGE,
                )?;
            }
            let mut shadow_values = vec![shadow.map_or(0., |s| s.records.len() as f32), 0., 0., 0.];
            if let Some(s) = shadow {
                for r in &s.records {
                    shadow_values.extend(*r);
                }
            }
            if shadow_values.len() == 4 {
                shadow_values.resize(28, 0.);
            }
            let shadow_data = bytes(shadow_values);
            let shadow_offset = data.len() as u64;
            if self.lit {
                data.extend_from_slice(&shadow_data);
                changed |= quasar_frame::grow_buffer(
                    gpu,
                    &mut self.shadow_buffer,
                    "3D shadow records",
                    shadow_data.len() as u64,
                    BufferUsages::STORAGE,
                )?;
            }
            let view = shadow.map_or(&self.empty_shadow, |s| &s.view);
            if changed || self.last_shadow.as_ref() != Some(view) || self.groups.len() > 4096 {
                self.groups.clear();
                self.last_shadow = Some(view.clone());
            }
            let upload = quasar_frame::staging_buffer(gpu, "3D frame upload", &data);
            let mut encoder = gpu.encoder("3D drawing");
            encoder.copy_buffer_to_buffer(
                &upload,
                0,
                self.instances.as_ref().unwrap(),
                0,
                instance_size,
            );
            if self.lit {
                encoder.copy_buffer_to_buffer(
                    &upload,
                    lights_offset,
                    self.light_buffer.as_ref().unwrap(),
                    0,
                    light_data.len() as u64,
                );
                encoder.copy_buffer_to_buffer(
                    &upload,
                    shadow_offset,
                    self.shadow_buffer.as_ref().unwrap(),
                    0,
                    shadow_data.len() as u64,
                );
            }
            let mut groups = Vec::with_capacity(batches.len());
            for b in &batches {
                let key = BindingKey {
                    vertices: b.mesh.vertices.as_ref().unwrap().clone(),
                    image: b.image.clone(),
                    normal: b.normal.clone(),
                    sampler: b.sampler.clone(),
                    normal_sampler: b.normal_sampler.clone(),
                };
                if let Some(group) = self.groups.get(&key) {
                    groups.push(group.clone());
                    continue;
                }
                let mut entries = vec![
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&b.image),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: b.mesh.vertices.as_ref().unwrap().as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: self.batch_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 7,
                        resource: self.instances.as_ref().unwrap().as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 16,
                        resource: BindingResource::Sampler(&b.sampler),
                    },
                ];
                if self.lit {
                    entries.extend([
                        BindGroupEntry {
                            binding: 3,
                            resource: self.light_buffer.as_ref().unwrap().as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 4,
                            resource: BindingResource::TextureView(&b.normal),
                        },
                        BindGroupEntry {
                            binding: 5,
                            resource: BindingResource::TextureView(
                                shadow.map_or(&self.empty_shadow, |s| &s.view),
                            ),
                        },
                        BindGroupEntry {
                            binding: 6,
                            resource: self.shadow_buffer.as_ref().unwrap().as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 20,
                            resource: BindingResource::Sampler(&b.normal_sampler),
                        },
                        BindGroupEntry {
                            binding: 21,
                            resource: BindingResource::Sampler(&self.comparison),
                        },
                    ]);
                }
                let group = gpu.device.create_bind_group(&BindGroupDescriptor {
                    label: Some("3D draw"),
                    layout: &self.bindings,
                    entries: &entries,
                });
                self.groups.insert(key, group.clone());
                groups.push(group);
            }
            let depth = if let Some(d) = &target.depth {
                d.clone()
            } else {
                if !self.depths.contains_key(&target.color) {
                    if self.depths.len() >= 8 {
                        self.depths.clear();
                    }
                    self.depths.insert(
                        target.color.clone(),
                        texture_2d(
                            &gpu.device,
                            "3D depth",
                            target.width,
                            target.height,
                            TextureFormat::Depth32Float,
                            TextureUsages::RENDER_ATTACHMENT,
                        )?,
                    );
                }
                self.depths[&target.color].create_view(&Default::default())
            };
            {
                let colors = [Some(RenderPassColorAttachment {
                    view: &target.color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: options.clear_color.map_or(LoadOp::Load, |c| {
                            LoadOp::Clear(Color {
                                r: c[0] as f64,
                                g: c[1] as f64,
                                b: c[2] as f64,
                                a: c[3] as f64,
                            })
                        }),
                        store: StoreOp::Store,
                    },
                })];
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("3D"),
                    color_attachments: &colors,
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(Operations {
                            load: if options.clear_depth {
                                LoadOp::Clear(1.)
                            } else {
                                LoadOp::Load
                            },
                            store: StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                for (b, group) in batches.iter().zip(&groups) {
                    pass.set_pipeline(&self.pipelines[&b.key]);
                    pass.set_bind_group(0, group, &[]);
                    pass.set_index_buffer(
                        b.mesh.indices.as_ref().unwrap().slice(..),
                        IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..b.mesh.count, 0, b.first..b.first + b.count);
                }
            }
            Ok(Prepared3D {
                commands: encoder.finish(),
                stats,
            })
        })?
    }
    fn release(&mut self) {
        self.groups.clear();
        self.instances = None;
        self.light_buffer = None;
        self.shadow_buffer = None;
        self.last_shadow = None;
        self.pipelines.clear();
        self.depths.clear();
        self.samplers.clear();
        self.normal_samplers.clear();
        self.released = true;
    }
}
pub struct RenderState3D {
    core: Core3D,
}
impl RenderState3D {
    pub fn new(gpu: &Gpu) -> GraphicsResult<Self> {
        Self::with_submission(gpu, SubmissionConfig::default())
    }
    pub fn with_submission(gpu: &Gpu, submission: SubmissionConfig) -> GraphicsResult<Self> {
        Ok(Self {
            core: Core3D::new(gpu, false, submission, None)?,
        })
    }
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        target: &RenderTarget,
        camera: Camera3D,
        draws: &[Draw3D<'_>],
        options: PassOptions,
    ) -> GraphicsResult<Prepared3D> {
        let input: Vec<_> = draws
            .iter()
            .map(|d| Input {
                mesh: d.mesh,
                model: d.model,
                base: &d.material,
                lit: None,
                normal: None,
                receives_shadows: false,
            })
            .collect();
        self.core
            .prepare(gpu, target, camera, &input, options, None, None)
    }
    pub fn work_stats(&self) -> RenderWork {
        self.core.work
    }
    #[cfg(feature = "diagnostics")]
    pub fn set_diagnostics(&mut self, d: quasar_diagnostics::Diagnostics) {
        self.core.diagnostics = d;
    }
    pub fn clear_bindings(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.core.samplers.clear();
        self.core.groups.clear();
        Ok(())
    }
    pub fn release_targets(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.core.depths.clear();
        Ok(())
    }
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.core.release();
        Ok(())
    }
}
fn mirrored(m: Mat4) -> bool {
    let a = m.0;
    a[0][0] * (a[1][1] * a[2][2] - a[2][1] * a[1][2])
        - a[1][0] * (a[0][1] * a[2][2] - a[2][1] * a[0][2])
        + a[2][0] * (a[0][1] * a[1][2] - a[1][1] * a[0][2])
        < 0.
}
