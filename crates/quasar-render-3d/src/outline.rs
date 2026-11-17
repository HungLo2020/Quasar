//! Screen-space outlines and see-through silhouettes.
//!
//! [`OutlineRenderer::prepare`] runs after the opaque pass on a
//! [`SceneTarget`]. It renders the given meshes into a small id mask,
//! depth-testing against the scene depth: pixels where a mesh is visible get
//! its style id, pixels where it is hidden behind other geometry get the id
//! with an "occluded" flag. A full-screen composite then draws, per
//! [`OutlineStyle`], a ring of `width` pixels around each silhouette (in the
//! visible or occluded colour) and a translucent fill over occluded parts, so
//! enemies behind walls stay readable.
use super::*;

/// Colours (linear RGBA, alpha-blended) and width of one outline look.
/// Zero alpha disables a part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutlineStyle {
    /// Ring around the parts of the mesh that are visible.
    pub outline: [f32; 4],
    /// Ring around the parts hidden behind other geometry.
    pub occluded_outline: [f32; 4],
    /// Fill over the hidden parts.
    pub occluded_fill: [f32; 4],
    /// Ring width in pixels (0..=8).
    pub width: f32,
}

impl Default for OutlineStyle {
    fn default() -> Self {
        Self {
            outline: [1., 0.6, 0.1, 1.],
            occluded_outline: [1., 0.3, 0.1, 0.9],
            occluded_fill: [1., 0.3, 0.1, 0.25],
            width: 2.,
        }
    }
}

/// One mesh to outline with `styles[style]`.
pub struct OutlineDraw<'a> {
    pub mesh: &'a Mesh,
    pub model: Mat4,
    pub style: u8,
}

/// Largest number of styles in one prepare.
pub const MAX_OUTLINE_STYLES: usize = 64;

pub struct OutlineRenderer {
    mask_layout: BindGroupLayout,
    composite_layout: BindGroupLayout,
    mask_module: ShaderModule,
    composite_module: ShaderModule,
    mask_pipeline_layout: PipelineLayout,
    composite_pipeline_layout: PipelineLayout,
    visible: Option<RenderPipeline>,
    occluded: Option<RenderPipeline>,
    composite: HashMap<TextureFormat, RenderPipeline>,
    mask: Option<(Texture, TextureView)>,
    instances: Option<Buffer>,
    styles: Buffer,
    batches: Vec<Buffer>,
    draws: usize,
}

