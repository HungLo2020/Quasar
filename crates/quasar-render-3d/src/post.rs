//! Linear HDR scene targets, sorted transparent draws and a post chain.
//!
//! The default lit pass may encode sRGB in its fragment shader so it can draw
//! straight into a UNORM swapchain. Blending after that encode is wrong, so
//! transparent work needs a linear target and one final encode. This module
//! provides that path without taking ownership of the frame:
//!
//! 1. [`SceneTarget`] is an `Rgba16Float` colour + depth target. Render
//!    opaque lit/surface/shadowed draws into it with
//!    [`ColorSpace::Linear`](crate::ColorSpace::Linear) output.
//! 2. [`LitRenderState3D::prepare_transparent`] draws [`TransparentDraw3D`]s
//!    into the same target, sorted back to front, with alpha, additive or
//!    premultiplied blending and (by default) no depth writes.
//! 3. [`PostChain`] reads the scene view and writes the caller's output
//!    target: exposure, bloom, tonemapping, colour grading and vignette, then
//!    exactly one sRGB encode (in-shader for UNORM targets, by the hardware
//!    for `*Srgb` formats).
//!
//! Every stage takes caller-owned views and returns command buffers or
//! records into the caller's encoder, so raw VulkanicGAL passes can run
//! between any two stages on the same device and frame.
use super::*;

/// Colour format of [`SceneTarget`]: linear, HDR, blendable and filterable.
pub const SCENE_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// How a transparent draw combines with the colour already in the target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlendMode {
    /// `src * a + dst * (1 - a)`: glass, holograms, ghost previews.
    #[default]
    Alpha,
    /// `src * a + dst`: beams, flames, sparks and glows. Order-independent.
    Additive,
    /// `src + dst * (1 - a)` for colours already multiplied by alpha.
    Premultiplied,
}

impl BlendMode {
    pub(crate) fn state(self) -> BlendState {
        match self {
            Self::Alpha => BlendState::ALPHA_BLENDING,
            Self::Premultiplied => BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            Self::Additive => BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::SrcAlpha,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                },
                alpha: BlendComponent {
                    src_factor: BlendFactor::Zero,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                },
            },
        }
    }
}

impl PassOptions {
    /// Keeps existing colour and depth: the usual options for a transparent
    /// pass after the opaque pass.
    pub const fn load() -> Self {
        Self {
            clear_color: None,
            clear_depth: false,
        }
    }
}

/// Caller-owned linear HDR colour + depth target for a 3D scene.
pub struct SceneTarget {
    color: Texture,
    _depth: Texture,
    target: RenderTarget,
}

impl SceneTarget {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> GraphicsResult<Self> {
        let color = texture_2d(
            &gpu.device,
            "HDR scene color",
            width,
            height,
            SCENE_FORMAT,
            TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
        )?;
        let depth = texture_2d(
            &gpu.device,
            "HDR scene depth",
            width,
            height,
            TextureFormat::Depth32Float,
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        )?;
        let mut target = RenderTarget::from_texture(&color);
        target.depth = Some(depth.create_view(&Default::default()));
        Ok(Self {
            color,
            _depth: depth,
            target,
        })
    }
    /// Recreates the textures when the size changes; returns whether it did.
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) -> GraphicsResult<bool> {
        if self.size() == (width, height) {
            return Ok(false);
        }
        *self = Self::new(gpu, width, height)?;
        Ok(true)
    }
    /// Render target for 3D passes (colour plus this scene's depth).
    pub fn target(&self) -> &RenderTarget {
        &self.target
    }
    /// Sampled view for [`PostChain`] or custom passes.
    pub fn color_view(&self) -> &TextureView {
        &self.target.color
    }
    pub fn color_texture(&self) -> &Texture {
        &self.color
    }
    /// Depth view, e.g. for soft particles or custom fog passes.
    pub fn depth_view(&self) -> &TextureView {
        self.target.depth.as_ref().expect("scene targets own depth")
    }
    pub fn size(&self) -> (u32, u32) {
        self.target.size()
    }
}

