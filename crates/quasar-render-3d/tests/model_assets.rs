#![cfg(feature = "model-assets")]
use quasar_assets::model::*;
use quasar_frame::Gpu;
use quasar_frame::OffscreenTarget;
use quasar_render_3d::*;
fn model(color: [u8; 4], unlit: bool) -> Model3D {
    let vertices = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
        .into_iter()
        .zip([[0., 1.], [1., 1.], [1., 0.], [0., 0.]])
        .map(|(position, uv)| ModelVertex {
            position,
            normal: [0., 0., 1.],
            uv,
        })
        .collect();
    Model3D {
        primitives: vec![ModelPrimitive {
            name: "quad".into(),
            vertices,
            indices: vec![0, 1, 2, 0, 2, 3],
            material: 0,
        }],
        instances: vec![ModelInstance {
            node: 0,
            name: "part".into(),
            primitive: 0,
            transform: Mat4::IDENTITY.0,
        }],
        materials: vec![ModelMaterial {
            base_texture: Some(0),
            metallic: 0.,
            unlit,
            ..ModelMaterial::default()
        }],
        images: vec![ModelImage {
            width: 1,
            height: 1,
            rgba: color.to_vec(),
        }],
        textures: vec![ModelTexture {
            image: 0,
            sampler: ModelSampler::default(),
        }],
        ..Model3D::default()
    }
}
fn gal() -> Gpu {
    Gpu::headless().unwrap()
}
fn camera() -> LitCamera3D {
    LitCamera3D {
        position: [0., 0., 3.],
        view_projection: Mat4::orthographic(-1., 1., -1., 1., 0.1, 10.).unwrap()
            * Mat4::translation(0., 0., -3.),
    }
}
fn center(image: &[u8]) -> [u8; 4] {
    image[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4]
        .try_into()
        .unwrap()
}
#[test]
#[ignore = "requires GPU"]
fn uploaded_generations_keep_pending_commands_alive_and_reject_invalid_replacements() {
    let gal = gal();
    let data = model([255, 0, 0, 0], true);
    let mut old = UploadedModel::upload(&gal, &data, ModelUploadOptions::default()).unwrap();
    let mut renderer = SurfaceRenderState3D::with_submission(
        &gal,
        0,
        0,
        SubmissionConfig {
            instancing: true,
            ..SubmissionConfig::default()
        },
    )
    .unwrap();
    let target = OffscreenTarget::new(&gal, 64, 64).unwrap();
    old.bind_samplers(&gal, &mut renderer).unwrap();
    let old_commands = renderer
        .prepare(
            &gal,
            target.target(),
            camera(),
            Lighting::default(),
            &old.draws(Mat4::IDENTITY),
            None,
            PassOptions::default(),
        )
        .unwrap()
        .commands;
    old.release(&gal).unwrap();
    renderer.clear_bindings(&gal).unwrap();
    let mut bad = data.clone();
    bad.primitives[0].material = 99;
    assert!(UploadedModel::upload(&gal, &bad, ModelUploadOptions::default()).is_err());
    let mut data = data;
    data.images[0].rgba = vec![0, 255, 0, 255];
    let mut new = UploadedModel::upload(&gal, &data, ModelUploadOptions::default()).unwrap();
    new.bind_samplers(&gal, &mut renderer).unwrap();
    let new_commands = renderer
        .prepare(
            &gal,
            target.target(),
            camera(),
            Lighting::default(),
            &new.draws(Mat4::IDENTITY),
            None,
            PassOptions::default(),
        )
        .unwrap()
        .commands;
    let green = target.render(&gal, |_, _| Ok(vec![new_commands])).unwrap();
    assert_eq!(center(&green.rgba), [0, 255, 0, 255]);
    let red = target.render(&gal, |_, _| Ok(vec![old_commands])).unwrap();
    assert_eq!(center(&red.rgba), [255, 0, 0, 255]);
    old.release(&gal).unwrap();
    old.release(&gal).unwrap();
    assert!(old.bounds().is_none());
    assert!(old.draws(Mat4::IDENTITY).is_empty());
    old.release(&gal).unwrap();
    renderer.clear_bindings(&gal).unwrap();
    new.release(&gal).unwrap();
    renderer.release(&gal).unwrap();
    target.release(&gal).unwrap();

    gal.wait().unwrap();
}
#[test]
#[ignore = "requires GPU"]
fn uploaded_mips_and_sampler_overrides_preserve_linear_color_energy() {
    let gal = gal();
    let mut data = model([255; 4], true);
    data.images[0] = ModelImage {
        width: 8,
        height: 8,
        rgba: (0..64)
            .flat_map(|i| {
                if (i % 8 + i / 8) % 2 == 0 {
                    [0, 0, 0, 255]
                } else {
                    [255; 4]
                }
            })
            .collect(),
    };
    data.textures[0].sampler.mip = Some(TextureFilter::Nearest);
    let mut uploaded = UploadedModel::upload(&gal, &data, ModelUploadOptions::default()).unwrap();
    let mut renderer = SurfaceRenderState3D::new(&gal, 0, 0).unwrap();
    let target = OffscreenTarget::new(&gal, 64, 64).unwrap();
    uploaded.bind_samplers(&gal, &mut renderer).unwrap();
    let image = target
        .render(&gal, |gal, t| {
            Ok(vec![
                renderer
                    .prepare(
                        gal,
                        t,
                        camera(),
                        Lighting::default(),
                        &uploaded.draws(Mat4::scale(0.05, 0.05, 1.)),
                        None,
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap();
    let c = center(&image.rgba);
    assert!(
        (123..=132).contains(&c[0]),
        "color-correct mip should decode to half linear energy: {c:?}"
    );
    let view = uploaded.draws(Mat4::IDENTITY)[0]
        .material
        .lit
        .base
        .texture_view
        .clone();
    assert_eq!(view.texture().mip_level_count(), 4);
    renderer.release(&gal).unwrap();
    uploaded.release(&gal).unwrap();
    target.release(&gal).unwrap();

    gal.wait().unwrap();
}
#[test]
#[ignore = "requires GPU"]
fn face_policies_and_mirrored_transforms_match_authored_materials() {
    let mut gal = gal();
    let data = model([255; 4], false);
    let options = ModelUploadOptions {
        material_conversion: MaterialConversion::ClassicApproximation,
        ..ModelUploadOptions::default()
    };
    assert!(UploadedModel::upload(&gal, &data, ModelUploadOptions::default()).is_err());
    let mut single = UploadedModel::upload(&gal, &data, options).unwrap();
    let mut data = data;
    data.materials[0].double_sided = true;
    let mut double = UploadedModel::upload(&gal, &data, options).unwrap();
    let mut renderer = SurfaceRenderState3D::with_submission(
        &gal,
        1,
        0,
        SubmissionConfig {
            instancing: true,
            ..SubmissionConfig::default()
        },
    )
    .unwrap();
    let target = OffscreenTarget::new(&gal, 64, 64).unwrap();
    let lights = [Light::Directional {
        direction: [0., 0., -1.],
        color: [1.; 3],
        intensity: 1.,
    }];
    let render =
        |gal: &mut Gpu, renderer: &mut SurfaceRenderState3D, asset: &UploadedModel, root: Mat4| {
            asset.bind_samplers(gal, renderer).unwrap();
            target
                .render(gal, |gal, t| {
                    Ok(vec![
                        renderer
                            .prepare(
                                gal,
                                t,
                                camera(),
                                Lighting {
                                    ambient: [0.; 3],
                                    lights: &lights,
                                    ..Lighting::default()
                                },
                                &asset.draws(root),
                                None,
                                PassOptions::default(),
                            )?
                            .commands,
                    ])
                })
                .unwrap()
                .rgba
        };
    assert!(center(&render(&mut gal, &mut renderer, &single, Mat4::IDENTITY))[0] > 200);
    assert!(
        center(&render(
            &mut gal,
            &mut renderer,
            &single,
            Mat4::rotation_y(std::f32::consts::PI)
        ))[0]
            < 30
    );
    assert!(
        center(&render(
            &mut gal,
            &mut renderer,
            &double,
            Mat4::rotation_y(std::f32::consts::PI)
        ))[0]
            > 200
    );
    assert!(
        center(&render(
            &mut gal,
            &mut renderer,
            &single,
            Mat4::scale(-1., 1., 1.)
        ))[0]
            > 200
    );
    renderer.release(&gal).unwrap();
    single.release(&gal).unwrap();
    double.release(&gal).unwrap();
    target.release(&gal).unwrap();

    gal.wait().unwrap();
}
