//! GPU linear-blend skinning with a joint-palette storage buffer.
//!
//! [`SkinnedMesh`] keeps a bind-pose copy of its vertices, per-vertex joint
//! influences and a joint palette (`world(joint) * inverse_bind`, as produced
//! by `quasar_assets::model::ModelSkin::joint_matrices` or
//! `quasar_animation::model::SkinBinding::joint_matrices`). Each frame,
//! [`SkinningPass`] runs one compute dispatch per mesh that writes the posed
//! vertices into an ordinary [`Mesh`] vertex buffer.
//!
//! Because the output is a normal `Mesh`, every existing path draws it
//! unchanged: legacy, lit, surface/normal-mapped and transparent passes, and
//! [`ShadowCaster`](crate::ShadowCaster)s in the shadow pass. Skinning runs
//! once per frame no matter how many passes (camera, shadow cascades,
//! reflections) draw the character. Submit the skinning command buffer
//! before the passes that read the mesh; wgpu orders storage writes and
//! vertex reads within and across submissions on the queue.
//!
//! Draw skinned meshes with the character's own model matrix: the palette
//! already contains the joints' model-space placement.
use super::*;

/// Upper bound on joints per mesh (palette entries).
pub const MAX_JOINTS: usize = 1024;

/// Up to four joint influences for one vertex. Weights are normalised on
/// upload; joints index the palette.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkinInfluence {
    pub joints: [u32; 4],
    pub weights: [f32; 4],
}

impl SkinInfluence {
    /// Fully bound to one joint.
    pub const fn single(joint: u32) -> Self {
        Self {
            joints: [joint, 0, 0, 0],
            weights: [1., 0., 0., 0.],
        }
    }
}

fn normalized(
    influences: &[SkinInfluence],
    joint_count: usize,
) -> GraphicsResult<Vec<SkinInfluence>> {
    influences
        .iter()
        .map(|influence| {
            if influence.weights.iter().any(|w| !w.is_finite() || *w < 0.) {
                return Err(error("skin weights must be finite and non-negative"));
            }
            let sum: f32 = influence.weights.iter().sum();
            if sum <= f32::EPSILON {
                return Err(error("every skinned vertex needs a positive weight"));
            }
            if influence
                .joints
                .iter()
                .zip(influence.weights)
                .any(|(joint, weight)| weight > 0. && *joint as usize >= joint_count)
            {
                return Err(error("skin joint index exceeds the joint count"));
            }
            Ok(SkinInfluence {
                joints: influence.joints.map(|j| j.min(joint_count as u32 - 1)),
                weights: influence.weights.map(|w| w / sum),
            })
        })
        .collect()
}

fn palette_valid(palette: &[Mat4], joint_count: usize) -> bool {
    palette.len() == joint_count && palette.iter().all(|m| m.finite())
}

/// CPU reference skinning with the same maths as the GPU pass, for picking,
/// physics proxies, tests and headless tools.
pub fn skin_vertices(
    vertices: &[Vertex],
    influences: &[SkinInfluence],
    palette: &[Mat4],
) -> GraphicsResult<Vec<Vertex>> {
    if vertices.len() != influences.len() || palette.is_empty() {
        return Err(error(
            "skinning needs one influence per vertex and a palette",
        ));
    }
    let influences = normalized(influences, palette.len())?;
    if !palette_valid(palette, palette.len()) {
        return Err(error("skinning palette must be finite"));
    }
    Ok(vertices
        .iter()
        .zip(&influences)
        .map(|(vertex, influence)| {
            let mut skin = [[0f32; 4]; 4];
            for (joint, weight) in influence.joints.iter().zip(influence.weights) {
                let matrix = palette[*joint as usize].0;
                for c in 0..4 {
                    for r in 0..4 {
                        skin[c][r] += matrix[c][r] * weight;
                    }
                }
            }
            let skin = Mat4(skin);
            let p = skin.transform([
                vertex.position[0],
                vertex.position[1],
                vertex.position[2],
                1.,
            ]);
            let n = skin.transform([vertex.normal[0], vertex.normal[1], vertex.normal[2], 0.]);
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            Vertex {
                position: [p[0] / p[3], p[1] / p[3], p[2] / p[3]],
                normal: if length > 1e-6 {
                    [n[0] / length, n[1] / length, n[2] / length]
                } else {
                    vertex.normal
                },
                uv: vertex.uv,
            }
        })
        .collect())
}

