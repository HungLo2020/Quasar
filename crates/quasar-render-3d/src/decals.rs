//! Projected box decals (scorch marks, mining scars, footprints).
//!
//! A [`Decal`] is a unit cube (`-0.5..0.5` on every axis) placed in the
//! world by its transform. After the opaque pass, [`DecalRenderer::prepare`]
//! reads the [`SceneTarget`]'s depth, reconstructs the surface under each
//! pixel the box covers and, where that surface lies inside the box,
//! projects the decal texture onto it along the box's local Y axis
//! (`local.x` → U, `-local.z` → V). The pass blends into the scene colour
//! without a depth attachment, so it composes with any opaque renderer that
//! wrote the scene depth: meshes, voxels or custom passes.
use super::*;

/// One decal for [`DecalRenderer::prepare`].
#[derive(Clone, Debug)]
pub struct Decal<'a> {
    /// Unit cube to world. Local Y is the projection axis; scale it to the
    /// depth range the decal should reach.
    pub transform: Mat4,
    pub texture_view: &'a TextureView,
    /// Linear RGBA multiplied with the texture.
    pub tint: [f32; 4],
    /// Fraction of the half height over which the decal fades out towards
    /// the box's top and bottom (0 = hard edge, 1 = fade from the centre).
    pub fade: f32,
}

impl<'a> Decal<'a> {
    pub fn new(transform: Mat4, texture_view: &'a TextureView) -> Self {
        Self {
            transform,
            texture_view,
            tint: [1.; 4],
            fade: 0.25,
        }
    }
}

/// Pipelines and per-frame buffers for projected decals.
pub struct DecalRenderer {
    layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    module: ShaderModule,
    pipelines: HashMap<(TextureFormat, BlendMode, bool), RenderPipeline>,
    decals: Option<Buffer>,
    frame: Buffer,
    batches: Vec<Buffer>,
    sampler: Sampler,
    draws: usize,
}

impl DecalRenderer {
    pub fn new(gpu: &Gpu) -> GraphicsResult<Self> {
        let uniform = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("decals"),
                entries: &[
                    storage_entry(0, ShaderStages::VERTEX_FRAGMENT),
                    uniform(1),
                    image_entry(2, TextureViewDimension::D2, true),
                    image_entry(3, TextureViewDimension::D2, false),
                    sampler_entry(4, false),
                    uniform(5),
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("decals"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let module = checked(gpu, || {
            gpu.device.create_shader_module(ShaderModuleDescriptor {
                label: Some("decals"),
                source: ShaderSource::Wgsl(include_str!("decals.wgsl").into()),
            })
        })?;
        let frame = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("decal frame"),
            size: 144,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = gpu.device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            layout,
            pipeline_layout,
            module,
            pipelines: HashMap::new(),
            decals: None,
            frame,
            batches: Vec::new(),
            sampler,
            draws: 0,
        })
    }

    /// Draw calls issued by the last prepare (one per run of decals sharing
    /// a texture and blend mode).
    pub fn draws(&self) -> usize {
        self.draws
    }

