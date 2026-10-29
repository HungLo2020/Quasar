//! Optional mesh rendering through VulkanicGAL's public API. The game owns GAL,
//! assets, draw ordering, frame targets, submission and presentation.
mod lighting;
mod math;
#[cfg(feature = "model-assets")]
mod model_assets;
mod shadows;
mod submission;
#[cfg(feature = "model-assets")]
pub use model_assets::{MaterialConversion, ModelUploadOptions, UploadedModel, UploadedTexture};
mod surface;
pub use lighting::DEFAULT_LIT_FRAGMENT_SHADER;
pub use lighting::{
    Attenuation, ColorSpace, Light, Lighting, LitCamera3D, LitDraw3D, LitMaterial, LitPassOptions,
    LitRenderState3D, ShadingModel,
};
pub use math::Mat4;
pub use shadows::{
    PreparedShadows, ShadowBinding, ShadowCaster, ShadowConfig, ShadowFilter, ShadowRenderState3D,
    ShadowSettings, ShadowStats, ShadowView,
};
use std::collections::BTreeMap;
pub use submission::{RenderWork, SubmissionConfig};
use submission::{Visibility, instanced_shader};
pub use surface::{
    DEFAULT_SURFACE_FRAGMENT_SHADER, NormalMap, SurfaceDraw3D, SurfaceMaterial,
    SurfaceRenderState3D,
};
use vulkanic_gal::{commands::SubmissionUsage, resources::*, *};

