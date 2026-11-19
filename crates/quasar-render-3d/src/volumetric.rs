//! Ray-marched volumetric fog: fog with shape and light in it.
//!
//! The analytic [`Fog`] in the lighting module is cheap but uniform: it can
//! only fade colours with distance. [`VolumetricFog`] marches every pixel
//! through a fog volume instead: a layer that hugs the ground (thinning
//! with height), carved by drifting 3D noise into banks and wisps, lit at
//! each sample by the pass's lights (with a forward-scattering phase, so
//! lamps you look towards wear halos and spot lights draw beams, cookies
//! included) plus ambient light. It composites `scene * transmittance +
//! in-scattered light` into a [`SceneTarget`] with one blended draw, and
//! open sky is fogged too, so there is no horizon seam.
//!
//! Run it after the opaque passes (it reads the scene depth) and before the
//! post chain. Cost is `steps x lights` per pixel.
use super::*;

/// Settings for [`VolumetricFog::prepare`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumetricFogSettings {
    /// Scattering colour (albedo, 0..1 per channel).
    pub albedo: [f32; 3],
    /// Extinction per metre at `base` height.
    pub density: f32,
    /// World height where the layer is densest.
    pub base: f32,
    /// Density falls by `exp(-falloff * metres above base)`.
    pub falloff: f32,
    /// Extinction per metre everywhere (thin haze above the layer).
    pub haze: f32,
    /// Noise frequency per metre (banks are about `1 / frequency` wide).
    pub noise_frequency: f32,
    /// 0 (uniform fog) to 1 (fully broken into banks and gaps).
    pub noise_strength: f32,
    /// Drift in metres per second.
    pub wind: [f32; 3],
    /// Henyey-Greenstein anisotropy, -1..1 (0.3 to 0.7: halos around
    /// lights you look towards).
    pub anisotropy: f32,
    /// Light reaching the fog from everywhere (sky, moon bounce), linear.
    pub ambient: [f32; 3],
    /// Samples per pixel (16 to 64).
    pub steps: u32,
    /// Furthest marched distance; the sky uses it.
    pub max_distance: f32,
}

impl Default for VolumetricFogSettings {
    fn default() -> Self {
        Self {
            albedo: [0.9; 3],
            density: 0.25,
            base: 0.0,
            falloff: 0.9,
            haze: 0.01,
            noise_frequency: 0.18,
            noise_strength: 0.8,
            wind: [0.35, 0.0, 0.12],
            anisotropy: 0.45,
            ambient: [0.02, 0.025, 0.035],
            steps: 32,
            max_distance: 60.0,
        }
    }
}

impl VolumetricFogSettings {
    fn valid(&self) -> bool {
        let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
        finite(&self.albedo)
            && finite(&self.wind)
            && finite(&self.ambient)
            && finite(&[
                self.density,
                self.base,
                self.falloff,
                self.haze,
                self.noise_frequency,
                self.noise_strength,
                self.anisotropy,
                self.max_distance,
            ])
            && self.albedo.iter().all(|v| (0. ..=1.).contains(v))
            && self.ambient.iter().all(|v| *v >= 0.)
            && self.density >= 0.
            && self.falloff >= 0.
            && self.haze >= 0.
            && self.noise_frequency > 0.
            && (0. ..=1.).contains(&self.noise_strength)
            && self.anisotropy.abs() < 1.
            && (1..=256).contains(&self.steps)
            && self.max_distance > 0.
    }
}

const PARAM_BYTES: u64 = 160;

/// See the module docs.
pub struct VolumetricFog {
    layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    module: ShaderModule,
    pipelines: HashMap<TextureFormat, RenderPipeline>,
    params: Buffer,
    lights: Option<Buffer>,
    shadow_records: Buffer,
    no_shadow: TextureView,
    comparison: Sampler,
}

