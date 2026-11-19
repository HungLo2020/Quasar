//! Depth-driven screen effects over a [`SceneTarget`]: ink edges for toon
//! looks and radial god rays.
//!
//! Both read only the scene depth and blend into the scene colour, so they
//! run between the opaque passes and [`PostChain`] with no colour copy:
//!
//! - [`ScreenEffects::prepare_edges`] inks silhouettes, depth steps and
//!   creases. Creases come from the surface reconstructed from depth (its
//!   direction either side of each pixel), so they follow real surface
//!   folds rather than depth slope. Pair it with [`ShadingModel::Toon`] for a
//!   cel-shaded look.
//! - [`ScreenEffects::prepare_god_rays`] blurs the open sky (pixels with no
//!   geometry) radially from a light's screen position and adds it, for sun
//!   shafts past silhouettes. For light through openings inside geometry,
//!   use fog shafts ([`Fog::with_shafts`]).
use super::*;

/// Ink edges; see [`ScreenEffects::prepare_edges`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeSettings {
    /// Linear RGB ink and its opacity.
    pub color: [f32; 4],
    /// Sample offset in pixels (line thickness), 1 to 4.
    pub width: f32,
    /// Depth step that counts as an edge, relative to the viewing distance
    /// (0.02 = 2%).
    pub depth_threshold: f32,
    /// Cosine of the bend that counts as a crease: the surface direction
    /// either side of a pixel must turn by more than its angle (0.8 is
    /// about 37 degrees; 1 inks every bend, -1 none).
    pub crease_threshold: f32,
    /// Edges fade out by this distance in metres (0 = never).
    pub fade_distance: f32,
}

impl Default for EdgeSettings {
    fn default() -> Self {
        Self {
            color: [0.02, 0.015, 0.01, 1.],
            width: 1.,
            depth_threshold: 0.03,
            crease_threshold: 0.8,
            fade_distance: 0.,
        }
    }
}

impl EdgeSettings {
    fn valid(&self) -> bool {
        self.color.iter().all(|v| v.is_finite() && *v >= 0.)
            && self.color[3] <= 1.
            && (1. ..=4.).contains(&self.width)
            && self.depth_threshold.is_finite()
            && self.depth_threshold > 0.
            && (-1. ..=1.).contains(&self.crease_threshold)
            && self.fade_distance.is_finite()
            && self.fade_distance >= 0.
    }
}

/// Where god rays radiate from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GodRaySource {
    /// A directional light: the direction its rays travel.
    Direction([f32; 3]),
    /// A point in the world (a lamp, the sun disc of a skybox).
    Position([f32; 3]),
}

/// Screen-space god rays; see [`ScreenEffects::prepare_god_rays`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GodRays {
    pub source: GodRaySource,
    /// Linear RGB.
    pub color: [f32; 3],
    pub intensity: f32,
    /// Per-sample weight decay along each ray (0.9 to 0.99).
    pub decay: f32,
    /// How far towards the light each ray reaches (0 to 1).
    pub density: f32,
    /// Samples per pixel (8 to 128).
    pub samples: u32,
}

impl Default for GodRays {
    fn default() -> Self {
        Self {
            source: GodRaySource::Direction([0., -1., 0.]),
            color: [1., 0.92, 0.75],
            intensity: 0.6,
            decay: 0.96,
            density: 0.85,
            samples: 48,
        }
    }
}

impl GodRays {
    fn valid(&self) -> bool {
        let source = match self.source {
            GodRaySource::Direction(d) | GodRaySource::Position(d) => d,
        };
        source.iter().all(|v| v.is_finite())
            && lighting::positive_rgb(self.color)
            && self.intensity.is_finite()
            && self.intensity >= 0.
            && (0. ..=1.).contains(&self.decay)
            && (0. ..=1.).contains(&self.density)
            && (1..=256).contains(&self.samples)
    }
}

const PARAM_BYTES: u64 = 128;