/// A mesh posed on the GPU from a joint palette. See the module docs.
pub struct SkinnedMesh {
    mesh: Mesh,
    bind_pose: Buffer,
    influences: Buffer,
    palette: Buffer,
    params: Buffer,
    vertex_count: u32,
    joint_count: u32,
    group: Option<BindGroup>,
}

impl SkinnedMesh {
    /// Uploads bind-pose vertices, influences and indices. The posed output
    /// starts in the bind pose so it is drawable before the first skinning
    /// pass.
    pub fn upload(
        gpu: &Gpu,
        vertices: &[Vertex],
        influences: &[SkinInfluence],
        indices: &[u32],
        joint_count: usize,
    ) -> GraphicsResult<Self> {
        Self::upload_with_colors(gpu, vertices, None, influences, indices, joint_count)
    }

    /// [`Self::upload`] with per-vertex linear RGBA colours (see
    /// [`Mesh::upload_colored`]); skinning carries them through.
    pub fn upload_with_colors(
        gpu: &Gpu,
        vertices: &[Vertex],
        colors: Option<&[[f32; 4]]>,
        influences: &[SkinInfluence],
        indices: &[u32],
        joint_count: usize,
    ) -> GraphicsResult<Self> {
        if !(1..=MAX_JOINTS).contains(&joint_count) || influences.len() != vertices.len() {
            return Err(error(
                "skinned meshes need 1..=1024 joints and one influence per vertex",
            ));
        }
        if colors.is_some_and(|c| c.len() != vertices.len()) {
            return Err(error("vertex colours need one value per vertex"));
        }
        let influences = normalized(influences, joint_count)?;
        let mut mesh =
            Mesh::upload_encoded(gpu, vertices, colors, indices, BufferUsages::COPY_SRC)?;
        // Bind-pose bounds are a poor fit once joints move; grow them so
        // frustum culling stays conservative. Callers with known motion
        // ranges can tighten them with `set_local_bounds`.
        let (min, max) = mesh.bounds;
        let center: [f32; 3] = std::array::from_fn(|a| (min[a] + max[a]) * 0.5);
        let radius = (0..3)
            .map(|a| max[a] - min[a])
            .fold(0f32, f32::max)
            .max(1e-3);
        mesh.bounds = (
            std::array::from_fn(|a| center[a] - radius),
            std::array::from_fn(|a| center[a] + radius),
        );
        let bind_pose = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("skinning bind pose"),
            size: vertices.len() as u64 * 48,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.encoder("skinning setup");
        encoder.copy_buffer_to_buffer(
            mesh.vertices.as_ref().expect("fresh mesh"),
            0,
            &bind_pose,
            0,
            vertices.len() as u64 * 48,
        );
        gpu.queue.submit([encoder.finish()]);
        let influence_bytes: Vec<u8> = influences
            .iter()
            .flat_map(|i| {
                i.joints
                    .iter()
                    .flat_map(|j| j.to_ne_bytes())
                    .chain(i.weights.iter().flat_map(|w| w.to_ne_bytes()))
                    .collect::<Vec<_>>()
            })
            .collect();
        let influences = buffer(
            gpu,
            "skinning influences",
            &influence_bytes,
            BufferUsages::STORAGE,
        );
        let identity: Vec<f32> = (0..joint_count)
            .flat_map(|_| Mat4::IDENTITY.0.into_iter().flatten())
            .collect();
        let palette = buffer(
            gpu,
            "skinning joint palette",
            &bytes(identity),
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
        );
        let mut param_bytes = Vec::with_capacity(16);
        for value in [vertices.len() as u32, joint_count as u32, 1, 0] {
            param_bytes.extend(value.to_ne_bytes());
        }
        let params = buffer(
            gpu,
            "skinning parameters",
            &param_bytes,
            BufferUsages::UNIFORM,
        );
        Ok(Self {
            mesh,
            bind_pose,
            influences,
            palette,
            params,
            vertex_count: vertices.len() as u32,
            joint_count: joint_count as u32,
            group: None,
        })
    }