/// One sorted, blended draw for [`LitRenderState3D::prepare_transparent`].
pub struct TransparentDraw3D<'a> {
    pub mesh: &'a Mesh,
    pub model: Mat4,
    /// Lit, diffuse or unlit material; `base.tint[3]` scales alpha.
    pub material: LitMaterial,
    pub blend: BlendMode,
    /// Transparent surfaces normally test depth without writing it so they
    /// do not hide each other; enable for e.g. alpha-tested foliage.
    pub depth_write: bool,
}

impl<'a> TransparentDraw3D<'a> {
    pub fn new(mesh: &'a Mesh, model: Mat4, material: LitMaterial, blend: BlendMode) -> Self {
        Self {
            mesh,
            model,
            material,
            blend,
            depth_write: false,
        }
    }
}

/// Indices of `items` (model matrix, local bounds) ordered farthest first by
/// the NDC depth of each bounds centre. Points behind the camera sort last;
/// ties keep submission order, so the result is deterministic.
pub fn sort_back_to_front(
    view_projection: Mat4,
    items: impl IntoIterator<Item = (Mat4, ([f32; 3], [f32; 3]))>,
) -> Vec<usize> {
    let mut keyed: Vec<(usize, f32)> = items
        .into_iter()
        .enumerate()
        .map(|(index, (model, (min, max)))| {
            let center = [
                (min[0] + max[0]) * 0.5,
                (min[1] + max[1]) * 0.5,
                (min[2] + max[2]) * 0.5,
                1.0,
            ];
            let clip = (view_projection * model).transform(center);
            let depth = if clip[3] > f32::EPSILON && (clip[2] / clip[3]).is_finite() {
                clip[2] / clip[3]
            } else {
                f32::NEG_INFINITY
            };
            (index, depth)
        })
        .collect();
    keyed.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    keyed.into_iter().map(|(index, _)| index).collect()
}

impl LitRenderState3D {
    /// Draws blended surfaces back to front into a linear target, usually a
    /// [`SceneTarget`] after the opaque pass (pass [`PassOptions::load`]).
    ///
    /// Requires `lighting.output_color_space == ColorSpace::Linear`; encode
    /// once afterwards with [`PostChain`].
    pub fn prepare_transparent(
        &mut self,
        gpu: &Gpu,
        target: &RenderTarget,
        camera: LitCamera3D,
        lighting: Lighting<'_>,
        draws: &[TransparentDraw3D<'_>],
        options: impl Into<LitPassOptions>,
    ) -> GraphicsResult<Prepared3D> {
        if lighting.output_color_space != ColorSpace::Linear {
            return Err(error(
                "transparent draws need a linear target; encode sRGB once in the post chain",
            ));
        }
        let materials: Vec<LitMaterial> = draws
            .iter()
            .map(|d| {
                let mut material = d.material.clone();
                material.base.transparent = true;
                material.base.depth_write = d.depth_write;
                material
            })
            .collect();
        if materials.iter().any(|m| !m.valid()) {
            return Err(error("invalid lit material"));
        }
        let order = sort_back_to_front(
            camera.view_projection,
            draws.iter().map(|d| (d.model, d.mesh.local_bounds())),
        );
        let data = self.light_bytes(camera, lighting)?;
        let input: Vec<_> = order
            .iter()
            .map(|&index| Input {
                mesh: draws[index].mesh,
                model: draws[index].model,
                base: &materials[index].base,
                lit: Some(&materials[index]),
                normal: None,
                receives_shadows: false,
                blend: Some(draws[index].blend),
            })
            .collect();
        self.core.prepare(
            gpu,
            target,
            Camera3D {
                view_projection: camera.view_projection,
                light_direction: [0., 1., 0.],
            },
            &input,
            options.into().pass,
            Some(&data),
            None,
        )
    }
}

/// Display mapping from HDR scene values to `[0, 1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tonemap {
    /// Clamp only; for LDR scenes and tests.
    None,
    /// `x / (1 + x)`.
    Reinhard,
    /// Fitted ACES filmic curve.
    #[default]
    Aces,
}

