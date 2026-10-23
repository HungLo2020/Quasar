//! Optional mesh rendering through VulkanicGAL's public API. The game owns GAL,
//! assets, draw ordering, frame targets, submission and presentation.
mod math;
pub use math::Mat4;
use std::collections::BTreeMap;
use vulkanic_gal::{commands::SubmissionUsage, resources::*, *};

#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}
/// Immutable uploaded geometry on one GAL. Explicitly release when no prepared
/// command copies remain. Submitted uses are retired by GAL, without idle waits.
pub struct Mesh {
    vertices: Handle,
    indices: Handle,
    count: u32,
    usage: SubmissionUsage,
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
            usage: SubmissionUsage::default(),
        })
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
    capacity: usize,
    sets: BTreeMap<(Handle, Handle), Handle>,
    passes: BTreeMap<Handle, Handle>,
    usage: SubmissionUsage,
}
/// Rendering state on a caller-owned device. Always use its construction GAL.
/// Keeps bounded transient slots; release explicitly before dropping.
pub struct RenderState3D {
    resource_layout: Handle,
    layout: Handle,
    vertex: Handle,
    fragment: Handle,
    sampler: Handle,
    pairs: BTreeMap<Handle, Handle>,
    pipelines: BTreeMap<(TextureFormat, TextureFormat, bool, bool), Handle>,
    slots: Vec<Slot>,
    stride: usize,
}
impl RenderState3D {
    pub fn new(gal: &mut VulkanicGal) -> GalResult<Self> {
        let mut created = Vec::new();
        let result = (|| {
            if gal.capabilities().shader_conventions.glsl_dialect != GlslDialect::ExplicitBindings
                || !gal.capabilities().shader_conventions.zero_to_one_clip_depth
            {
                return Err(GalError::unsupported_feature(
                    "3D shaders require explicit bindings and [0,1] clip depth",
                ));
            }
            let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: "3d-resources".into(),
                bindings: vec![
                    desc(0, ResourceBindingKind::CombinedTextureSampler, 0),
                    desc(1, ResourceBindingKind::StorageBuffer, 0),
                    desc(2, ResourceBindingKind::UniformBuffer, 1),
                ],
            })?;
            created.push(resource_layout);
            let layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: "3d-layout".into(),
                resource_layouts: vec![resource_layout],
            })?;
            created.push(layout);
            let vertex = gal.create_shader_module(shader(
                "3d-vertex",
                ShaderStage::Vertex,
                include_str!("mesh.vert"),
            ))?;
            created.push(vertex);
            let fragment = gal.create_shader_module(shader(
                "3d-fragment",
                ShaderStage::Fragment,
                include_str!("mesh.frag"),
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
            Ok(Self {
                resource_layout,
                layout,
                vertex,
                fragment,
                sampler,
                pairs: BTreeMap::new(),
                pipelines: BTreeMap::new(),
                slots: Vec::new(),
                stride: (gal.capabilities().limits.uniform_buffer_offset_alignment as usize)
                    .max(160)
                    .next_power_of_two(),
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
        if self.layout.is_null()
            || draws.len() > 4096
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
    ) -> GalResult<Prepared3D> {
        let bytes = draws.len().max(1) * self.stride;
        let completed = gal.poll_completed();
        let mut selected = self.slots.iter().position(|s| {
            !s.usage.has_pending_commands() && s.usage.last_submission() <= completed
        });
        if selected.is_none() && self.slots.len() < 3 {
            self.slots.push(Slot {
                uniform: buffer(
                    gal,
                    "3d-uniforms",
                    bytes.next_power_of_two(),
                    BufferUsage::Uniform,
                )?,
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
            let pair = if let Some(h) = self.pairs.get(&draw.material.texture_view) {
                *h
            } else {
                let h = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: "3d-image".into(),
                    texture_view: draw.material.texture_view,
                    sampler: self.sampler,
                })?;
                self.pairs.insert(draw.material.texture_view, h);
                h
            };
            let slot = &mut self.slots[index];
            let key = (draw.mesh.vertices, draw.material.texture_view);
            if let std::collections::btree_map::Entry::Vacant(entry) = slot.sets.entry(key) {
                let set = gal.create_resource_set(ResourceSetDesc {
                    label: "3d-set".into(),
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
                            draw.mesh.vertices,
                            ResourceBindingKind::StorageBuffer,
                            None,
                            vec![],
                        ),
                        binding(
                            2,
                            slot.uniform,
                            ResourceBindingKind::UniformBuffer,
                            Some(160),
                            vec![0],
                        ),
                    ],
                })?;
                entry.insert(set);
            }
            let set = slot.sets[&key];
            let key = (
                format,
                depth,
                draw.material.transparent,
                draw.material.depth_write,
            );
            if let std::collections::btree_map::Entry::Vacant(entry) = self.pipelines.entry(key) {
                let h = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                    label: "3d-meshes".into(),
                    layout: self.layout,
                    vertex_shader: self.vertex,
                    fragment_shader: self.fragment,
                    topology: PrimitiveTopology::Triangles,
                    cull_mode: CullMode::None,
                    front_face: FrontFace::CounterClockwise,
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
            let values: Vec<f32> = camera
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
            let data: Vec<u8> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
            uniforms[i * self.stride..i * self.stride + 160].copy_from_slice(&data);
            work.extend([
                CommandOp::TrackSubmission(draw.mesh.usage.clone()),
                CommandOp::BindGraphicsPipeline(self.pipelines[&key]),
                CommandOp::BindResourceSet {
                    pipeline_layout: self.layout,
                    set_index: 0,
                    set,
                    dynamic_offsets: vec![(i * self.stride) as u64],
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
        }
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
                if options.clear_depth {
                    TextureUsageState::Undefined
                } else {
                    TextureUsageState::DepthStencilAttachment
                },
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
        operations.extend(work);
        operations.push(CommandOp::EndPass);
        Ok(Prepared3D {
            commands: gal.create_command_list(CommandListDesc {
                label: "3d-prepared".into(),
                operations,
            })?,
            stats: RenderStats {
                draws: draws.len(),
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
