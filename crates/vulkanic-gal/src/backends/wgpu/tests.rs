//! GPU tests of the wgpu backend. Each rendering test runs one GAL program
//! on wgpu and, when the Vulkan backend is compiled in and has a device, on
//! Vulkan too, and requires identical results: Vulkan is the reference for
//! orientation, winding, blending, depth, stencil and copy semantics.
//!
//! Like the other backends' GPU tests these report an environment gap and
//! pass when no adapter exists; set `VULKANIC_REQUIRE_GPU=1` to fail instead.

use super::WgpuBackend;
use crate::commands::*;
use crate::gal::VulkanicGal;
use crate::handles::Handle;
use crate::resources::*;

fn require_gpu() -> bool {
    std::env::var("VULKANIC_REQUIRE_GPU").is_ok_and(|value| value == "1")
}

fn wgpu_gal(label: &str) -> Option<VulkanicGal> {
    match WgpuBackend::new(label, crate::create::WgpuApi::Auto) {
        Ok(backend) => {
            eprintln!("wgpu adapter: {}", backend.adapter_name_for_test());
            Some(VulkanicGal::new_with_backend(Box::new(backend)))
        }
        Err(error) => {
            let message = error.to_string();
            assert!(
                !require_gpu() && message.contains("adapter"),
                "wgpu backend unavailable: {message}"
            );
            None
        }
    }
}

#[cfg(feature = "vulkan")]
fn vulkan_gal(label: &str) -> Option<VulkanicGal> {
    crate::test_support::vulkan_gal(label).ok()
}

#[cfg(not(feature = "vulkan"))]
fn vulkan_gal(_label: &str) -> Option<VulkanicGal> {
    None
}

/// Runs `program` on wgpu, and on Vulkan when available, asserting both
/// produce the same bytes within `tolerance` per byte. Returns wgpu's.
fn run_on_backends(
    name: &str,
    tolerance: u8,
    program: impl Fn(&mut VulkanicGal) -> Vec<u8>,
) -> Option<Vec<u8>> {
    // The backends share a process-wide lock, so run them one at a time.
    let wgpu = {
        let mut gal = wgpu_gal(name)?;
        program(&mut gal)
    };
    if let Some(mut gal) = vulkan_gal(name) {
        // Pinning wgpu to another adapter compares two GPUs, which may round
        // normalized values differently.
        let tolerance = if std::env::var_os("WGPU_ADAPTER_NAME").is_some() {
            tolerance.max(2)
        } else {
            tolerance
        };
        let vulkan = program(&mut gal);
        assert_eq!(vulkan.len(), wgpu.len(), "{name}: output sizes differ");
        let worst = vulkan
            .iter()
            .zip(&wgpu)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        assert!(
            worst <= tolerance,
            "{name}: wgpu differs from Vulkan by up to {worst}\n vulkan: {:?}\n   wgpu: {:?}",
            &vulkan[..vulkan.len().min(64)],
            &wgpu[..wgpu.len().min(64)]
        );
    } else {
        eprintln!("{name}: Vulkan reference unavailable; checked wgpu alone");
    }
    Some(wgpu)
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

fn mip_barrier(
    texture: Handle,
    base_mip: u32,
    mip_count: u32,
    before: TextureUsageState,
    after: TextureUsageState,
) -> CommandOp {
    CommandOp::Barrier(ResourceBarrier {
        resource: texture,
        subresources: Some(TextureSubresourceRange {
            base_mip,
            mip_count,
            base_layer: 0,
            layer_count: 1,
        }),
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    })
}

fn extent(width: u32, height: u32) -> Extent3d {
    Extent3d {
        width,
        height,
        depth: 1,
    }
}

fn binding(binding: u32, kind: ResourceBindingKind, dynamic: u32) -> ResourceBindingDesc {
    ResourceBindingDesc {
        binding,
        kind,
        stages: PipelineStageFlags::DRAW,
        array_count: 1,
        optional: false,
        dynamic_offset_count: dynamic,
    }
}

fn bound(binding: u32, kind: ResourceBindingKind, resource: Handle) -> ResourceBinding {
    ResourceBinding {
        binding,
        array_index: 0,
        resource,
        kind,
        access: AccessFlags::READ,
        dynamic_offsets: Vec::new(),
        buffer_range: None,
    }
}

const FULLSCREEN_VS: &str = "#version 450
layout(location=0) out vec2 uv;
void main() {
    vec2 p = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2));
    uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.5, 1.0);
}";

/// A GAL program builder that destroys what it created.
struct Kit<'a> {
    gal: &'a mut VulkanicGal,
    owned: Vec<Handle>,
}

impl<'a> Kit<'a> {
    fn new(gal: &'a mut VulkanicGal) -> Self {
        Self {
            gal,
            owned: Vec::new(),
        }
    }

    fn own(&mut self, handle: Handle) -> Handle {
        self.owned.push(handle);
        handle
    }

    fn buffer(&mut self, size: u64, memory: MemoryDomain, usages: &[BufferUsage]) -> Handle {
        let handle = self
            .gal
            .create_buffer(BufferDesc {
                label: "test-buffer".into(),
                size,
                memory,
                usages: usages.to_vec(),
            })
            .expect("buffer");
        self.own(handle)
    }

    fn texture(
        &mut self,
        size: Extent3d,
        format: TextureFormat,
        mips: u32,
        usages: &[TextureUsage],
    ) -> (Handle, Handle) {
        let texture = self
            .gal
            .create_texture(TextureDesc {
                label: "test-texture".into(),
                dimension: TextureDimension::D2,
                format,
                extent: size,
                mip_levels: mips,
                array_layers: 1,
                usages: usages.to_vec(),
            })
            .expect("texture");
        self.own(texture);
        let view = self.view(texture, format, 0, mips);
        (texture, view)
    }

    fn view(
        &mut self,
        texture: Handle,
        format: TextureFormat,
        base_mip: u32,
        mip_count: u32,
    ) -> Handle {
        let view = self
            .gal
            .create_texture_view(TextureViewDesc {
                label: "test-view".into(),
                texture,
                format,
                base_mip,
                mip_count,
                base_layer: 0,
                layer_count: 1,
            })
            .expect("view");
        self.own(view)
    }

    fn sampler(&mut self, filter: SamplerFilter, comparison: Option<CompareOp>) -> Handle {
        let sampler = self
            .gal
            .create_sampler(SamplerDesc {
                label: "test-sampler".into(),
                min_filter: filter,
                mag_filter: filter,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison,
            })
            .expect("sampler");
        self.own(sampler)
    }

    fn combined(&mut self, view: Handle, sampler: Handle) -> Handle {
        let combined = self
            .gal
            .create_combined_texture_sampler(CombinedTextureSamplerDesc {
                label: "test-combined".into(),
                texture_view: view,
                sampler,
            })
            .expect("combined sampler");
        self.own(combined)
    }