/// How the composite writes its output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputEncoding {
    /// sRGB in the shader for UNORM targets; linear for `*Srgb` formats
    /// (the hardware encodes) and float targets.
    #[default]
    Auto,
    /// Always write linear values.
    Linear,
    /// Always encode sRGB in the shader.
    Srgb,
}

/// Bright-pass bloom over a mip chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomSettings {
    pub enabled: bool,
    /// Scene brightness (after exposure) where bloom starts.
    pub threshold: f32,
    /// Soft-knee width as a fraction of `threshold` (0 = hard cut).
    pub knee: f32,
    /// Strength of the bloom added back to the scene.
    pub intensity: f32,
    /// Mip levels (1..=8); more levels spread the glow further.
    pub levels: u32,
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold: 1.0,
            knee: 0.5,
            intensity: 0.6,
            levels: 5,
        }
    }
}

/// Darkens the image towards its corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VignetteSettings {
    /// 0 = off, 1 = black corners.
    pub strength: f32,
    /// Normalised distance from the centre where darkening starts.
    pub radius: f32,
    /// Distance over which it reaches full strength.
    pub softness: f32,
}

impl Default for VignetteSettings {
    fn default() -> Self {
        Self {
            strength: 0.0,
            radius: 0.55,
            softness: 0.45,
        }
    }
}

/// Simple display-space colour grade, applied after tonemapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorGrade {
    /// 0 = greyscale, 1 = unchanged.
    pub saturation: f32,
    /// Contrast around 18% grey; 1 = unchanged.
    pub contrast: f32,
    /// Per-channel multiplier (tint).
    pub gain: [f32; 3],
    /// Per-channel offset added to shadows.
    pub lift: [f32; 3],
}

impl Default for ColorGrade {
    fn default() -> Self {
        Self {
            saturation: 1.0,
            contrast: 1.0,
            gain: [1.0; 3],
            lift: [0.0; 3],
        }
    }
}

/// Settings for one [`PostChain`] frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PostSettings {
    /// Linear multiplier before bloom and tonemapping (1 = unchanged). The
    /// default 0 means 1; set any positive value explicitly.
    pub exposure: f32,
    pub tonemap: Tonemap,
    pub bloom: BloomSettings,
    pub vignette: VignetteSettings,
    pub grade: ColorGrade,
    pub output: OutputEncoding,
}