    /// Uploads a skinned `quasar-assets` primitive. `joint_count` is the
    /// owning skin's joint count.
    #[cfg(feature = "model-assets")]
    pub fn from_model_primitive(
        gpu: &Gpu,
        primitive: &quasar_assets::model::ModelPrimitive,
        joint_count: usize,
    ) -> GraphicsResult<Self> {
        let skin = primitive
            .skin
            .as_ref()
            .ok_or_else(|| error("model primitive has no skin weights"))?;
        let vertices: Vec<Vertex> = primitive
            .vertices
            .iter()
            .map(|v| Vertex {
                position: v.position,
                normal: v.normal,
                uv: v.uv,
            })
            .collect();
        let influences: Vec<SkinInfluence> = skin
            .joints
            .iter()
            .zip(&skin.weights)
            .map(|(joints, weights)| SkinInfluence {
                joints: joints.map(u32::from),
                weights: *weights,
            })
            .collect();
        Self::upload_with_colors(
            gpu,
            &vertices,
            primitive.colors.as_deref(),
            &influences,
            &primitive.indices,
            joint_count,
        )
    }

    /// The posed mesh, drawable by every 3D pass and shadow caster.
    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }
    /// Mutable access for face policy or opaque-alpha settings.
    pub fn mesh_mut(&mut self) -> &mut Mesh {
        &mut self.mesh
    }
    pub fn joint_count(&self) -> usize {
        self.joint_count as usize
    }
    pub fn vertex_count(&self) -> usize {
        self.vertex_count as usize
    }
    /// Overrides the culling bounds used for the posed mesh.
    pub fn set_local_bounds(&mut self, min: [f32; 3], max: [f32; 3]) -> GraphicsResult<()> {
        if !min.iter().chain(&max).all(|v| v.is_finite()) || (0..3).any(|a| min[a] > max[a]) {
            return Err(error("invalid skinned mesh bounds"));
        }
        self.mesh.bounds = (min, max);
        Ok(())
    }
    /// Reads the posed vertices back (for tests, tools and headless checks;
    /// never needed for rendering).
    pub fn read_vertices(&self, gpu: &Gpu) -> GraphicsResult<Vec<Vertex>> {
        let output = self
            .mesh
            .vertices
            .as_ref()
            .ok_or_else(|| error("released skinned mesh"))?;
        read_buffer_vertices(gpu, output, 0, u64::from(self.vertex_count) * 48)
    }
    /// Drops this owner's GPU references.
    pub fn release(&mut self, gpu: &Gpu) -> GraphicsResult<()> {
        self.group = None;
        self.mesh.release(gpu)
    }
}

/// Workgroup counts covering `threads` invocations of the 64-wide skinning
/// kernel, split over two dimensions so large crowds stay under the
/// per-dimension dispatch limit.
fn dispatch_size(threads: u32) -> (u32, u32) {
    let groups = threads.div_ceil(64).max(1);
    let x = groups.min(65_535);
    (x, groups.div_ceil(x))
}

/// Many posed copies of one skinned mesh, skinned by a single dispatch with
/// a single palette upload and drawn as one instanced draw.
///
/// Every slot ([`Self::instance`]) is an ordinary [`Mesh`] sharing the
/// crowd's vertex and index buffers at its own vertex offset, so lit,
/// surface, transparent and shadow passes draw it unchanged. With
/// [`SubmissionConfig::instancing`] enabled, consecutive draws of one
/// crowd's slots with the same material merge into one instanced draw call. Use this for hordes: a [`SkinnedMesh`] per enemy
/// costs a dispatch, an upload and a draw each.
pub struct SkinnedCrowd {
    instances: Vec<Mesh>,
    output: Buffer,
    bind_pose: Buffer,
    influences: Buffer,
    palette: Buffer,
    params: Buffer,
    vertex_count: u32,
    joint_count: u32,
    group: Option<BindGroup>,
}