impl OutlineRenderer {
    pub fn new(gpu: &Gpu) -> GraphicsResult<Self> {
        let uniform = |binding, visibility| BindGroupLayoutEntry {
            binding,
            visibility,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let mask_layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("outline mask"),
                entries: &[
                    storage_entry(0, ShaderStages::VERTEX),
                    storage_entry(1, ShaderStages::VERTEX),
                    uniform(2, ShaderStages::VERTEX),
                ],
            });
        let composite_layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("outline composite"),
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        visibility: ShaderStages::FRAGMENT,
                        ty: BindingType::Texture {
                            sample_type: TextureSampleType::Uint,
                            view_dimension: TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    uniform(1, ShaderStages::FRAGMENT),
                ],
            });
        let source = include_str!("outline.wgsl");
        let (mask_src, composite_src) = source
            .split_once("// --- composite ---")
            .expect("outline shader sections");
        let module = |label, code: &str| {
            checked(gpu, || {
                gpu.device.create_shader_module(ShaderModuleDescriptor {
                    label: Some(label),
                    source: ShaderSource::Wgsl(code.to_owned().into()),
                })
            })
        };
        let mask_module = module("outline mask", mask_src)?;
        let composite_module = module("outline composite", composite_src)?;
        let layout = |label, group: &BindGroupLayout| {
            gpu.device
                .create_pipeline_layout(&PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &[Some(group)],
                    immediate_size: 0,
                })
        };
        let mask_pipeline_layout = layout("outline mask", &mask_layout);
        let composite_pipeline_layout = layout("outline composite", &composite_layout);
        let styles = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("outline styles"),
            size: 16 + 64 * MAX_OUTLINE_STYLES as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            mask_layout,
            composite_layout,
            mask_module,
            composite_module,
            mask_pipeline_layout,
            composite_pipeline_layout,
            visible: None,
            occluded: None,
            composite: HashMap::new(),
            mask: None,
            instances: None,
            styles,
            batches: Vec::new(),
            draws: 0,
        })
    }

    /// Mask draw calls issued by the last prepare (two per run of draws
    /// sharing geometry buffers: occluded, then visible).
    pub fn draws(&self) -> usize {
        self.draws
    }

    fn mask_pipeline(&self, gpu: &Gpu, occluded: bool) -> RenderPipeline {
        gpu.device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("outline mask"),
                layout: Some(&self.mask_pipeline_layout),
                vertex: VertexState {
                    module: &self.mask_module,
                    entry_point: Some("vs_mask"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: PrimitiveState::default(),
                depth_stencil: Some(DepthStencilState {
                    format: TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(if occluded {
                        CompareFunction::Greater
                    } else {
                        CompareFunction::LessEqual
                    }),
                    stencil: Default::default(),
                    // Lets the outlined mesh's own front faces (already in the
                    // scene depth) pass the visible test.
                    bias: DepthBiasState {
                        constant: if occluded { 0 } else { -2 },
                        slope_scale: if occluded { 0. } else { -1. },
                        clamp: 0.,
                    },
                }),
                multisample: Default::default(),
                fragment: Some(FragmentState {
                    module: &self.mask_module,
                    entry_point: Some(if occluded {
                        "fs_occluded"
                    } else {
                        "fs_visible"
                    }),
                    compilation_options: Default::default(),
                    targets: &[Some(ColorTargetState {
                        format: TextureFormat::R8Uint,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    /// Outlines `draws` over `scene` (after its opaque pass) with the camera
    /// the scene was drawn with.
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        scene: &SceneTarget,
        view_projection: Mat4,
        draws: &[OutlineDraw<'_>],
        styles: &[OutlineStyle],
    ) -> GraphicsResult<CommandBuffer> {
        if styles.is_empty() || styles.len() > MAX_OUTLINE_STYLES {
            return Err(error("outlines need 1..=64 styles"));
        }
        if styles.iter().any(|s| {
            !(0. ..=8.).contains(&s.width)
                || s.outline
                    .iter()
                    .chain(&s.occluded_outline)
                    .chain(&s.occluded_fill)
                    .any(|v| !v.is_finite() || *v < 0.)
        }) {
            return Err(error(
                "outline widths must be 0..=8 and colours non-negative",
            ));
        }
        if draws.iter().any(|d| {
            usize::from(d.style) >= styles.len() || d.mesh.vertices.is_none() || !d.model.finite()
        }) || !view_projection.finite()
        {
            return Err(error("outline draws need a valid style, mesh and model"));
        }
        checked(gpu, || {
            let mut encoder = gpu.encoder("outlines");
            self.draws = 0;
            if draws.is_empty() {
                return Ok(encoder.finish());
            }
            if self.visible.is_none() {
                self.visible = Some(self.mask_pipeline(gpu, false));
                self.occluded = Some(self.mask_pipeline(gpu, true));
            }
            let (width, height) = scene.size();
            if self
                .mask
                .as_ref()
                .is_none_or(|(t, _)| (t.width(), t.height()) != (width, height))
            {
                let texture = texture_2d(
                    &gpu.device,
                    "outline mask",
                    width,
                    height,
                    TextureFormat::R8Uint,
                    TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                )?;
                let view = texture.create_view(&Default::default());
                self.mask = Some((texture, view));
            }
            let mut data = Vec::with_capacity(draws.len() * 20);
            let mut runs: Vec<(usize, u32)> = Vec::new();
            for (index, d) in draws.iter().enumerate() {
                data.extend((view_projection * d.model).0.iter().flatten().copied());
                data.extend([d.mesh.vertex_base as f32, f32::from(d.style), 0., 0.]);
                match runs.last_mut() {
                    Some((start, count)) if draws[*start].mesh.shares_buffers(d.mesh) => {
                        *count += 1
                    }
                    _ => runs.push((index, 1)),
                }
            }
            let instance_bytes = bytes(data);
            quasar_frame::grow_buffer(
                gpu,
                &mut self.instances,
                "outline instances",
                instance_bytes.len() as u64,
                BufferUsages::STORAGE,
            )?;
            let radius = styles.iter().map(|s| s.width).fold(0f32, f32::max).ceil();
            let mut style_values = vec![radius, 0., 0., 0.];
            for s in styles {
                style_values.extend(s.outline);
                style_values.extend(s.occluded_outline);
                style_values.extend(s.occluded_fill);
                style_values.extend([s.width, 0., 0., 0.]);
            }
            let style_bytes = bytes(style_values);
            let upload = quasar_frame::staging_buffer(
                gpu,
                "outline upload",
                &[instance_bytes.as_slice(), &style_bytes].concat(),
            );
            encoder.copy_buffer_to_buffer(
                &upload,
                0,
                self.instances.as_ref().unwrap(),
                0,
                instance_bytes.len() as u64,
            );
            encoder.copy_buffer_to_buffer(
                &upload,
                instance_bytes.len() as u64,
                &self.styles,
                0,
                style_bytes.len() as u64,
            );
            while self.batches.len() < runs.len() {
                self.batches
                    .push(gpu.device.create_buffer(&BufferDescriptor {
                        label: Some("outline batch"),
                        size: 16,
                        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }));
            }
            let mut groups = Vec::with_capacity(runs.len());
            for (run, (start, _)) in runs.iter().enumerate() {
                let upload = quasar_frame::staging_buffer(
                    gpu,
                    "outline batch upload",
                    &[(*start as u32).to_ne_bytes(), [0; 4], [0; 4], [0; 4]].concat(),
                );
                encoder.copy_buffer_to_buffer(&upload, 0, &self.batches[run], 0, 16);
                groups.push(
                    gpu.device.create_bind_group(&BindGroupDescriptor {
                        label: Some("outline mask"),
                        layout: &self.mask_layout,
                        entries: &[
                            BindGroupEntry {
                                binding: 0,
                                resource: draws[*start]
                                    .mesh
                                    .vertices
                                    .as_ref()
                                    .unwrap()
                                    .as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 1,
                                resource: self.instances.as_ref().unwrap().as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 2,
                                resource: self.batches[run].as_entire_binding(),
                            },
                        ],
                    }),
                );
            }
            let mask_view = &self.mask.as_ref().unwrap().1;
            {
                let colors = [Some(RenderPassColorAttachment {
                    view: mask_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::TRANSPARENT),
                        store: StoreOp::Store,
                    },
                })];
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("outline mask"),
                    color_attachments: &colors,
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: scene.depth_view(),
                        depth_ops: None,
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                // Occluded first so visible coverage wins where both apply.
                for pipeline in [self.occluded.as_ref(), self.visible.as_ref()] {
                    pass.set_pipeline(pipeline.unwrap());
                    for ((start, count), group) in runs.iter().zip(&groups) {
                        let mesh = draws[*start].mesh;
                        pass.set_bind_group(0, group, &[]);
                        pass.set_index_buffer(
                            mesh.indices.as_ref().unwrap().slice(..),
                            IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..mesh.count, 0, 0..*count);
                    }
                }
            }
            let target = scene.target();
            if !self.composite.contains_key(&target.format) {
                let pipeline = gpu
                    .device
                    .create_render_pipeline(&RenderPipelineDescriptor {
                        label: Some("outline composite"),
                        layout: Some(&self.composite_pipeline_layout),
                        vertex: VertexState {
                            module: &self.composite_module,
                            entry_point: Some("vs_full"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: PrimitiveState::default(),
                        depth_stencil: None,
                        multisample: Default::default(),
                        fragment: Some(FragmentState {
                            module: &self.composite_module,
                            entry_point: Some("fs_composite"),
                            compilation_options: Default::default(),
                            targets: &[Some(ColorTargetState {
                                format: target.format,
                                blend: Some(BlendMode::Alpha.state()),
                                write_mask: ColorWrites::COLOR,
                            })],
                        }),
                        multiview_mask: None,
                        cache: None,
                    });
                self.composite.insert(target.format, pipeline);
            }
            let group = gpu.device.create_bind_group(&BindGroupDescriptor {
                label: Some("outline composite"),
                layout: &self.composite_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(mask_view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: self.styles.as_entire_binding(),
                    },
                ],
            });
            {
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
                    label: Some("outline composite"),
                    color_attachments: &colors,
                    ..Default::default()
                });
                pass.set_pipeline(&self.composite[&target.format]);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
            self.draws = runs.len() * 2;
            Ok(encoder.finish())
        })?
    }

    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.visible = None;
        self.occluded = None;
        self.composite.clear();
        self.mask = None;
        self.instances = None;
        self.batches.clear();
        Ok(())
    }
}