impl VolumetricFog {
    pub fn new(gpu: &Gpu) -> GraphicsResult<Self> {
        let buffer_entry = |binding, ty| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("volumetric fog"),
                entries: &[
                    image_entry(0, TextureViewDimension::D2, true),
                    buffer_entry(1, BufferBindingType::Uniform),
                    buffer_entry(3, BufferBindingType::Storage { read_only: true }),
                    image_entry(5, TextureViewDimension::D2Array, true),
                    buffer_entry(6, BufferBindingType::Storage { read_only: true }),
                    sampler_entry(21, true),
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("volumetric fog"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let source = format!(
            "diagnostic(off, derivative_uniformity);\n{}{}",
            LIGHTING_WGSL,
            include_str!("volumetric.wgsl")
        );
        let module = checked(gpu, || {
            gpu.device.create_shader_module(ShaderModuleDescriptor {
                label: Some("volumetric fog"),
                source: ShaderSource::Wgsl(source.into()),
            })
        })?;
        let params = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("volumetric fog params"),
            size: PARAM_BYTES,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_records = buffer(
            gpu,
            "volumetric fog shadows",
            &bytes([0.0f32; 28]),
            BufferUsages::STORAGE,
        );
        let no_shadow = texture_2d(
            &gpu.device,
            "volumetric fog no shadow",
            1,
            1,
            TextureFormat::Depth32Float,
            TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        )?
        .create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..Default::default()
        });
        let comparison = gpu.device.create_sampler(&SamplerDescriptor {
            compare: Some(CompareFunction::LessEqual),
            ..Default::default()
        });
        Ok(Self {
            layout,
            pipeline_layout,
            module,
            pipelines: HashMap::new(),
            params,
            lights: None,
            shadow_records,
            no_shadow,
            comparison,
        })
    }

    fn pipeline(&mut self, gpu: &Gpu, format: TextureFormat) {
        if self.pipelines.contains_key(&format) {
            return;
        }
        // scene * transmittance (alpha) + in-scattered light.
        let blend = BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::SrcAlpha,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent {
                src_factor: BlendFactor::Zero,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
        };
        let pipeline = gpu
            .device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("volumetric fog"),
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
                    entry_point: Some("march"),
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
        self.pipelines.insert(format, pipeline);
    }

    /// Fogs `scene` for the camera its opaque pass used, lit by
    /// `lighting` (point and spot lights with their falloff and cookies,
    /// directional lights everywhere). `time` drives the drift.
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        scene: &SceneTarget,
        camera: LitCamera3D,
        lighting: Lighting<'_>,
        settings: &VolumetricFogSettings,
        time: f32,
    ) -> GraphicsResult<CommandBuffer> {
        if !settings.valid() || !time.is_finite() {
            return Err(error("invalid volumetric fog settings"));
        }
        let inverse = camera
            .view_projection
            .inverse()
            .filter(|m| m.finite())
            .ok_or_else(|| error("volumetric fog camera must be invertible"))?;
        let lights = encode_lighting_with(
            camera,
            lighting,
            lighting.lights.len().max(1),
            &LightingOptions {
                clusters: LightClusters::DISABLED,
                ..LightingOptions::default()
            },
        )?;
        let (width, height) = scene.size();
        let s = settings;
        // Keep the drift coordinate small so noise stays precise.
        let time = time.rem_euclid(4096.);
        let mut params: Vec<f32> = inverse.0.iter().flatten().copied().collect();
        params.extend([width as f32, height as f32, time, s.steps as f32]);
        params.extend([
            camera.position[0],
            camera.position[1],
            camera.position[2],
            s.max_distance,
        ]);
        params.extend([s.albedo[0], s.albedo[1], s.albedo[2], s.density]);
        params.extend([s.base, s.falloff, s.noise_frequency, s.noise_strength]);
        params.extend([s.wind[0], s.wind[1], s.wind[2], s.anisotropy]);
        params.extend([s.ambient[0], s.ambient[1], s.ambient[2], s.haze]);
        let format = scene.target().format;
        checked(gpu, || {
            self.pipeline(gpu, format);
            quasar_frame::grow_buffer(
                gpu,
                &mut self.lights,
                "volumetric fog lights",
                lights.len() as u64,
                BufferUsages::STORAGE,
            )?;
            let mut encoder = gpu.encoder("volumetric fog");
            let params = bytes(params);
            let upload = quasar_frame::staging_buffer(
                gpu,
                "volumetric fog upload",
                &[params.as_slice(), &lights].concat(),
            );
            encoder.copy_buffer_to_buffer(&upload, 0, &self.params, 0, PARAM_BYTES);
            encoder.copy_buffer_to_buffer(
                &upload,
                PARAM_BYTES,
                self.lights.as_ref().unwrap(),
                0,
                lights.len() as u64,
            );
            let group = gpu.device.create_bind_group(&BindGroupDescriptor {
                label: Some("volumetric fog"),
                layout: &self.layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(scene.depth_view()),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: self.params.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: self.lights.as_ref().unwrap().as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: BindingResource::TextureView(&self.no_shadow),
                    },
                    BindGroupEntry {
                        binding: 6,
                        resource: self.shadow_records.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 21,
                        resource: BindingResource::Sampler(&self.comparison),
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
                label: Some("volumetric fog"),
                color_attachments: &colors,
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines[&format]);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            Ok(encoder.finish())
        })?
    }

    /// Drops cached pipelines and buffers.
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.pipelines.clear();
        self.lights = None;
        Ok(())
    }
}