impl SkinnedCrowd {
    /// Uploads one bind pose for `capacity` instances. Every instance starts
    /// in the bind pose.
    pub fn upload(
        gpu: &Gpu,
        vertices: &[Vertex],
        colors: Option<&[[f32; 4]]>,
        influences: &[SkinInfluence],
        indices: &[u32],
        joint_count: usize,
        capacity: usize,
    ) -> GraphicsResult<Self> {
        let template = SkinnedMesh::upload_with_colors(
            gpu,
            vertices,
            colors,
            influences,
            indices,
            joint_count,
        )?;
        let limits = gpu.device.limits();
        let vertex_bytes = vertices.len() as u64 * 48;
        let output_size = vertex_bytes.saturating_mul(capacity as u64);
        if capacity == 0
            || output_size > limits.max_storage_buffer_binding_size
            || output_size > limits.max_buffer_size
            || (joint_count * capacity) as u64 * 64 > limits.max_storage_buffer_binding_size
            || vertices.len() as u64 * capacity as u64 > u64::from(u32::MAX)
        {
            return Err(error("crowd capacity exceeds the storage buffer limits"));
        }
        let output = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("skinned crowd vertices"),
            size: output_size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.encoder("skinned crowd setup");
        for slot in 0..capacity as u64 {
            encoder.copy_buffer_to_buffer(
                template.mesh.vertices.as_ref().expect("fresh mesh"),
                0,
                &output,
                slot * vertex_bytes,
                vertex_bytes,
            );
        }
        gpu.queue.submit([encoder.finish()]);
        let palette = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("skinned crowd palettes"),
            size: (joint_count * capacity) as u64 * 64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("skinned crowd parameters"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let base = &template.mesh;
        let instances = (0..capacity)
            .map(|slot| Mesh {
                vertices: Some(output.clone()),
                indices: base.indices.clone(),
                count: base.count,
                face_policy: base.face_policy,
                opaque_alpha: base.opaque_alpha,
                bounds: base.bounds,
                vertex_base: (slot * vertices.len()) as u32,
            })
            .collect();
        Ok(Self {
            instances,
            output,
            bind_pose: template.bind_pose.clone(),
            influences: template.influences.clone(),
            palette,
            params,
            vertex_count: template.vertex_count,
            joint_count: template.joint_count,
            group: None,
        })
    }

    /// Uploads a skinned `quasar-assets` primitive for `capacity` instances.
    #[cfg(feature = "model-assets")]
    pub fn from_model_primitive(
        gpu: &Gpu,
        primitive: &quasar_assets::model::ModelPrimitive,
        joint_count: usize,
        capacity: usize,
    ) -> GraphicsResult<Self> {
        let skin = primitive
            .skin
            .as_ref()
            .ok_or_else(|| error("model primitive has no skin weights"))?;
        let vertices: Vec<Vertex> = primitive
            .vertices
            .iter()
            .map(|v| Vertex {
                position: v.position,
                normal: v.normal,
                uv: v.uv,
            })
            .collect();
        let influences: Vec<SkinInfluence> = skin
            .joints
            .iter()
            .zip(&skin.weights)
            .map(|(joints, weights)| SkinInfluence {
                joints: joints.map(u32::from),
                weights: *weights,
            })
            .collect();
        Self::upload(
            gpu,
            &vertices,
            primitive.colors.as_deref(),
            &influences,
            &primitive.indices,
            joint_count,
            capacity,
        )
    }

    /// The posed mesh of slot `index`.
    pub fn instance(&self, index: usize) -> &Mesh {
        &self.instances[index]
    }
    /// Every slot, in order.
    pub fn instances(&self) -> &[Mesh] {
        &self.instances
    }
    pub fn capacity(&self) -> usize {
        self.instances.len()
    }
    pub fn joint_count(&self) -> usize {
        self.joint_count as usize
    }
    pub fn vertex_count(&self) -> usize {
        self.vertex_count as usize
    }
    /// Face policy for every slot.
    pub fn set_face_policy(&mut self, policy: MeshFacePolicy) {
        for mesh in &mut self.instances {
            mesh.face_policy = policy;
        }
    }
    /// Culling bounds for every slot (see [`SkinnedMesh::set_local_bounds`]).
    pub fn set_local_bounds(&mut self, min: [f32; 3], max: [f32; 3]) -> GraphicsResult<()> {
        if !min.iter().chain(&max).all(|v| v.is_finite()) || (0..3).any(|a| min[a] > max[a]) {
            return Err(error("invalid skinned crowd bounds"));
        }
        for mesh in &mut self.instances {
            mesh.bounds = (min, max);
        }
        Ok(())
    }
    /// Reads one slot's posed vertices back (tests and tools).
    pub fn read_vertices(&self, gpu: &Gpu, index: usize) -> GraphicsResult<Vec<Vertex>> {
        if index >= self.instances.len() {
            return Err(error("crowd slot out of range"));
        }
        read_buffer_vertices(
            gpu,
            &self.output,
            u64::from(self.vertex_count) * 48 * index as u64,
            u64::from(self.vertex_count) * 48,
        )
    }
    /// Drops this owner's GPU references.
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.group = None;
        self.instances.clear();
        Ok(())
    }
}

