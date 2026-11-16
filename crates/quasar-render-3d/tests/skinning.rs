//! Compute skinning: CPU reference, GPU parity, drawing and shadows from the
//! posed mesh, and the glTF -> animation -> GPU path.
mod support;

use quasar_frame::{Gpu, OffscreenTarget};
use quasar_render_3d::*;
use support::upload_color;

fn quad(offset: f32) -> Vec<Vertex> {
    [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
        .map(|[x, y, z]| Vertex {
            position: [x * 0.25 + offset, y * 0.25, z],
            normal: [0., 0., 1.],
            uv: [0.5; 2],
        })
        .to_vec()
}
const INDICES: [u32; 6] = [0, 1, 2, 0, 2, 3];

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
}

#[test]
fn cpu_reference_blends_joints_and_validates_input() {
    let vertices = quad(0.);
    let palette = [Mat4::IDENTITY, Mat4::translation(2., 0., 0.)];
    let half = vec![
        SkinInfluence {
            joints: [0, 1, 0, 0],
            weights: [1., 1., 0., 0.], // normalised to 0.5 / 0.5
        };
        4
    ];
    let out = skin_vertices(&vertices, &half, &palette).unwrap();
    assert!(close(out[0].position, [-0.25 + 1., -0.25, 0.]));
    let rotated = skin_vertices(
        &vertices,
        &[SkinInfluence::single(0); 4],
        &[Mat4::rotation_y(std::f32::consts::FRAC_PI_2)],
    )
    .unwrap();
    assert!(
        close(rotated[0].normal, [1., 0., 0.]),
        "{:?}",
        rotated[0].normal
    );
    assert!(skin_vertices(&vertices, &[SkinInfluence::single(3); 4], &palette).is_err());
    let zero = SkinInfluence {
        joints: [0; 4],
        weights: [0.; 4],
    };
    assert!(skin_vertices(&vertices, &[zero; 4], &palette).is_err());
    assert!(skin_vertices(&vertices, &half[..2], &palette).is_err());
}

#[test]
#[ignore = "requires Vulkan"]
fn gpu_skinning_matches_the_cpu_reference() {
    let gpu = Gpu::headless().unwrap();
    let vertices = quad(0.3);
    let influences = [
        SkinInfluence::single(0),
        SkinInfluence {
            joints: [0, 1, 0, 0],
            weights: [0.25, 0.75, 0., 0.],
        },
        SkinInfluence::single(1),
        SkinInfluence {
            joints: [1, 0, 0, 0],
            weights: [0.5, 0.5, 0., 0.],
        },
    ];
    let mut mesh = SkinnedMesh::upload(&gpu, &vertices, &influences, &INDICES, 2).unwrap();
    // Before skinning the posed mesh is the bind pose.
    let bind = mesh.read_vertices(&gpu).unwrap();
    assert!(close(bind[2].position, vertices[2].position));
    let palette = [
        Mat4::translation(0., 1., 0.) * Mat4::rotation_x(0.4),
        Mat4::translation(-1., 0., 2.) * Mat4::rotation_y(1.1) * Mat4::scale(1.5, 1.5, 1.5),
    ];
    let mut pass = SkinningPass::new(&gpu).unwrap();
    let commands = pass
        .prepare(&gpu, &mut [(&mut mesh, &palette[..])])
        .unwrap();
    gpu.queue.submit([commands]);
    let gpu_out = mesh.read_vertices(&gpu).unwrap();
    let cpu_out = skin_vertices(&vertices, &influences, &palette).unwrap();
    for (g, c) in gpu_out.iter().zip(&cpu_out) {
        assert!(close(g.position, c.position), "{g:?} vs {c:?}");
        assert!(close(g.normal, c.normal), "{g:?} vs {c:?}");
        assert_eq!(g.uv, c.uv);
    }
    assert!(
        pass.prepare(&gpu, &mut [(&mut mesh, &palette[..1])])
            .is_err()
    );
    assert!(SkinnedMesh::upload(&gpu, &vertices, &influences, &INDICES, 0).is_err());
    assert!(mesh.set_local_bounds([1.; 3], [0.; 3]).is_err());
    mesh.set_local_bounds([-5.; 3], [5.; 3]).unwrap();
    assert_eq!(mesh.mesh().local_bounds(), ([-5.; 3], [5.; 3]));
}

fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
    rgba[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4]
        .try_into()
        .unwrap()
}