    fn shader(&mut self, stage: ShaderStage, source: &str) -> Handle {
        let shader = self
            .gal
            .create_shader_module(ShaderModuleDesc {
                label: format!("test-{stage:?}"),
                stage,
                code_format: ShaderCodeFormat::Glsl,
                code: source.as_bytes().to_vec(),
                entry_point: "main".into(),
            })
            .expect("shader");
        self.own(shader)
    }

    fn layout(&mut self, bindings: Vec<ResourceBindingDesc>) -> (Handle, Handle) {
        let layout = self
            .gal
            .create_resource_layout(ResourceLayoutDesc {
                label: "test-layout".into(),
                bindings,
            })
            .expect("resource layout");
        self.own(layout);
        let pipeline_layout = self
            .gal
            .create_pipeline_layout(PipelineLayoutDesc {
                label: "test-pipeline-layout".into(),
                resource_layouts: vec![layout],
            })
            .expect("pipeline layout");
        (layout, self.own(pipeline_layout))
    }

    fn set(&mut self, layout: Handle, bindings: Vec<ResourceBinding>) -> Handle {
        let set = self
            .gal
            .create_resource_set(ResourceSetDesc {
                label: "test-set".into(),
                layout,
                bindings,
            })
            .expect("resource set");
        self.own(set)
    }

    fn pipeline(&mut self, desc: GraphicsPipelineDesc) -> Handle {
        let pipeline = self
            .gal
            .create_graphics_pipeline(desc)
            .expect("graphics pipeline");
        self.own(pipeline)
    }

    fn target(&mut self, colors: &[Handle], depth: Option<Handle>, size: Extent3d) -> Handle {
        let target = self
            .gal
            .create_render_target(RenderTargetDesc {
                label: "test-target".into(),
                color_views: colors.to_vec(),
                depth_stencil_view: depth,
                extent: size,
            })
            .expect("render target");
        self.own(target)
    }

    fn pass(
        &mut self,
        target: Handle,
        colors: &[TextureFormat],
        depth: Option<TextureFormat>,
    ) -> Handle {
        let pass = self
            .gal
            .create_render_pass(RenderPassDesc {
                label: "test-pass".into(),
                target,
                color_formats: colors.to_vec(),
                depth_format: depth,
            })
            .expect("render pass");
        self.own(pass)
    }

    /// Submits `operations` followed by a host read of `readback`, waits,
    /// and returns the read bytes.
    fn run_and_read(
        &mut self,
        mut operations: Vec<CommandOp>,
        readback: Handle,
        size: u64,
    ) -> Vec<u8> {
        operations.push(barrier(
            readback,
            TextureUsageState::TransferDst,
            TextureUsageState::TransferSrc,
        ));
        operations.push(CommandOp::HostReadBuffer {
            buffer: readback,
            offset: 0,
            size,
        });
        let list = self
            .gal
            .create_command_list(CommandListDesc {
                label: "test-list".into(),
                operations,
            })
            .expect("command list validates");
        let token = self
            .gal
            .submit(SubmissionBatch {
                label: "test-batch".into(),
                command_lists: vec![list],
            })
            .expect("submission");
        self.gal.retire_through(token.submission).expect("retire");
        self.gal
            .completed_host_reads()
            .into_iter()
            .rev()
            .find(|read| read.buffer == readback && read.submission == token.submission)
            .expect("host read completed")
            .bytes
    }
}

impl Drop for Kit<'_> {
    fn drop(&mut self) {
        for handle in self.owned.drain(..).rev() {
            let _ = self.gal.destroy(handle);
        }
        let latest = self.gal.latest_submission_id();
        let _ = self.gal.retire_through(latest);
    }
}

fn pipeline_desc(
    layout: Handle,
    vertex: Handle,
    fragment: Handle,
    color: TextureFormat,
) -> GraphicsPipelineDesc {
    GraphicsPipelineDesc {
        label: "test-pipeline".into(),
        layout,
        vertex_shader: vertex,
        fragment_shader: fragment,
        topology: PrimitiveTopology::Triangles,
        cull_mode: CullMode::None,
        front_face: FrontFace::CounterClockwise,
        provoking_vertex: ProvokingVertex::First,
        raster_y_direction: RasterYDirection::Up,
        blend: BlendMode::Disabled,
        depth_compare: None,
        depth_write: false,
        depth_bias: None,
        color_formats: vec![color],
        depth_format: None,
        stencil: None,
    }
}

fn color_pass_ops(
    pass: Handle,
    target: Handle,
    view: Handle,
    texture: Handle,
    clear: ClearColor,
    body: Vec<CommandOp>,
) -> Vec<CommandOp> {
    let mut operations = vec![
        barrier(
            texture,
            TextureUsageState::Undefined,
            TextureUsageState::ColorAttachment,
        ),
        CommandOp::BeginPass {
            pass,
            target,
            colors: vec![PassAttachment {
                view,
                load_op: AttachmentLoadOp::Clear,
                store_op: AttachmentStoreOp::Store,
                clear_color: Some(clear),
            }],
            depth_stencil: None,
        },
    ];
    operations.extend(body);
    operations.push(CommandOp::EndPass);
    operations
}

/// Reads a whole RGBA8 texture with tightly packed rows (exercising the
/// row repack path whenever `width * 4` is not a multiple of 256).
fn read_rgba(
    kit: &mut Kit<'_>,
    mut operations: Vec<CommandOp>,
    texture: Handle,
    size: Extent3d,
) -> Vec<u8> {
    let bytes = u64::from(size.width) * u64::from(size.height) * 4;
    let readback = kit.buffer(
        bytes,
        MemoryDomain::Readback,
        &[BufferUsage::TransferDst, BufferUsage::HostRead],
    );
    operations.push(barrier(
        texture,
        TextureUsageState::ColorAttachment,
        TextureUsageState::TransferSrc,
    ));
    operations.push(CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
        buffer: readback,
        buffer_offset: 0,
        bytes_per_row: size.width * 4,
        rows_per_image: size.height,
        texture,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: size,
    }));
    kit.run_and_read(operations, readback, bytes)
}

fn pixel(image: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * width + x) * 4) as usize;
    image[index..index + 4].try_into().expect("in range")
}

#[test]
fn wgpu_backend_reports_vulkan_shader_conventions() {
    let Some(gal) = wgpu_gal("wgpu capabilities") else {
        return;
    };
    let capabilities = gal.capabilities();
    // The Vulkan backend's conventions, so renderers need no branches.
    assert_eq!(
        capabilities.shader_conventions,
        ShaderConventions {
            glsl_dialect: GlslDialect::ExplicitBindings,
            zero_to_one_clip_depth: true,
            flip_fullscreen_uv_y: true,
            readback_rows_bottom_up: false,
        }
    );
    assert!(capabilities.supports(BackendFeature::Graphics));
    assert!(capabilities.supports(BackendFeature::DynamicBufferOffsets));
    assert!(capabilities
        .limits
        .uniform_buffer_offset_alignment
        .is_power_of_two());
    assert!(!capabilities.supports(BackendFeature::Presentation));
}

