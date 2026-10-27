//! CPU cost of recording and submitting work through VulkanicGAL.
//!
//! ```text
//! cargo run --release -p vulkanic-gal --example submit_bench --features wgpu -- [backend ...]
//! ```
//!
//! Backends are `GraphicsBackend` ids (`vulkan`, `wgpu-vulkan`, ...); the
//! default is every compiled-in windowless backend that starts. Each workload
//! runs `FRAMES` frames after a warm-up and reports the 25th-percentile frame
//! in microseconds (steadier than the median under CPU frequency scaling):
//!
//! - `record`: `create_command_list`.
//! - `submit`: `submit` wall time (no profiling).
//! - `frontend`, `encode`, `queue`: the `submit_profiled` breakdown (GAL
//!   frontend, backend encode, backend submit), from a separate profiled run.
//!
//! Workloads:
//! - `rebind-N`: N textured draws, each rebinding one resource set with a new
//!   dynamic uniform offset (the 2D renderer's pattern), plus 3 host writes.
//! - `static-N`: N draws under one bind.
//! - `upload`: 64 host writes of 256 bytes and a tightly packed 300x200
//!   texture upload.

use vulkanic_gal::commands::*;
use vulkanic_gal::metrics::SubmitProfile;
use vulkanic_gal::resources::*;
use vulkanic_gal::*;

const WARMUP: usize = 300;
const FRAMES: usize = 600;

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

struct Scene {
    alignment: u64,
    target: Handle,
    target_texture: Handle,
    target_view: Handle,
    pass: Handle,
    pipeline: Handle,
    pipeline_layout: Handle,
    set: Handle,
    uniforms: Handle,
    storage: Handle,
    upload: Handle,
    upload_texture: Handle,
}

const MAX_DRAWS: u64 = 4000;

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

fn shader(gal: &mut VulkanicGal, stage: ShaderStage, source: &str) -> GalResult<Handle> {
    gal.create_shader_module(ShaderModuleDesc {
        label: format!("bench-{stage:?}"),
        stage,
        code_format: ShaderCodeFormat::Glsl,
        code: source.as_bytes().to_vec(),
        entry_point: "main".into(),
    })
}

fn texture(
    gal: &mut VulkanicGal,
    width: u32,
    height: u32,
    usages: Vec<TextureUsage>,
) -> GalResult<(Handle, Handle)> {
    let texture = gal.create_texture(TextureDesc {
        label: "bench-texture".into(),
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        extent: Extent3d {
            width,
            height,
            depth: 1,
        },
        mip_levels: 1,
        array_layers: 1,
        usages,
    })?;
    let view = gal.create_texture_view(TextureViewDesc {
        label: "bench-view".into(),
        texture,
        format: TextureFormat::Rgba8Unorm,
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    })?;
    Ok((texture, view))
}

