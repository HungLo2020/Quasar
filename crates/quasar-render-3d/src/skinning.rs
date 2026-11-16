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
        if !(1..=MAX_JOINTS).contains(&joint_count) || influences.len() != vertices.len() {
            return Err(error(
                "skinned meshes need 1..=1024 joints and one influence per vertex",
            ));
        }
        let influences = normalized(influences, joint_count)?;
        let mut mesh = Mesh::upload_with_usage(gpu, vertices, indices, BufferUsages::COPY_SRC)?;
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
        for value in [vertices.len() as u32, joint_count as u32, 0, 0] {
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
        Self::upload(gpu, &vertices, &influences, &primitive.indices, joint_count)
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
        let size = u64::from(self.vertex_count) * 48;
        let staging = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("skinned vertex readback"),
            size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.encoder("skinned vertex readback");
        encoder.copy_buffer_to_buffer(
            self.mesh
                .vertices
                .as_ref()
                .ok_or_else(|| error("released skinned mesh"))?,
            0,
            &staging,
            0,
            size,
        );
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
    /// Drops this owner's GPU references.
    pub fn release(&mut self, gpu: &Gpu) -> GraphicsResult<()> {
        self.group = None;
        self.mesh.release(gpu)
    }
}

/// Compute pipeline that poses [`SkinnedMesh`]es. One per device is enough.
pub struct SkinningPass {
    pipeline: ComputePipeline,
    layout: BindGroupLayout,
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
        Ok(Self { pipeline, layout })
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
        let data = bytes(palette.iter().flat_map(|m| m.0.into_iter().flatten()));
        let upload = quasar_frame::staging_buffer(gpu, "skinning palette upload", &data);
        encoder.copy_buffer_to_buffer(&upload, 0, &mesh.palette, 0, data.len() as u64);
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
        pass.dispatch_workgroups(mesh.vertex_count.div_ceil(64), 1, 1);
        Ok(())
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