#[test]
fn wgpu_clears_and_reads_back_tightly_packed_rows() {
    let size = extent(7, 5);
    let Some(image) = run_on_backends("clear readback", 0, |gal| {
        let mut kit = Kit::new(gal);
        let (texture, view) = kit.texture(
            size,
            TextureFormat::Rgba8Unorm,
            1,
            &[TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        );
        let target = kit.target(&[view], None, size);
        let pass = kit.pass(target, &[TextureFormat::Rgba8Unorm], None);
        let clear = ClearColor {
            r: 1.0,
            g: 0.5,
            b: 0.0,
            a: 1.0,
        };
        let operations = color_pass_ops(pass, target, view, texture, clear, Vec::new());
        read_rgba(&mut kit, operations, texture, size)
    }) else {
        return;
    };
    assert_eq!(image.len(), 7 * 5 * 4);
    assert!(image
        .chunks(4)
        .all(|texel| texel[0] == 255 && texel[2] == 0 && texel[3] == 255));
}

/// Draws one triangle covering the clip-space quadrant with positive X
/// and Y, under `direction`, with back faces culled.
fn quadrant_program(
    direction: RasterYDirection,
    front_face: FrontFace,
) -> impl Fn(&mut VulkanicGal) -> Vec<u8> {
    move |gal| {
        let size = extent(16, 16);
        let mut kit = Kit::new(gal);
        let (texture, view) = kit.texture(
            size,
            TextureFormat::Rgba8Unorm,
            1,
            &[TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        );
        let target = kit.target(&[view], None, size);
        let pass = kit.pass(target, &[TextureFormat::Rgba8Unorm], None);
        // Counter-clockwise in clip space.
        let vertex = kit.shader(
            ShaderStage::Vertex,
            "#version 450
            void main() {
                vec2 p[3] = vec2[3](vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0));
                gl_Position = vec4(p[gl_VertexIndex], 0.0, 1.0);
            }",
        );
        let fragment = kit.shader(
            ShaderStage::Fragment,
            "#version 450
            layout(location=0) out vec4 color;
            void main() { color = vec4(0.0, 1.0, 0.0, 1.0); }",
        );
        let (_, layout) = kit.layout(Vec::new());
        let mut desc = pipeline_desc(layout, vertex, fragment, TextureFormat::Rgba8Unorm);
        desc.raster_y_direction = direction;
        desc.cull_mode = CullMode::Back;
        desc.front_face = front_face;
        let pipeline = kit.pipeline(desc);
        let operations = color_pass_ops(
            pass,
            target,
            view,
            texture,
            ClearColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            vec![
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::Draw {
                    vertices: 3,
                    instances: 1,
                },
            ],
        );
        read_rgba(&mut kit, operations, texture, size)
    }
}

#[test]
fn wgpu_raster_directions_and_winding_match_vulkan() {
    for (direction, row) in [(RasterYDirection::Up, 3), (RasterYDirection::Down, 12)] {
        let name = format!("quadrant {direction:?}");
        let Some(image) = run_on_backends(
            &name,
            0,
            quadrant_program(direction, FrontFace::CounterClockwise),
        ) else {
            return;
        };
        // +Y points up the image for Up and down it for Down; +X is right.
        assert_eq!(
            pixel(&image, 16, 14, row)[1],
            255,
            "{name}: triangle missing"
        );
        assert_eq!(
            pixel(&image, 16, 1, row)[1],
            0,
            "{name}: triangle mirrored in X"
        );
        assert_eq!(
            pixel(&image, 16, 14, 15 - row)[1],
            0,
            "{name}: triangle mirrored in Y"
        );
        // Declaring the other winding front culls the same triangle.
        let culled = run_on_backends(&name, 0, quadrant_program(direction, FrontFace::Clockwise))
            .expect("adapter present");
        assert!(
            culled.chunks(4).all(|texel| texel[1] == 0),
            "{name}: winding not culled"
        );
    }
}

#[test]
fn wgpu_combined_samplers_dynamic_offsets_and_tight_uploads_match_vulkan() {
    let size = extent(8, 8);
    run_on_backends("combined sampler quad", 0, |gal| {
        let alignment = gal.capabilities().limits.uniform_buffer_offset_alignment;
        let mut kit = Kit::new(gal);
        let (texture, view) = kit.texture(
            size,
            TextureFormat::Rgba8Unorm,
            1,
            &[TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        );
        let target = kit.target(&[view], None, size);
        let pass = kit.pass(target, &[TextureFormat::Rgba8Unorm], None);
        // A 3x2 source image with rows of 12 bytes: not a 256 multiple.
        let source_size = extent(3, 2);
        let (source, source_view) = kit.texture(
            source_size,
            TextureFormat::Rgba8Unorm,
            1,
            &[TextureUsage::Sampled, TextureUsage::TransferDst],
        );
        let texels: Vec<u8> = (0..6u8)
            .flat_map(|index| [index * 40, 255 - index * 40, index * 20 + 10, 255])
            .collect();
        let upload = kit.buffer(
            texels.len() as u64,
            MemoryDomain::Upload,
            &[BufferUsage::HostWrite, BufferUsage::TransferSrc],
        );
        let uniforms = kit.buffer(
            alignment * 2,
            MemoryDomain::Upload,
            &[BufferUsage::Uniform, BufferUsage::HostWrite],
        );
        let sampler = kit.sampler(SamplerFilter::Nearest, None);
        let combined = kit.combined(source_view, sampler);
        let vertex = kit.shader(
            ShaderStage::Vertex,
            "#version 450
            layout(set=0,binding=1,std140) uniform Draw { vec4 rect; vec4 tint; } draw;
            layout(location=0) out vec2 uv;
            layout(location=1) out vec4 tint;
            void main() {
                vec2 corners[6] = vec2[6](vec2(0,0),vec2(1,0),vec2(0,1),vec2(0,1),vec2(1,0),vec2(1,1));
                vec2 p = corners[gl_VertexIndex];
                uv = p;
                tint = draw.tint;
                gl_Position = vec4(mix(draw.rect.xy, draw.rect.zw, p), 0.0, 1.0);
            }",
        );
        let fragment = kit.shader(
            ShaderStage::Fragment,
            "#version 450
            layout(set=0,binding=0) uniform sampler2D image;
            layout(location=0) in vec2 uv;
            layout(location=1) in vec4 tint;
            layout(location=0) out vec4 color;
            void main() { color = texture(image, uv) * tint; }",
        );
        let (layout, pipeline_layout) = kit.layout(vec![
            binding(0, ResourceBindingKind::CombinedTextureSampler, 0),
            binding(1, ResourceBindingKind::UniformBuffer, 1),
        ]);
        let mut uniform = bound(1, ResourceBindingKind::UniformBuffer, uniforms);
        uniform.dynamic_offsets = vec![0];
        uniform.buffer_range = Some(32);
        let set = kit.set(
            layout,
            vec![
                bound(0, ResourceBindingKind::CombinedTextureSampler, combined),
                uniform,
            ],
        );
        let mut desc = pipeline_desc(pipeline_layout, vertex, fragment, TextureFormat::Rgba8Unorm);
        desc.raster_y_direction = RasterYDirection::Down;
        let pipeline = kit.pipeline(desc);
        let mut uniform_bytes = vec![0u8; (alignment * 2) as usize];
        for (draw, values) in [
            [-1.0f32, -1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0, 1.0, 0.5, 0.25, 1.0],
        ]
        .iter()
        .enumerate()
        {
            for (index, value) in values.iter().enumerate() {
                let at = draw * alignment as usize + index * 4;
                uniform_bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        let mut operations = vec![
            CommandOp::HostWriteBuffer {
                buffer: upload,
                offset: 0,
                data: texels,
            },
            CommandOp::HostWriteBuffer {
                buffer: uniforms,
                offset: 0,
                data: uniform_bytes,
            },
            barrier(
                upload,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ),
            barrier(
                source,
                TextureUsageState::Undefined,
                TextureUsageState::TransferDst,
            ),
            CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                buffer: upload,
                buffer_offset: 0,
                bytes_per_row: 12,
                rows_per_image: 2,
                texture: source,
                texture_mip: 0,
                texture_layer: 0,
                texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                extent: source_size,
            }),
            barrier(
                source,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ),
            barrier(
                uniforms,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ),
        ];
        operations.extend(color_pass_ops(
            pass,
            target,
            view,
            texture,
            ClearColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
            vec![
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index: 0,
                    set,
                    dynamic_offsets: vec![0],
                },
                CommandOp::Draw {
                    vertices: 6,
                    instances: 1,
                },
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index: 0,
                    set,
                    dynamic_offsets: vec![alignment],
                },
                CommandOp::Draw {
                    vertices: 6,
                    instances: 1,
                },
            ],
        ));
        let image = read_rgba(&mut kit, operations, texture, size);
        // Down: the first quad (-1..0) lands in the top-left quadrant.
        assert_eq!(pixel(&image, 8, 0, 0), [0, 255, 10, 255]);
        assert_ne!(pixel(&image, 8, 7, 7), [0, 0, 0, 0]);
        assert_eq!(pixel(&image, 8, 7, 0), [0, 0, 0, 0]);
        image
    });
}