#[test]
#[ignore = "requires Vulkan"]
fn posed_meshes_draw_and_cast_shadows_where_the_joints_put_them() {
    let gpu = Gpu::headless().unwrap();
    let (_texture, white) = upload_color(&gpu, [255; 4]).unwrap();
    let mut skinned =
        SkinnedMesh::upload(&gpu, &quad(0.), &[SkinInfluence::single(0); 4], &INDICES, 1).unwrap();
    let mut pass = SkinningPass::new(&gpu).unwrap();
    let mut lit = LitRenderState3D::new(&gpu, 1).unwrap();
    let target = OffscreenTarget::new(&gpu, 64, 64).unwrap();
    let camera = LitCamera3D {
        view_projection: Mat4::orthographic(-2., 2., -2., 2., 0.1, 10.).unwrap()
            * Mat4::translation(0., 0., -4.),
        position: [0., 0., 4.],
    };
    let mut material = LitMaterial::textured(&white);
    material.shading = ShadingModel::Unlit;
    let palette = [Mat4::translation(1., 0., 0.)];
    let image = target
        .render(&gpu, |gpu, target| {
            let skin = pass.prepare(gpu, &mut [(&mut skinned, &palette[..])])?;
            let draw = lit
                .prepare(
                    gpu,
                    target,
                    camera,
                    Lighting::default(),
                    &[LitDraw3D {
                        mesh: skinned.mesh(),
                        model: Mat4::IDENTITY,
                        material: material.clone(),
                    }],
                    PassOptions {
                        clear_color: Some([0., 0., 0., 1.]),
                        clear_depth: true,
                    },
                )?
                .commands;
            Ok(vec![skin, draw])
        })
        .unwrap();
    // Joint moved +1 world unit = +16 pixels; the bind position stays empty.
    assert!(
        pixel(&image.rgba, 48, 32)[0] > 250,
        "drawn at the posed spot"
    );
    assert_eq!(pixel(&image.rgba, 32, 32)[0], 0, "not at the bind pose");

    // The shadow pass reads the same posed vertices.
    let mut surfaces = SurfaceRenderState3D::new(&gpu, 1, 1).unwrap();
    let mut shadows = ShadowRenderState3D::new(
        &gpu,
        ShadowConfig {
            resolution: 64,
            max_maps: 1,
            ..ShadowConfig::default()
        },
    )
    .unwrap();
    let floor = MeshData::plane(2., 2., 1)
        .transformed(Mat4::rotation_x(std::f32::consts::FRAC_PI_2))
        .upload(&gpu)
        .unwrap();
    let mut diffuse = LitMaterial::textured(&white);
    diffuse.shading = ShadingModel::Diffuse;
    let lights = [Light::Directional {
        direction: [0., 0., -1.],
        color: [1.; 3],
        intensity: 1.,
    }];
    let image = target
        .render(&gpu, |gpu, target| {
            let skin = pass.prepare(gpu, &mut [(&mut skinned, &palette[..])])?;
            let casters = [ShadowCaster {
                mesh: skinned.mesh(),
                model: Mat4::translation(0., 0., 1.),
                texture_view: white.clone(),
                alpha_cutoff: 0.,
                opacity: 1.,
            }];
            let maps = shadows.prepare(
                gpu,
                &[ShadowView {
                    light_index: 0,
                    view_projection: camera.view_projection,
                    settings: ShadowSettings {
                        strength: 1.,
                        normal_bias: 0.,
                        filter: ShadowFilter::Hard,
                        ..ShadowSettings::default()
                    },
                    casters: &casters,
                }],
            )?;
            let receiver = surfaces.prepare(
                gpu,
                target,
                camera,
                Lighting {
                    ambient: [0.; 3],
                    lights: &lights,
                    ..Lighting::default()
                },
                &[SurfaceDraw3D {
                    mesh: &floor,
                    model: Mat4::IDENTITY,
                    material: diffuse.clone().into(),
                }],
                Some(maps.binding()),
                PassOptions::default(),
            )?;
            Ok(vec![skin, maps.commands, receiver.commands])
        })
        .unwrap();
    let shadowed = pixel(&image.rgba, 48, 32)[0];
    let lit_spot = pixel(&image.rgba, 32, 32)[0];
    assert!(
        shadowed < 30 && lit_spot > 200,
        "shadow follows pose: {shadowed} {lit_spot}"
    );
}

#[cfg(feature = "model-assets")]
#[test]
#[ignore = "requires Vulkan"]
fn gltf_skins_animate_on_the_gpu() {
    use quasar_animation::model::{clip_from_model, skeleton_from_model};
    use quasar_assets::{AssetServer, GltfLoader, MemorySource};
    let mut source = MemorySource::new("fixture");
    source
        .insert(
            "bar.gltf",
            include_bytes!("../../quasar-assets/tests/fixtures/skinned_bar.gltf").to_vec(),
        )
        .unwrap();
    let mut server = AssetServer::new();
    server.mount("", std::sync::Arc::new(source)).unwrap();
    server.register(GltfLoader::animated());
    let model = server.load::<GltfLoader>("bar.gltf").unwrap();
    let (skeleton, binding) = skeleton_from_model(&model, 0).unwrap();
    let clip = clip_from_model(&model, 0).unwrap();
    let palette: Vec<Mat4> = binding
        .joint_matrices(&skeleton.sample(&clip.sample(0.5)))
        .into_iter()
        .map(Mat4)
        .collect();
    let gpu = Gpu::headless().unwrap();
    let primitive = &model.primitives[0];
    let mut mesh =
        SkinnedMesh::from_model_primitive(&gpu, primitive, model.skins[0].joints.len()).unwrap();
    let mut pass = SkinningPass::new(&gpu).unwrap();
    gpu.queue.submit([pass
        .prepare(&gpu, &mut [(&mut mesh, &palette[..])])
        .unwrap()]);
    let posed = mesh.read_vertices(&gpu).unwrap();
    let skin = primitive.skin.as_ref().unwrap();
    let influences: Vec<_> = skin
        .joints
        .iter()
        .zip(&skin.weights)
        .map(|(j, w)| SkinInfluence {
            joints: j.map(u32::from),
            weights: *w,
        })
        .collect();
    let vertices: Vec<_> = primitive
        .vertices
        .iter()
        .map(|v| Vertex {
            position: v.position,
            normal: v.normal,
            uv: v.uv,
        })
        .collect();
    let expected = skin_vertices(&vertices, &influences, &palette).unwrap();
    for (g, c) in posed.iter().zip(&expected) {
        assert!(close(g.position, c.position), "{g:?} vs {c:?}");
    }
    // Halfway through, the bar tip has bent and moved away from bind pose.
    let tip = posed
        .iter()
        .zip(&vertices)
        .map(|(p, v)| (p.position[0] - v.position[0]).abs() + (p.position[1] - v.position[1]).abs())
        .fold(0f32, f32::max);
    assert!(tip > 0.3, "{tip}");
}
