use crate::{
    Canvas, DisplayOptions, Filter, PixelState, Rect, Sprite, TextureId, Textures, Viewport,
};
use quasar_frame::{
    CompletionPolicy, FrameReport, FrameSurface, Gpu, GraphicsResult, OffscreenTarget,
    RenderTarget, error, texture_2d,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use vulkanic_gal as gpu;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentOutcome {
    Presented,
    Deferred,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicTexture {
    id: TextureId,
    size: (u32, u32),
}
impl DynamicTexture {
    pub fn id(self) -> TextureId {
        self.id
    }
    pub fn size(self) -> (u32, u32) {
        self.size
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOp {
    Clear,
    Load,
}
#[derive(Debug, Default, Clone, Copy)]
pub struct RenderStats {
    pub quads: usize,
    pub batches: usize,
    pub geometry_upload_bytes: usize,
    pub texture_upload_bytes: usize,
}
/// A native, single-use command buffer. Upstream wgpu retains its referenced resources.
pub struct Prepared2D {
    pub commands: gpu::CommandBuffer,
    pub stats: RenderStats,
}
struct TextureGpu {
    view: gpu::TextureView,
    dynamic: bool,
    initialized: bool,
}
// Reference and alpha/index effects are dynamic draw data, not pipeline state.
fn pipeline_pixels(p: PixelState) -> PixelState {
    PixelState {
        read_mask: p.read_mask,
        write_mask: p.write_mask,
        color_write: p.color_write,
        ..Default::default()
    }
}
struct Batch {
    texture: TextureId,
    palette: TextureId,
    pixels: PixelState,
    linear: bool,
    first: u32,
    count: u32,
}
pub struct RenderState2D {
    layout: gpu::PipelineLayout,
    geometry_layout: gpu::BindGroupLayout,
    image_layout: gpu::BindGroupLayout,
    shader: gpu::ShaderModule,
    pipelines: HashMap<(gpu::TextureFormat, bool, PixelState), gpu::RenderPipeline>,
    images: BTreeMap<TextureId, TextureGpu>,
    pending: BTreeMap<TextureId, Vec<u8>>,
    bindings: HashMap<(gpu::TextureView, gpu::TextureView, bool), gpu::BindGroup>,
    samplers: [gpu::Sampler; 2],
    masks: HashMap<(u32, u32), gpu::Texture>,
    stats: RenderStats,
    geometry: Option<gpu::Buffer>,
    viewport_buffer: Option<gpu::Buffer>,
    geometry_group: Option<gpu::BindGroup>,
    released: bool,
}
impl RenderState2D {
    pub fn new(gpu: &Gpu) -> GraphicsResult<Self> {
        let device = &gpu.device;
        if device.limits().max_storage_buffers_per_shader_stage < 1
            || device.limits().max_bind_groups < 2
        {
            return Err(error(
                "2D rendering requires vertex storage buffers and two bind groups",
            ));
        }
        let geometry_layout = device.create_bind_group_layout(&gpu::BindGroupLayoutDescriptor {
            label: Some("2D geometry"),
            entries: &[
                gpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: gpu::ShaderStages::VERTEX,
                    ty: gpu::BindingType::Buffer {
                        ty: gpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                gpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: gpu::ShaderStages::VERTEX,
                    ty: gpu::BindingType::Buffer {
                        ty: gpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let image_layout = device.create_bind_group_layout(&gpu::BindGroupLayoutDescriptor {
            label: Some("2D images"),
            entries: &[
                gpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: gpu::ShaderStages::FRAGMENT,
                    ty: gpu::BindingType::Texture {
                        sample_type: gpu::TextureSampleType::Float { filterable: true },
                        view_dimension: gpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                gpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: gpu::ShaderStages::FRAGMENT,
                    ty: gpu::BindingType::Sampler(gpu::SamplerBindingType::Filtering),
                    count: None,
                },
                gpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: gpu::ShaderStages::FRAGMENT,
                    ty: gpu::BindingType::Texture {
                        sample_type: gpu::TextureSampleType::Float { filterable: true },
                        view_dimension: gpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&gpu::PipelineLayoutDescriptor {
            label: Some("2D"),
            bind_group_layouts: &[Some(&geometry_layout), Some(&image_layout)],
            immediate_size: 0,
        });
        let shader = quasar_frame::validation_scope(gpu, || {
            device.create_shader_module(gpu::ShaderModuleDescriptor {
                label: Some("2D WGSL"),
                source: gpu::ShaderSource::Wgsl(include_str!("quad.wgsl").into()),
            })
        })?;
        let samplers = [gpu::FilterMode::Nearest, gpu::FilterMode::Linear].map(|filter| {
            device.create_sampler(&gpu::SamplerDescriptor {
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        });
        Ok(Self {
            layout,
            geometry_layout,
            image_layout,
            shader,
            samplers,
            pipelines: HashMap::new(),
            images: BTreeMap::new(),
            pending: BTreeMap::new(),
            bindings: HashMap::new(),
            masks: HashMap::new(),
            stats: RenderStats::default(),
            geometry: None,
            viewport_buffer: None,
            geometry_group: None,
            released: false,
        })
    }
    pub fn stats(&self) -> RenderStats {
        self.stats
    }
    pub fn dynamic_texture_count(&self) -> usize {
        self.images.values().filter(|t| t.dynamic).count()
    }
    /// The native view retains the caller's texture; no resource copy or second device.
    pub fn register_texture(
        &mut self,
        _gpu: &Gpu,
        assets: &mut Textures,
        view: &gpu::TextureView,
    ) -> GraphicsResult<TextureId> {
        if self.released {
            return Err(error("2D renderer released"));
        }
        let texture = view.texture();
        if texture.dimension() != gpu::TextureDimension::D2
            || texture.depth_or_array_layers() != 1
            || !texture
                .usage()
                .contains(gpu::TextureUsages::TEXTURE_BINDING)
            || !matches!(
                texture.format(),
                gpu::TextureFormat::Rgba8Unorm
                    | gpu::TextureFormat::Bgra8Unorm
                    | gpu::TextureFormat::Rgba8UnormSrgb
                    | gpu::TextureFormat::Bgra8UnormSrgb
            )
        {
            return Err(error(
                "shared image requires a sampled single-layer RGBA8/BGRA8 texture",
            ));
        }
        quasar_frame::validation_scope(_gpu, || {
            _gpu.device.create_bind_group(&gpu::BindGroupDescriptor {
                label: Some("validate shared image"),
                layout: &self.image_layout,
                entries: &[
                    gpu::BindGroupEntry {
                        binding: 0,
                        resource: gpu::BindingResource::TextureView(view),
                    },
                    gpu::BindGroupEntry {
                        binding: 1,
                        resource: gpu::BindingResource::Sampler(&self.samplers[0]),
                    },
                    gpu::BindGroupEntry {
                        binding: 2,
                        resource: gpu::BindingResource::TextureView(view),
                    },
                ],
            })
        })?;
        let id = assets
            .declare_external(texture.width(), texture.height())
            .map_err(error)?;
        self.images.insert(
            id,
            TextureGpu {
                view: view.clone(),
                dynamic: false,
                initialized: true,
            },
        );
        Ok(id)
    }
    pub fn create_dynamic_texture(
        &mut self,
        gpu: &Gpu,
        assets: &mut Textures,
        width: u32,
        height: u32,
    ) -> GraphicsResult<DynamicTexture> {
        if self.released {
            return Err(error("2D renderer released"));
        }
        let texture = texture_2d(
            &gpu.device,
            "2D dynamic",
            width,
            height,
            gpu::TextureFormat::Rgba8Unorm,
            gpu::TextureUsages::TEXTURE_BINDING | gpu::TextureUsages::COPY_DST,
        )?;
        let id = assets.declare_external(width, height).map_err(error)?;
        self.images.insert(
            id,
            TextureGpu {
                view: texture.create_view(&Default::default()),
                dynamic: true,
                initialized: false,
            },
        );
        Ok(DynamicTexture {
            id,
            size: (width, height),
        })
    }
    pub fn update_dynamic_texture(
        &mut self,
        texture: DynamicTexture,
        rgba: &[u8],
    ) -> GraphicsResult<()> {
        self.update_dynamic_texture_owned(texture, rgba.to_vec())
    }
    pub fn update_dynamic_texture_owned(
        &mut self,
        texture: DynamicTexture,
        rgba: Vec<u8>,
    ) -> GraphicsResult<()> {
        if !self.images.get(&texture.id).is_some_and(|t| t.dynamic)
            || rgba.len() as u64 != u64::from(texture.size.0) * u64::from(texture.size.1) * 4
        {
            return Err(error("invalid dynamic texture or pixel length"));
        }
        self.pending.insert(texture.id, rgba);
        Ok(())
    }
    pub fn destroy_dynamic_texture(
        &mut self,
        _gpu: &Gpu,
        assets: &mut Textures,
        texture: DynamicTexture,
    ) -> GraphicsResult<()> {
        if !self.images.get(&texture.id).is_some_and(|t| t.dynamic) {
            return Err(error("unknown dynamic texture"));
        }
        self.images.remove(&texture.id);
        self.pending.remove(&texture.id);
        assets.remove(texture.id);
        self.bindings.clear();
        Ok(())
    }
    fn image(&mut self, gpu: &Gpu, assets: &Textures, id: TextureId) -> GraphicsResult<()> {
        if let Some(pixels) = self.pending.remove(&id) {
            // Immutable generations preserve queued and abandoned prepared frames.
            let (width, height) = assets
                .dimensions(id)
                .ok_or_else(|| error("missing dynamic texture metadata"))?;
            let texture = texture_2d(
                &gpu.device,
                "2D dynamic generation",
                width,
                height,
                gpu::TextureFormat::Rgba8Unorm,
                gpu::TextureUsages::TEXTURE_BINDING | gpu::TextureUsages::COPY_DST,
            )?;
            gpu.queue.write_texture(
                texture.as_image_copy(),
                &pixels,
                gpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                texture.size(),
            );
            self.stats.texture_upload_bytes += pixels.len();
            self.images.insert(
                id,
                TextureGpu {
                    view: texture.create_view(&Default::default()),
                    dynamic: true,
                    initialized: true,
                },
            );
            self.bindings.clear();
        }
        if let Some(texture) = self.images.get(&id) {
            return if texture.initialized {
                Ok(())
            } else {
                Err(error("dynamic texture must be updated before drawing"))
            };
        }
        let image = assets
            .get(id)
            .ok_or_else(|| error("missing texture or unregistered external image"))?;
        let texture = texture_2d(
            &gpu.device,
            "2D immutable image",
            image.width(),
            image.height(),
            gpu::TextureFormat::Rgba8Unorm,
            gpu::TextureUsages::TEXTURE_BINDING | gpu::TextureUsages::COPY_DST,
        )?;
        gpu.queue.write_texture(
            texture.as_image_copy(),
            image.rgba(),
            gpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width() * 4),
                rows_per_image: Some(image.height()),
            },
            texture.size(),
        );
        self.stats.texture_upload_bytes += image.rgba().len();
        self.images.insert(
            id,
            TextureGpu {
                view: texture.create_view(&Default::default()),
                dynamic: false,
                initialized: true,
            },
        );
        Ok(())
    }
    fn pipeline(
        &mut self,
        gpu: &Gpu,
        format: gpu::TextureFormat,
        masked: bool,
        pixels: PixelState,
    ) -> GraphicsResult<()> {
        let pixels = pipeline_pixels(pixels);
        let key = (format, masked, pixels);
        if self.pipelines.contains_key(&key) {
            return Ok(());
        }
        if self.pipelines.len() >= 256 {
            return Err(error(
                "2D pipeline state budget exceeded in one preparation",
            ));
        }
        let face = gpu::StencilFaceState {
            compare: if pixels.read_mask != 0 {
                gpu::CompareFunction::Equal
            } else {
                gpu::CompareFunction::Always
            },
            pass_op: if pixels.write_mask != 0 {
                gpu::StencilOperation::Replace
            } else {
                gpu::StencilOperation::Keep
            },
            ..Default::default()
        };
        let pipeline = quasar_frame::validation_scope(gpu, || {
            gpu.device
                .create_render_pipeline(&gpu::RenderPipelineDescriptor {
                    label: Some("2D pipeline"),
                    layout: Some(&self.layout),
                    vertex: gpu::VertexState {
                        module: &self.shader,
                        entry_point: Some("vertex"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: gpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: masked.then_some(gpu::DepthStencilState {
                        format: gpu::TextureFormat::Depth24PlusStencil8,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(gpu::CompareFunction::Always),
                        stencil: gpu::StencilState {
                            front: face,
                            back: face,
                            read_mask: u32::from(pixels.read_mask),
                            write_mask: u32::from(pixels.write_mask),
                        },
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(gpu::FragmentState {
                        module: &self.shader,
                        entry_point: Some("fragment"),
                        compilation_options: Default::default(),
                        targets: &[Some(gpu::ColorTargetState {
                            format,
                            blend: Some(gpu::BlendState::ALPHA_BLENDING),
                            write_mask: if pixels.color_write {
                                gpu::ColorWrites::ALL
                            } else {
                                gpu::ColorWrites::empty()
                            },
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
        })?;
        self.pipelines.insert(key, pipeline);
        Ok(())
    }
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        canvas: &Canvas,
        assets: &Textures,
        target: &RenderTarget,
        options: DisplayOptions,
        load: LoadOp,
    ) -> GraphicsResult<Prepared2D> {
        quasar_frame::validation_scope(gpu, || {
            self.prepare_inner(gpu, canvas, assets, target, options, load)
        })?
    }
    fn prepare_inner(
        &mut self,
        gpu: &Gpu,
        canvas: &Canvas,
        assets: &Textures,
        target: &RenderTarget,
        options: DisplayOptions,
        load: LoadOp,
    ) -> GraphicsResult<Prepared2D> {
        if self.released {
            return Err(error("2D renderer released"));
        }
        options.validate().map_err(error)?;
        if canvas.quads.len() > 1_000_000
            || canvas.quads.len().max(1) as u64 * 80
                > gpu.device.limits().max_storage_buffer_binding_size
        {
            return Err(error("2D quad budget exceeded"));
        }
        let viewport = Viewport::calculate((canvas.width, canvas.height), target.size(), options)
            .ok_or_else(|| error("invalid 2D viewport"))?;
        for q in &canvas.quads {
            if assets.dimensions(q.texture).is_none()
                || q.palette
                    .is_some_and(|p| assets.dimensions(p.texture) != Some((256, 1)))
            {
                return Err(error("missing image or invalid 256x1 palette"));
            }
            if q.origin
                .iter()
                .chain(&q.x)
                .chain(&q.y)
                .any(|f| !f.is_finite())
            {
                return Err(error("nonfinite quad geometry"));
            }
        }
        self.images.retain(|id, _| assets.dimensions(*id).is_some());
        #[allow(
            clippy::mutable_key_type,
            reason = "Native handle equality/hash use immutable resource identity."
        )]
        let active: std::collections::HashSet<_> =
            self.images.values().map(|t| t.view.clone()).collect();
        self.bindings
            .retain(|(image, palette, _), _| active.contains(image) && active.contains(palette));
        self.stats = RenderStats {
            quads: canvas.quads.len(),
            ..Default::default()
        };
        let mut data = Vec::with_capacity(canvas.quads.len() * 80);
        let mut batches: Vec<Batch> = Vec::new();
        for (index, q) in canvas.quads.iter().enumerate() {
            let image = assets.dimensions(q.texture).unwrap();
            let source = q
                .source
                .unwrap_or(Rect::new(0., 0., image.0 as f32, image.1 as f32));
            let flags = u32::from(q.palette.is_some())
                | (q.palette
                    .and_then(|p| p.transparent_index)
                    .map_or(0, |i| u32::from(i) + 1)
                    << 1)
                | (q.pixels.exclude_index.map_or(0, |i| u32::from(i) + 1) << 10);
            let values = [
                q.origin[0],
                q.origin[1],
                q.x[0],
                q.x[1],
                q.y[0],
                q.y[1],
                f32::from_bits(flags),
                f32::from_bits(u32::from(q.pixels.alpha_min)),
                source.x / image.0 as f32,
                source.y / image.1 as f32,
                (source.x + source.width) / image.0 as f32,
                (source.y + source.height) / image.1 as f32,
                q.color.0[0] as f32 / 255.,
                q.color.0[1] as f32 / 255.,
                q.color.0[2] as f32 / 255.,
                q.color.0[3] as f32 / 255.,
                q.clip.x,
                q.clip.y,
                q.clip.x + q.clip.width,
                q.clip.y + q.clip.height,
            ];
            data.extend(values.into_iter().flat_map(f32::to_ne_bytes));
            let palette = q.palette.map_or(q.texture, |p| p.texture);
            let linear =
                q.palette.is_none() && q.filter.unwrap_or(options.filter) == Filter::Linear;
            if let Some(b) = batches.last_mut()
                && b.texture == q.texture
                && b.palette == palette
                && b.pixels == q.pixels
                && b.linear == linear
            {
                b.count += 1;
            } else {
                batches.push(Batch {
                    texture: q.texture,
                    palette,
                    pixels: q.pixels,
                    linear,
                    first: index as u32,
                    count: 1,
                });
            }
        }
        self.stats.geometry_upload_bytes = data.len();
        self.stats.batches = batches.len();
        let masked = batches
            .iter()
            .any(|b| b.pixels.read_mask != 0 || b.pixels.write_mask != 0);
        let required: std::collections::HashSet<_> = batches
            .iter()
            .map(|b| (target.format, masked, pipeline_pixels(b.pixels)))
            .collect();
        if required.len() > 256 {
            return Err(error(
                "2D pipeline state budget exceeded in one preparation",
            ));
        }
        let missing = required
            .iter()
            .filter(|key| !self.pipelines.contains_key(key))
            .count();
        if self.pipelines.len() + missing > 256 {
            self.pipelines.retain(|key, _| required.contains(key));
        }
        for b in &batches {
            self.image(gpu, assets, b.texture)?;
            self.image(gpu, assets, b.palette)?;
            self.pipeline(gpu, target.format, masked, b.pixels)?;
        }
        for b in &batches {
            let a = &self.images[&b.texture].view;
            let p = &self.images[&b.palette].view;
            let key = (a.clone(), p.clone(), b.linear);
            self.bindings.entry(key).or_insert_with(|| {
                gpu.device.create_bind_group(&gpu::BindGroupDescriptor {
                    label: Some("2D images"),
                    layout: &self.image_layout,
                    entries: &[
                        gpu::BindGroupEntry {
                            binding: 0,
                            resource: gpu::BindingResource::TextureView(a),
                        },
                        gpu::BindGroupEntry {
                            binding: 1,
                            resource: gpu::BindingResource::Sampler(
                                &self.samplers[usize::from(b.linear)],
                            ),
                        },
                        gpu::BindGroupEntry {
                            binding: 2,
                            resource: gpu::BindingResource::TextureView(p),
                        },
                    ],
                })
            });
        }
        if self.bindings.len() > 4096 {
            return Err(error("2D image binding budget exceeded"));
        }
        if data.is_empty() {
            data.resize(80, 0);
        }
        let uniform: [f32; 8] = [
            canvas.width as f32,
            canvas.height as f32,
            0.,
            0.,
            viewport.x as f32 / target.width as f32,
            viewport.y as f32 / target.height as f32,
            viewport.width as f32 / target.width as f32,
            viewport.height as f32 / target.height as f32,
        ];
        let frame_data: Vec<_> = uniform.into_iter().flat_map(f32::to_ne_bytes).collect();
        let geometry_size = data.len() as u64;
        let geometry_changed = quasar_frame::grow_buffer(
            gpu,
            &mut self.geometry,
            "2D quads",
            geometry_size,
            gpu::BufferUsages::STORAGE,
        )?;
        let viewport_changed = quasar_frame::grow_buffer(
            gpu,
            &mut self.viewport_buffer,
            "2D viewport",
            32,
            gpu::BufferUsages::UNIFORM,
        )?;
        data.extend_from_slice(&frame_data);
        let upload = quasar_frame::staging_buffer(gpu, "2D frame upload", &data);
        let mut encoder = gpu.encoder("2D drawing");
        encoder.copy_buffer_to_buffer(
            &upload,
            0,
            self.geometry.as_ref().unwrap(),
            0,
            geometry_size,
        );
        encoder.copy_buffer_to_buffer(
            &upload,
            geometry_size,
            self.viewport_buffer.as_ref().unwrap(),
            0,
            32,
        );
        if geometry_changed || viewport_changed || self.geometry_group.is_none() {
            self.geometry_group = Some(gpu.device.create_bind_group(&gpu::BindGroupDescriptor {
                label: Some("2D geometry"),
                layout: &self.geometry_layout,
                entries: &[
                    gpu::BindGroupEntry {
                        binding: 0,
                        resource: self.geometry.as_ref().unwrap().as_entire_binding(),
                    },
                    gpu::BindGroupEntry {
                        binding: 1,
                        resource: self.viewport_buffer.as_ref().unwrap().as_entire_binding(),
                    },
                ],
            }));
        }
        let geometry_group = self.geometry_group.as_ref().unwrap();
        let mask = if masked {
            if self.masks.len() >= 8 && !self.masks.contains_key(&target.size()) {
                self.masks.clear();
            }
            if let std::collections::hash_map::Entry::Vacant(e) = self.masks.entry(target.size()) {
                e.insert(texture_2d(
                    &gpu.device,
                    "2D stencil",
                    target.width,
                    target.height,
                    gpu::TextureFormat::Depth24PlusStencil8,
                    gpu::TextureUsages::RENDER_ATTACHMENT,
                )?);
            }
            Some(self.masks[&target.size()].create_view(&Default::default()))
        } else {
            None
        };
        {
            let clear = canvas.clear.0.map(|c| f64::from(c) / 255.);
            let color = [Some(gpu::RenderPassColorAttachment {
                view: &target.color,
                depth_slice: None,
                resolve_target: None,
                ops: gpu::Operations {
                    load: match load {
                        LoadOp::Clear => gpu::LoadOp::Clear(gpu::Color {
                            r: clear[0],
                            g: clear[1],
                            b: clear[2],
                            a: clear[3],
                        }),
                        LoadOp::Load => gpu::LoadOp::Load,
                    },
                    store: gpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&gpu::RenderPassDescriptor {
                label: Some("2D"),
                color_attachments: &color,
                depth_stencil_attachment: mask.as_ref().map(|view| {
                    gpu::RenderPassDepthStencilAttachment {
                        view,
                        depth_ops: None,
                        stencil_ops: Some(gpu::Operations {
                            load: gpu::LoadOp::Clear(0),
                            store: gpu::StoreOp::Discard,
                        }),
                    }
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, geometry_group, &[]);
            for b in &batches {
                pass.set_pipeline(
                    &self.pipelines[&(target.format, masked, pipeline_pixels(b.pixels))],
                );
                pass.set_stencil_reference(u32::from(b.pixels.reference));
                let key = (
                    self.images[&b.texture].view.clone(),
                    self.images[&b.palette].view.clone(),
                    b.linear,
                );
                pass.set_bind_group(1, &self.bindings[&key], &[]);
                pass.draw(0..6, b.first..b.first + b.count);
            }
        }
        Ok(Prepared2D {
            commands: encoder.finish(),
            stats: self.stats,
        })
    }
    pub fn release_targets(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.masks.clear();
        Ok(())
    }
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.geometry_group = None;
        self.geometry = None;
        self.viewport_buffer = None;
        self.released = true;
        self.images.clear();
        self.bindings.clear();
        self.pending.clear();
        self.masks.clear();
        self.pipelines.clear();
        Ok(())
    }
}

pub struct Renderer2D {
    pub gpu: Gpu,
    pub state: RenderState2D,
    surface: Option<FrameSurface>,
    options: DisplayOptions,
    overlap: bool,
    report: FrameReport,
    device_name: String,
}
impl Renderer2D {
    pub fn headless() -> GraphicsResult<Self> {
        Self::from_gpu(Gpu::headless()?)
    }
    pub fn from_gpu(gpu: Gpu) -> GraphicsResult<Self> {
        let state = RenderState2D::new(&gpu)?;
        let device_name = gpu.adapter.get_info().name;
        Ok(Self {
            gpu,
            state,
            surface: None,
            options: DisplayOptions::default(),
            overlap: false,
            report: FrameReport::default(),
            device_name,
        })
    }
    pub fn for_window<W>(
        window: Arc<W>,
        size: (u32, u32),
        options: DisplayOptions,
    ) -> GraphicsResult<Self>
    where
        W: gpu::DisplayAndWindowHandle + std::fmt::Debug + Send + Sync + 'static,
    {
        let (gpu, surface) = FrameSurface::for_window(window, size.0, size.1)?;
        let mut r = Self::from_gpu(gpu)?;
        r.set_options(options)?;
        r.surface = Some(surface);
        Ok(r)
    }
    pub fn options(&self) -> DisplayOptions {
        self.options
    }
    pub fn set_options(&mut self, options: DisplayOptions) -> GraphicsResult<()> {
        options.validate().map_err(error)?;
        self.options = options;
        Ok(())
    }
    pub fn set_frame_overlap(&mut self, enabled: bool) {
        self.overlap = enabled;
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(surface) = &mut self.surface {
            surface.resize(width, height);
        }
    }
    pub fn stats(&self) -> RenderStats {
        self.state.stats()
    }
    pub fn frame_report(&self) -> &FrameReport {
        &self.report
    }
    pub fn device_name(&self) -> &str {
        &self.device_name
    }
    pub fn backend_info(&self) -> gpu::AdapterInfo {
        self.gpu.adapter.get_info()
    }
    pub fn viewport(&self, canvas: &Canvas) -> Option<Viewport> {
        Viewport::calculate(
            (canvas.width, canvas.height),
            self.surface
                .as_ref()
                .map_or((canvas.width, canvas.height), FrameSurface::size),
            self.options,
        )
    }
    pub fn present(
        &mut self,
        canvas: &Canvas,
        textures: &Textures,
    ) -> GraphicsResult<PresentOutcome> {
        self.present_uploaded(canvas, textures, |_| Ok(None))
    }
    fn present_uploaded(
        &mut self,
        canvas: &Canvas,
        textures: &Textures,
        upload: impl FnOnce(&Gpu) -> GraphicsResult<Option<gpu::CommandBuffer>>,
    ) -> GraphicsResult<PresentOutcome> {
        let surface = self
            .surface
            .as_mut()
            .ok_or_else(|| error("renderer has no presentation surface"))?;
        let Some(frame) = surface.acquire(&self.gpu)? else {
            return Ok(PresentOutcome::Deferred);
        };
        let prepare_start = std::time::Instant::now();
        let mut commands: Vec<_> = upload(&self.gpu)?.into_iter().collect();
        let prepared = self.state.prepare(
            &self.gpu,
            canvas,
            textures,
            &frame.target,
            self.options,
            LoadOp::Clear,
        )?;
        commands.push(prepared.commands);
        let prepare_nanos = quasar_frame::nanos(prepare_start.elapsed());
        self.report = surface.present(
            &self.gpu,
            frame,
            commands,
            if self.overlap {
                CompletionPolicy::Poll
            } else {
                CompletionPolicy::Wait
            },
        )?;
        self.report.prepare_nanos = prepare_nanos;
        Ok(PresentOutcome::Presented)
    }
    pub fn render_to_image(
        &mut self,
        canvas: &Canvas,
        textures: &Textures,
    ) -> GraphicsResult<Vec<u8>> {
        self.image_uploaded(canvas, textures, |_| Ok(None))
    }
    fn image_uploaded(
        &mut self,
        canvas: &Canvas,
        textures: &Textures,
        upload: impl FnOnce(&Gpu) -> GraphicsResult<Option<gpu::CommandBuffer>>,
    ) -> GraphicsResult<Vec<u8>> {
        let target = OffscreenTarget::new(&self.gpu, canvas.width, canvas.height)?;
        let image = target.render(&self.gpu, |gpu, target| {
            let mut commands: Vec<_> = upload(gpu)?.into_iter().collect();
            commands.push(
                self.state
                    .prepare(gpu, canvas, textures, target, self.options, LoadOp::Clear)?
                    .commands,
            );
            Ok(commands)
        })?;
        self.report = image.profile;
        Ok(image.rgba)
    }
    pub fn dynamic_texture_count(&self) -> usize {
        self.state.dynamic_texture_count()
    }
    pub fn create_dynamic_texture(
        &mut self,
        assets: &mut Textures,
        width: u32,
        height: u32,
    ) -> GraphicsResult<DynamicTexture> {
        self.state
            .create_dynamic_texture(&self.gpu, assets, width, height)
    }
    pub fn update_dynamic_texture(&mut self, t: DynamicTexture, rgba: &[u8]) -> GraphicsResult<()> {
        self.state.update_dynamic_texture(t, rgba)
    }
    pub fn update_dynamic_texture_owned(
        &mut self,
        t: DynamicTexture,
        rgba: Vec<u8>,
    ) -> GraphicsResult<()> {
        self.state.update_dynamic_texture_owned(t, rgba)
    }
    pub fn destroy_dynamic_texture(
        &mut self,
        assets: &mut Textures,
        t: DynamicTexture,
    ) -> GraphicsResult<()> {
        self.state.destroy_dynamic_texture(&self.gpu, assets, t)
    }
}
pub struct FramebufferPresenter {
    pub renderer: Renderer2D,
    source: (u32, u32),
    textures: Textures,
    texture: gpu::Texture,
    canvas: Canvas,
}
impl FramebufferPresenter {
    pub fn new<W>(
        window: Arc<W>,
        size: (u32, u32),
        source: (u32, u32),
        options: DisplayOptions,
    ) -> GraphicsResult<Self>
    where
        W: gpu::DisplayAndWindowHandle + std::fmt::Debug + Send + Sync + 'static,
    {
        Self::from_renderer(Renderer2D::for_window(window, size, options)?, source)
    }
    pub fn from_renderer(mut renderer: Renderer2D, source: (u32, u32)) -> GraphicsResult<Self> {
        let texture = texture_2d(
            &renderer.gpu.device,
            "software framebuffer",
            source.0,
            source.1,
            gpu::TextureFormat::Rgba8Unorm,
            gpu::TextureUsages::TEXTURE_BINDING | gpu::TextureUsages::COPY_DST,
        )?;
        let mut textures = Textures::default();
        let view = texture.create_view(&Default::default());
        let id = renderer
            .state
            .register_texture(&renderer.gpu, &mut textures, &view)?;
        let mut canvas = Canvas::new(source.0, source.1).map_err(error)?;
        canvas.sprite(Sprite::new(
            id,
            Rect::new(0., 0., source.0 as f32, source.1 as f32),
        ));
        Ok(Self {
            renderer,
            source,
            textures,
            texture,
            canvas,
        })
    }
    fn validate_pixels(&self, pixels: &[u8]) -> GraphicsResult<()> {
        if pixels.len() as u64 != u64::from(self.source.0) * u64::from(self.source.1) * 4 {
            return Err(error("invalid framebuffer pixel length"));
        }
        Ok(())
    }
    fn upload(
        gpu: &Gpu,
        texture: &gpu::Texture,
        pixels: &[u8],
    ) -> GraphicsResult<Option<gpu::CommandBuffer>> {
        let mut encoder = gpu.encoder("software framebuffer upload");
        quasar_frame::upload_rgba8(gpu, &mut encoder, texture, pixels)?;
        Ok(Some(encoder.finish()))
    }
    pub fn present(&mut self, pixels: &[u8]) -> GraphicsResult<PresentOutcome> {
        self.validate_pixels(pixels)?;
        let outcome = self
            .renderer
            .present_uploaded(&self.canvas, &self.textures, |gpu| {
                Self::upload(gpu, &self.texture, pixels)
            })?;
        if outcome == PresentOutcome::Presented {
            self.renderer.state.stats.texture_upload_bytes += pixels.len();
        }
        Ok(outcome)
    }
    pub fn present_owned(&mut self, pixels: Vec<u8>) -> GraphicsResult<PresentOutcome> {
        self.present(&pixels)
    }
    pub fn render_to_image(&mut self, pixels: &[u8]) -> GraphicsResult<Vec<u8>> {
        self.validate_pixels(pixels)?;
        let image = self
            .renderer
            .image_uploaded(&self.canvas, &self.textures, |gpu| {
                Self::upload(gpu, &self.texture, pixels)
            })?;
        self.renderer.state.stats.texture_upload_bytes += pixels.len();
        Ok(image)
    }
    pub fn source_size(&self) -> (u32, u32) {
        self.source
    }
    pub fn resize(&mut self, w: u32, h: u32) {
        self.renderer.resize(w, h)
    }
    pub fn options(&self) -> DisplayOptions {
        self.renderer.options()
    }
    pub fn set_options(&mut self, o: DisplayOptions) -> GraphicsResult<()> {
        self.renderer.set_options(o)
    }
    pub fn viewport(&self) -> Option<Viewport> {
        self.renderer.viewport(&self.canvas)
    }
    pub fn set_frame_overlap(&mut self, v: bool) {
        self.renderer.set_frame_overlap(v)
    }
    pub fn stats(&self) -> RenderStats {
        self.renderer.stats()
    }
    pub fn device_name(&self) -> &str {
        self.renderer.device_name()
    }
    pub fn backend_info(&self) -> gpu::AdapterInfo {
        self.renderer.backend_info()
    }
    pub fn frame_report(&self) -> &FrameReport {
        self.renderer.frame_report()
    }
}