#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}
/// Immutable uploaded geometry on one GAL. Explicitly release when no prepared
/// command copies remain. Submitted uses are retired by GAL, without idle waits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshFacePolicy {
    #[default]
    Legacy,
    SingleSided,
    DoubleSided,
}
pub struct Mesh {
    vertices: Handle,
    indices: Handle,
    count: u32,
    face_policy: MeshFacePolicy,
    opaque_alpha: bool,
    usage: SubmissionUsage,
    bounds: ([f32; 3], [f32; 3]),
}
impl Mesh {
    pub fn upload(gal: &mut VulkanicGal, vertices: &[Vertex], indices: &[u32]) -> GalResult<Self> {
        if vertices.is_empty()
            || indices.is_empty()
            || indices.len() > u32::MAX as usize
            || !indices.len().is_multiple_of(3)
            || indices.iter().any(|i| *i as usize >= vertices.len())
            || vertices.iter().any(|v| {
                v.position
                    .iter()
                    .chain(&v.normal)
                    .chain(&v.uv)
                    .any(|f| !f.is_finite())
            })
        {
            return Err(GalError::invalid_argument("invalid indexed triangle mesh"));
        }
        let vertex_data: Vec<u8> = vertices
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
        let index_data: Vec<u8> = indices.iter().flat_map(|i| i.to_ne_bytes()).collect();
        let vb = buffer(
            gal,
            "3d-mesh-vertices",
            vertex_data.len(),
            BufferUsage::Storage,
        )?;
        let ib = match buffer(gal, "3d-mesh-indices", index_data.len(), BufferUsage::Index) {
            Ok(h) => h,
            Err(e) => {
                let _ = gal.retire(vb);
                return Err(e);
            }
        };
        let result = gal.submit(SubmissionBatch {
            label: "3d-mesh-upload".into(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "3d-mesh-upload".into(),
                operations: vec![
                    CommandOp::HostWriteBuffer {
                        buffer: vb,
                        offset: 0,
                        data: vertex_data,
                    },
                    barrier(
                        vb,
                        TextureUsageState::TransferDst,
                        TextureUsageState::ShaderRead,
                    ),
                    CommandOp::HostWriteBuffer {
                        buffer: ib,
                        offset: 0,
                        data: index_data,
                    },
                    barrier(
                        ib,
                        TextureUsageState::TransferDst,
                        TextureUsageState::IndexRead,
                    ),
                ],
            })],
        });
        if let Err(e) = result {
            let _ = gal.retire(ib);
            let _ = gal.retire(vb);
            return Err(e);
        }
        Ok(Self {
            vertices: vb,
            indices: ib,
            count: indices.len() as u32,
            face_policy: MeshFacePolicy::Legacy,
            opaque_alpha: false,
            bounds: (
                std::array::from_fn(|axis| {
                    vertices
                        .iter()
                        .map(|v| v.position[axis])
                        .fold(f32::INFINITY, f32::min)
                }),
                std::array::from_fn(|axis| {
                    vertices
                        .iter()
                        .map(|v| v.position[axis])
                        .fold(f32::NEG_INFINITY, f32::max)
                }),
            ),
            usage: SubmissionUsage::default(),
        })
    }
    /// Ignore sampled/tint alpha on lit/surface paths, as required by opaque model materials.
    pub fn set_opaque_alpha(&mut self, opaque: bool) {
        self.opaque_alpha = opaque;
    }
    pub fn set_face_policy(&mut self, policy: MeshFacePolicy) {
        self.face_policy = policy;
    }
    /// Conservative mesh-space minimum/maximum positions, retained at upload.
    pub fn local_bounds(&self) -> ([f32; 3], [f32; 3]) {
        self.bounds
    }
    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self.usage.has_pending_commands() {
            return Err(GalError::submission(
                StatusCode::InFlight,
                "prepared commands still reference mesh",
            ));
        }
        if !self.vertices.is_null() {
            gal.retire(self.vertices)?;
            gal.retire(self.indices)?;
            self.vertices = Handle::NULL;
            self.indices = Handle::NULL;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Material {
    pub texture_view: Handle,
    pub tint: [f32; 4],
    pub transparent: bool,
    pub depth_write: bool,
}
impl Material {
    pub fn textured(texture_view: Handle) -> Self {
        Self {
            texture_view,
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
#[derive(Debug, Clone, Copy)]
pub struct Camera3D {
    pub view_projection: Mat4,
    pub light_direction: [f32; 3],
}
/// Draw lists preserve caller order. Sort opaque/transparent objects as needed.
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderStats {
    pub draws: usize,
    pub uniform_upload_bytes: usize,
}
#[derive(Debug, Clone)]
pub struct Prepared3D {
    pub commands: CommandList,
    pub stats: RenderStats,
}
struct Slot {
    uniform: Handle,
    instances: Option<Handle>,
    instance_capacity: usize,
    capacity: usize,
    sets: BTreeMap<(Handle, Handle, Handle, Handle), Handle>,
    passes: BTreeMap<Handle, Handle>,
    usage: SubmissionUsage,
    lights: Option<Handle>,
    shadows: Option<Handle>,
}
struct EmptyShadow {
    texture: Handle,
    view: Handle,
    sampler: Handle,
    pair: Handle,
}
/// Rendering state on a caller-owned device. Always use its construction GAL.
/// Keeps bounded transient slots; release explicitly before dropping.
pub struct RenderState3D {
    resource_layout: Handle,
    layout: Handle,
    vertex: Handle,
    fragment: Handle,
    sampler: Handle,
    normal_sampler: Option<Handle>,
    empty_shadow: Option<EmptyShadow>,
    pairs: BTreeMap<(Handle, bool), Handle>,
    sampler_overrides: BTreeMap<(Handle, bool), Handle>,
    pipelines: BTreeMap<(TextureFormat, TextureFormat, bool, bool, bool, bool), Handle>,
    slots: Vec<Slot>,
    stride: usize,
    uniform_size: usize,
    light_buffer_size: usize,
    shadow_buffer_size: usize,
    draw_size: usize,
    submission: SubmissionConfig,
    work: RenderWork,
    #[cfg(feature = "diagnostics")]
    diagnostics: quasar_diagnostics::Diagnostics,
}
impl RenderState3D {
    pub fn new(gal: &mut VulkanicGal) -> GalResult<Self> {
        Self::new_configured(gal, None, 0)
    }
    pub fn with_submission(gal: &mut VulkanicGal, submission: SubmissionConfig) -> GalResult<Self> {
        Self::new_submission_configured(gal, None, 0, 0, submission)
    }
    /// Caller owns the sampler. New views can be registered while old frames are pending.
    pub fn set_texture_sampler(
        &mut self,
        gal: &mut VulkanicGal,
        view: Handle,
        sampler: Handle,
        normal: bool,
    ) -> GalResult<()> {
        let key = (view, normal);
        if self.sampler_overrides.get(&key) == Some(&sampler) {
            return Ok(());
        }
        if self.pairs.contains_key(&key) {
            let associations = std::mem::take(&mut self.sampler_overrides);
            let result = self.clear_bindings(gal);
            self.sampler_overrides = associations;
            result?;
        }
        self.sampler_overrides.insert(key, sampler);
        Ok(())
    }
    pub fn work_stats(&self) -> RenderWork {
        self.work
    }
    #[cfg(feature = "diagnostics")]
    pub fn set_diagnostics(&mut self, diagnostics: quasar_diagnostics::Diagnostics) {
        self.diagnostics = diagnostics;
    }
    fn new_configured(
        gal: &mut VulkanicGal,
        lit_fragment: Option<&str>,
        light_buffer_size: usize,
    ) -> GalResult<Self> {
        Self::new_surface_configured(gal, lit_fragment, light_buffer_size, 0)
    }
    fn new_surface_configured(
        gal: &mut VulkanicGal,
        lit_fragment: Option<&str>,
        light_buffer_size: usize,
        shadow_buffer_size: usize,
    ) -> GalResult<Self> {
        Self::new_submission_configured(
            gal,
            lit_fragment,
            light_buffer_size,
            shadow_buffer_size,
            SubmissionConfig::default(),
        )
    }
    fn new_submission_configured(
        gal: &mut VulkanicGal,
        lit_fragment: Option<&str>,
        light_buffer_size: usize,
        shadow_buffer_size: usize,
        submission: SubmissionConfig,
    ) -> GalResult<Self> {
        submission.validate(gal)?;
        let caps = gal.capabilities();
        let required_bindings = if submission.instancing {
            8
        } else if shadow_buffer_size > 0 {
            7
        } else if lit_fragment.is_some() {
            4
        } else {
            3
        };
        if !caps.features.storage_buffers
            || !caps.features.uniform_buffers
            || !caps.features.dynamic_buffer_offsets
            || caps.limits.max_resource_layout_bindings < required_bindings
            || light_buffer_size.max(shadow_buffer_size) as u64 > caps.limits.max_buffer_size
            || (shadow_buffer_size > 0 && !caps.features.texture_array_layers)
        {
            return Err(GalError::unsupported_feature(
                "3D submission requires compatible buffer, binding and texture-array limits",
            ));
        }
        let draw_size = if shadow_buffer_size > 0 {
            208
        } else if lit_fragment.is_some() {
            192
        } else {
            160
        };
        let uniform_size = if submission.instancing { 16 } else { draw_size };
        let mut created = Vec::new();
        let result = (|| {
            if gal.capabilities().shader_conventions.glsl_dialect != GlslDialect::ExplicitBindings
                || !gal.capabilities().shader_conventions.zero_to_one_clip_depth
            {
                return Err(GalError::unsupported_feature(
                    "3D shaders require explicit bindings and [0,1] clip depth",
                ));
            }
            let mut bindings = vec![
                desc(0, ResourceBindingKind::CombinedTextureSampler, 0),
                desc(1, ResourceBindingKind::StorageBuffer, 0),
                desc(2, ResourceBindingKind::UniformBuffer, 1),
            ];
            if lit_fragment.is_some() {
                bindings.push(desc(3, ResourceBindingKind::StorageBuffer, 0));
            }
            if shadow_buffer_size > 0 {
                bindings.extend([
                    desc(4, ResourceBindingKind::CombinedTextureSampler, 0),
                    desc(5, ResourceBindingKind::CombinedTextureSampler, 0),
                    desc(6, ResourceBindingKind::StorageBuffer, 0),
                ]);
            }
            if submission.instancing {
                bindings.push(desc(7, ResourceBindingKind::StorageBuffer, 0));
            }
            let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: "3d-resources".into(),
                bindings,
            })?;
            created.push(resource_layout);
            let layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: "3d-layout".into(),
                resource_layouts: vec![resource_layout],
            })?;
            created.push(layout);
            let vertex_source = if lit_fragment.is_some() {
                if shadow_buffer_size > 0 {
                    include_str!("surface.vert")
                } else {
                    include_str!("lit.vert")
                }
            } else {
                include_str!("mesh.vert")
            };
            let vertex_source = if submission.instancing {
                instanced_shader(vertex_source, true)
            } else {
                vertex_source.into()
            };
            let vertex =
                gal.create_shader_module(shader("3d-vertex", ShaderStage::Vertex, &vertex_source))?;
            created.push(vertex);
            let fragment_source = lit_fragment.unwrap_or(include_str!("mesh.frag"));
            let fragment_source = if submission.instancing {
                instanced_shader(fragment_source, false)
            } else {
                fragment_source.into()
            };
            let fragment = gal.create_shader_module(shader(
                "3d-fragment",
                ShaderStage::Fragment,
                &fragment_source,
            ))?;
            created.push(fragment);
            let sampler = gal.create_sampler(SamplerDesc {
                label: "3d-sampler".into(),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            created.push(sampler);
            let normal_sampler = if shadow_buffer_size > 0 {
                let h = gal.create_sampler(SamplerDesc {
                    label: "3d-normal-sampler".into(),
                    min_filter: SamplerFilter::Linear,
                    mag_filter: SamplerFilter::Linear,
                    mip_filter: SamplerFilter::Nearest,
                    address_u: SamplerAddressMode::Repeat,
                    address_v: SamplerAddressMode::Repeat,
                    address_w: SamplerAddressMode::Repeat,
                    comparison: None,
                })?;
                created.push(h);
                Some(h)
            } else {
                None
            };
            // An 8-byte, never-sampled depth array satisfies the fixed shader layout
            // for normal-mapped draws without requiring a shadow renderer or passes.
            let empty_shadow = if shadow_buffer_size > 0 {
                let texture = gal.create_texture(TextureDesc {
                    label: "3d-empty-shadow".into(),
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Depth32Float,
                    extent: Extent3d {
                        width: 1,
                        height: 1,
                        depth: 1,
                    },
                    mip_levels: 1,
                    array_layers: 2,
                    usages: vec![TextureUsage::Sampled],
                })?;
                created.push(texture);
                let view = gal.create_texture_view(TextureViewDesc {
                    label: "3d-empty-shadow-view".into(),
                    texture,
                    format: TextureFormat::Depth32Float,
                    base_mip: 0,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 2,
                })?;
                created.push(view);
                let sampler = gal.create_sampler(SamplerDesc {
                    label: "3d-empty-shadow-sampler".into(),
                    min_filter: SamplerFilter::Nearest,
                    mag_filter: SamplerFilter::Nearest,
                    mip_filter: SamplerFilter::Nearest,
                    address_u: SamplerAddressMode::ClampToEdge,
                    address_v: SamplerAddressMode::ClampToEdge,
                    address_w: SamplerAddressMode::ClampToEdge,
                    comparison: Some(CompareOp::LessOrEqual),
                })?;
                created.push(sampler);
                let pair = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: "3d-empty-shadow-pair".into(),
                    texture_view: view,
                    sampler,
                })?;
                created.push(pair);
                Some(EmptyShadow {
                    texture,
                    view,
                    sampler,
                    pair,
                })
            } else {
                None
            };
            Ok(Self {
                resource_layout,
                layout,
                vertex,
                fragment,
                sampler,
                normal_sampler,
                empty_shadow,
                pairs: BTreeMap::new(),
                sampler_overrides: BTreeMap::new(),
                pipelines: BTreeMap::new(),
                slots: Vec::new(),
                stride: (gal.capabilities().limits.uniform_buffer_offset_alignment as usize)
                    .max(uniform_size)
                    .next_power_of_two(),
                uniform_size,
                light_buffer_size,
                shadow_buffer_size,
                draw_size,
                submission,
                work: RenderWork::default(),
                #[cfg(feature = "diagnostics")]
                diagnostics: quasar_diagnostics::Diagnostics::disabled(),
            })
        })();
        if result.is_err() {
            for h in created.into_iter().rev() {
                let _ = gal.retire(h);
            }
        }
        result
    }
    pub fn prepare(
        &mut self,
        gal: &mut VulkanicGal,
        target: Handle,
        camera: Camera3D,
        draws: &[Draw3D<'_>],
        options: PassOptions,
    ) -> GalResult<Prepared3D> {
        self.prepare_frame(gal, target, camera, draws, options, None)
    }
    #[allow(clippy::too_many_arguments)]
    fn prepare_frame(
        &mut self,
        gal: &mut VulkanicGal,
        target: Handle,
        camera: Camera3D,
        draws: &[Draw3D<'_>],
        options: PassOptions,
        lighting: Option<&lighting::LightingFrame<'_>>,
    ) -> GalResult<Prepared3D> {
        if self.layout.is_null()
            || draws.len() > self.submission.max_instances
            || !camera.view_projection.finite()
            || !camera.light_direction.iter().all(|f| f.is_finite())
            || camera.light_direction.iter().map(|f| f * f).sum::<f32>() < 1e-12
            || options
                .clear_color
                .is_some_and(|c| c.iter().any(|f| !f.is_finite()))
            || draws.iter().any(|d| {
                d.mesh.vertices.is_null()
                    || !d.model.finite()
                    || d.material.tint.iter().any(|f| !f.is_finite())
            })
        {
            return Err(GalError::invalid_argument(
                "invalid 3D draw/camera or released renderer/mesh",
            ));
        }
        #[cfg(feature = "diagnostics")]
        let _span = self.diagnostics.span("quasar.render3d", "prepare");
        let format = gal.pass_target_color_format(target)?;
        let (depth_texture, depth_view) = if target.kind() == Some(HandleKind::FrameTarget) {
            gal.frame_target_owned_depth_attachment(target)?
        } else {
            gal.pass_target_depth_attachment(target)?
                .ok_or_else(|| GalError::invalid_argument("3D pass needs a depth attachment"))?
        };
        let depth = gal.texture_view_info(depth_view)?.format;
        gal.begin_command_recording()?;
        let result = self.record(
            gal,
            target,
            format,
            depth_texture,
            depth_view,
            depth,
            camera,
            draws,
            options,
            lighting,
        );
        let finish = gal.finish_command_recording();
        result.and_then(|p| finish.map(|()| p))
    }
    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        gal: &mut VulkanicGal,
        target: Handle,
        format: TextureFormat,
        depth_texture: Handle,
        depth_view: Handle,
        depth: TextureFormat,
        camera: Camera3D,
        draws: &[Draw3D<'_>],
        options: PassOptions,
        lighting: Option<&lighting::LightingFrame<'_>>,
    ) -> GalResult<Prepared3D> {
        let bytes = draws.len().max(1) * self.stride;
        let completed = gal.poll_completed();
        let mut selected = self.slots.iter().position(|s| {
            !s.usage.has_pending_commands() && s.usage.last_submission() <= completed
        });
        if selected.is_none() && self.slots.len() < 3 {
            let lights = if self.light_buffer_size > 0 {
                Some(buffer(
                    gal,
                    "3d-lights",
                    self.light_buffer_size,
                    BufferUsage::Storage,
                )?)
            } else {
                None
            };
            let uniform = match buffer(
                gal,
                "3d-uniforms",
                bytes.next_power_of_two(),
                BufferUsage::Uniform,
            ) {
                Ok(handle) => handle,
                Err(error) => {
                    if let Some(handle) = lights {
                        let _ = gal.retire(handle);
                    }
                    return Err(error);
                }
            };
            let shadows = if self.shadow_buffer_size > 0 {
                match buffer(
                    gal,
                    "3d-shadow-parameters",
                    self.shadow_buffer_size,
                    BufferUsage::Storage,
                ) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        let _ = gal.retire(uniform);
                        if let Some(h) = lights {
                            let _ = gal.retire(h);
                        }
                        return Err(e);
                    }
                }
            } else {
                None
            };
            let instance_bytes = draws.len().max(1) * self.draw_size;
            let instances = if self.submission.instancing {
                match buffer(
                    gal,
                    "3d-instances",
                    instance_bytes.next_power_of_two(),
                    BufferUsage::Storage,
                ) {
                    Ok(h) => Some(h),
                    Err(e) => {
                        let _ = gal.retire(uniform);
                        if let Some(h) = lights {
                            let _ = gal.retire(h);
                        }
                        if let Some(h) = shadows {
                            let _ = gal.retire(h);
                        }
                        return Err(e);
                    }
                }
            } else {
                None
            };
            self.slots.push(Slot {
                uniform,
                instances,
                instance_capacity: instance_bytes.next_power_of_two(),
                lights,
                shadows,
                capacity: bytes.next_power_of_two(),
                sets: BTreeMap::new(),
                passes: BTreeMap::new(),
                usage: SubmissionUsage::default(),
            });
            selected = Some(self.slots.len() - 1);
        }
        if selected.is_none() {
            let (index, id) = self
                .slots
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.usage.has_pending_commands())
                .map(|(i, s)| (i, s.usage.last_submission()))
                .min_by_key(|(_, id)| *id)
                .ok_or_else(|| {
                    GalError::submission(
                        StatusCode::InFlight,
                        "all 3D slots have unsubmitted commands",
                    )
                })?;
            gal.retire_through(id)?;
            selected = Some(index);
        }
        let index = selected.unwrap();
        if self.slots[index].capacity < bytes {
            let slot = &mut self.slots[index];
            for h in std::mem::take(&mut slot.sets).into_values() {
                gal.retire(h)?;
            }
            gal.retire(slot.uniform)?;
            slot.capacity = bytes.next_power_of_two();
            slot.uniform = buffer(gal, "3d-uniforms", slot.capacity, BufferUsage::Uniform)?;
        }
        let instance_bytes = draws.len().max(1) * self.draw_size;
        if self.submission.instancing && self.slots[index].instance_capacity < instance_bytes {
            let new = buffer(
                gal,
                "3d-instances",
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
        if !self.slots[index].passes.contains_key(&target) {
            let slot = &mut self.slots[index];
            if slot.passes.len() >= 8 {
                for h in std::mem::take(&mut slot.passes).into_values() {
                    gal.retire(h)?;
                }
            }
            let pass = gal.create_render_pass(RenderPassDesc {
                label: "3d-pass".into(),
                target,
                color_formats: vec![format],
                depth_format: Some(depth),
            })?;
            slot.passes.insert(target, pass);
        }
        let mut uniforms = vec![0; bytes];
        let mut work = Vec::new();
        let mut instance_data = Vec::new();
        let visibility = Visibility::new(self.submission.frustum_culling, camera.view_projection);
        let mut visible = 0usize;
        let mut batches = 0usize;
        let mut previous = None;
        for (i, draw) in draws.iter().enumerate() {
            let info = gal.texture_view_info(draw.material.texture_view)?;
            if info.dimension != TextureDimension::D2
                || info.range.layer_count != 1
                || info.format == TextureFormat::R8Uint
                || !info.usages.contains(&TextureUsage::Sampled)
            {
                return Err(GalError::invalid_argument(
                    "3D material needs a sampled 2D floating-point/normalized view",
                ));
            }
            if !visibility.intersects(draw.mesh, draw.model) {
                continue;
            }
            let pair = if let Some(h) = self.pairs.get(&(draw.material.texture_view, false)) {
                *h
            } else {
                let h = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: "3d-image".into(),
                    texture_view: draw.material.texture_view,
                    sampler: self
                        .sampler_overrides
                        .get(&(draw.material.texture_view, false))
                        .copied()
                        .unwrap_or(self.sampler),
                })?;
                self.pairs.insert((draw.material.texture_view, false), h);
                h
            };
            let surface = lighting.and_then(|frame| frame.surface.as_ref());
            let normal = surface
                .and_then(|frame| frame.materials[i].normal_map)
                .map_or(draw.material.texture_view, |map| map.texture_view);
            let normal_pair = if surface.and_then(|f| f.materials[i].normal_map).is_some() {
                let info = gal.texture_view_info(normal)?;
                if info.dimension != TextureDimension::D2
                    || info.range.layer_count != 1
                    || (surface.and_then(|f| f.materials[i].normal_map).is_some()
                        && info.format != TextureFormat::Rgba8Unorm)
                    || !info.usages.contains(&TextureUsage::Sampled)
                {
                    return Err(GalError::invalid_argument(
                        "normal map requires a sampled RGBA8 UNORM 2D view",
                    ));
                }
                if let Some(h) = self.pairs.get(&(normal, true)) {
                    *h
                } else {
                    let h = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                        label: "3d-normal-map".into(),
                        texture_view: normal,
                        sampler: self
                            .sampler_overrides
                            .get(&(normal, true))
                            .copied()
                            .unwrap_or(self.normal_sampler.unwrap()),
                    })?;
                    self.pairs.insert((normal, true), h);
                    h
                }
            } else {
                pair
            };
            let shadow_view = surface.map_or(Handle::NULL, |frame| {
                frame
                    .shadows
                    .map_or(self.empty_shadow.as_ref().unwrap().view, |s| s.view)
            });
            let slot = &mut self.slots[index];
            let key = (
                draw.mesh.vertices,
                draw.material.texture_view,
                surface
                    .and_then(|frame| frame.materials[i].normal_map)
                    .map_or(Handle::NULL, |map| map.texture_view),
                shadow_view,
            );
            if let std::collections::btree_map::Entry::Vacant(entry) = slot.sets.entry(key) {
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
                        draw.mesh.vertices,
                        ResourceBindingKind::StorageBuffer,
                        None,
                        vec![],
                    ),
                    binding(
                        2,
                        slot.uniform,
                        ResourceBindingKind::UniformBuffer,
                        Some(self.uniform_size as u64),
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
                if let Some(lights) = slot.lights {
                    bindings.push(binding(
                        3,
                        lights,
                        ResourceBindingKind::StorageBuffer,
                        Some(self.light_buffer_size as u64),
                        vec![],
                    ));
                }
                if let Some(surface) = surface {
                    bindings.push(binding(
                        4,
                        normal_pair,
                        ResourceBindingKind::CombinedTextureSampler,
                        None,
                        vec![],
                    ));
                    bindings.push(binding(
                        5,
                        surface
                            .shadows
                            .map_or(self.empty_shadow.as_ref().unwrap().pair, |s| s.pair),
                        ResourceBindingKind::CombinedTextureSampler,
                        None,
                        vec![],
                    ));
                    bindings.push(binding(
                        6,
                        slot.shadows.unwrap(),
                        ResourceBindingKind::StorageBuffer,
                        Some(self.shadow_buffer_size as u64),
                        vec![],
                    ));
                }
                let set = gal.create_resource_set(ResourceSetDesc {
                    label: "3d-set".into(),
                    layout: self.resource_layout,
                    bindings,
                })?;
                entry.insert(set);
            }
            let set = slot.sets[&key];
            let key = (
                format,
                depth,
                draw.material.transparent,
                draw.material.depth_write,
                draw.mesh.face_policy == MeshFacePolicy::SingleSided,
                draw.mesh.face_policy == MeshFacePolicy::SingleSided && mirrored(draw.model),
            );
            if let std::collections::btree_map::Entry::Vacant(entry) = self.pipelines.entry(key) {
                let h = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                    label: "3d-meshes".into(),
                    layout: self.layout,
                    vertex_shader: self.vertex,
                    fragment_shader: self.fragment,
                    topology: PrimitiveTopology::Triangles,
                    cull_mode: if key.4 {
                        CullMode::Back
                    } else {
                        CullMode::None
                    },
                    front_face: if key.5 {
                        FrontFace::Clockwise
                    } else {
                        FrontFace::CounterClockwise
                    },
                    provoking_vertex: ProvokingVertex::First,
                    raster_y_direction: RasterYDirection::Up,
                    blend: if draw.material.transparent {
                        BlendMode::Alpha
                    } else {
                        BlendMode::Disabled
                    },
                    depth_compare: Some(CompareOp::LessOrEqual),
                    depth_write: draw.material.depth_write,
                    depth_bias: None,
                    color_formats: vec![format],
                    depth_format: Some(depth),
                    stencil: None,
                })?;
                entry.insert(h);
            }
            let mut values: Vec<f32> = camera
                .view_projection
                .0
                .iter()
                .flatten()
                .chain(draw.model.0.iter().flatten())
                .copied()
                .chain(draw.material.tint)
                .chain([
                    camera.light_direction[0],
                    camera.light_direction[1],
                    camera.light_direction[2],
                    0.,
                ])
                .collect();
            if let Some(frame) = lighting {
                values.truncate(36); // viewProjection, model, base tint
                values.extend(frame.materials[i].parameters());
                let material_flags = if draw.mesh.face_policy == MeshFacePolicy::DoubleSided {
                    2.
                } else {
                    0.
                } + if draw.mesh.opaque_alpha { 4. } else { 0. };
                values[46] = material_flags;
                if let Some(surface) = &frame.surface {
                    values[46] = f32::from(surface.materials[i].receives_shadows) + material_flags;
                    values[47] = f32::from(surface.materials[i].normal_map.is_some());
                    values.extend(surface.materials[i].normal_map.map_or(
                        [1., 1., 0., 1.],
                        |map| {
                            [
                                map.uv_scale[0],
                                map.uv_scale[1],
                                map.strength,
                                if map.flip_y { -1. } else { 1. },
                            ]
                        },
                    ));
                }
            }
            let data: Vec<u8> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
            let pipeline = self.pipelines[&key];
            let batch_key = (set, pipeline, draw.mesh.indices);
            if self.submission.instancing {
                instance_data.extend_from_slice(&data);
                if previous == Some(batch_key) && !draw.material.transparent {
                    if let Some(CommandOp::DrawIndexed { instances, .. }) = work.last_mut() {
                        *instances += 1;
                    }
                    visible += 1;
                    continue;
                }
                uniforms[batches * self.stride..batches * self.stride + 4]
                    .copy_from_slice(&(visible as u32).to_ne_bytes());
            } else {
                uniforms[batches * self.stride..batches * self.stride + self.uniform_size]
                    .copy_from_slice(&data);
            }
            work.extend([
                CommandOp::TrackSubmission(draw.mesh.usage.clone()),
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::BindResourceSet {
                    pipeline_layout: self.layout,
                    set_index: 0,
                    set,
                    dynamic_offsets: [(batches * self.stride) as u64].into(),
                },
                CommandOp::SetIndexBuffer {
                    buffer: draw.mesh.indices,
                    offset: 0,
                    index_type: IndexType::U32,
                },
                CommandOp::DrawIndexed {
                    indices: draw.mesh.count,
                    instances: 1,
                },
            ]);
            previous = if draw.material.transparent {
                None
            } else {
                Some(batch_key)
            };
            batches += 1;
            visible += 1;
        }
        let bytes = batches.max(1) * self.stride;
        uniforms.truncate(bytes);
        self.work = RenderWork {
            candidates: draws.len(),
            visible,
            culled: draws.len() - visible,
            draws: batches,
            instance_upload_bytes: instance_data.len(),
        };

        #[cfg(feature = "diagnostics")]
        self.work.report(&self.diagnostics);
        let slot = &self.slots[index];
        let attachment = gal.pass_target_color_attachment(target)?;
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
                data: uniforms,
            },
            barrier(
                slot.uniform,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ),
            barrier(
                depth_texture,
                lighting.map_or_else(
                    || {
                        if options.clear_depth {
                            TextureUsageState::Undefined
                        } else {
                            TextureUsageState::DepthStencilAttachment
                        }
                    },
                    |frame| frame.depth_before,
                ),
                TextureUsageState::DepthStencilAttachment,
            ),
            CommandOp::BeginPass {
                pass: slot.passes[&target],
                target,
                colors: vec![PassAttachment {
                    view: attachment,
                    load_op: if options.clear_color.is_some() {
                        AttachmentLoadOp::Clear
                    } else {
                        AttachmentLoadOp::Load
                    },
                    store_op: AttachmentStoreOp::Store,
                    clear_color: options.clear_color.map(|c| ClearColor {
                        r: c[0],
                        g: c[1],
                        b: c[2],
                        a: c[3],
                    }),
                }],
                depth_stencil: Some(PassAttachment {
                    view: depth_view,
                    load_op: if options.clear_depth {
                        AttachmentLoadOp::Clear
                    } else {
                        AttachmentLoadOp::Load
                    },
                    store_op: AttachmentStoreOp::Store,
                    clear_color: None,
                }),
            },
        ];
        if let Some(instances) = slot.instances
            && !instance_data.is_empty()
        {
            let upload = [
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
            ];
            let before_pass = operations.len() - 1;
            operations.splice(before_pass..before_pass, upload);
        }

        if let Some(frame) = lighting {
            let lights = slot.lights.expect("lit renderer owns light storage");
            let upload = [
                barrier(
                    lights,
                    TextureUsageState::ShaderRead,
                    TextureUsageState::TransferDst,
                ),
                CommandOp::HostWriteBuffer {
                    buffer: lights,
                    offset: 0,
                    data: frame.bytes.clone(),
                },
                barrier(
                    lights,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ),
            ];
            let before_pass = operations.len() - 1;
            operations.splice(before_pass..before_pass, upload);
        }
        if let Some(surface) = lighting.and_then(|frame| frame.surface.as_ref()) {
            let shadow_buffer = slot.shadows.unwrap();
            let mut bytes = Vec::new();
            let records = surface.shadows.map_or(&[][..], |s| s.records.as_slice());
            for v in [records.len() as f32, 0., 0., 0.] {
                bytes.extend(v.to_ne_bytes());
            }
            for record in records {
                for v in record {
                    bytes.extend(v.to_ne_bytes());
                }
            }
            let dependency = surface.shadows.map_or_else(
                || {
                    barrier(
                        self.empty_shadow.as_ref().unwrap().texture,
                        TextureUsageState::Undefined,
                        TextureUsageState::ShaderRead,
                    )
                },
                |s| CommandOp::TrackSubmission(s.usage.clone()),
            );
            let upload = [
                dependency,
                barrier(
                    shadow_buffer,
                    TextureUsageState::ShaderRead,
                    TextureUsageState::TransferDst,
                ),
                CommandOp::HostWriteBuffer {
                    buffer: shadow_buffer,
                    offset: 0,
                    data: bytes,
                },
                barrier(
                    shadow_buffer,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ),
            ];
            let before_pass = operations.len() - 1;
            operations.splice(before_pass..before_pass, upload);
        }
        operations.extend(work);
        operations.push(CommandOp::EndPass);
        Ok(Prepared3D {
            commands: gal.create_command_list(CommandListDesc {
                label: "3d-prepared".into(),
                operations,
            })?,
            stats: RenderStats {
                draws: batches,
                uniform_upload_bytes: bytes,
            },
        })
    }
    /// Releases material/mesh descriptor caches after an asset generation changes.
    /// Prepared commands must first be submitted or abandoned; accepted GPU uses
    /// retire normally. Mesh owners can then release their buffers/views.
    pub fn clear_bindings(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.require_no_pending()?;
        for slot in &mut self.slots {
            for h in std::mem::take(&mut slot.sets).into_values() {
                gal.retire(h)?;
            }
        }
        for h in std::mem::take(&mut self.pairs).into_values() {
            gal.retire(h)?;
        }
        self.sampler_overrides.clear();
        Ok(())
    }
    pub fn release_targets(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.require_no_pending()?;
        for s in &mut self.slots {
            for h in std::mem::take(&mut s.passes).into_values() {
                gal.retire(h)?;
            }
        }
        Ok(())
    }
    fn require_no_pending(&self) -> GalResult<()> {
        if self.slots.iter().any(|s| s.usage.has_pending_commands()) {
            return Err(GalError::submission(
                StatusCode::InFlight,
                "3D commands still retain resources",
            ));
        }
        Ok(())
    }
    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self.layout.is_null() {
            return Ok(());
        }
        self.require_no_pending()?;
        self.release_targets(gal)?;
        for s in self.slots.drain(..) {
            for h in s.sets.into_values() {
                gal.retire(h)?;
            }
            gal.retire(s.uniform)?;
            if let Some(instances) = s.instances {
                gal.retire(instances)?;
            }
            if let Some(lights) = s.lights {
                gal.retire(lights)?;
            }
            if let Some(shadows) = s.shadows {
                gal.retire(shadows)?;
            }
        }
        for h in std::mem::take(&mut self.pairs).into_values() {
            gal.retire(h)?;
        }
        for h in std::mem::take(&mut self.pipelines).into_values() {
            gal.retire(h)?;
        }
        for h in [
            self.sampler,
            self.layout,
            self.vertex,
            self.fragment,
            self.resource_layout,
        ] {
            gal.retire(h)?;
        }
        if let Some(normal_sampler) = self.normal_sampler.take() {
            gal.retire(normal_sampler)?;
        }
        if let Some(empty) = self.empty_shadow.take() {
            for h in [empty.pair, empty.sampler, empty.view, empty.texture] {
                gal.retire(h)?;
            }
        }
        self.layout = Handle::NULL;
        Ok(())
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
        buffer_range,
        dynamic_offsets,
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

fn mirrored(m: Mat4) -> bool {
    let a = m.0;
    a[0][0] * (a[1][1] * a[2][2] - a[2][1] * a[1][2])
        - a[1][0] * (a[0][1] * a[2][2] - a[2][1] * a[0][2])
        + a[2][0] * (a[0][1] * a[1][2] - a[1][1] * a[0][2])
        < 0.
}
