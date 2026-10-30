use quasar_frame::{Gpu, OffscreenTarget};
use quasar_render_3d::*;
use vulkanic_gal::{self as gpu, TextureFormat, TextureUsages};
fn white(gpu: &Gpu) -> gpu::TextureView {
    let texture = quasar_frame::texture_2d(
        &gpu.device,
        "test white",
        1,
        1,
        TextureFormat::Rgba8Unorm,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
    )
    .unwrap();
    gpu.queue.write_texture(
        texture.as_image_copy(),
        &[255; 4],
        gpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        texture.size(),
    );
    texture.create_view(&Default::default())
}
fn quad(gpu: &Gpu, half: f32, z: f32) -> Mesh {
    Mesh::upload(
        gpu,
        &[
            Vertex {
                position: [-half, -half, z],
                normal: [0., 0., 1.],
                uv: [0., 1.],
            },
            Vertex {
                position: [half, -half, z],
                normal: [0., 0., 1.],
                uv: [1., 1.],
            },
            Vertex {
                position: [half, half, z],
                normal: [0., 0., 1.],
                uv: [1., 0.],
            },
            Vertex {
                position: [-half, half, z],
                normal: [0., 0., 1.],
                uv: [0., 0.],
            },
        ],
        &[0, 1, 2, 0, 2, 3],
    )
    .unwrap()
}
fn camera() -> LitCamera3D {
    LitCamera3D {
        view_projection: Mat4::IDENTITY,
        position: [0., 0., 2.],
    }
}
fn center(image: &[u8]) -> [u8; 4] {
    image[(32 * 64 + 32) * 4..][..4].try_into().unwrap()
}
#[test]
#[ignore = "native GPU"]
fn native_lighting_normals_and_shadow_maps_change_expected_pixels() {
    let gpu = Gpu::headless().unwrap();
    let view = white(&gpu);
    let floor = quad(&gpu, 1., 0.5);
    let caster = quad(&gpu, 0.25, 0.2);
    let target = OffscreenTarget::new(&gpu, 64, 64).unwrap();
    let mut renderer = SurfaceRenderState3D::new(&gpu, 4, 2).unwrap();
    let mut shadow_renderer = ShadowRenderState3D::new(
        &gpu,
        ShadowConfig {
            resolution: 64,
            ..Default::default()
        },
    )
    .unwrap();
    let lights = [Light::Directional {
        direction: [0., 0., -1.],
        color: [1.; 3],
        intensity: 0.8,
    }];
    let mut material = SurfaceMaterial::from(LitMaterial::textured(&view));
    material.lit.shading = ShadingModel::Diffuse;
    let draw = |material| SurfaceDraw3D {
        mesh: &floor,
        model: Mat4::IDENTITY,
        material,
    };
    let lighting = Lighting {
        ambient: [0.05; 3],
        lights: &lights,
        output_color_space: ColorSpace::Linear,
    };
    let baseline = target
        .render(&gpu, |gpu, target| {
            Ok(vec![
                renderer
                    .prepare(
                        gpu,
                        target,
                        camera(),
                        lighting,
                        &[draw(material.clone())],
                        None,
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap()
        .rgba;
    assert!(center(&baseline)[0] > 200);
    let casters = [ShadowCaster {
        mesh: &caster,
        model: Mat4::IDENTITY,
        texture_view: view.clone(),
        alpha_cutoff: 0.,
        opacity: 1.,
    }];
    let views = [ShadowView {
        light_index: 0,
        view_projection: Mat4::IDENTITY,
        settings: ShadowSettings::default(),
        casters: &casters,
    }];
    let shadow = shadow_renderer.prepare(&gpu, &views).unwrap();
    let binding = shadow.binding().clone();
    let shaded = target
        .render(&gpu, |gpu, target| {
            Ok(vec![
                shadow.commands,
                renderer
                    .prepare(
                        gpu,
                        target,
                        camera(),
                        lighting,
                        &[draw(material.clone())],
                        Some(&binding),
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap()
        .rgba;
    assert!(
        center(&shaded)[0] < 40,
        "shadow center={:?}",
        center(&shaded)
    );
    assert_ne!(baseline, shaded);
    let normal = quasar_frame::texture_2d(
        &gpu.device,
        "tilted normal",
        1,
        1,
        TextureFormat::Rgba8Unorm,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
    )
    .unwrap();
    gpu.queue.write_texture(
        normal.as_image_copy(),
        &[255, 128, 128, 255],
        gpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        normal.size(),
    );
    material.normal_map = Some(NormalMap::new(&normal.create_view(&Default::default())));
    let normal_image = target
        .render(&gpu, |gpu, target| {
            Ok(vec![
                renderer
                    .prepare(
                        gpu,
                        target,
                        camera(),
                        lighting,
                        &[draw(material.clone())],
                        None,
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap()
        .rgba;
    assert!(
        center(&normal_image)[0] < 40,
        "normal center={:?}",
        center(&normal_image)
    );
}
#[test]
#[ignore = "native GPU"]
fn legacy_native_mesh_and_lit_instancing_have_matching_scalar_pixels() {
    let gpu = Gpu::headless().unwrap();
    let view = white(&gpu);
    let mesh = quad(&gpu, 0.25, 0.5);
    let target = OffscreenTarget::new(&gpu, 64, 64).unwrap();
    let mut scalar = LitRenderState3D::new(&gpu, 1).unwrap();
    let mut instanced = LitRenderState3D::with_submission(
        &gpu,
        1,
        SubmissionConfig {
            instancing: true,
            ..Default::default()
        },
    )
    .unwrap();
    let draws: Vec<_> = (0..4)
        .map(|i| {
            let mut material = LitMaterial::textured(&view);
            material.shading = ShadingModel::Unlit;
            material.base.tint = [i as f32 / 4., 0.5, 1., 1.];
            LitDraw3D {
                mesh: &mesh,
                model: Mat4::translation(-0.6 + i as f32 * 0.4, 0., 0.),
                material,
            }
        })
        .collect();
    let a = target
        .render(&gpu, |gpu, t| {
            Ok(vec![
                scalar
                    .prepare(
                        gpu,
                        t,
                        camera(),
                        Lighting::default(),
                        &draws,
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap()
        .rgba;
    let b = target
        .render(&gpu, |gpu, t| {
            Ok(vec![
                instanced
                    .prepare(
                        gpu,
                        t,
                        camera(),
                        Lighting::default(),
                        &draws,
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap()
        .rgba;
    assert_eq!(a, b);
    assert_eq!(scalar.work_stats().draws, 4);
    assert_eq!(instanced.work_stats().draws, 1);
    let mut legacy = RenderState3D::new(&gpu).unwrap();
    let image = target
        .render(&gpu, |gpu, t| {
            Ok(vec![
                legacy
                    .prepare(
                        gpu,
                        t,
                        Camera3D {
                            view_projection: Mat4::IDENTITY,
                            light_direction: [0., 0., 1.],
                        },
                        &[Draw3D {
                            mesh: &mesh,
                            model: Mat4::IDENTITY,
                            material: Material::textured(&view),
                        }],
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap()
        .rgba;
    assert_eq!(center(&image), [255; 4]);
}
