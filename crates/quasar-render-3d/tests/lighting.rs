#![cfg(feature = "vulkan")]
use quasar_frame::OffscreenTarget;
use quasar_render_3d::*;
use vulkanic_gal::commands::BufferImageCopyRegion;
use vulkanic_gal::{resources::*, *};

fn plane(gal: &mut VulkanicGal) -> Mesh {
    let vertices =
        [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.5; 2],
        });
    Mesh::upload(gal, &vertices, &[0, 1, 2, 0, 2, 3]).unwrap()
}
fn camera() -> LitCamera3D {
    LitCamera3D {
        view_projection: Mat4::perspective(1., 1., 0.1, 20.).unwrap()
            * Mat4::translation(0., 0., -3.),
        position: [0., 0., 3.],
    }
}
fn center(pixels: &[u8]) -> [u8; 4] {
    pixels[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4]
        .try_into()
        .unwrap()
}
#[test]
#[ignore = "requires Vulkan"]
fn lights_materials_composition_and_retirement() {
    lighting_composition_suite(SubmissionConfig::default());
}

#[test]
#[ignore = "requires GPU"]
fn instanced_lit_and_legacy_paths_compose_with_direct_gal_and_2d() {
    lighting_composition_suite(SubmissionConfig {
        instancing: true,
        ..SubmissionConfig::default()
    });
}