fn scene(gal: &mut VulkanicGal) -> GalResult<Scene> {
    let alignment = gal.capabilities().limits.uniform_buffer_offset_alignment;
    let (target_texture, target_view) =
        texture(gal, 1280, 720, vec![TextureUsage::ColorAttachment])?;
    let target = gal.create_render_target(RenderTargetDesc {
        label: "bench-target".into(),
        color_views: vec![target_view],
        depth_stencil_view: None,
        extent: Extent3d {
            width: 1280,
            height: 720,
            depth: 1,
        },
    })?;
    let pass = gal.create_render_pass(RenderPassDesc {
        label: "bench-pass".into(),
        target,
        color_formats: vec![TextureFormat::Rgba8Unorm],
        depth_format: None,
    })?;
    let (_, image_view) = texture(gal, 4, 4, vec![TextureUsage::Sampled])?;
    let sampler = gal.create_sampler(SamplerDesc {
        label: "bench-sampler".into(),
        min_filter: SamplerFilter::Linear,
        mag_filter: SamplerFilter::Linear,
        mip_filter: SamplerFilter::Nearest,
        address_u: SamplerAddressMode::ClampToEdge,
        address_v: SamplerAddressMode::ClampToEdge,
        address_w: SamplerAddressMode::ClampToEdge,
        comparison: None,
    })?;
    let combined = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
        label: "bench-combined".into(),
        texture_view: image_view,
        sampler,
    })?;
    let uniforms = gal.create_buffer(BufferDesc {
        label: "bench-uniforms".into(),
        size: alignment * MAX_DRAWS,
        memory: MemoryDomain::Upload,
        usages: vec![BufferUsage::Uniform, BufferUsage::HostWrite],
    })?;
    let storage = gal.create_buffer(BufferDesc {
        label: "bench-storage".into(),
        size: 64 * 1024,
        memory: MemoryDomain::Upload,
        usages: vec![BufferUsage::Storage, BufferUsage::HostWrite],
    })?;
    let upload = gal.create_buffer(BufferDesc {
        label: "bench-upload".into(),
        size: 300 * 200 * 4,
        memory: MemoryDomain::Upload,
        usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
    })?;
    let (upload_texture, _) = texture(
        gal,
        300,
        200,
        vec![TextureUsage::TransferDst, TextureUsage::Sampled],
    )?;
    let layout = gal.create_resource_layout(ResourceLayoutDesc {
        label: "bench-layout".into(),
        bindings: vec![
            binding(0, ResourceBindingKind::CombinedTextureSampler, 0),
            binding(1, ResourceBindingKind::StorageBuffer, 0),
            binding(2, ResourceBindingKind::UniformBuffer, 1),
        ],
    })?;
    let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
        label: "bench-pipeline-layout".into(),
        resource_layouts: vec![layout],
    })?;
    let mut uniform = bound(2, ResourceBindingKind::UniformBuffer, uniforms);
    uniform.dynamic_offsets = vec![0];
    uniform.buffer_range = Some(64);
    let set = gal.create_resource_set(ResourceSetDesc {
        label: "bench-set".into(),
        layout,
        bindings: vec![
            bound(0, ResourceBindingKind::CombinedTextureSampler, combined),
            bound(1, ResourceBindingKind::StorageBuffer, storage),
            uniform,
        ],
    })?;
    let vertex = shader(
        gal,
        ShaderStage::Vertex,
        "#version 450
        layout(set=0,binding=1,std430) readonly buffer G { vec4 v[]; } g;
        layout(set=0,binding=2,std140) uniform D { vec4 rect; } d;
        layout(location=0) out vec2 uv;
        void main() {
            vec2 p = vec2(gl_VertexIndex & 1, gl_VertexIndex >> 1);
            uv = p;
            gl_Position = vec4(mix(d.rect.xy, d.rect.zw, p) + g.v[0].xy * 0.0, 0.0, 1.0);
        }",
    )?;
    let fragment = shader(
        gal,
        ShaderStage::Fragment,
        "#version 450
        layout(set=0,binding=0) uniform sampler2D image;
        layout(location=0) in vec2 uv;
        layout(location=0) out vec4 color;
        void main() { color = texture(image, uv); }",
    )?;
    let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
        label: "bench-pipeline".into(),
        layout: pipeline_layout,
        vertex_shader: vertex,
        fragment_shader: fragment,
        topology: PrimitiveTopology::Triangles,
        cull_mode: CullMode::None,
        front_face: FrontFace::CounterClockwise,
        provoking_vertex: ProvokingVertex::First,
        raster_y_direction: RasterYDirection::Down,
        blend: BlendMode::Alpha,
        depth_compare: None,
        depth_write: false,
        depth_bias: None,
        color_formats: vec![TextureFormat::Rgba8Unorm],
        depth_format: None,
        stencil: None,
    })?;
    Ok(Scene {
        alignment,
        target,
        target_texture,
        target_view,
        pass,
        pipeline,
        pipeline_layout,
        set,
        uniforms,
        storage,
        upload,
        upload_texture,
    })
}

#[derive(Clone, Copy)]
enum Workload {
    Rebind(u32),
    Static(u32),
    Upload,
}

impl Workload {
    fn name(self) -> String {
        match self {
            Self::Rebind(draws) => format!("rebind-{draws}"),
            Self::Static(draws) => format!("static-{draws}"),
            Self::Upload => "upload".into(),
        }
    }

    fn operations(self, scene: &Scene, frame: usize) -> Vec<CommandOp> {
        let mut operations = Vec::new();
        let draws = match self {
            Self::Rebind(draws) | Self::Static(draws) => draws,
            Self::Upload => {
                for index in 0..64u64 {
                    operations.push(CommandOp::HostWriteBuffer {
                        buffer: scene.storage,
                        offset: index * 256,
                        data: vec![frame as u8; 256],
                    });
                }
                operations.push(CommandOp::HostWriteBuffer {
                    buffer: scene.upload,
                    offset: 0,
                    data: vec![frame as u8; 300 * 200 * 4],
                });
                operations.push(barrier(
                    scene.upload,
                    TextureUsageState::TransferDst,
                    TextureUsageState::TransferSrc,
                ));
                operations.push(barrier(
                    scene.upload_texture,
                    TextureUsageState::Undefined,
                    TextureUsageState::TransferDst,
                ));
                operations.push(CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                    buffer: scene.upload,
                    buffer_offset: 0,
                    bytes_per_row: 300 * 4,
                    rows_per_image: 200,
                    texture: scene.upload_texture,
                    texture_mip: 0,
                    texture_layer: 0,
                    texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent: Extent3d {
                        width: 300,
                        height: 200,
                        depth: 1,
                    },
                }));
                operations.push(barrier(
                    scene.upload_texture,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ));
                return operations;
            }
        };
        operations.extend([
            CommandOp::HostWriteBuffer {
                buffer: scene.uniforms,
                offset: 0,
                data: vec![frame as u8; (scene.alignment * u64::from(draws)) as usize],
            },
            CommandOp::HostWriteBuffer {
                buffer: scene.storage,
                offset: 0,
                data: vec![0; 4096],
            },
            CommandOp::HostWriteBuffer {
                buffer: scene.storage,
                offset: 8192,
                data: vec![0; 4096],
            },
            barrier(
                scene.uniforms,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ),
            barrier(
                scene.storage,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ),
            barrier(
                scene.target_texture,
                TextureUsageState::Undefined,
                TextureUsageState::ColorAttachment,
            ),
            CommandOp::BeginPass {
                pass: scene.pass,
                target: scene.target,
                colors: vec![PassAttachment {
                    view: scene.target_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: Some(ClearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                }],
                depth_stencil: None,
            },
            CommandOp::BindGraphicsPipeline(scene.pipeline),
        ]);
        for draw in 0..u64::from(draws) {
            let rebind = matches!(self, Self::Rebind(_)) || draw == 0;
            if rebind {
                operations.push(CommandOp::BindResourceSet {
                    pipeline_layout: scene.pipeline_layout,
                    set_index: 0,
                    set: scene.set,
                    dynamic_offsets: vec![scene.alignment * draw].into(),
                });
            }
            operations.push(CommandOp::Draw {
                vertices: 4,
                instances: 1,
            });
        }
        operations.push(CommandOp::EndPass);
        operations
    }
}