impl PostSettings {
    /// No bloom, tonemap, grade or vignette: the composite only encodes. Use
    /// it to present an LDR linear scene unchanged.
    pub fn passthrough() -> Self {
        Self {
            exposure: 1.0,
            tonemap: Tonemap::None,
            bloom: BloomSettings {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        }
    }
    fn exposure(&self) -> f32 {
        if self.exposure == 0.0 {
            1.0
        } else {
            self.exposure
        }
    }
    /// Checks that every value is finite and in range.
    pub fn validate(&self) -> GraphicsResult<()> {
        let finite = |values: &[f32]| values.iter().all(|v| v.is_finite());
        let ok = finite(&[
            self.exposure,
            self.bloom.threshold,
            self.bloom.knee,
            self.bloom.intensity,
            self.vignette.strength,
            self.vignette.radius,
            self.vignette.softness,
            self.grade.saturation,
            self.grade.contrast,
        ]) && finite(&self.grade.gain)
            && finite(&self.grade.lift)
            && self.exposure >= 0.0
            && self.bloom.threshold >= 0.0
            && (0.0..=1.0).contains(&self.bloom.knee)
            && self.bloom.intensity >= 0.0
            && (1..=8).contains(&self.bloom.levels)
            && (0.0..=1.0).contains(&self.vignette.strength)
            && self.vignette.radius >= 0.0
            && self.vignette.softness >= 0.0
            && self.grade.saturation >= 0.0
            && self.grade.contrast >= 0.0
            && self.grade.gain.iter().all(|v| *v >= 0.0);
        if ok {
            Ok(())
        } else {
            Err(error("invalid post-processing settings"))
        }
    }
    fn encode_in_shader(&self, format: TextureFormat) -> bool {
        match self.output {
            OutputEncoding::Linear => false,
            OutputEncoding::Srgb => true,
            OutputEncoding::Auto => {
                !format.is_srgb()
                    && matches!(
                        format,
                        TextureFormat::Rgba8Unorm
                            | TextureFormat::Bgra8Unorm
                            | TextureFormat::Rgb10a2Unorm
                    )
            }
        }
    }
    fn params(&self, texel: [f32; 2], format: TextureFormat, bloom: bool) -> [f32; 20] {
        let tonemap = match self.tonemap {
            Tonemap::None => 0.0,
            Tonemap::Reinhard => 1.0,
            Tonemap::Aces => 2.0,
        };
        [
            texel[0],
            texel[1],
            self.bloom.threshold,
            self.bloom.knee,
            self.exposure(),
            self.bloom.intensity,
            tonemap,
            f32::from(self.encode_in_shader(format)),
            self.grade.saturation,
            self.grade.contrast,
            self.vignette.strength,
            self.vignette.radius,
            self.grade.gain[0],
            self.grade.gain[1],
            self.grade.gain[2],
            self.vignette.softness,
            self.grade.lift[0],
            self.grade.lift[1],
            self.grade.lift[2],
            f32::from(bloom),
        ]
    }
}

const PARAM_BYTES: u64 = 80;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PostPipelineKey {
    entry: &'static str,
    format: TextureFormat,
    additive: bool,
}

struct BloomLevel {
    _texture: Texture,
    view: TextureView,
    size: (u32, u32),
}

/// Bloom + tonemap + grade + vignette + single sRGB encode. See the
/// module docs.
pub struct PostChain {
    module: ShaderModule,
    layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    sampler: Sampler,
    black: TextureView,
    pipelines: HashMap<PostPipelineKey, RenderPipeline>,
    levels: Vec<BloomLevel>,
    level_key: ((u32, u32), u32),
    params: Option<Buffer>,
    slot: u64,
    groups: HashMap<(TextureView, TextureView), BindGroup>,
}

struct PostPass {
    entry: &'static str,
    source: TextureView,
    bloom: TextureView,
    target: TextureView,
    format: TextureFormat,
    additive: bool,
    clear: bool,
    params: [f32; 20],
}

impl PostChain {
    pub fn new(gpu: &Gpu) -> GraphicsResult<Self> {
        let module = checked(gpu, || {
            gpu.device.create_shader_module(ShaderModuleDescriptor {
                label: Some("Quasar post chain"),
                source: ShaderSource::Wgsl(include_str!("post.wgsl").into()),
            })
        })?;
        let texture = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("post chain"),
                entries: &[
                    texture(0),
                    BindGroupLayoutEntry {
                        binding: 1,
                        visibility: ShaderStages::FRAGMENT,
                        ty: BindingType::Sampler(SamplerBindingType::Filtering),
                        count: None,
                    },
                    BindGroupLayoutEntry {
                        binding: 2,
                        visibility: ShaderStages::FRAGMENT,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: BufferSize::new(PARAM_BYTES),
                        },
                        count: None,
                    },
                    texture(3),
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("post chain"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let sampler = gpu.device.create_sampler(&SamplerDescriptor {
            label: Some("post chain linear clamp"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        let black = texture_2d(
            &gpu.device,
            "post chain black",
            1,
            1,
            SCENE_FORMAT,
            TextureUsages::TEXTURE_BINDING,
        )?
        .create_view(&Default::default());
        Ok(Self {
            module,
            layout,
            pipeline_layout,
            sampler,
            black,
            pipelines: HashMap::new(),
            levels: Vec::new(),
            level_key: ((0, 0), 0),
            params: None,
            slot: 256,
            groups: HashMap::new(),
        })
    }

    /// The final (largest) bloom level from the last frame that used bloom.
    pub fn bloom_view(&self) -> Option<&TextureView> {
        self.levels.first().map(|level| &level.view)
    }

    fn pipeline(&mut self, gpu: &Gpu, key: PostPipelineKey) -> GraphicsResult<()> {
        if self.pipelines.contains_key(&key) {
            return Ok(());
        }
        let blend = key.additive.then_some(BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent::REPLACE,
        });
        let pipeline = checked(gpu, || {
            gpu.device
                .create_render_pipeline(&RenderPipelineDescriptor {
                    label: Some(key.entry),
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
                        entry_point: Some(key.entry),
                        compilation_options: Default::default(),
                        targets: &[Some(ColorTargetState {
                            format: key.format,
                            blend,
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

    fn ensure_levels(&mut self, gpu: &Gpu, size: (u32, u32), count: u32) -> GraphicsResult<()> {
        if self.level_key == (size, count) && !self.levels.is_empty() {
            return Ok(());
        }
        self.levels.clear();
        self.groups.clear();
        let (mut width, mut height) = size;
        for _ in 0..count {
            width = (width / 2).max(1);
            height = (height / 2).max(1);
            let texture = texture_2d(
                &gpu.device,
                "bloom level",
                width,
                height,
                SCENE_FORMAT,
                TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            )?;
            let view = texture.create_view(&Default::default());
            self.levels.push(BloomLevel {
                _texture: texture,
                view,
                size: (width, height),
            });
        }
        self.level_key = (size, count);
        Ok(())
    }

    /// Records bloom and composite passes into `encoder`, reading `scene`
    /// (`size` pixels) and writing `target.color`.
    pub fn record(
        &mut self,
        gpu: &Gpu,
        encoder: &mut CommandEncoder,
        scene: &TextureView,
        size: (u32, u32),
        target: &RenderTarget,
        settings: &PostSettings,
    ) -> GraphicsResult<()> {
        settings.validate()?;
        if size.0 == 0 || size.1 == 0 || target.width == 0 || target.height == 0 {
            return Err(error("post chain sizes must be nonzero"));
        }
        let texel = |(w, h): (u32, u32)| [1.0 / w as f32, 1.0 / h as f32];
        let mut passes = Vec::new();
        let bloom = settings.bloom.enabled && settings.bloom.intensity > 0.0;
        if bloom {
            self.ensure_levels(gpu, size, settings.bloom.levels)?;
            let hdr = |params| params;
            passes.push(PostPass {
                entry: "prefilter",
                source: scene.clone(),
                bloom: self.black.clone(),
                target: self.levels[0].view.clone(),
                format: SCENE_FORMAT,
                additive: false,
                clear: true,
                params: hdr(settings.params(texel(size), SCENE_FORMAT, true)),
            });
            for index in 1..self.levels.len() {
                passes.push(PostPass {
                    entry: "downsample",
                    source: self.levels[index - 1].view.clone(),
                    bloom: self.black.clone(),
                    target: self.levels[index].view.clone(),
                    format: SCENE_FORMAT,
                    additive: false,
                    clear: true,
                    params: settings.params(texel(self.levels[index - 1].size), SCENE_FORMAT, true),
                });
            }
            for index in (1..self.levels.len()).rev() {
                passes.push(PostPass {
                    entry: "upsample",
                    source: self.levels[index].view.clone(),
                    bloom: self.black.clone(),
                    target: self.levels[index - 1].view.clone(),
                    format: SCENE_FORMAT,
                    additive: true,
                    clear: false,
                    params: settings.params(texel(self.levels[index].size), SCENE_FORMAT, true),
                });
            }
        }
        passes.push(PostPass {
            entry: "composite",
            source: scene.clone(),
            bloom: if bloom {
                self.levels[0].view.clone()
            } else {
                self.black.clone()
            },
            target: target.color.clone(),
            format: target.format,
            additive: false,
            clear: true,
            params: settings.params(texel(size), target.format, bloom),
        });
        let alignment = u64::from(gpu.device.limits().min_uniform_buffer_offset_alignment).max(1);
        self.slot = PARAM_BYTES.div_ceil(alignment) * alignment;
        let total = self.slot * passes.len() as u64;
        let grown = quasar_frame::grow_buffer(
            gpu,
            &mut self.params,
            "post chain parameters",
            total,
            BufferUsages::UNIFORM,
        )?;
        if grown {
            self.groups.clear();
        }
        let mut data = vec![0u8; total as usize];
        for (index, pass) in passes.iter().enumerate() {
            let start = index * self.slot as usize;
            data[start..start + PARAM_BYTES as usize].copy_from_slice(&bytes(pass.params));
        }
        let upload = quasar_frame::staging_buffer(gpu, "post chain upload", &data);
        let params = self.params.clone().expect("grown above");
        encoder.copy_buffer_to_buffer(&upload, 0, &params, 0, total);
        if self.groups.len() > 64 {
            self.groups.clear();
        }
        for (index, pass) in passes.iter().enumerate() {
            let key = PostPipelineKey {
                entry: pass.entry,
                format: pass.format,
                additive: pass.additive,
            };
            self.pipeline(gpu, key)?;
            let group_key = (pass.source.clone(), pass.bloom.clone());
            if !self.groups.contains_key(&group_key) {
                let group = gpu.device.create_bind_group(&BindGroupDescriptor {
                    label: Some("post chain pass"),
                    layout: &self.layout,
                    entries: &[
                        BindGroupEntry {
                            binding: 0,
                            resource: BindingResource::TextureView(&pass.source),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: BindingResource::Sampler(&self.sampler),
                        },
                        BindGroupEntry {
                            binding: 2,
                            resource: BindingResource::Buffer(BufferBinding {
                                buffer: &params,
                                offset: 0,
                                size: BufferSize::new(PARAM_BYTES),
                            }),
                        },
                        BindGroupEntry {
                            binding: 3,
                            resource: BindingResource::TextureView(&pass.bloom),
                        },
                    ],
                });
                self.groups.insert(group_key.clone(), group);
            }
            let mut render = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some(pass.entry),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &pass.target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: if pass.clear {
                            LoadOp::Clear(Color::BLACK)
                        } else {
                            LoadOp::Load
                        },
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            render.set_pipeline(&self.pipelines[&key]);
            render.set_bind_group(
                0,
                &self.groups[&group_key],
                &[(index as u64 * self.slot) as u32],
            );
            render.draw(0..3, 0..1);
        }
        Ok(())
    }

    /// Records the chain into a new command buffer for the caller to submit
    /// after the scene's passes.
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        scene: &TextureView,
        size: (u32, u32),
        target: &RenderTarget,
        settings: &PostSettings,
    ) -> GraphicsResult<CommandBuffer> {
        checked(gpu, || {
            let mut encoder = gpu.encoder("post chain");
            self.record(gpu, &mut encoder, scene, size, target, settings)?;
            Ok(encoder.finish())
        })?
    }

    /// [`Self::prepare`] for a [`SceneTarget`].
    pub fn prepare_scene(
        &mut self,
        gpu: &Gpu,
        scene: &SceneTarget,
        target: &RenderTarget,
        settings: &PostSettings,
    ) -> GraphicsResult<CommandBuffer> {
        self.prepare(gpu, scene.color_view(), scene.size(), target, settings)
    }

    /// Drops cached GPU objects; the chain can still be used afterwards.
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.levels.clear();
        self.groups.clear();
        self.params = None;
        self.level_key = ((0, 0), 0);
        Ok(())
    }
}