fn lighting_composition_suite(submission: SubmissionConfig) {
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "lit renderer tests").unwrap();
    let mut renderer = LitRenderState3D::with_submission(&mut gal, 8, submission).unwrap();
    let mut legacy = RenderState3D::with_submission(&mut gal, submission).unwrap();
    let mut mesh = plane(&mut gal);
    let (texture, view) = upload_white(&mut gal).unwrap();
    let mut target = OffscreenTarget::new(&mut gal, 64, 64).unwrap();
    let mut material = LitMaterial::textured(view);
    material.shading = ShadingModel::Diffuse;
    let mut render = |gal: &mut VulkanicGal,
                      renderer: &mut LitRenderState3D,
                      material: LitMaterial,
                      lights: &[Light],
                      ambient: [f32; 3]| {
        target
            .render(gal, |gal, target| {
                Ok(vec![
                    renderer
                        .prepare(
                            gal,
                            target,
                            camera(),
                            Lighting {
                                ambient,
                                lights,
                                ..Lighting::default()
                            },
                            &[LitDraw3D {
                                mesh: &mesh,
                                model: Mat4::IDENTITY,
                                material,
                            }],
                            PassOptions {
                                clear_color: Some([0., 0., 0., 1.]),
                                clear_depth: true,
                            },
                        )?
                        .commands,
                ])
            })
            .unwrap()
            .rgba
    };
    let point = Light::Point {
        position: [0., 0., 2.],
        color: [1., 0., 0.],
        intensity: 1.,
        range: 4.,
        attenuation: Attenuation::Smooth,
    };
    let red = render(&mut gal, &mut renderer, material, &[point], [0.; 3]);
    let c = center(&red);
    assert!(c[0] > 100 && c[1] == 0 && c[2] == 0, "{c:?}");
    let edge = red[(32 * 64 + 16) * 4];
    assert!(
        c[0] > edge,
        "point light must vary per fragment: {c:?}, edge {edge}"
    );
    let directional = Light::Directional {
        direction: [0., 0., -1.],
        color: [0., 1., 0.],
        intensity: 0.5,
    };
    let c = center(&render(
        &mut gal,
        &mut renderer,
        material,
        &[point, directional],
        [0.; 3],
    ));
    assert!(c[0] > 100 && c[1] > 100 && c[2] == 0, "{c:?}");
    let spot = |direction| Light::Spot {
        position: [0., 0., 2.],
        direction,
        color: [1.; 3],
        intensity: 1.,
        range: 4.,
        inner_angle: 0.1,
        outer_angle: 0.3,
        attenuation: Attenuation::Smooth,
    };
    assert!(
        center(&render(
            &mut gal,
            &mut renderer,
            material,
            &[spot([0., 0., -1.])],
            [0.; 3]
        ))[0]
            > 100
    );
    assert_eq!(
        center(&render(
            &mut gal,
            &mut renderer,
            material,
            &[spot([0., 0., 1.])],
            [0.; 3]
        ))[0],
        0
    );
    material.shading = ShadingModel::Unlit;
    material.base.tint = [0.4, 0.2, 0.1, 1.];
    let unlit = render(&mut gal, &mut renderer, material, &[], [0.; 3]);
    let lit = render(
        &mut gal,
        &mut renderer,
        material,
        &[point, directional],
        [10.; 3],
    );
    assert_eq!(unlit, lit);
    material.shading = ShadingModel::BlinnPhong;
    material.base.tint = [0.; 4];
    material.base.tint[3] = 1.;
    material.specular = [1.; 3];
    assert!(
        center(&render(
            &mut gal,
            &mut renderer,
            material,
            &[Light::Directional {
                direction: [0., 0., -1.],
                color: [1.; 3],
                intensity: 1.
            }],
            [0.; 3]
        ))[0]
            > 200
    );
    material.shading = ShadingModel::Unlit;
    material.emissive = [0.2, 0.4, 0.6];
    let c = center(&render(&mut gal, &mut renderer, material, &[], [0.; 3]));
    assert!(c[2] > c[1] && c[1] > c[0]);
    material.alpha_cutoff = 0.5;
    material.base.tint[3] = 0.;
    assert_eq!(
        center(&render(&mut gal, &mut renderer, material, &[], [0.; 3])),
        [0, 0, 0, 255]
    );
    // Legacy shader output still composes with opt-in lighting, direct GAL and 2D.
    let mut two = quasar_render_2d::RenderState2D::new(&mut gal).unwrap();
    let mut canvas = quasar_render_2d::Canvas::new(64, 64).unwrap();
    canvas.rect(
        quasar_render_2d::Rect::new(0., 0., 8., 8.),
        quasar_render_2d::Color([255, 0, 255, 255]),
    );
    let image = target
        .render(&mut gal, |gal, target| {
            let a = legacy.prepare(
                gal,
                target,
                Camera3D {
                    view_projection: camera().view_projection,
                    light_direction: [0., 0., 1.],
                },
                &[Draw3D {
                    mesh: &mesh,
                    model: Mat4::IDENTITY,
                    material: Material::textured(view),
                }],
                PassOptions::default(),
            )?;
            let direct = gal.create_command_list(CommandListDesc {
                label: "custom GAL barrier".into(),
                operations: vec![barrier(
                    gal.pass_target_color_attachment(target)?,
                    TextureUsageState::ColorAttachment,
                    TextureUsageState::ColorAttachment,
                )],
            })?;
            let b = renderer.prepare(
                gal,
                target,
                camera(),
                Lighting::default(),
                &[],
                PassOptions {
                    clear_color: None,
                    clear_depth: false,
                },
            )?;
            let c = two.prepare(
                gal,
                &canvas,
                &quasar_render_2d::Textures::default(),
                target,
                quasar_render_2d::DisplayOptions::default(),
                quasar_render_2d::LoadOp::Load,
            )?;
            Ok(vec![a.commands, direct, b.commands, c.commands])
        })
        .unwrap();
    assert_eq!(&image.rgba[..4], &[255, 0, 255, 255]);
    assert_eq!(center(&image.rgba), [255; 4]);
    // Pending command receipts protect both light slots and mesh data.
    let pending = renderer
        .prepare(
            &mut gal,
            target.target(),
            camera(),
            Lighting::default(),
            &[LitDraw3D {
                mesh: &mesh,
                model: Mat4::IDENTITY,
                material: LitMaterial::textured(view),
            }],
            PassOptions::default(),
        )
        .unwrap();
    assert!(renderer.release(&mut gal).is_err());
    assert!(mesh.release(&mut gal).is_err());
    drop(pending);
    two.release(&mut gal).unwrap();
    renderer.release(&mut gal).unwrap();
    legacy.release(&mut gal).unwrap();
    mesh.release(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    gal.retire(view).unwrap();
    gal.retire(texture).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
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
fn upload_white(gal: &mut VulkanicGal) -> GalResult<(Handle, Handle)> {
    upload_color(gal, [255; 4])
}
fn upload_color(gal: &mut VulkanicGal, color: [u8; 4]) -> GalResult<(Handle, Handle)> {
    let size = Extent3d {
        width: 1,
        height: 1,
        depth: 1,
    };
    let texture = gal.create_texture(TextureDesc {
        label: "playground-white".into(),
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        extent: size,
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
    })?;
    let view = match gal.create_texture_view(TextureViewDesc {
        label: "playground-white-view".into(),
        texture,
        format: TextureFormat::Rgba8Unorm,
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    }) {
        Ok(view) => view,
        Err(error) => {
            let _ = gal.retire(texture);
            return Err(error);
        }
    };
    let upload = match gal.create_buffer(BufferDesc {
        label: "playground-white-upload".into(),
        size: 4,
        memory: MemoryDomain::Upload,
        usages: vec![BufferUsage::TransferSrc, BufferUsage::HostWrite],
    }) {
        Ok(upload) => upload,
        Err(error) => {
            let _ = gal.retire(view);
            let _ = gal.retire(texture);
            return Err(error);
        }
    };
    let result = gal.submit(SubmissionBatch {
        label: "playground-white-upload".into(),
        command_lists: vec![CommandList::from(CommandListDesc {
            label: "playground-white-upload".into(),
            operations: vec![
                CommandOp::HostWriteBuffer {
                    buffer: upload,
                    offset: 0,
                    data: color.to_vec(),
                },
                CommandOp::Barrier(ResourceBarrier {
                    resource: upload,
                    subresources: None,
                    before: TextureUsageState::TransferDst,
                    after: TextureUsageState::TransferSrc,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: None,
                    before: TextureUsageState::Undefined,
                    after: TextureUsageState::TransferDst,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
                CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                    buffer: upload,
                    buffer_offset: 0,
                    bytes_per_row: 4,
                    rows_per_image: 1,
                    texture,
                    texture_mip: 0,
                    texture_layer: 0,
                    texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent: size,
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: None,
                    before: TextureUsageState::TransferDst,
                    after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
            ],
        })],
    });
    let _ = gal.retire(upload);
    if let Err(error) = result {
        let _ = gal.retire(view);
        let _ = gal.retire(texture);
        return Err(error);
    }
    Ok((texture, view))
}

#[test]
#[ignore = "requires Vulkan"]
fn lit_slots_preserve_out_of_order_frames_and_reject_overflow() {
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "lit slot tests").unwrap();
    let mut renderer = LitRenderState3D::new(&mut gal, 1).unwrap();
    let mut mesh = plane(&mut gal);
    let (texture, view) = upload_white(&mut gal).unwrap();
    let mut target = OffscreenTarget::new(&mut gal, 64, 64).unwrap();
    let mut material = LitMaterial::textured(view);
    material.shading = ShadingModel::Diffuse;
    let draws = [LitDraw3D {
        mesh: &mesh,
        model: Mat4::IDENTITY,
        material,
    }];
    let make_light = |color| Light::Directional {
        direction: [0., 0., -1.],
        color,
        intensity: 1.,
    };
    assert!(
        renderer
            .prepare(
                &mut gal,
                target.target(),
                camera(),
                Lighting {
                    lights: &[make_light([1.; 3]); 2],
                    ..Lighting::default()
                },
                &draws,
                PassOptions::default()
            )
            .is_err()
    );
    assert!(
        renderer
            .prepare(
                &mut gal,
                target.target(),
                LitCamera3D {
                    position: [f32::NAN; 3],
                    ..camera()
                },
                Lighting::default(),
                &draws,
                PassOptions::default()
            )
            .is_err()
    );
    let mut prepared = Vec::new();
    for color in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] {
        prepared.push(
            renderer
                .prepare(
                    &mut gal,
                    target.target(),
                    camera(),
                    Lighting {
                        ambient: [0.; 3],
                        lights: &[make_light(color)],
                        ..Lighting::default()
                    },
                    &draws,
                    PassOptions::default(),
                )
                .unwrap(),
        );
    }
    assert!(
        renderer
            .prepare(
                &mut gal,
                target.target(),
                camera(),
                Lighting::default(),
                &draws,
                PassOptions::default()
            )
            .is_err()
    );
    // Submit in reverse preparation order: light upload slots must stay isolated.
    for expected in [[0, 0, 255, 255], [0, 255, 0, 255], [255, 0, 0, 255]] {
        let frame = prepared.pop().unwrap();
        let image = target
            .render(&mut gal, |_, _| Ok(vec![frame.commands.clone()]))
            .unwrap();
        assert_eq!(center(&image.rgba), expected);
    }
    // Reuse completed slots with no resource churn after warmup.
    let before = gal.metrics().resource_creates;
    for _ in 0..5 {
        target
            .render(&mut gal, |gal, target| {
                Ok(vec![
                    renderer
                        .prepare(
                            gal,
                            target,
                            camera(),
                            Lighting::default(),
                            &draws,
                            PassOptions::default(),
                        )?
                        .commands,
                ])
            })
            .unwrap();
    }
    assert_eq!(gal.metrics().resource_creates, before);
    renderer.release(&mut gal).unwrap();
    mesh.release(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    gal.retire(view).unwrap();
    gal.retire(texture).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn color_encoding_custom_shaders_and_normal_transforms_are_explicit() {
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "lit color tests").unwrap();
    let mut renderer = LitRenderState3D::new(&mut gal, 1).unwrap();
    let vertices =
        [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|position| Vertex {
            position,
            normal: [1., 0., 1.],
            uv: [0.5; 2],
        });
    let mut mesh = Mesh::upload(&mut gal, &vertices, &[0, 1, 2, 0, 2, 3]).unwrap();
    let (texture, view) = upload_white(&mut gal).unwrap();
    let mut target = OffscreenTarget::new(&mut gal, 64, 64).unwrap();
    let mut material = LitMaterial::textured(view);
    material.shading = ShadingModel::Diffuse;
    let result = target
        .render(&mut gal, |gal, target| {
            Ok(vec![
                renderer
                    .prepare(
                        gal,
                        target,
                        camera(),
                        Lighting {
                            ambient: [0.; 3],
                            lights: &[Light::Directional {
                                direction: [0., 0., -1.],
                                color: [1.; 3],
                                intensity: 1.,
                            }],
                            ..Lighting::default()
                        },
                        &[LitDraw3D {
                            mesh: &mesh,
                            model: Mat4::scale(2., 1., 1.),
                            material,
                        }],
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap();
    assert!(
        (center(&result.rgba)[0] as i32 - 228).abs() <= 2,
        "inverse transpose normals: {:?}",
        center(&result.rgba)
    );
    material.shading = ShadingModel::Unlit;
    material.base.tint = [0.21404114; 4];
    material.base.tint[3] = 1.;
    let result = target
        .render(&mut gal, |gal, target| {
            Ok(vec![
                renderer
                    .prepare(
                        gal,
                        target,
                        camera(),
                        Lighting {
                            output_color_space: ColorSpace::Srgb,
                            ..Lighting::default()
                        },
                        &[LitDraw3D {
                            mesh: &mesh,
                            model: Mat4::IDENTITY,
                            material,
                        }],
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap();
    assert!((center(&result.rgba)[0] as i32 - 128).abs() <= 1);
    material.base.transparent = true;
    assert!(
        renderer
            .prepare(
                &mut gal,
                target.target(),
                camera(),
                Lighting {
                    output_color_space: ColorSpace::Srgb,
                    ..Lighting::default()
                },
                &[LitDraw3D {
                    mesh: &mesh,
                    model: Mat4::IDENTITY,
                    material
                }],
                PassOptions::default()
            )
            .is_err()
    );
    let (gray_texture, gray_view) = upload_color(&mut gal, [128; 4]).unwrap();
    let mut gray = LitMaterial::textured(gray_view);
    gray.shading = ShadingModel::Unlit;
    gray.texture_color_space = ColorSpace::Srgb;
    for (encoding, expected) in [(ColorSpace::Linear, 55u8), (ColorSpace::Srgb, 128u8)] {
        let result = target
            .render(&mut gal, |gal, target| {
                Ok(vec![
                    renderer
                        .prepare(
                            gal,
                            target,
                            camera(),
                            Lighting {
                                output_color_space: encoding,
                                ..Lighting::default()
                            },
                            &[LitDraw3D {
                                mesh: &mesh,
                                model: Mat4::IDENTITY,
                                material: gray,
                            }],
                            PassOptions::default(),
                        )?
                        .commands,
                ])
            })
            .unwrap();
        assert_eq!(
            center(&result.rgba),
            [expected, expected, expected, 128],
            "sRGB decode/encode must preserve linear alpha"
        );
    }
    // Full custom fragment variants retain the ordinary vertex/resource ABI.
    let source = "#version 450\nlayout(location=0) out vec4 outputColor;\nvoid main(){outputColor=vec4(0,0,1,1);}";
    let mut custom = LitRenderState3D::with_fragment_shader(&mut gal, 0, source).unwrap();
    material.base.transparent = false;
    let result = target
        .render(&mut gal, |gal, target| {
            Ok(vec![
                custom
                    .prepare(
                        gal,
                        target,
                        camera(),
                        Lighting::default(),
                        &[LitDraw3D {
                            mesh: &mesh,
                            model: Mat4::IDENTITY,
                            material,
                        }],
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap();
    assert_eq!(center(&result.rgba), [0, 0, 255, 255]);
    custom.release(&mut gal).unwrap();
    renderer.release(&mut gal).unwrap();
    mesh.release(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    gal.retire(gray_view).unwrap();
    gal.retire(gray_texture).unwrap();
    gal.retire(view).unwrap();
    gal.retire(texture).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}

#[cfg(feature = "opengl")]
#[test]
#[ignore = "requires an OpenGL context"]
fn opengl_capability_mismatch_is_an_explicit_error() {
    let mut gal =
        VulkanicGal::create(BackendChoice::OpenGl, "lit OpenGL capability check").unwrap();
    let conventions = gal.capabilities().shader_conventions;
    let compatible = conventions.glsl_dialect == GlslDialect::ExplicitBindings
        && conventions.zero_to_one_clip_depth;
    match LitRenderState3D::new(&mut gal, 8) {
        Ok(mut renderer) => {
            assert!(compatible);
            renderer.release(&mut gal).unwrap();
        }
        Err(error) => {
            assert!(!compatible);
            assert_eq!(error.code, StatusCode::UnsupportedFeature);
        }
    }
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}