#[test]
fn wgpu_host_writes_and_buffer_copies_handle_unaligned_ranges() {
    let Some(bytes) = run_on_backends("unaligned buffers", 0, |gal| {
        let mut kit = Kit::new(gal);
        let upload = kit.buffer(
            64,
            MemoryDomain::Upload,
            &[
                BufferUsage::HostWrite,
                BufferUsage::TransferSrc,
                BufferUsage::TransferDst,
            ],
        );
        let readback = kit.buffer(
            64,
            MemoryDomain::Readback,
            &[BufferUsage::TransferDst, BufferUsage::HostRead],
        );
        let operations = vec![
            CommandOp::HostWriteBuffer {
                buffer: upload,
                offset: 0,
                data: (0..64).collect(),
            },
            barrier(
                upload,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferDst,
            ),
            CommandOp::HostWriteBuffer {
                buffer: upload,
                offset: 3,
                data: vec![200, 201, 202, 203, 204],
            },
            barrier(
                upload,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ),
            CommandOp::CopyBufferRegion {
                src: upload,
                src_offset: 1,
                dst: readback,
                dst_offset: 0,
                size: 61,
            },
            barrier(
                readback,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferDst,
            ),
            CommandOp::CopyBufferRegion {
                src: upload,
                src_offset: 2,
                dst: readback,
                dst_offset: 61,
                size: 3,
            },
        ];
        kit.run_and_read(operations, readback, 64)
    }) else {
        return;
    };
    let mut source: Vec<u8> = (0..64).collect();
    source[3..8].copy_from_slice(&[200, 201, 202, 203, 204]);
    let mut expected: Vec<u8> = source[1..62].to_vec();
    expected.extend_from_slice(&source[2..5]);
    assert_eq!(bytes, expected);
}

#[test]
fn wgpu_host_reads_report_partial_unaligned_ranges() {
    let Some(mut gal) = wgpu_gal("partial host read") else {
        return;
    };
    let mut kit = Kit::new(&mut gal);
    let upload = kit.buffer(
        32,
        MemoryDomain::Upload,
        &[BufferUsage::HostWrite, BufferUsage::TransferSrc],
    );
    let readback = kit.buffer(
        32,
        MemoryDomain::Readback,
        &[BufferUsage::TransferDst, BufferUsage::HostRead],
    );
    let operations = vec![
        CommandOp::HostWriteBuffer {
            buffer: upload,
            offset: 0,
            data: (100..132).collect(),
        },
        barrier(
            upload,
            TextureUsageState::TransferDst,
            TextureUsageState::TransferSrc,
        ),
        CommandOp::CopyBuffer {
            src: upload,
            dst: readback,
            size: 32,
        },
        barrier(
            readback,
            TextureUsageState::TransferDst,
            TextureUsageState::TransferSrc,
        ),
    ];
    let list = kit
        .gal
        .create_command_list(CommandListDesc {
            label: "partial".into(),
            operations: operations
                .into_iter()
                .chain([CommandOp::HostReadBuffer {
                    buffer: readback,
                    offset: 5,
                    size: 7,
                }])
                .collect(),
        })
        .expect("validates");
    let token = kit
        .gal
        .submit(SubmissionBatch {
            label: "partial".into(),
            command_lists: vec![list],
        })
        .expect("submits");
    kit.gal.retire_through(token.submission).expect("retires");
    let read = kit
        .gal
        .completed_host_reads()
        .into_iter()
        .find(|read| read.buffer == readback)
        .expect("read completed");
    assert_eq!(read.offset, 5);
    assert_eq!(read.bytes, (105..112).collect::<Vec<u8>>());
}

#[test]
fn wgpu_texture_copies_repack_rows_and_reverse_like_vulkan() {
    let size = extent(5, 3);
    let Some(bytes) = run_on_backends("texture row copies", 0, |gal| {
        let mut kit = Kit::new(gal);
        let usages = [TextureUsage::TransferDst, TextureUsage::TransferSrc];
        let (first, _) = kit.texture(size, TextureFormat::R8Unorm, 1, &usages);
        let (second, _) = kit.texture(size, TextureFormat::R8Unorm, 1, &usages);
        let upload = kit.buffer(
            15,
            MemoryDomain::Upload,
            &[BufferUsage::HostWrite, BufferUsage::TransferSrc],
        );
        let readback = kit.buffer(
            32,
            MemoryDomain::Readback,
            &[BufferUsage::TransferDst, BufferUsage::HostRead],
        );
        let region = |buffer, texture, offset| BufferImageCopyRegion {
            buffer,
            buffer_offset: offset,
            bytes_per_row: 5,
            rows_per_image: 3,
            texture,
            texture_mip: 0,
            texture_layer: 0,
            texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            extent: size,
        };
        let operations = vec![
            CommandOp::HostWriteBuffer {
                buffer: upload,
                offset: 0,
                data: (1..=15).collect(),
            },
            barrier(
                upload,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ),
            barrier(
                first,
                TextureUsageState::Undefined,
                TextureUsageState::TransferDst,
            ),
            CommandOp::CopyBufferToTexture(region(upload, first, 0)),
            barrier(
                first,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ),
            barrier(
                second,
                TextureUsageState::Undefined,
                TextureUsageState::TransferDst,
            ),
            CommandOp::CopyTexture(TextureImageCopyRegion {
                row_order: TextureRowOrder::Reverse,
                src_texture: first,
                src_mip: 0,
                src_layer: 0,
                src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                dst_texture: second,
                dst_mip: 0,
                dst_layer: 0,
                dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                extent: size,
            }),
            barrier(
                second,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ),
            CommandOp::CopyTextureToBuffer(region(readback, first, 0)),
            barrier(
                readback,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferDst,
            ),
            CommandOp::CopyTextureToBuffer(region(readback, second, 16)),
        ];
        kit.run_and_read(operations, readback, 32)
    }) else {
        return;
    };
    let rows: Vec<u8> = (1..=15).collect();
    assert_eq!(&bytes[..15], &rows[..]);
    let reversed: Vec<u8> = rows.chunks(5).rev().flatten().copied().collect();
    assert_eq!(&bytes[16..31], &reversed[..]);
}