fn read_buffer_vertices(
    gpu: &Gpu,
    source: &Buffer,
    offset: u64,
    size: u64,
) -> GraphicsResult<Vec<Vertex>> {
    let staging = gpu.device.create_buffer(&BufferDescriptor {
        label: Some("skinned vertex readback"),
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.encoder("skinned vertex readback");
    encoder.copy_buffer_to_buffer(source, offset, &staging, 0, size);
    let index = gpu.queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::sync_channel(1);
    staging.map_async(MapMode::Read, .., move |result| {
        let _ = send.send(result);
    });
    gpu.device
        .poll(PollType::Wait {
            submission_index: Some(index),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .map_err(error)?;
    recv.recv_timeout(std::time::Duration::from_secs(30))
        .map_err(error)?
        .map_err(error)?;
    let mapped = staging.get_mapped_range(..).map_err(error)?;
    let floats: Vec<f32> = mapped
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    drop(mapped);
    staging.unmap();
    Ok(floats
        .chunks_exact(12)
        .map(|v| Vertex {
            position: [v[0], v[1], v[2]],
            normal: [v[4], v[5], v[6]],
            uv: [v[8], v[9]],
        })
        .collect())
}

/// Compute pipeline that poses [`SkinnedMesh`]es. One per device is enough.
pub struct SkinningPass {
    pipeline: ComputePipeline,
    layout: BindGroupLayout,
    /// Reused, persistently mapped upload chunks for palettes. Chunks return
    /// to the belt once the command buffer that copies from them is done.
    belt: vulkanic_gal::util::StagingBelt,
    /// Reused CPU staging for the bytes copied into the belt.
    scratch: Vec<u8>,
}

/// Default staging chunk: a 1,000-instance crowd of 32 joints fits twice.
const PALETTE_CHUNK: u64 = 4 << 20;

/// Writes `matrices` as native-endian f32 columns into `out`
/// (64 bytes each) without per-float iterator overhead.
fn write_matrices(out: &mut [u8], matrices: &[Mat4]) {
    for (dst, matrix) in out.chunks_exact_mut(64).zip(matrices) {
        for (column, values) in dst.chunks_exact_mut(16).zip(&matrix.0) {
            for (bytes, value) in column.chunks_exact_mut(4).zip(values) {
                bytes.copy_from_slice(&value.to_ne_bytes());
            }
        }
    }
}

impl SkinningPass {
    pub fn new(gpu: &Gpu) -> GraphicsResult<Self> {
        let storage = |binding, read_only| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("skinning"),
                entries: &[
                    storage(0, true),
                    storage(1, true),
                    storage(2, true),
                    storage(3, false),
                    BindGroupLayoutEntry {
                        binding: 4,
                        visibility: ShaderStages::COMPUTE,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("skinning"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = checked(gpu, || {
            let module = gpu.device.create_shader_module(ShaderModuleDescriptor {
                label: Some("skinning"),
                source: ShaderSource::Wgsl(include_str!("skinning.wgsl").into()),
            });
            gpu.device
                .create_compute_pipeline(&ComputePipelineDescriptor {
                    label: Some("skinning"),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                })
        })?;
        Ok(Self {
            pipeline,
            layout,
            belt: vulkanic_gal::util::StagingBelt::new(gpu.device.clone(), PALETTE_CHUNK),
            scratch: Vec::new(),
        })
    }

    /// Uploads `palette` (one matrix per joint) and records the dispatch
    /// that poses `mesh` into `encoder`.
    pub fn record(
        &mut self,
        gpu: &Gpu,
        encoder: &mut CommandEncoder,
        mesh: &mut SkinnedMesh,
        palette: &[Mat4],
    ) -> GraphicsResult<()> {
        if !palette_valid(palette, mesh.joint_count as usize) {
            return Err(error("palette must hold one finite matrix per joint"));
        }
        let output = mesh
            .mesh
            .vertices
            .clone()
            .ok_or_else(|| error("released skinned mesh"))?;
        let size = palette.len() as u64 * 64;
        if let Some(size) = BufferSize::new(size) {
            self.scratch.clear();
            self.scratch.resize(size.get() as usize, 0);
            write_matrices(&mut self.scratch, palette);
            self.belt
                .write_buffer(encoder, &mesh.palette, 0, size)
                .copy_from_slice(&self.scratch);
        }
        self.belt.finish_and_recall_on_submit(encoder);
        if mesh.group.is_none() {
            mesh.group = Some(gpu.device.create_bind_group(&BindGroupDescriptor {
                label: Some("skinning"),
                layout: &self.layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: mesh.bind_pose.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: mesh.influences.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: mesh.palette.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: output.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: mesh.params.as_entire_binding(),
                    },
                ],
            }));
        }
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("skinning"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, mesh.group.as_ref(), &[]);
        let (x, y) = dispatch_size(mesh.vertex_count);
        pass.dispatch_workgroups(x, y, 1);
        Ok(())
    }

    /// Poses the first `palettes.len()` slots of `crowd` (one palette of
    /// `joint_count` matrices each) with one upload and one dispatch.
    pub fn record_crowd(
        &mut self,
        gpu: &Gpu,
        encoder: &mut CommandEncoder,
        crowd: &mut SkinnedCrowd,
        palettes: &[&[Mat4]],
    ) -> GraphicsResult<()> {
        if palettes.len() > crowd.capacity() {
            return Err(error("more crowd palettes than slots"));
        }
        if palettes
            .iter()
            .any(|p| !palette_valid(p, crowd.joint_count as usize))
        {
            return Err(error("palette must hold one finite matrix per joint"));
        }
        if palettes.is_empty() {
            return Ok(());
        }
        let stride = crowd.joint_count as usize * 64;
        let total = 16 + palettes.len() * stride;
        self.scratch.clear();
        self.scratch.resize(total, 0);
        for (bytes, value) in self.scratch[..16].chunks_exact_mut(4).zip([
            crowd.vertex_count,
            crowd.joint_count,
            palettes.len() as u32,
            0u32,
        ]) {
            bytes.copy_from_slice(&value.to_ne_bytes());
        }
        for (dst, palette) in self.scratch[16..].chunks_exact_mut(stride).zip(palettes) {
            write_matrices(dst, palette);
        }
        let header = BufferSize::new(16).expect("non-zero");
        self.belt
            .write_buffer(encoder, &crowd.params, 0, header)
            .copy_from_slice(&self.scratch[..16]);
        if let Some(size) = BufferSize::new((total - 16) as u64) {
            self.belt
                .write_buffer(encoder, &crowd.palette, 0, size)
                .copy_from_slice(&self.scratch[16..]);
        }
        self.belt.finish_and_recall_on_submit(encoder);
        if crowd.group.is_none() {
            crowd.group = Some(gpu.device.create_bind_group(&BindGroupDescriptor {
                label: Some("skinned crowd"),
                layout: &self.layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: crowd.bind_pose.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: crowd.influences.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: crowd.palette.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: crowd.output.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: crowd.params.as_entire_binding(),
                    },
                ],
            }));
        }
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("skinned crowd"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, crowd.group.as_ref(), &[]);
        let (x, y) = dispatch_size(crowd.vertex_count * palettes.len() as u32);
        pass.dispatch_workgroups(x, y, 1);
        Ok(())
    }

    /// Poses several crowds into one command buffer.
    pub fn prepare_crowds(
        &mut self,
        gpu: &Gpu,
        jobs: &mut [(&mut SkinnedCrowd, &[&[Mat4]])],
    ) -> GraphicsResult<CommandBuffer> {
        checked(gpu, || {
            let mut encoder = gpu.encoder("skinned crowds");
            for (crowd, palettes) in jobs.iter_mut() {
                self.record_crowd(gpu, &mut encoder, crowd, palettes)?;
            }
            Ok(encoder.finish())
        })?
    }

    /// Poses every mesh with its palette into one command buffer. Submit it
    /// before the passes that draw these meshes.
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        jobs: &mut [(&mut SkinnedMesh, &[Mat4])],
    ) -> GraphicsResult<CommandBuffer> {
        checked(gpu, || {
            let mut encoder = gpu.encoder("skinning");
            for (mesh, palette) in jobs.iter_mut() {
                self.record(gpu, &mut encoder, mesh, palette)?;
            }
            Ok(encoder.finish())
        })?
    }
}
