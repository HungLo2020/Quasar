//! Supplemental GPU observation using exact r526 game VS output bits.
//! This diagnostic does not admit visual parity or substitute for Frozen.
use crate::commands::*;
use crate::gal::VulkanicGal;
use crate::resources::*;

#[test]
#[ignore = "opt-in numeric diagnostic, not a visual acceptance gate"]
fn observe_frozen_triangle_interpolation_and_y_reflection() {
    for opengl in [false, true] {
        for mirrored in [false, true] {
            for rotation in 0..3 {
                for expression in ["v_uv.x", "v_uv.y", "gl_FragCoord.w", "gl_FragCoord.z"] {
                    let bits = observe(opengl, mirrored, rotation, expression, CullMode::None);
                    eprintln!("terrain-interpolation-observation backend={} mirrored={mirrored} rotation={rotation} value={expression} bits={bits:#010x} float={}",
                    if opengl { "opengl" } else { "vulkan" }, f32::from_bits(bits));
                    assert!(f32::from_bits(bits).is_finite());
                    assert!(
                        (0.0..1.0).contains(&f32::from_bits(bits)),
                        "must read covered triangle, not clear/background"
                    );
                }
            }
        }
    }
}

#[test]
fn explicit_raster_y_direction_preserves_culling_and_observes_frozen_orientation() {
    let mut visible = Vec::new();
    for opengl in [false, true] {
        let mut modes = Vec::new();
        for mirrored in [false, true] {
            let none = observe(opengl, mirrored, 0, "v_uv.y", CullMode::None);
            let back = observe(opengl, mirrored, 0, "v_uv.y", CullMode::Back);
            let front = observe(opengl, mirrored, 0, "v_uv.y", CullMode::Front);
            assert_ne!(none, 0);
            assert_ne!(
                back == 0,
                front == 0,
                "exactly one cull face must retain the triangle"
            );
            assert_eq!(if back != 0 { back } else { front }, none);
            modes.push((none, back != 0));
        }
        assert_eq!(
            modes[0].1, modes[1].1,
            "Y direction must preserve logical front-face meaning"
        );
        visible.push(modes);
    }
    assert_eq!(
        visible[0][1].0, visible[1][0].0,
        "explicit Vulkan Down must match measured Frozen/OpenGL orientation on this fixture"
    );
}