#[test]
fn wgpu_generates_mipmaps_like_vulkan() {
    let size = extent(4, 4);
    let Some(bytes) = run_on_backends("mipmaps", 1, |gal| {
        let mut kit = Kit::new(gal);
        let (texture, _) = kit.texture(
            size,
            TextureFormat::Rgba8Unorm,
            3,
            &[
                TextureUsage::Sampled,
                TextureUsage::TransferDst,
                TextureUsage::TransferSrc,
            ],
        );
        let upload = kit.buffer(
            64,
            MemoryDomain::Upload,
            &[BufferUsage::HostWrite, BufferUsage::TransferSrc],
        );
        let readback = kit.buffer(
            20,
            MemoryDomain::Readback,
            &[BufferUsage::TransferDst, BufferUsage::HostRead],
        );
        // 2x2 blocks of solid colors, so every level averages exactly.
        let mut texels = Vec::new();
        for y in 0..4u8 {
            for x in 0..4u8 {
                let block = (y / 2) * 2 + x / 2;
                texels.extend_from_slice(&[block * 60, 255 - block * 60, 128, 255]);
            }
        }
        let whole = TextureSubresourceRange {
            base_mip: 0,
            mip_count: 3,
            base_layer: 0,
            layer_count: 1,
        };
        let level = |mip, offset, side| BufferImageCopyRegion {
            buffer: readback,
            buffer_offset: offset,
            bytes_per_row: side * 4,
            rows_per_image: side,
            texture,
            texture_mip: mip,
            texture_layer: 0,
            texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            extent: extent(side, side),
        };
        let operations = vec![
            CommandOp::HostWriteBuffer {
                buffer: upload,
                offset: 0,
                data: texels,
            },
            barrier(
                upload,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ),
            barrier(
                texture,
                TextureUsageState::Undefined,
                TextureUsageState::TransferDst,
            ),
            CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                buffer: upload,
                buffer_offset: 0,
                bytes_per_row: 16,
                rows_per_image: 4,
                texture,
                texture_mip: 0,
                texture_layer: 0,
                texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                extent: size,
            }),
            mip_barrier(
                texture,
                0,
                1,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ),
            mip_barrier(
                texture,
                1,
                2,
                TextureUsageState::Undefined,
                TextureUsageState::TransferDst,
            ),
            CommandOp::GenerateMipmaps {
                texture,
                subresources: whole,
            },
            mip_barrier(
                texture,
                1,
                2,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ),
            CommandOp::CopyTextureToBuffer(level(1, 0, 2)),
            barrier(
                readback,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferDst,
            ),
            CommandOp::CopyTextureToBuffer(level(2, 16, 1)),
        ];
        kit.run_and_read(operations, readback, 20)
    }) else {
        return;
    };
    for block in 0..4u8 {
        let texel = &bytes[block as usize * 4..block as usize * 4 + 4];
        assert!(texel[0].abs_diff(block * 60) <= 1 && texel[1].abs_diff(255 - block * 60) <= 1);
    }
    // The 1x1 level averages all four blocks.
    assert!(bytes[16].abs_diff(90) <= 2, "mip 2 red {}", bytes[16]);
}