fn quartile(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 4]
}

/// 25th-percentile microseconds: record, submit wall, frontend, encode, queue.
fn measure(gal: &mut VulkanicGal, scene: &Scene, workload: Workload) -> GalResult<[f64; 5]> {
    let mut record = Vec::with_capacity(FRAMES);
    let mut submit = Vec::with_capacity(FRAMES);
    let (mut frontend, mut encode, mut queue) = (Vec::new(), Vec::new(), Vec::new());
    for profiled in [false, true] {
        for frame in 0..WARMUP + FRAMES {
            let operations = workload.operations(scene, frame);
            let started = std::time::Instant::now();
            let list = gal.create_command_list(CommandListDesc {
                label: "bench-frame".into(),
                operations,
            })?;
            let recorded = started.elapsed();
            let batch = SubmissionBatch {
                label: "bench-frame".into(),
                command_lists: vec![list],
            };
            let mut profile = SubmitProfile::default();
            let started = std::time::Instant::now();
            let token = if profiled {
                gal.submit_profiled(batch, &mut profile)?
            } else {
                gal.submit(batch)?
            };
            let submitted = started.elapsed();
            gal.retire_through(token.submission)?;
            if frame < WARMUP {
                continue;
            }
            let micros = |nanos: u64| nanos as f64 / 1000.0;
            if profiled {
                let backend = profile.backend_encode_nanos + profile.backend_submit_nanos;
                frontend.push(micros(
                    profile.gal_submit_total_nanos.saturating_sub(backend),
                ));
                encode.push(micros(profile.backend_encode_nanos));
                queue.push(micros(profile.backend_submit_nanos));
            } else {
                record.push(recorded.as_secs_f64() * 1e6);
                submit.push(submitted.as_secs_f64() * 1e6);
            }
        }
    }
    Ok([
        quartile(&mut record),
        quartile(&mut submit),
        quartile(&mut frontend),
        quartile(&mut encode),
        quartile(&mut queue),
    ])
}

fn main() {
    let requested: Vec<String> = std::env::args().skip(1).collect();
    let backends: Vec<GraphicsBackend> = if requested.is_empty() {
        GraphicsBackend::selectable(false)
            .into_iter()
            .filter(|backend| {
                !matches!(
                    backend,
                    GraphicsBackend::Auto
                        | GraphicsBackend::OpenGl
                        | GraphicsBackend::Wgpu(WgpuApi::Auto)
                )
            })
            .collect()
    } else {
        requested
            .iter()
            .map(|id| id.parse().unwrap_or_else(|error| panic!("{error}")))
            .collect()
    };
    let workloads = [
        Workload::Rebind(145),
        Workload::Rebind(1000),
        Workload::Static(1000),
        Workload::Upload,
    ];
    println!(
        "{:<14} {:<12} {:>9} {:>9} {:>9} {:>9} {:>9} {:>10}",
        "backend", "workload", "record", "submit", "frontend", "encode", "queue", "total"
    );
    for backend in backends {
        let mut gal = match VulkanicGal::create_with(GalOptions::new(backend), "submit bench") {
            Ok(gal) => gal,
            Err(error) => {
                println!("{:<14} unavailable: {error}", backend.id());
                continue;
            }
        };
        eprintln!("{}", gal.backend_info());
        let scene = scene(&mut gal).expect("bench scene");
        for workload in workloads {
            let [record, submit, frontend, encode, queue] =
                measure(&mut gal, &scene, workload).expect("bench workload");
            println!(
                "{:<14} {:<12} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>10.1}",
                backend.id(),
                workload.name(),
                record,
                submit,
                frontend,
                encode,
                queue,
                record + submit
            );
        }
    }
}