    fn pipeline(&mut self, gpu: &Gpu, format: TextureFormat, blend: BlendMode, mirrored: bool) {
        let key = (format, blend, mirrored);
        if self.pipelines.contains_key(&key) {
            return;
        }
        let pipeline = gpu
            .device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("decals"),
                layout: Some(&self.pipeline_layout),
                vertex: VertexState {
                    module: &self.module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                // Back faces only: one fragment per covered pixel, even with
                // the camera inside the box.
                primitive: PrimitiveState {
                    cull_mode: Some(Face::Front),
                    front_face: if mirrored {
                        FrontFace::Cw
                    } else {
                        FrontFace::Ccw
                    },
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(FragmentState {
                    module: &self.module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(ColorTargetState {
                        format,
                        blend: Some(blend.state()),
                        write_mask: ColorWrites::COLOR,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines.insert(key, pipeline);
    }

    /// Projects `decals` onto the scene. `view_projection` must be the
    /// camera the opaque pass used. Decals sharing a texture are drawn
    /// together; order between textures follows `decals`.
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        scene: &SceneTarget,
        view_projection: Mat4,
        decals: &[Decal<'_>],
        blend: BlendMode,
    ) -> GraphicsResult<CommandBuffer> {
        let inverse = view_projection
            .inverse()
            .filter(|m| m.finite())
            .ok_or_else(|| error("decal camera must be invertible"))?;
        let mut data = Vec::with_capacity(decals.len() * 36);
        for decal in decals {
            let local = decal
                .transform
                .inverse()
                .filter(|m| m.finite() && lighting::invertible_affine(decal.transform))
                .ok_or_else(|| error("decal transforms must be invertible affine matrices"))?;
            if decal.tint.iter().any(|v| !v.is_finite() || *v < 0.)
                || !(0. ..=1.).contains(&decal.fade)
            {
                return Err(error("decal tint must be non-negative and fade in 0..=1"));
            }
            data.extend(decal.transform.0.iter().flatten().copied());
            data.extend(local.0.iter().flatten().copied());
            data.extend(decal.tint);
            data.extend([decal.fade, 0., 0., 0.]);
        }
        checked(gpu, || {
            let mut encoder = gpu.encoder("decals");
            self.draws = 0;
            if decals.is_empty() {
                return Ok(encoder.finish());
            }
            let bytes = bytes(data);
            quasar_frame::grow_buffer(
                gpu,
                &mut self.decals,
                "decals",
                bytes.len() as u64,
                BufferUsages::STORAGE,
            )?;
            let (width, height) = scene.size();
            let mut frame: Vec<f32> = view_projection.0.iter().flatten().copied().collect();
            frame.extend(inverse.0.iter().flatten().copied());
            frame.extend([width as f32, height as f32, 0., 0.]);
            let frame = self::bytes(frame);
            let upload = quasar_frame::staging_buffer(
                gpu,
                "decal upload",
                &[bytes.as_slice(), &frame].concat(),
            );
            encoder.copy_buffer_to_buffer(
                &upload,
                0,
                self.decals.as_ref().unwrap(),
                0,
                bytes.len() as u64,
            );
            encoder.copy_buffer_to_buffer(&upload, bytes.len() as u64, &self.frame, 0, 144);
            // Runs of decals sharing a texture and handedness.
            let mut runs: Vec<(usize, usize, bool)> = Vec::new();
            for (index, decal) in decals.iter().enumerate() {
                let flipped = mirrored(decal.transform);
                match runs.last_mut() {
                    Some((start, count, m))
                        if *m == flipped && decals[*start].texture_view == decal.texture_view =>
                    {
                        *count += 1
                    }
                    _ => runs.push((index, 1, flipped)),
                }
            }
            while self.batches.len() < runs.len() {
                self.batches
                    .push(gpu.device.create_buffer(&BufferDescriptor {
                        label: Some("decal batch"),
                        size: 16,
                        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }));
            }
            let target = scene.target();
            let mut groups = Vec::with_capacity(runs.len());
            for (run, (start, _, flipped)) in runs.iter().enumerate() {
                self.pipeline(gpu, target.format, blend, *flipped);
                let base = (*start as u32).to_ne_bytes();
                let upload = quasar_frame::staging_buffer(
                    gpu,
                    "decal batch upload",
                    &[base, [0; 4], [0; 4], [0; 4]].concat(),
                );
                encoder.copy_buffer_to_buffer(&upload, 0, &self.batches[run], 0, 16);
                groups.push(gpu.device.create_bind_group(&BindGroupDescriptor {
                    label: Some("decals"),
                    layout: &self.layout,
                    entries: &[
                        BindGroupEntry {
                            binding: 0,
                            resource: self.decals.as_ref().unwrap().as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: self.frame.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 2,
                            resource: BindingResource::TextureView(scene.depth_view()),
                        },
                        BindGroupEntry {
                            binding: 3,
                            resource: BindingResource::TextureView(decals[*start].texture_view),
                        },
                        BindGroupEntry {
                            binding: 4,
                            resource: BindingResource::Sampler(&self.sampler),
                        },
                        BindGroupEntry {
                            binding: 5,
                            resource: self.batches[run].as_entire_binding(),
                        },
                    ],
                }));
            }
            let colors = [Some(RenderPassColorAttachment {
                view: &target.color,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("decals"),
                color_attachments: &colors,
                ..Default::default()
            });
            for ((_, count, flipped), group) in runs.iter().zip(&groups) {
                pass.set_pipeline(&self.pipelines[&(target.format, blend, *flipped)]);
                pass.set_bind_group(0, group, &[]);
                pass.draw(0..36, 0..*count as u32);
            }
            drop(pass);
            self.draws = runs.len();
            Ok(encoder.finish())
        })?
    }

    /// Drops cached pipelines and buffers.
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.pipelines.clear();
        self.decals = None;
        self.batches.clear();
        Ok(())
    }
}
