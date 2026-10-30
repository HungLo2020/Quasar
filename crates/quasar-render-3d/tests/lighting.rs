use quasar_frame::{Gpu, GraphicsResult};

use quasar_frame::OffscreenTarget;
use quasar_render_3d::*;

use vulkanic_gal::*;

fn plane(gal: &mut Gpu) -> Mesh {
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
    let mut gal = Gpu::headless().unwrap();
    let mut renderer = LitRenderState3D::with_submission(&gal, 8, submission).unwrap();
    let mut legacy = RenderState3D::with_submission(&gal, submission).unwrap();
    let mut mesh = plane(&mut gal);
    let (_texture, view) = upload_white(&gal).unwrap();
    let target = OffscreenTarget::new(&gal, 64, 64).unwrap();
    let mut material = LitMaterial::textured(&view);
    material.shading = ShadingModel::Diffuse;
    let render = |gal: &mut Gpu,
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
                                material: material.clone(),
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
    let red = render(&mut gal, &mut renderer, material.clone(), &[point], [0.; 3]);
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
        material.clone(),
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
            material.clone(),
            &[spot([0., 0., -1.])],
            [0.; 3]
        ))[0]
            > 100
    );
    assert_eq!(
        center(&render(
            &mut gal,
            &mut renderer,
            material.clone(),
            &[spot([0., 0., 1.])],
            [0.; 3]
        ))[0],
        0
    );
    material.shading = ShadingModel::Unlit;
    material.base.tint = [0.4, 0.2, 0.1, 1.];
    let unlit = render(&mut gal, &mut renderer, material.clone(), &[], [0.; 3]);
    let lit = render(
        &mut gal,
        &mut renderer,
        material.clone(),
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
            material.clone(),
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
    let c = center(&render(
        &mut gal,
        &mut renderer,
        material.clone(),
        &[],
        [0.; 3],
    ));
    assert!(c[2] > c[1] && c[1] > c[0]);
    material.alpha_cutoff = 0.5;
    material.base.tint[3] = 0.;
    assert_eq!(
        center(&render(
            &mut gal,
            &mut renderer,
            material.clone(),
            &[],
            [0.; 3]
        )),
        [0, 0, 0, 255]
    );
    // Legacy shader output still composes with opt-in lighting, direct GAL and 2D.
    let mut two = quasar_render_2d::RenderState2D::new(&gal).unwrap();
    let mut canvas = quasar_render_2d::Canvas::new(64, 64).unwrap();
    canvas.rect(
        quasar_render_2d::Rect::new(0., 0., 8., 8.),
        quasar_render_2d::Color([255, 0, 255, 255]),
    );
    let image = target
        .render(&gal, |gal, target| {
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
                    material: Material::textured(&view),
                }],
                PassOptions::default(),
            )?;
            let mut encoder = gal.encoder("direct native pass");
            {
                let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("direct load"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &target.color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: vulkanic_gal::LoadOp::Load,
                            store: StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            }
            let direct = encoder.finish();
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
    // Native encoded commands retain light and mesh buffers after owner release.
    let pending = renderer
        .prepare(
            &gal,
            target.target(),
            camera(),
            Lighting::default(),
            &[LitDraw3D {
                mesh: &mesh,
                model: Mat4::IDENTITY,
                material: LitMaterial::textured(&view),
            }],
            PassOptions::default(),
        )
        .unwrap();
    renderer.release(&gal).unwrap();
    mesh.release(&gal).unwrap();
    drop(pending);
    two.release(&gal).unwrap();
    renderer.release(&gal).unwrap();
    legacy.release(&gal).unwrap();
    mesh.release(&gal).unwrap();
    target.release(&gal).unwrap();

    gal.wait().unwrap();
}
mod support;
fn upload_white(gpu: &Gpu) -> GraphicsResult<(Texture, TextureView)> {
    support::upload_color(gpu, [255; 4])
}
fn upload_color(gpu: &Gpu, color: [u8; 4]) -> GraphicsResult<(Texture, TextureView)> {
    support::upload_color(gpu, color)
}
#[test]
#[ignore = "requires Vulkan"]
fn lit_slots_preserve_out_of_order_frames_and_reject_overflow() {
    let mut gal = Gpu::headless().unwrap();
    let mut renderer = LitRenderState3D::new(&gal, 1).unwrap();
    let mut mesh = plane(&mut gal);
    let (_texture, view) = upload_white(&gal).unwrap();
    let target = OffscreenTarget::new(&gal, 64, 64).unwrap();
    let mut material = LitMaterial::textured(&view);
    material.shading = ShadingModel::Diffuse;
    let draws = [LitDraw3D {
        mesh: &mesh,
        model: Mat4::IDENTITY,
        material: material.clone(),
    }];
    let make_light = |color| Light::Directional {
        direction: [0., 0., -1.],
        color,
        intensity: 1.,
    };
    assert!(
        renderer
            .prepare(
                &gal,
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
                &gal,
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
                    &gal,
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
    let extra = renderer
        .prepare(
            &gal,
            target.target(),
            camera(),
            Lighting::default(),
            &draws,
            PassOptions::default(),
        )
        .unwrap();
    drop(extra);
    // Submit in reverse preparation order: light upload slots must stay isolated.
    for expected in [[0, 0, 255, 255], [0, 255, 0, 255], [255, 0, 0, 255]] {
        let frame = prepared.pop().unwrap();
        let image = target
            .render(&gal, |_, _| Ok(vec![frame.commands]))
            .unwrap();
        assert_eq!(center(&image.rgba), expected);
    }
    // Repeated preparations remain valid after earlier frames complete.

    for _ in 0..5 {
        target
            .render(&gal, |gal, target| {
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

    renderer.release(&gal).unwrap();
    mesh.release(&gal).unwrap();
    target.release(&gal).unwrap();

    gal.wait().unwrap();
}

#[test]
#[ignore = "requires Vulkan"]
fn color_encoding_custom_shaders_and_normal_transforms_are_explicit() {
    let gal = Gpu::headless().unwrap();
    let mut renderer = LitRenderState3D::new(&gal, 1).unwrap();
    let vertices =
        [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|position| Vertex {
            position,
            normal: [1., 0., 1.],
            uv: [0.5; 2],
        });
    let mut mesh = Mesh::upload(&gal, &vertices, &[0, 1, 2, 0, 2, 3]).unwrap();
    let (_texture, view) = upload_white(&gal).unwrap();
    let target = OffscreenTarget::new(&gal, 64, 64).unwrap();
    let mut material = LitMaterial::textured(&view);
    material.shading = ShadingModel::Diffuse;
    let result = target
        .render(&gal, |gal, target| {
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
                            material: material.clone(),
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
        .render(&gal, |gal, target| {
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
                            material: material.clone(),
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
                &gal,
                target.target(),
                camera(),
                Lighting {
                    output_color_space: ColorSpace::Srgb,
                    ..Lighting::default()
                },
                &[LitDraw3D {
                    mesh: &mesh,
                    model: Mat4::IDENTITY,
                    material: material.clone()
                }],
                PassOptions::default()
            )
            .is_err()
    );
    let (_gray_texture, gray_view) = upload_color(&gal, [128; 4]).unwrap();
    let mut gray = LitMaterial::textured(&gray_view);
    gray.shading = ShadingModel::Unlit;
    gray.texture_color_space = ColorSpace::Srgb;
    for (encoding, expected) in [(ColorSpace::Linear, 55u8), (ColorSpace::Srgb, 128u8)] {
        let result = target
            .render(&gal, |gal, target| {
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
                                material: gray.clone(),
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
    let source = "@fragment fn main()->@location(0) vec4<f32>{return vec4<f32>(0.,0.,1.,1.);}";
    let mut custom = LitRenderState3D::with_fragment_shader(&gal, 0, source).unwrap();
    material.base.transparent = false;
    let result = target
        .render(&gal, |gal, target| {
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
                            material: material.clone(),
                        }],
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap();
    assert_eq!(center(&result.rgba), [0, 0, 255, 255]);
    custom.release(&gal).unwrap();
    renderer.release(&gal).unwrap();
    mesh.release(&gal).unwrap();
    target.release(&gal).unwrap();

    gal.wait().unwrap();
}