/// Ink edges and god rays over a scene. See the module docs.
pub struct ScreenEffects {
    layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    module: ShaderModule,
    pipelines: HashMap<(&'static str, TextureFormat), RenderPipeline>,
    params: Buffer,
}

impl ScreenEffects {
    pub fn new(gpu: &Gpu) -> GraphicsResult<Self> {
        let layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("screen effects"),
                entries: &[
                    image_entry(0, TextureViewDimension::D2, true),
                    BindGroupLayoutEntry {
                        binding: 1,
                        visibility: ShaderStages::FRAGMENT,
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
                label: Some("screen effects"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let module = checked(gpu, || {
            gpu.device.create_shader_module(ShaderModuleDescriptor {
                label: Some("screen effects"),
                source: ShaderSource::Wgsl(include_str!("screen.wgsl").into()),
            })
        })?;
        let params = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("screen effect params"),
            size: PARAM_BYTES * 2,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            layout,
            pipeline_layout,
            module,
            pipelines: HashMap::new(),
            params,
        })
    }

    fn pipeline(&mut self, gpu: &Gpu, entry: &'static str, format: TextureFormat) {
        if self.pipelines.contains_key(&(entry, format)) {
            return;
        }
        let blend = if entry == "edges" {
            BlendState::ALPHA_BLENDING
        } else {
            BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                },
                alpha: BlendComponent::OVER,
            }
        };
        let pipeline = gpu
            .device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&self.pipeline_layout),
                vertex: VertexState {
                    module: &self.module,
                    entry_point: Some("fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(FragmentState {
                    module: &self.module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: ColorWrites::COLOR,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines.insert((entry, format), pipeline);
    }

    fn record(
        &mut self,
        gpu: &Gpu,
        scene: &SceneTarget,
        entry: &'static str,
        slot: u64,
        params: Vec<f32>,
    ) -> GraphicsResult<CommandBuffer> {
        let format = scene.target().format;
        checked(gpu, || {
            self.pipeline(gpu, entry, format);
            let mut encoder = gpu.encoder(entry);
            let data = bytes(params);
            debug_assert_eq!(data.len() as u64, PARAM_BYTES);
            let upload = quasar_frame::staging_buffer(gpu, "screen effect params", &data);
            encoder.copy_buffer_to_buffer(
                &upload,
                0,
                &self.params,
                slot * PARAM_BYTES,
                PARAM_BYTES,
            );
            let group = gpu.device.create_bind_group(&BindGroupDescriptor {
                label: Some(entry),
                layout: &self.layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(scene.depth_view()),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Buffer(BufferBinding {
                            buffer: &self.params,
                            offset: slot * PARAM_BYTES,
                            size: BufferSize::new(PARAM_BYTES),
                        }),
                    },
                ],
            });
            let colors = [Some(RenderPassColorAttachment {
                view: &scene.target().color,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some(entry),
                color_attachments: &colors,
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines[&(entry, format)]);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            Ok(encoder.finish())
        })?
    }

    /// Inks edges into `scene` for the camera its opaque pass used.
    pub fn prepare_edges(
        &mut self,
        gpu: &Gpu,
        scene: &SceneTarget,
        camera: LitCamera3D,
        settings: &EdgeSettings,
    ) -> GraphicsResult<CommandBuffer> {
        if !settings.valid() {
            return Err(error("invalid edge settings"));
        }
        let inverse = camera
            .view_projection
            .inverse()
            .filter(|m| m.finite())
            .ok_or_else(|| error("edge camera must be invertible"))?;
        let (width, height) = scene.size();
        let mut params: Vec<f32> = inverse.0.iter().flatten().copied().collect();
        params.extend([width as f32, height as f32, 0., 0.]);
        params.extend([
            camera.position[0],
            camera.position[1],
            camera.position[2],
            0.,
        ]);
        params.extend(settings.color);
        params.extend([
            settings.width,
            settings.depth_threshold,
            settings.crease_threshold,
            settings.fade_distance,
        ]);
        self.record(gpu, scene, "edges", 0, params)
    }

    /// Adds god rays to `scene`. Returns an empty command buffer when the
    /// light is behind the camera.
    pub fn prepare_god_rays(
        &mut self,
        gpu: &Gpu,
        scene: &SceneTarget,
        camera: LitCamera3D,
        rays: &GodRays,
    ) -> GraphicsResult<CommandBuffer> {
        if !rays.valid() {
            return Err(error("invalid god ray settings"));
        }
        let clip = match rays.source {
            GodRaySource::Direction(d) => {
                let m = camera.view_projection.0;
                let v = [-d[0], -d[1], -d[2], 0.];
                std::array::from_fn::<f32, 4, _>(|r| (0..4).map(|c| m[c][r] * v[c]).sum())
            }
            GodRaySource::Position(p) => {
                let m = camera.view_projection.0;
                let v = [p[0], p[1], p[2], 1.];
                std::array::from_fn::<f32, 4, _>(|r| (0..4).map(|c| m[c][r] * v[c]).sum())
            }
        };
        if clip[3] <= 1e-6 || rays.intensity == 0. {
            return Ok(gpu.encoder("god rays (off)").finish());
        }
        let uv = [clip[0] / clip[3] * 0.5 + 0.5, 0.5 - clip[1] / clip[3] * 0.5];
        // Fade out as the light leaves the screen.
        let outside = (uv[0] - 0.5).abs().max((uv[1] - 0.5).abs()) - 0.5;
        let weight = (1. - outside.max(0.) / 0.5).clamp(0., 1.);
        let inverse = camera
            .view_projection
            .inverse()
            .filter(|m| m.finite())
            .ok_or_else(|| error("god ray camera must be invertible"))?;
        let (width, height) = scene.size();
        let mut params: Vec<f32> = inverse.0.iter().flatten().copied().collect();
        params.extend([width as f32, height as f32, uv[0], uv[1]]);
        params.extend([
            camera.position[0],
            camera.position[1],
            camera.position[2],
            0.,
        ]);
        params.extend([
            rays.color[0] * rays.intensity,
            rays.color[1] * rays.intensity,
            rays.color[2] * rays.intensity,
            1.,
        ]);
        params.extend([rays.samples as f32, rays.decay, rays.density, weight]);
        self.record(gpu, scene, "god_rays", 1, params)
    }

    /// Drops cached pipelines.
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.pipelines.clear();
        Ok(())
    }
}