fn observe(
    opengl: bool,
    mirrored: bool,
    rotation: u32,
    expression: &str,
    cull_mode: CullMode,
) -> u32 {
    let mut gal = if opengl {
        VulkanicGal::new_with_backend(Box::new(
            super::opengl::OpenGlBackend::new("terrain interpolation observation").unwrap(),
        ))
    } else {
        VulkanicGal::new_with_backend(Box::new(
            super::vulkan::VulkanBackend::new("terrain interpolation observation").unwrap(),
        ))
    };
    let mut owned = Vec::new();
    macro_rules! own {
        ($call:expr) => {{
            let h = $call.unwrap();
            owned.push(h);
            h
        }};
    }
    let extent = Extent3d {
        width: 1280,
        height: 720,
        depth: 1,
    };
    let color = own!(gal.create_texture(TextureDesc {
        label: "interpolation.color".into(),
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        extent,
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc]
    }));
    let view = own!(gal.create_texture_view(TextureViewDesc {
        label: "interpolation.view".into(),
        texture: color,
        format: TextureFormat::Rgba8Unorm,
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1
    }));
    let target = own!(gal.create_render_target(RenderTargetDesc {
        label: "interpolation.target".into(),
        color_views: vec![view],
        depth_stencil_view: None,
        extent
    }));
    let pass = own!(gal.create_render_pass(RenderPassDesc {
        label: "interpolation.pass".into(),
        target,
        color_formats: vec![TextureFormat::Rgba8Unorm],
        depth_format: None
    }));
    let layout = own!(gal.create_pipeline_layout(PipelineLayoutDesc {
        label: "interpolation.layout".into(),
        resource_layouts: vec![]
    }));
    let id = format!(
        "({}+{rotation})%3",
        if opengl {
            "gl_VertexID"
        } else {
            "gl_VertexIndex"
        }
    );
    // Actual second front-face triangle: vertex indices 2,3,0. All values below
    // were measured equal on Rust Vulkan and Frozen OpenGL (r526), not CPU math.
    let vertex = format!(
        r#"#version 450
layout(location=0) out vec2 v_uv;
const uvec4 p[3] = uvec4[3](
    uvec4(1058089181u,3189729878u,1076964127u,1077382980u),
    uvec4(1058089181u,1066044388u,1076235759u,1076654648u),
    uvec4(3186467614u,1065570418u,1075166633u,1075585574u));
const uvec2 t[3] = uvec2[3](uvec2(1054998560u,1062469600u),
    uvec2(1054998560u,1062338592u),uvec2(1055129568u,1062338592u));
void main() {{
    gl_Position=uintBitsToFloat(p[{id}]);
    v_uv=uintBitsToFloat(t[{id}]);
    {depth}
    {mirror}
}}
"#,
        depth = if opengl {
            ""
        } else {
            "gl_Position.z=gl_Position.z*0.5+gl_Position.w*0.5;"
        },
        mirror = ""
    );
    let vertex_shader = own!(gal.create_shader_module(ShaderModuleDesc {
        label: "interpolation.vertex".into(),
        stage: ShaderStage::Vertex,
        code_format: ShaderCodeFormat::Glsl,
        code: vertex.into_bytes(),
        entry_point: "main".into()
    }));
    let fragment = format!(
        r#"#version 450
layout(location=0) in vec2 v_uv;
layout(location=0) out vec4 color;
void main() {{ uint w=floatBitsToUint({expression}); color=vec4(uvec4(w,w>>8u,w>>16u,w>>24u)&255u)/255.0; }}
"#
    );
    let fragment_shader = own!(gal.create_shader_module(ShaderModuleDesc {
        label: "interpolation.fragment".into(),
        stage: ShaderStage::Fragment,
        code_format: ShaderCodeFormat::Glsl,
        code: fragment.into_bytes(),
        entry_point: "main".into()
    }));
    let pipeline = own!(gal.create_graphics_pipeline(GraphicsPipelineDesc {
        label: "interpolation.pipeline".into(),
        layout,
        vertex_shader,
        fragment_shader,
        topology: PrimitiveTopology::Triangles,
        cull_mode,
        front_face: FrontFace::CounterClockwise,
        provoking_vertex: ProvokingVertex::Last,
        raster_y_direction: if mirrored {
            RasterYDirection::Down
        } else {
            RasterYDirection::Up
        },
        blend: BlendMode::Disabled,
        depth_compare: None,
        depth_write: false,
        depth_bias: None,
        color_formats: vec![TextureFormat::Rgba8Unorm],
        depth_format: None,
        stencil: None
    }));
    let readback = own!(gal.create_buffer(BufferDesc {
        label: "interpolation.readback".into(),
        size: 4,
        memory: MemoryDomain::Readback,
        usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead]
    }));
    let barrier = |resource, before, after, subresources| {
        CommandOp::Barrier(ResourceBarrier {
            resource,
            before,
            after,
            subresources,
            src_queue: QueueClass::Graphics,
            dst_queue: QueueClass::Graphics,
        })
    };
    let range = Some(TextureSubresourceRange {
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    });
    let mut operations = vec![
        barrier(
            color,
            TextureUsageState::Undefined,
            TextureUsageState::ColorAttachment,
            range,
        ),
        CommandOp::BeginPass {
            pass,
            target,
            colors: vec![PassAttachment {
                view,
                load_op: AttachmentLoadOp::Clear,
                store_op: AttachmentStoreOp::Store,
                clear_color: Some(ClearColor {
                    r: 0.,
                    g: 0.,
                    b: 0.,
                    a: 0.,
                }),
            }],
            depth_stencil: None,
        },
        CommandOp::BindGraphicsPipeline(pipeline),
        CommandOp::Draw {
            vertices: 3,
            instances: 1,
        },
        CommandOp::EndPass,
        barrier(
            color,
            TextureUsageState::ColorAttachment,
            TextureUsageState::TransferSrc,
            range,
        ),
    ];
    // Vulkan owns the complete raster-plus-row-copy prerequisite. Rust OpenGL
    // does not advertise row reversal; its direction is observed at the reflected
    // coordinate without submitting an unsupported operation or a fallback.
    let canonical_color = if mirrored && !opengl {
        let canonical = own!(gal.create_texture(TextureDesc {
            label: "interpolation.canonical-color".into(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::TransferDst, TextureUsage::TransferSrc]
        }));
        operations.push(barrier(
            canonical,
            TextureUsageState::Undefined,
            TextureUsageState::TransferDst,
            range,
        ));
        operations.push(CommandOp::CopyTexture(TextureImageCopyRegion {
            row_order: TextureRowOrder::Reverse,
            src_texture: color,
            src_mip: 0,
            src_layer: 0,
            src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            dst_texture: canonical,
            dst_mip: 0,
            dst_layer: 0,
            dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            extent,
        }));
        operations.push(barrier(
            canonical,
            TextureUsageState::TransferDst,
            TextureUsageState::TransferSrc,
            range,
        ));
        canonical
    } else {
        color
    };
    operations.extend([
        CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
            buffer: readback,
            buffer_offset: 0,
            bytes_per_row: 4,
            rows_per_image: 1,
            texture: canonical_color,
            texture_mip: 0,
            texture_layer: 0,
            texture_origin: TextureOrigin3d {
                x: 764,
                y: if mirrored && opengl { 399 } else { 320 },
                z: 0,
            },
            extent: Extent3d {
                width: 1,
                height: 1,
                depth: 1,
            },
        }),
        barrier(
            readback,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
            None,
        ),
        CommandOp::HostReadBuffer {
            buffer: readback,
            offset: 0,
            size: 4,
        },
    ]);
    let list = gal
        .create_command_list(CommandListDesc {
            label: "interpolation.commands".into(),
            operations,
        })
        .unwrap();
    let token = gal
        .submit(SubmissionBatch {
            label: "interpolation.submit".into(),
            command_lists: vec![list],
        })
        .unwrap();
    gal.retire_through(token.submission).unwrap();
    let reads = gal.completed_host_reads();
    let bytes = &reads.iter().find(|r| r.buffer == readback).unwrap().bytes;
    let word = u32::from_le_bytes(bytes.as_slice().try_into().unwrap());
    for handle in owned.into_iter().rev() {
        gal.destroy(handle).unwrap();
    }
    assert_eq!(gal.metrics().validation_failures, 0);
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
    word
}