/// Draws a hexagon as a triangle fan or as the equivalent list.
fn fan_program(fan: bool) -> impl Fn(&mut VulkanicGal) -> Vec<u8> {
    move |gal| {
        let size = extent(24, 24);
        let mut kit = Kit::new(gal);
        let (texture, view) = kit.texture(
            size,
            TextureFormat::Rgba8Unorm,
            1,
            &[TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        );
        let target = kit.target(&[view], None, size);
        let pass = kit.pass(target, &[TextureFormat::Rgba8Unorm], None);
        let source = if fan {
            "#version 450
            layout(location=0) flat out vec4 shade;
            void main() {
                float a = float(gl_VertexIndex) * 1.0471976;
                vec2 p = gl_VertexIndex == 0 ? vec2(0.0) : 0.8 * vec2(cos(a), sin(a));
                shade = vec4(float(gl_VertexIndex) / 8.0, 0.0, 1.0, 1.0);
                gl_Position = vec4(p, 0.0, 1.0);
            }"
        } else {
            "#version 450
            layout(location=0) flat out vec4 shade;
            void main() {
                int triangle = gl_VertexIndex / 3;
                int corner = gl_VertexIndex % 3;
                // Fan triangle i is (i + 1, i + 2, 0).
                int index = corner == 2 ? 0 : triangle + 1 + corner;
                float a = float(index) * 1.0471976;
                vec2 p = index == 0 ? vec2(0.0) : 0.8 * vec2(cos(a), sin(a));
                shade = vec4(float(triangle + 1) / 8.0, 0.0, 1.0, 1.0);
                gl_Position = vec4(p, 0.0, 1.0);
            }"
        };
        let vertex = kit.shader(ShaderStage::Vertex, source);
        let fragment = kit.shader(
            ShaderStage::Fragment,
            "#version 450
            layout(location=0) flat in vec4 shade;
            layout(location=0) out vec4 color;
            void main() { color = shade; }",
        );
        let (_, layout) = kit.layout(Vec::new());
        let mut desc = pipeline_desc(layout, vertex, fragment, TextureFormat::Rgba8Unorm);
        desc.topology = if fan {
            PrimitiveTopology::TriangleFan
        } else {
            PrimitiveTopology::Triangles
        };
        desc.cull_mode = CullMode::Back;
        let pipeline = kit.pipeline(desc);
        let operations = color_pass_ops(
            pass,
            target,
            view,
            texture,
            ClearColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
            vec![
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::Draw {
                    vertices: if fan { 7 } else { 15 },
                    instances: 1,
                },
            ],
        );
        read_rgba(&mut kit, operations, texture, size)
    }
}

#[test]
fn wgpu_triangle_fans_match_lists_and_vulkan() {
    let Some(fan) = run_on_backends("triangle fan", 0, fan_program(true)) else {
        return;
    };
    let list = run_on_backends("triangle list", 0, fan_program(false)).expect("adapter present");
    assert_eq!(
        fan, list,
        "fan and list draws differ (winding or provoking vertex)"
    );
    assert!(fan.chunks(4).any(|texel| texel[2] == 255));
}

#[test]
fn wgpu_compute_and_indirect_work_matches_vulkan() {
    let size = extent(16, 16);
    run_on_backends("compute indirect", 0, |gal| {
        let mut kit = Kit::new(gal);
        let (texture, view) = kit.texture(
            size,
            TextureFormat::Rgba8Unorm,
            1,
            &[TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        );
        let target = kit.target(&[view], None, size);
        let pass = kit.pass(target, &[TextureFormat::Rgba8Unorm], None);
        let args = kit.buffer(
            64,
            MemoryDomain::DeviceLocal,
            &[BufferUsage::Storage, BufferUsage::Indirect],
        );
        let compute_shader = kit.shader(
            ShaderStage::Compute,
            "#version 450
            layout(local_size_x=1) in;
            layout(set=0,binding=0,std430) buffer Args { uint words[]; } args;
            void main() {
                // Two draw records: (vertex count, instances, first vertex, first instance).
                args.words[0] = 3u; args.words[1] = 1u; args.words[2] = 0u; args.words[3] = 0u;
                args.words[4] = 3u; args.words[5] = 1u; args.words[6] = 3u; args.words[7] = 0u;
            }",
        );
        let mut storage = binding(0, ResourceBindingKind::StorageBuffer, 0);
        storage.stages = PipelineStageFlags::COMPUTE;
        let (compute_layout, compute_pipeline_layout) = kit.layout(vec![storage]);
        let mut written = bound(0, ResourceBindingKind::StorageBuffer, args);
        written.access = AccessFlags::WRITE;
        let compute_set = kit.set(compute_layout, vec![written]);
        let compute = kit
            .gal
            .create_compute_pipeline(ComputePipelineDesc {
                label: "test-compute".into(),
                layout: compute_pipeline_layout,
                shader: compute_shader,
            })
            .expect("compute pipeline");
        kit.own(compute);
        let vertex = kit.shader(
            ShaderStage::Vertex,
            "#version 450
            void main() {
                vec2 p[6] = vec2[6](vec2(-1,-1), vec2(0,-1), vec2(-1,0), vec2(0,0), vec2(1,0), vec2(0,1));
                gl_Position = vec4(p[gl_VertexIndex], 0.0, 1.0);
            }",
        );
        let fragment = kit.shader(
            ShaderStage::Fragment,
            "#version 450
            layout(location=0) out vec4 color;
            void main() { color = vec4(1.0, 1.0, 0.0, 1.0); }",
        );
        let (_, layout) = kit.layout(Vec::new());
        let pipeline = kit.pipeline(pipeline_desc(
            layout,
            vertex,
            fragment,
            TextureFormat::Rgba8Unorm,
        ));
        let mut operations = vec![
            CommandOp::BindComputePipeline(compute),
            CommandOp::BindResourceSet {
                pipeline_layout: compute_pipeline_layout,
                set_index: 0,
                set: compute_set,
                dynamic_offsets: Vec::new(),
            },
            CommandOp::Dispatch {
                groups_x: 1,
                groups_y: 1,
                groups_z: 1,
            },
            barrier(
                args,
                TextureUsageState::ShaderWrite,
                TextureUsageState::IndirectRead,
            ),
        ];
        operations.extend(color_pass_ops(
            pass,
            target,
            view,
            texture,
            ClearColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            vec![
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::DrawIndirect {
                    buffer: args,
                    offset: 0,
                    draw_count: 2,
                },
            ],
        ));
        let image = read_rgba(&mut kit, operations, texture, size);
        assert!(image.chunks(4).any(|texel| texel[0] == 255));
        image
    });
}

#[test]
fn wgpu_depth_textures_sample_as_values_and_comparisons_like_vulkan() {
    let size = extent(8, 8);
    run_on_backends("depth sampling", 1, |gal| {
        let mut kit = Kit::new(gal);
        let (depth, depth_view) = kit.texture(
            size,
            TextureFormat::Depth32Float,
            1,
            &[TextureUsage::DepthStencilAttachment, TextureUsage::Sampled],
        );
        let depth_target = kit.target(&[], Some(depth_view), size);
        let depth_pass = kit.pass(depth_target, &[], Some(TextureFormat::Depth32Float));
        let (color, color_view) = kit.texture(
            size,
            TextureFormat::Rgba8Unorm,
            1,
            &[TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        );
        let color_target = kit.target(&[color_view], None, size);
        let color_pass = kit.pass(color_target, &[TextureFormat::Rgba8Unorm], None);
        let depth_vertex = kit.shader(
            ShaderStage::Vertex,
            "#version 450
            void main() {
                vec2 p[3] = vec2[3](vec2(-1,-1), vec2(3,-1), vec2(-1,3));
                gl_Position = vec4(p[gl_VertexIndex], 0.25, 1.0);
            }",
        );
        let depth_fragment = kit.shader(ShaderStage::Fragment, "#version 450\nvoid main() {}");
        let (_, empty_layout) = kit.layout(Vec::new());
        let mut depth_desc = pipeline_desc(
            empty_layout,
            depth_vertex,
            depth_fragment,
            TextureFormat::Rgba8Unorm,
        );
        depth_desc.color_formats.clear();
        depth_desc.depth_format = Some(TextureFormat::Depth32Float);
        depth_desc.depth_compare = Some(CompareOp::Always);
        depth_desc.depth_write = true;
        let depth_pipeline = kit.pipeline(depth_desc);
        let value_sampler = kit.sampler(SamplerFilter::Nearest, None);
        let compare_sampler = kit.sampler(SamplerFilter::Nearest, Some(CompareOp::Greater));
        let as_value = kit.combined(depth_view, value_sampler);
        let as_shadow = kit.combined(depth_view, compare_sampler);
        let vertex = kit.shader(ShaderStage::Vertex, FULLSCREEN_VS);
        let fragment = kit.shader(
            ShaderStage::Fragment,
            "#version 450
            layout(set=0,binding=0) uniform sampler2D depthValue;
            layout(set=0,binding=1) uniform sampler2DShadow depthShadow;
            layout(location=0) in vec2 uv;
            layout(location=0) out vec4 color;
            void main() {
                float depth = texture(depthValue, uv).r;
                float lit = texture(depthShadow, vec3(uv, 0.5));
                color = vec4(depth, lit, 0.0, 1.0);
            }",
        );
        let (layout, pipeline_layout) = kit.layout(vec![
            binding(0, ResourceBindingKind::CombinedTextureSampler, 0),
            binding(1, ResourceBindingKind::CombinedTextureSampler, 0),
        ]);
        let set = kit.set(
            layout,
            vec![
                bound(0, ResourceBindingKind::CombinedTextureSampler, as_value),
                bound(1, ResourceBindingKind::CombinedTextureSampler, as_shadow),
            ],
        );
        let pipeline = kit.pipeline(pipeline_desc(
            pipeline_layout,
            vertex,
            fragment,
            TextureFormat::Rgba8Unorm,
        ));
        let mut operations = vec![
            barrier(
                depth,
                TextureUsageState::Undefined,
                TextureUsageState::DepthStencilAttachment,
            ),
            CommandOp::BeginPass {
                pass: depth_pass,
                target: depth_target,
                colors: Vec::new(),
                depth_stencil: Some(PassAttachment {
                    view: depth_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: None,
                }),
            },
            CommandOp::BindGraphicsPipeline(depth_pipeline),
            CommandOp::Draw {
                vertices: 3,
                instances: 1,
            },
            CommandOp::EndPass,
            barrier(
                depth,
                TextureUsageState::DepthStencilAttachment,
                TextureUsageState::ShaderRead,
            ),
        ];
        operations.extend(color_pass_ops(
            color_pass,
            color_target,
            color_view,
            color,
            ClearColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
            vec![
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index: 0,
                    set,
                    dynamic_offsets: Vec::new(),
                },
                CommandOp::Draw {
                    vertices: 3,
                    instances: 1,
                },
            ],
        ));
        let image = read_rgba(&mut kit, operations, color, size);
        let texel = pixel(&image, 8, 4, 4);
        assert!(texel[0].abs_diff(64) <= 1, "depth sampled as {}", texel[0]);
        // Reference 0.5 compared Greater against the stored 0.25 passes.
        assert_eq!(texel[1], 255, "shadow comparison");
        image
    });
}

#[test]
fn wgpu_stencil_and_blend_modes_match_vulkan() {
    let size = extent(8, 8);
    run_on_backends("stencil blend", 1, |gal| {
        let mut kit = Kit::new(gal);
        let (color, color_view) = kit.texture(
            size,
            TextureFormat::Rgba8Unorm,
            1,
            &[TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        );
        let (depth, depth_view) = kit.texture(
            size,
            TextureFormat::Depth24Stencil8,
            1,
            &[TextureUsage::DepthStencilAttachment],
        );
        let target = kit.target(&[color_view], Some(depth_view), size);
        let pass = kit.pass(
            target,
            &[TextureFormat::Rgba8Unorm],
            Some(TextureFormat::Depth24Stencil8),
        );
        // The left half writes stencil 1; the right half is untouched.
        let half = kit.shader(
            ShaderStage::Vertex,
            "#version 450
            void main() {
                vec2 p[3] = vec2[3](vec2(-1,-1), vec2(0,-1), vec2(0,3));
                vec2 q[3] = vec2[3](vec2(-1,-1), vec2(0,3), vec2(-1,3));
                gl_Position = vec4(gl_InstanceIndex == 0 ? p[gl_VertexIndex] : q[gl_VertexIndex], 0.5, 1.0);
            }",
        );
        let full = kit.shader(ShaderStage::Vertex, FULLSCREEN_VS);
        let red = kit.shader(
            ShaderStage::Fragment,
            "#version 450
            layout(location=0) out vec4 color;
            void main() { color = vec4(1.0, 0.0, 0.0, 0.5); }",
        );
        let (_, layout) = kit.layout(Vec::new());
        let mut mark = pipeline_desc(layout, half, red, TextureFormat::Rgba8Unorm);
        mark.depth_format = Some(TextureFormat::Depth24Stencil8);
        mark.blend = BlendMode::DepthMask;
        mark.stencil = Some(StencilState {
            front: StencilFaceState::replace(1, 0xff, 0xff),
            back: StencilFaceState::replace(1, 0xff, 0xff),
        });
        let mark = kit.pipeline(mark);
        let mut test = pipeline_desc(layout, full, red, TextureFormat::Rgba8Unorm);
        test.depth_format = Some(TextureFormat::Depth24Stencil8);
        test.blend = BlendMode::Alpha;
        test.stencil = Some(StencilState {
            front: StencilFaceState::keep(CompareOp::Equal, 1, 0xff),
            back: StencilFaceState::keep(CompareOp::Equal, 1, 0xff),
        });
        let test = kit.pipeline(test);
        let operations = vec![
            barrier(
                color,
                TextureUsageState::Undefined,
                TextureUsageState::ColorAttachment,
            ),
            barrier(
                depth,
                TextureUsageState::Undefined,
                TextureUsageState::DepthStencilAttachment,
            ),
            CommandOp::BeginPass {
                pass,
                target,
                colors: vec![PassAttachment {
                    view: color_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: Some(ClearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 1.0,
                        a: 1.0,
                    }),
                }],
                depth_stencil: Some(PassAttachment {
                    view: depth_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::DontCare,
                    clear_color: None,
                }),
            },
            CommandOp::BindGraphicsPipeline(mark),
            CommandOp::Draw {
                vertices: 3,
                instances: 2,
            },
            CommandOp::BindGraphicsPipeline(test),
            CommandOp::Draw {
                vertices: 3,
                instances: 1,
            },
            CommandOp::EndPass,
        ];
        let image = read_rgba(&mut kit, operations, color, size);
        let left = pixel(&image, 8, 1, 4);
        let right = pixel(&image, 8, 6, 4);
        assert!(
            left[0].abs_diff(128) <= 1 && left[2].abs_diff(128) <= 1,
            "left {left:?}"
        );
        assert_eq!(right, [0, 0, 255, 255]);
        image
    });
}

#[test]
fn wgpu_repeated_submissions_defer_destroys_until_retired() {
    let Some(mut gal) = wgpu_gal("wgpu lifetime stress") else {
        return;
    };
    let mut last = None;
    for index in 0..12 {
        let src = gal
            .create_buffer(BufferDesc {
                label: format!("stress-src-{index}"),
                size: 256,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::TransferSrc, BufferUsage::HostWrite],
            })
            .expect("source");
        let dst = gal
            .create_buffer(BufferDesc {
                label: format!("stress-dst-{index}"),
                size: 256,
                memory: MemoryDomain::Readback,
                usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
            })
            .expect("destination");
        let list = gal
            .create_command_list(CommandListDesc {
                label: format!("stress-{index}"),
                operations: vec![CommandOp::CopyBuffer {
                    src,
                    dst,
                    size: 256,
                }],
            })
            .expect("validates");
        let token = gal
            .submit(SubmissionBatch {
                label: format!("stress-{index}"),
                command_lists: vec![list],
            })
            .expect("submits");
        gal.destroy(src).expect("deferred destroy");
        gal.destroy(dst).expect("deferred destroy");
        last = Some(token.submission);
    }
    let retired = gal
        .retire_through(last.expect("submitted"))
        .expect("retires");
    assert_eq!(retired.len(), 24);
    assert!(gal.poll_completed() >= last.expect("submitted"));
}

/// Two triangles (or a fan) whose vertices carry distinct flat colors,
/// drawn with `provoking`: the flat color reveals which vertex provokes.
fn provoking_program(
    provoking: ProvokingVertex,
    topology: PrimitiveTopology,
    draw: &'static str,
) -> impl Fn(&mut VulkanicGal) -> Vec<u8> {
    move |gal| {
        let size = extent(16, 16);
        let mut kit = Kit::new(gal);
        let (texture, view) = kit.texture(
            size,
            TextureFormat::Rgba8Unorm,
            1,
            &[TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        );
        let target = kit.target(&[view], None, size);
        let pass = kit.pass(target, &[TextureFormat::Rgba8Unorm], None);
        let vertex = kit.shader(
            ShaderStage::Vertex,
            "#version 450
            layout(location=0) flat out vec4 shade;
            void main() {
                // Vertices 0-2 cover the left half, 3-5 the right.
                vec2 p[6] = vec2[6](vec2(-1,-1), vec2(0,-1), vec2(-1,1),
                                    vec2(0,-1), vec2(1,-1), vec2(1,1));
                float id = float(gl_VertexIndex);
                shade = vec4(id / 8.0, 1.0 - id / 8.0, 0.25, 1.0);
                gl_Position = vec4(p[gl_VertexIndex], 0.0, 1.0);
            }",
        );
        let fan_vertex = kit.shader(
            ShaderStage::Vertex,
            "#version 450
            layout(location=0) flat out vec4 shade;
            void main() {
                vec2 fan[5] = vec2[5](vec2(-1,-1), vec2(1,-1), vec2(1,0), vec2(0,1), vec2(-1,1));
                float id = float(gl_VertexIndex);
                shade = vec4(id / 8.0, 1.0 - id / 8.0, 0.25, 1.0);
                gl_Position = vec4(fan[gl_VertexIndex], 0.0, 1.0);
            }",
        );
        let fragment = kit.shader(
            ShaderStage::Fragment,
            "#version 450
            layout(location=0) flat in vec4 shade;
            layout(location=0) out vec4 color;
            void main() { color = shade; }",
        );
        let (_, layout) = kit.layout(Vec::new());
        let fan = topology == PrimitiveTopology::TriangleFan;
        let mut desc = pipeline_desc(
            layout,
            if fan { fan_vertex } else { vertex },
            fragment,
            TextureFormat::Rgba8Unorm,
        );
        desc.topology = topology;
        desc.provoking_vertex = provoking;
        let pipeline = kit.pipeline(desc);
        let mut operations = Vec::new();
        let mut body = vec![CommandOp::BindGraphicsPipeline(pipeline)];
        match draw {
            "direct" => body.push(CommandOp::Draw {
                vertices: if fan { 5 } else { 6 },
                instances: 1,
            }),
            "u16" | "u32" | "indirect" => {
                let wide = draw != "u16";
                let indices = kit.buffer(
                    64,
                    MemoryDomain::Upload,
                    &[
                        BufferUsage::Index,
                        BufferUsage::HostWrite,
                        BufferUsage::Indirect,
                    ],
                );
                let mut bytes = Vec::new();
                for index in 0..6u32 {
                    if wide {
                        bytes.extend_from_slice(&index.to_le_bytes());
                    } else {
                        bytes.extend_from_slice(&(index as u16).to_le_bytes());
                    }
                }
                let args_offset = 32;
                operations.push(CommandOp::HostWriteBuffer {
                    buffer: indices,
                    offset: 0,
                    data: bytes,
                });
                // One indexed indirect record: 6 indices, 1 instance.
                let record: Vec<u8> = [6u32, 1, 0, 0, 0]
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect();
                operations.push(CommandOp::HostWriteBuffer {
                    buffer: indices,
                    offset: args_offset,
                    data: record,
                });
                operations.push(barrier(
                    indices,
                    TextureUsageState::TransferDst,
                    TextureUsageState::IndexRead,
                ));
                body.push(CommandOp::SetIndexBuffer {
                    buffer: indices,
                    offset: 0,
                    index_type: if wide { IndexType::U32 } else { IndexType::U16 },
                });
                body.push(if draw == "indirect" {
                    CommandOp::DrawIndexedIndirect {
                        buffer: indices,
                        offset: args_offset,
                        draw_count: 1,
                    }
                } else {
                    CommandOp::DrawIndexed {
                        indices: 6,
                        instances: 1,
                    }
                });
            }
            other => unreachable!("{other}"),
        }
        operations.extend(color_pass_ops(
            pass,
            target,
            view,
            texture,
            ClearColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
            body,
        ));
        read_rgba(&mut kit, operations, texture, size)
    }
}

#[test]
fn wgpu_last_provoking_vertex_matches_vulkan_for_every_draw_kind() {
    for (topology, draw) in [
        (PrimitiveTopology::Triangles, "direct"),
        (PrimitiveTopology::Triangles, "u16"),
        (PrimitiveTopology::Triangles, "u32"),
        (PrimitiveTopology::Triangles, "indirect"),
        (PrimitiveTopology::TriangleFan, "direct"),
    ] {
        let name = format!("provoking {topology:?} {draw}");
        let Some(last) = run_on_backends(
            &name,
            0,
            provoking_program(ProvokingVertex::Last, topology, draw),
        ) else {
            return;
        };
        let first = run_on_backends(
            &name,
            0,
            provoking_program(ProvokingVertex::First, topology, draw),
        )
        .expect("adapter present");
        assert_ne!(
            first, last,
            "{name}: provoking vertex had no visible effect"
        );
    }
}

#[test]
fn wgpu_rejects_indirect_fans_explicitly() {
    let Some(mut gal) = wgpu_gal("wgpu indirect fan") else {
        return;
    };
    let size = extent(4, 4);
    let mut kit = Kit::new(&mut gal);
    let (texture, view) = kit.texture(
        size,
        TextureFormat::Rgba8Unorm,
        1,
        &[TextureUsage::ColorAttachment],
    );
    let target = kit.target(&[view], None, size);
    let pass = kit.pass(target, &[TextureFormat::Rgba8Unorm], None);
    let vertex = kit.shader(ShaderStage::Vertex, FULLSCREEN_VS);
    let fragment = kit.shader(
        ShaderStage::Fragment,
        "#version 450\nlayout(location=0) out vec4 color;\nvoid main() { color = vec4(1.0); }",
    );
    let (_, layout) = kit.layout(Vec::new());
    let mut desc = pipeline_desc(layout, vertex, fragment, TextureFormat::Rgba8Unorm);
    desc.topology = PrimitiveTopology::TriangleFan;
    let pipeline = kit.pipeline(desc);
    let args = kit.buffer(16, MemoryDomain::DeviceLocal, &[BufferUsage::Indirect]);
    let operations = color_pass_ops(
        pass,
        target,
        view,
        texture,
        ClearColor {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        },
        vec![
            CommandOp::BindGraphicsPipeline(pipeline),
            CommandOp::DrawIndirect {
                buffer: args,
                offset: 0,
                draw_count: 1,
            },
        ],
    );
    let list = kit
        .gal
        .create_command_list(CommandListDesc {
            label: "indirect-fan".into(),
            operations,
        })
        .expect("validates");
    let error = kit
        .gal
        .submit(SubmissionBatch {
            label: "indirect-fan".into(),
            command_lists: vec![list],
        })
        .expect_err("indirect fans are unsupported");
    assert!(error.to_string().contains("direct or indexed"), "{error}");
}
